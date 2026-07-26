use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::NivError;
use crate::project::{LOCKFILE_NAME, MANIFEST_NAME, Manifest};
use crate::trust::{
    Advisory, PublisherAuthorization, RegistryStatus, ReleaseProvenance, verify_release,
};

const MAGIC: &[u8; 4] = b"NIVP";
const FORMAT_VERSION: u16 = 1;
const MAX_FILES: usize = 4096;
const MAX_FILE_SIZE: usize = 16 * 1024 * 1024;
const MAX_PACKAGE_SIZE: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Package {
    pub fn build(manifest: &Manifest) -> Result<Self, NivError> {
        let mut files = BTreeMap::new();
        collect_sources(&manifest.root, &manifest.root, &mut files)?;
        let manifest_source = fs::read(manifest.root.join(MANIFEST_NAME))
            .map_err(|error| package_error(format!("cannot read manifest: {error}")))?;
        files.insert(MANIFEST_NAME.into(), manifest_source);
        files.insert(
            LOCKFILE_NAME.into(),
            installed_lockfile(manifest)?.into_bytes(),
        );
        validate_files(&files)?;
        Ok(Self {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            files,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, NivError> {
        validate_identity(&self.name, &self.version)?;
        validate_files(&self.files)?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        write_string(&mut bytes, &self.name)?;
        write_string(&mut bytes, &self.version)?;
        write_u32(&mut bytes, self.files.len())?;
        for (path, contents) in &self.files {
            write_string(&mut bytes, path)?;
            write_u64(&mut bytes, contents.len())?;
            bytes.extend_from_slice(contents);
        }
        if bytes.len() > MAX_PACKAGE_SIZE {
            return Err(package_error("package exceeds 64 MiB"));
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, NivError> {
        if bytes.len() > MAX_PACKAGE_SIZE {
            return Err(package_error("package exceeds 64 MiB"));
        }
        let mut reader = Reader { bytes, offset: 0 };
        if reader.take(4)? != MAGIC {
            return Err(package_error("invalid package magic"));
        }
        let version = reader.u16()?;
        if version != FORMAT_VERSION {
            return Err(package_error(format!(
                "unsupported package format {version}"
            )));
        }
        let name = reader.string()?;
        let version = reader.string()?;
        validate_identity(&name, &version)?;
        let count = reader.u32()? as usize;
        if count == 0 || count > MAX_FILES {
            return Err(package_error("invalid package file count"));
        }
        let mut files = BTreeMap::new();
        for _ in 0..count {
            let path = reader.string()?;
            validate_path(&path)?;
            let length = usize::try_from(reader.u64()?)
                .map_err(|_| package_error("package file length exceeds platform range"))?;
            if length > MAX_FILE_SIZE {
                return Err(package_error("package file exceeds 16 MiB"));
            }
            let contents = reader.take(length)?.to_vec();
            if files.insert(path, contents).is_some() {
                return Err(package_error("duplicate package path"));
            }
        }
        if reader.offset != bytes.len() {
            return Err(package_error("trailing package data"));
        }
        validate_files(&files)?;
        let manifest_bytes = files
            .get(MANIFEST_NAME)
            .ok_or_else(|| package_error("package has no niv.toml"))?;
        let manifest_source = std::str::from_utf8(manifest_bytes)
            .map_err(|_| package_error("package manifest is not UTF-8"))?;
        let manifest = Manifest::parse(manifest_source, PathBuf::from("."))?;
        if manifest.name != name || manifest.version != version {
            return Err(package_error(
                "package identity does not match its manifest",
            ));
        }
        Ok(Self {
            name,
            version,
            files,
        })
    }

    pub fn extract(&self, destination: &Path) -> Result<(), NivError> {
        if destination.exists() {
            return Err(package_error(format!(
                "destination already exists: {}",
                destination.display()
            )));
        }
        let parent = destination.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|error| package_error(format!("cannot create destination parent: {error}")))?;
        let temporary = parent.join(format!(
            ".nivren-extract-{}-{}",
            std::process::id(),
            self.name
        ));
        if temporary.exists() {
            return Err(package_error("temporary extraction path already exists"));
        }
        fs::create_dir(&temporary)
            .map_err(|error| package_error(format!("cannot create extraction sandbox: {error}")))?;
        let result = (|| {
            for (path, contents) in &self.files {
                let output = temporary.join(path);
                if let Some(parent) = output.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        package_error(format!("cannot create package directory: {error}"))
                    })?;
                }
                fs::write(&output, contents).map_err(|error| {
                    package_error(format!("cannot extract {}: {error}", output.display()))
                })?;
            }
            fs::rename(&temporary, destination)
                .map_err(|error| package_error(format!("cannot commit extraction: {error}")))
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&temporary);
        }
        result
    }
}

pub fn publish(package_bytes: &[u8], registry: &Path) -> Result<PathBuf, NivError> {
    let package = Package::decode(package_bytes)?;
    let digest = sha256(package_bytes);
    let packages = registry.join("v1/packages").join(&package.name);
    let index = registry.join("v1/index").join(&package.name);
    fs::create_dir_all(&packages)
        .and_then(|_| fs::create_dir_all(&index))
        .map_err(|error| package_error(format!("cannot create registry: {error}")))?;
    let artifact = packages.join(format!("{}.nivpkg", package.version));
    if artifact.exists() {
        let existing = fs::read(&artifact)
            .map_err(|error| package_error(format!("cannot read registry artifact: {error}")))?;
        if sha256(&existing) != digest {
            return Err(package_error(
                "registry version already exists with different content",
            ));
        }
    } else {
        write_atomic(&artifact, package_bytes)?;
    }
    let metadata = json!({
        "format": 1,
        "name": package.name,
        "version": package.version,
        "sha256": digest,
        "size": package_bytes.len(),
    });
    let metadata = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| package_error(format!("cannot encode registry metadata: {error}")))?;
    let metadata_path = index.join(format!("{}.json", package.version));
    if metadata_path.exists() {
        let existing = fs::read(&metadata_path)
            .map_err(|error| package_error(format!("cannot read registry metadata: {error}")))?;
        if existing != metadata {
            return Err(package_error(
                "registry metadata already exists with different content",
            ));
        }
    } else {
        write_atomic(&metadata_path, &metadata)?;
    }
    Ok(artifact)
}

pub fn fetch(name: &str, version: &str, registry: &Path) -> Result<Vec<u8>, NivError> {
    validate_identity(name, version)?;
    let metadata_path = registry
        .join("v1/index")
        .join(name)
        .join(format!("{version}.json"));
    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(&metadata_path)
            .map_err(|error| package_error(format!("cannot read registry metadata: {error}")))?,
    )
    .map_err(|error| package_error(format!("invalid registry metadata: {error}")))?;
    if metadata["format"] != 1 || metadata["name"] != name || metadata["version"] != version {
        return Err(package_error("registry metadata identity is invalid"));
    }
    let expected = metadata["sha256"]
        .as_str()
        .ok_or_else(|| package_error("registry metadata has no checksum"))?;
    let expected_size = metadata["size"]
        .as_u64()
        .and_then(|size| usize::try_from(size).ok())
        .ok_or_else(|| package_error("registry metadata has an invalid size"))?;
    if expected_size > MAX_PACKAGE_SIZE {
        return Err(package_error("registry package exceeds size limit"));
    }
    let artifact = registry
        .join("v1/packages")
        .join(name)
        .join(format!("{version}.nivpkg"));
    let bytes = fs::read(&artifact)
        .map_err(|error| package_error(format!("cannot read registry artifact: {error}")))?;
    if bytes.len() != expected_size {
        return Err(package_error("registry artifact size mismatch"));
    }
    if sha256(&bytes) != expected {
        return Err(package_error("registry artifact checksum mismatch"));
    }
    let package = Package::decode(&bytes)?;
    if package.name != name || package.version != version {
        return Err(package_error("registry artifact identity mismatch"));
    }
    Ok(bytes)
}

pub fn install_dependencies(manifest: &Manifest, registry: &Path) -> Result<usize, NivError> {
    let state = manifest.root.join(".niv");
    reject_symlink_if_present(&state)?;
    let dependencies = state.join("deps");
    reject_symlink_if_present(&dependencies)?;
    fs::create_dir_all(&dependencies)
        .map_err(|error| package_error(format!("cannot create dependency store: {error}")))?;

    let mut pending: Vec<(String, String)> = manifest
        .dependencies
        .iter()
        .map(|(name, version)| (name.clone(), version.clone()))
        .collect();
    let mut resolved = BTreeMap::new();
    while let Some((name, version)) = pending.pop() {
        if resolved.contains_key(&(name.clone(), version.clone())) {
            continue;
        }
        let bytes = fetch(&name, &version, registry)?;
        let digest = sha256(&bytes);
        let package = Package::decode(&bytes)?;
        let embedded = package
            .files
            .get(MANIFEST_NAME)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .ok_or_else(|| package_error("dependency manifest is not UTF-8"))?;
        let destination = dependencies.join(format!("{name}-{version}"));
        let dependency_manifest = Manifest::parse(embedded, destination.clone())?;
        for (child_name, child_version) in &dependency_manifest.dependencies {
            pending.push((child_name.clone(), child_version.clone()));
        }

        install_package(&package, &destination, &digest, &bytes)?;
        resolved.insert((name, version), digest);
    }

    let lockfile = manifest.resolved_lockfile(&resolved);
    write_atomic(&manifest.root.join(LOCKFILE_NAME), lockfile.as_bytes())?;
    Ok(resolved.len())
}

pub fn install_trusted_dependencies(
    manifest: &Manifest,
    registry: &str,
    root_public_key: [u8; 32],
) -> Result<usize, NivError> {
    if !registry.starts_with("https://") {
        return Err(package_error("trusted registries must use https://"));
    }
    let base = registry.trim_end_matches('/');
    install_trusted_with(manifest, root_public_key, |path, maximum| {
        remote_get(base, path, maximum)
    })
}

fn install_trusted_with(
    manifest: &Manifest,
    root_public_key: [u8; 32],
    mut fetch: impl FnMut(&str, usize) -> Result<Vec<u8>, NivError>,
) -> Result<usize, NivError> {
    let root = fetch("v1/trust/root.pub", 4096)?;
    let advertised = crate::trust::parse_public_key(
        std::str::from_utf8(&root).map_err(|_| package_error("registry root key is not UTF-8"))?,
    )?;
    if advertised != root_public_key {
        return Err(package_error(
            "registry root key does not match the trusted key",
        ));
    }
    let status: RegistryStatus = remote_json_with(&mut fetch, "v1/trust/status.json")?;
    let advisories: Vec<Advisory> = remote_json_with(&mut fetch, "v1/trust/advisories.json")?;
    let state = manifest.root.join(".niv");
    reject_symlink_if_present(&state)?;
    let dependencies = state.join("deps");
    reject_symlink_if_present(&dependencies)?;
    fs::create_dir_all(&dependencies)
        .map_err(|error| package_error(format!("cannot create dependency store: {error}")))?;
    let generation_path = state.join("registry-generation");
    let persisted_generation = fs::read_to_string(&generation_path)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| package_error("system clock is before Unix epoch"))?
        .as_secs();

    let mut pending: Vec<(String, String)> = manifest
        .dependencies
        .iter()
        .map(|(name, version)| (name.clone(), version.clone()))
        .collect();
    let mut resolved = BTreeMap::new();
    while let Some((name, version)) = pending.pop() {
        if resolved.contains_key(&(name.clone(), version.clone())) {
            continue;
        }
        let package_bytes = fetch(
            &format!("v1/packages/{name}/{version}.nivpkg"),
            MAX_PACKAGE_SIZE,
        )?;
        let provenance: ReleaseProvenance =
            remote_json_with(&mut fetch, &format!("v1/provenance/{name}/{version}.json"))?;
        if provenance.publisher.is_empty()
            || provenance.publisher.len() > 128
            || !provenance
                .publisher
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(package_error(
                "provenance publisher is not a safe identifier",
            ));
        }
        let authorization: PublisherAuthorization = remote_json_with(
            &mut fetch,
            &format!("v1/authorizations/{}.json", provenance.publisher),
        )?;
        let package = verify_release(
            &package_bytes,
            &provenance,
            &authorization,
            &status,
            &advisories,
            root_public_key,
            now,
            persisted_generation,
        )?;
        if package.name != name || package.version != version {
            return Err(package_error("registry returned the wrong dependency"));
        }
        let digest = sha256(&package_bytes);
        let embedded = package
            .files
            .get(MANIFEST_NAME)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .ok_or_else(|| package_error("dependency manifest is not UTF-8"))?;
        let destination = dependencies.join(format!("{name}-{version}"));
        let dependency_manifest = Manifest::parse(embedded, destination.clone())?;
        for (child_name, child_version) in &dependency_manifest.dependencies {
            pending.push((child_name.clone(), child_version.clone()));
        }
        install_package(&package, &destination, &digest, &package_bytes)?;
        resolved.insert((name, version), digest);
    }
    write_atomic(
        &manifest.root.join(LOCKFILE_NAME),
        manifest.resolved_lockfile(&resolved).as_bytes(),
    )?;
    write_atomic(
        &generation_path,
        format!("{}\n", status.generation).as_bytes(),
    )?;
    Ok(resolved.len())
}

fn remote_get(base: &str, path: &str, maximum: usize) -> Result<Vec<u8>, NivError> {
    crate::runtime::http_get_binary(&format!("{base}/{path}"), Duration::from_secs(30), maximum)
        .map_err(|error| package_error(format!("registry request failed: {error}")))
}

fn remote_json_with<T: serde::de::DeserializeOwned>(
    fetch: &mut impl FnMut(&str, usize) -> Result<Vec<u8>, NivError>,
    path: &str,
) -> Result<T, NivError> {
    serde_json::from_slice(&fetch(path, 1024 * 1024)?)
        .map_err(|error| package_error(format!("invalid registry document '{path}': {error}")))
}

fn install_package(
    package: &Package,
    destination: &Path,
    digest: &str,
    archive: &[u8],
) -> Result<(), NivError> {
    reject_symlink_if_present(destination)?;
    if destination.exists() && installed_package_matches(package, destination, digest)? {
        return Ok(());
    }

    let parent = destination
        .parent()
        .ok_or_else(|| package_error("dependency destination has no parent"))?;
    let staging = parent.join(format!(
        ".install-{}-{}-{}",
        package.name,
        package.version,
        std::process::id()
    ));
    if staging.exists() {
        return Err(package_error("dependency staging path already exists"));
    }
    package.extract(&staging)?;
    fs::write(staging.join(".niv-package-sha256"), format!("{digest}\n"))
        .map_err(|error| package_error(format!("cannot write dependency checksum: {error}")))?;
    fs::write(staging.join(".niv-package"), archive)
        .map_err(|error| package_error(format!("cannot cache dependency archive: {error}")))?;

    if !destination.exists() {
        return fs::rename(&staging, destination)
            .map_err(|error| package_error(format!("cannot install dependency: {error}")));
    }

    let backup = parent.join(format!(
        ".backup-{}-{}-{}",
        package.name,
        package.version,
        std::process::id()
    ));
    fs::rename(destination, &backup)
        .map_err(|error| package_error(format!("cannot stage dependency replacement: {error}")))?;
    if let Err(error) = fs::rename(&staging, destination) {
        let _ = fs::rename(&backup, destination);
        let _ = fs::remove_dir_all(&staging);
        return Err(package_error(format!(
            "cannot commit dependency replacement: {error}"
        )));
    }
    fs::remove_dir_all(&backup)
        .map_err(|error| package_error(format!("cannot remove dependency backup: {error}")))?;
    Ok(())
}

fn installed_package_matches(
    package: &Package,
    destination: &Path,
    digest: &str,
) -> Result<bool, NivError> {
    if fs::read_to_string(destination.join(".niv-package-sha256"))
        .ok()
        .is_none_or(|value| value.trim() != digest)
    {
        return Ok(false);
    }
    if fs::read(destination.join(".niv-package"))
        .ok()
        .is_none_or(|archive| sha256(&archive) != digest)
    {
        return Ok(false);
    }
    for (path, expected) in &package.files {
        let actual = match fs::read(destination.join(path)) {
            Ok(actual) => actual,
            Err(_) => return Ok(false),
        };
        if &actual != expected {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn installed_lockfile(manifest: &Manifest) -> Result<String, NivError> {
    let store = manifest.root.join(".niv/deps");
    let mut pending: Vec<(String, String)> = manifest
        .dependencies
        .iter()
        .map(|(name, version)| (name.clone(), version.clone()))
        .collect();
    let mut resolved = BTreeMap::new();
    while let Some((name, version)) = pending.pop() {
        if resolved.contains_key(&(name.clone(), version.clone())) {
            continue;
        }
        let destination = store.join(format!("{name}-{version}"));
        reject_symlink_if_present(&destination)?;
        let archive = fs::read(destination.join(".niv-package")).map_err(|_| {
            package_error(format!(
                "package '{name}' {version} is not installed; run 'niv install'"
            ))
        })?;
        let digest = sha256(&archive);
        let recorded = fs::read_to_string(destination.join(".niv-package-sha256"))
            .map_err(|_| package_error(format!("package '{name}' has no checksum")))?;
        if recorded.trim() != digest {
            return Err(package_error(format!(
                "installed package '{name}' checksum does not match"
            )));
        }
        let package = Package::decode(&archive)?;
        if package.name != name || package.version != version {
            return Err(package_error(format!(
                "installed package '{name}' has the wrong identity"
            )));
        }
        if !installed_package_matches(&package, &destination, &digest)? {
            return Err(package_error(format!(
                "installed package '{name}' differs from its locked archive"
            )));
        }
        let dependency = Manifest::load(&destination)?;
        for (child_name, child_version) in &dependency.dependencies {
            pending.push((child_name.clone(), child_version.clone()));
        }
        resolved.insert((name, version), digest);
    }
    Ok(manifest.resolved_lockfile(&resolved))
}

fn reject_symlink_if_present(path: &Path) -> Result<(), NivError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(package_error(format!(
            "dependency path may not be a symlink: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(package_error(format!(
            "cannot inspect dependency path '{}': {error}",
            path.display()
        ))),
    }
}

fn collect_sources(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), NivError> {
    for entry in fs::read_dir(path)
        .map_err(|error| package_error(format!("cannot enumerate sources: {error}")))?
    {
        let entry = entry.map_err(|error| package_error(format!("cannot read entry: {error}")))?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child)
            .map_err(|error| package_error(format!("cannot inspect source: {error}")))?;
        if metadata.file_type().is_symlink() {
            return Err(package_error(format!(
                "package source may not be a symlink: {}",
                child.display()
            )));
        }
        if metadata.is_dir() {
            if child
                .file_name()
                .is_some_and(|name| name != "target" && name != ".git" && name != ".niv")
            {
                collect_sources(root, &child, files)?;
            }
        } else if child
            .extension()
            .is_some_and(|extension| extension == "niv")
        {
            let relative = child
                .strip_prefix(root)
                .map_err(|_| package_error("package source escaped its root"))?;
            let path = relative
                .to_str()
                .ok_or_else(|| package_error("package path is not UTF-8"))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            validate_path(&path)?;
            let contents = fs::read(&child)
                .map_err(|error| package_error(format!("cannot read source: {error}")))?;
            files.insert(path, contents);
        }
    }
    Ok(())
}

fn validate_files(files: &BTreeMap<String, Vec<u8>>) -> Result<(), NivError> {
    if files.is_empty() || files.len() > MAX_FILES {
        return Err(package_error("invalid package file count"));
    }
    let mut total = 0usize;
    for (path, contents) in files {
        validate_path(path)?;
        if contents.len() > MAX_FILE_SIZE {
            return Err(package_error("package file exceeds 16 MiB"));
        }
        total = total
            .checked_add(contents.len())
            .ok_or_else(|| package_error("package size overflow"))?;
    }
    if total > MAX_PACKAGE_SIZE {
        return Err(package_error("package contents exceed 64 MiB"));
    }
    Ok(())
}

fn validate_path(value: &str) -> Result<(), NivError> {
    if value.is_empty() || value.len() > 4096 || value.contains('\\') {
        return Err(package_error("invalid package path"));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(package_error(
            "package path must be normalized and relative",
        ));
    }
    Ok(())
}

fn validate_identity(name: &str, version: &str) -> Result<(), NivError> {
    let source =
        format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nentry = \"main.niv\"\n");
    Manifest::parse(&source, PathBuf::from(".")).map(|_| ())
}

fn sha256(bytes: &[u8]) -> String {
    encode_hex(&Sha256::digest(bytes))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), NivError> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, contents)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| package_error(format!("cannot write {}: {error}", path.display())))
}

fn write_string(bytes: &mut Vec<u8>, value: &str) -> Result<(), NivError> {
    write_u32(bytes, value.len())?;
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn write_u32(bytes: &mut Vec<u8>, value: usize) -> Result<(), NivError> {
    bytes.extend_from_slice(
        &u32::try_from(value)
            .map_err(|_| package_error("package value exceeds u32"))?
            .to_le_bytes(),
    );
    Ok(())
}

fn write_u64(bytes: &mut Vec<u8>, value: usize) -> Result<(), NivError> {
    bytes.extend_from_slice(
        &u64::try_from(value)
            .map_err(|_| package_error("package value exceeds u64"))?
            .to_le_bytes(),
    );
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], NivError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| package_error("package offset overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| package_error("truncated package"))?;
        self.offset = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, NivError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, NivError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, NivError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String, NivError> {
        let length = self.u32()? as usize;
        if length > 4096 {
            return Err(package_error("package string exceeds 4096 bytes"));
        }
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| package_error("package string is not UTF-8"))
    }
}

fn package_error(message: impl Into<String>) -> NivError {
    NivError::new(message, 1, 1)
}

#[cfg(test)]
mod tests {
    use super::install_trusted_with;
    use crate::project::Manifest;
    use crate::trust::{
        RegistryStatus, attest_release, authorize_publisher, public_key, sign_status,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn trusted_installer_verifies_the_complete_remote_flow() {
        let root = std::env::temp_dir().join(format!(
            "nivren-trusted-install-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let dependency_root = root.join("library");
        let app_root = root.join("app");
        fs::create_dir_all(&dependency_root).unwrap();
        fs::create_dir_all(&app_root).unwrap();
        fs::write(
            dependency_root.join("niv.toml"),
            "[package]\nname = \"library\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n",
        )
        .unwrap();
        fs::write(
            dependency_root.join("main.niv"),
            "let value = 42; export { value };",
        )
        .unwrap();
        fs::write(
            app_root.join("niv.toml"),
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n\n[dependencies]\nlibrary = \"1.0.0\"\n",
        )
        .unwrap();
        fs::write(
            app_root.join("main.niv"),
            "import \"@library\"; library.value",
        )
        .unwrap();

        let dependency = Manifest::load(&dependency_root).unwrap();
        let package = super::Package::build(&dependency)
            .unwrap()
            .encode()
            .unwrap();
        let root_secret = [7; 32];
        let publisher_secret = [9; 32];
        let root_key = public_key(root_secret);
        let publisher_key = super::encode_hex(&public_key(publisher_secret));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let authorization = authorize_publisher(
            root_secret,
            "publisher".into(),
            publisher_key,
            "owner/repository".into(),
            ".github/workflows/release.yml".into(),
            now + 3600,
        )
        .unwrap();
        let provenance = attest_release(
            publisher_secret,
            &package,
            "publisher".into(),
            "owner/repository".into(),
            ".github/workflows/release.yml".into(),
            "abcdef0123456789".into(),
            now,
        )
        .unwrap();
        let status = sign_status(
            root_secret,
            RegistryStatus {
                generation: 5,
                issued_at: now,
                expires_at: now + 3600,
                revoked_keys: BTreeSet::new(),
                frozen_packages: BTreeMap::new(),
                signature: String::new(),
            },
        );
        let mut responses = BTreeMap::from([
            (
                "v1/trust/root.pub".to_string(),
                format!("{}\n", super::encode_hex(&root_key)).into_bytes(),
            ),
            (
                "v1/trust/status.json".to_string(),
                serde_json::to_vec(&status).unwrap(),
            ),
            ("v1/trust/advisories.json".to_string(), b"[]".to_vec()),
            ("v1/packages/library/1.0.0.nivpkg".to_string(), package),
            (
                "v1/provenance/library/1.0.0.json".to_string(),
                serde_json::to_vec(&provenance).unwrap(),
            ),
            (
                "v1/authorizations/publisher.json".to_string(),
                serde_json::to_vec(&authorization).unwrap(),
            ),
        ]);
        let app = Manifest::load(&app_root).unwrap();
        let installed = install_trusted_with(&app, root_key, |path, maximum| {
            let bytes = responses
                .remove(path)
                .ok_or_else(|| super::package_error(format!("unexpected request: {path}")))?;
            if bytes.len() > maximum {
                return Err(super::package_error("test response exceeds limit"));
            }
            Ok(bytes)
        })
        .unwrap();
        assert_eq!(installed, 1);
        assert_eq!(
            fs::read_to_string(app_root.join(".niv/registry-generation"))
                .unwrap()
                .trim(),
            "5"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
