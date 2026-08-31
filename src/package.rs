use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::NivError;
use crate::project::{AUTHORITY_LOCKFILE_NAME, LOCKFILE_NAME, MANIFEST_NAME, Manifest};
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheEntry {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub bytes: u64,
    pub reachable: bool,
}

impl Package {
    pub fn build(manifest: &Manifest) -> Result<Self, NivError> {
        let mut files = BTreeMap::new();
        collect_sources(&manifest.root, &manifest.root, &mut files)?;
        let manifest_source = crate::source_io::read(manifest.root.join(MANIFEST_NAME))
            .map_err(|error| package_error(format!("cannot read manifest: {error}")))?;
        files.insert(MANIFEST_NAME.into(), manifest_source);
        files.insert(
            LOCKFILE_NAME.into(),
            installed_lockfile(manifest)?.into_bytes(),
        );
        files.insert(
            AUTHORITY_LOCKFILE_NAME.into(),
            installed_authority_lockfile(manifest)?.into_bytes(),
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
    let manifest_source = package
        .files
        .get(MANIFEST_NAME)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .ok_or_else(|| package_error("package manifest is not UTF-8"))?;
    let manifest = Manifest::parse(manifest_source, PathBuf::from("."))?;
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
        "yanked": false,
        "capabilities": manifest.capabilities,
        "capability_scopes": manifest.capability_scopes,
        "unsafe_modules": manifest.unsafe_modules,
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
    if metadata["yanked"].as_bool().unwrap_or(false) {
        return Err(package_error(format!(
            "package {name} {version} is yanked and cannot be newly installed"
        )));
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

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SearchResult {
    pub name: String,
    pub versions: Vec<String>,
}

pub fn search(query: &str, registry: &Path) -> Result<Vec<SearchResult>, NivError> {
    if query.len() > 64
        || !query
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(package_error(
            "registry search must use at most 64 ASCII letters, digits, '-' or '_'",
        ));
    }
    let index = registry.join("v1/index");
    let entries = match fs::read_dir(&index) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(package_error(format!("cannot search registry: {error}"))),
    };
    let query = query.to_ascii_lowercase();
    let mut results = vec![];
    for entry in entries {
        let entry =
            entry.map_err(|error| package_error(format!("cannot search registry: {error}")))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| package_error(format!("cannot inspect registry index: {error}")))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !name.to_ascii_lowercase().contains(&query) {
            continue;
        }
        let mut versions = vec![];
        for version in fs::read_dir(entry.path())
            .map_err(|error| package_error(format!("cannot read registry index: {error}")))?
        {
            let version = version
                .map_err(|error| package_error(format!("cannot read registry index: {error}")))?;
            let path = version.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
                && let Ok(document) = fs::read(&path)
                    .map_err(|error| {
                        package_error(format!("cannot read registry metadata: {error}"))
                    })
                    .and_then(|bytes| {
                        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| {
                            package_error(format!("invalid registry metadata: {error}"))
                        })
                    })
                && !document["yanked"].as_bool().unwrap_or(false)
                && let Some(version) = path.file_stem().and_then(|value| value.to_str())
                && validate_identity("search-result", version).is_ok()
            {
                versions.push(version.to_string());
            }
        }
        versions.sort_by_key(|version| std::cmp::Reverse(version_parts(version)));
        if !versions.is_empty() {
            results.push(SearchResult { name, versions });
        }
    }
    results.sort_by(|left, right| left.name.cmp(&right.name));
    results.truncate(100);
    Ok(results)
}

pub fn set_yanked(
    name: &str,
    version: &str,
    registry: &Path,
    yanked: bool,
) -> Result<(), NivError> {
    validate_identity(name, version)?;
    let metadata_path = registry
        .join("v1/index")
        .join(name)
        .join(format!("{version}.json"));
    reject_symlink_if_present(&metadata_path)?;
    let bytes = fs::read(&metadata_path)
        .map_err(|error| package_error(format!("cannot read registry metadata: {error}")))?;
    let mut metadata: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| package_error(format!("invalid registry metadata: {error}")))?;
    if metadata["format"] != 1 || metadata["name"] != name || metadata["version"] != version {
        return Err(package_error("registry metadata identity is invalid"));
    }
    metadata["yanked"] = serde_json::Value::Bool(yanked);
    let encoded = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| package_error(format!("cannot encode registry metadata: {error}")))?;
    write_atomic(&metadata_path, &encoded)
}

fn version_parts(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.').map(|part| part.parse().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
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
    guard_authority_lock(manifest)?;
    Ok(resolved.len())
}

pub fn install_offline_dependencies(manifest: &Manifest) -> Result<usize, NivError> {
    let lockfile = installed_lockfile(manifest)?;
    let count = lockfile.matches("[[dependency]]").count();
    write_atomic(&manifest.root.join(LOCKFILE_NAME), lockfile.as_bytes())?;
    guard_authority_lock(manifest)?;
    Ok(count)
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
    guard_authority_lock(manifest)?;
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

pub fn installed_authority_lockfile(manifest: &Manifest) -> Result<String, NivError> {
    installed_lockfile(manifest)?;
    let store = manifest.root.join(".niv/deps");
    let mut packages = BTreeMap::new();
    packages.insert(
        (manifest.name.clone(), manifest.version.clone()),
        ("root".to_string(), manifest.clone()),
    );
    let mut pending: Vec<(String, String)> = manifest
        .dependencies
        .iter()
        .map(|(name, version)| (name.clone(), version.clone()))
        .collect();
    while let Some((name, version)) = pending.pop() {
        if packages.contains_key(&(name.clone(), version.clone())) {
            continue;
        }
        let dependency = Manifest::load(&store.join(format!("{name}-{version}")))?;
        if dependency.name != name || dependency.version != version {
            return Err(package_error(format!(
                "installed package '{name}' has the wrong authority identity"
            )));
        }
        for (child_name, child_version) in &dependency.dependencies {
            pending.push((child_name.clone(), child_version.clone()));
        }
        packages.insert((name, version), ("dependency".to_string(), dependency));
    }

    let mut output =
        String::from("# This file is generated by Nivren. Review authority changes.\nformat = 1\n");
    for ((name, version), (source, package)) in packages {
        output.push_str("\n[[package]]\n");
        output.push_str(&format!(
            "name = {}\nversion = {}\nsource = {}\n",
            lock_string(&name),
            lock_string(&version),
            lock_string(&source)
        ));
        for capability in &package.capabilities {
            let scope = package
                .capability_scopes
                .get(capability)
                .map_or("allow", String::as_str);
            output.push_str("\n[[grant]]\n");
            output.push_str(&format!(
                "package = {}\nversion = {}\ncapability = {}\nscope = {}\n",
                lock_string(&name),
                lock_string(&version),
                lock_string(capability),
                lock_string(scope)
            ));
        }
        for module in &package.unsafe_modules {
            output.push_str("\n[[unsafe]]\n");
            output.push_str(&format!(
                "package = {}\nversion = {}\nmodule = {}\n",
                lock_string(&name),
                lock_string(&version),
                lock_string(module)
            ));
        }
    }
    Ok(output)
}

pub fn write_authority_lock(manifest: &Manifest) -> Result<(), NivError> {
    let contents = installed_authority_lockfile(manifest)?;
    write_atomic(
        &manifest.root.join(AUTHORITY_LOCKFILE_NAME),
        contents.as_bytes(),
    )
}

/// Refuses to change recorded authority silently. A first install writes the
/// initial lock; an unchanged lock passes; any difference stops the command
/// with a line diff until `niv authority lock` explicitly accepts it.
pub fn guard_authority_lock(manifest: &Manifest) -> Result<(), NivError> {
    let expected = installed_authority_lockfile(manifest)?;
    let path = manifest.root.join(AUTHORITY_LOCKFILE_NAME);
    match std::fs::read_to_string(&path) {
        Ok(actual) if actual == expected => Ok(()),
        Err(_) => write_atomic(&path, expected.as_bytes()),
        Ok(actual) => Err(package_error(format!(
            "dependency authority changed; review the difference and run 'niv authority lock' to accept it:\n{}",
            authority_diff(&actual, &expected)
        ))),
    }
}

/// Deterministic line diff between two authority locks: removed lines carry
/// '-', added lines carry '+'.
pub fn authority_diff(actual: &str, expected: &str) -> String {
    let old: std::collections::BTreeSet<&str> = actual.lines().collect();
    let new: std::collections::BTreeSet<&str> = expected.lines().collect();
    let mut output = String::new();
    for line in actual.lines() {
        if !new.contains(line) {
            output.push_str(&format!("  - {line}\n"));
        }
    }
    for line in expected.lines() {
        if !old.contains(line) {
            output.push_str(&format!("  + {line}\n"));
        }
    }
    if output.is_empty() {
        output.push_str("  (formatting-only difference)\n");
    }
    output
}

fn lock_string(value: &str) -> String {
    serde_json::to_string(value).expect("strings always serialize")
}

pub fn cache_entries(manifest: &Manifest) -> Result<Vec<CacheEntry>, NivError> {
    installed_lockfile(manifest)?;
    let store = manifest.root.join(".niv/deps");
    if !store.exists() {
        return Ok(Vec::new());
    }
    reject_symlink_if_present(&store)?;
    let reachable = reachable_dependencies(manifest)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&store)
        .map_err(|error| package_error(format!("cannot enumerate dependency cache: {error}")))?
    {
        let entry =
            entry.map_err(|error| package_error(format!("cannot read cache entry: {error}")))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| package_error(format!("cannot inspect cache entry: {error}")))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(package_error(format!(
                "dependency cache contains an unsafe entry: {}",
                path.display()
            )));
        }
        let archive = fs::read(path.join(".niv-package"))
            .map_err(|error| package_error(format!("cannot read cached package: {error}")))?;
        let package = Package::decode(&archive)?;
        let expected_directory = format!("{}-{}", package.name, package.version);
        if path.file_name().and_then(|name| name.to_str()) != Some(expected_directory.as_str()) {
            return Err(package_error(
                "cached package directory has the wrong identity",
            ));
        }
        let digest = sha256(&archive);
        let recorded = fs::read_to_string(path.join(".niv-package-sha256"))
            .map_err(|error| package_error(format!("cannot read cached checksum: {error}")))?;
        if recorded.trim() != digest || !installed_package_matches(&package, &path, &digest)? {
            return Err(package_error(format!(
                "cached package '{}' {} failed integrity verification",
                package.name, package.version
            )));
        }
        entries.push(CacheEntry {
            reachable: reachable.contains(&(package.name.clone(), package.version.clone())),
            name: package.name,
            version: package.version,
            sha256: digest,
            bytes: u64::try_from(archive.len())
                .map_err(|_| package_error("cached package size exceeds platform range"))?,
        });
        if entries.len() > 4096 {
            return Err(package_error("dependency cache exceeds 4096 packages"));
        }
    }
    entries.sort_by(|left, right| (&left.name, &left.version).cmp(&(&right.name, &right.version)));
    Ok(entries)
}

pub fn prune_cache(manifest: &Manifest) -> Result<(usize, u64), NivError> {
    let entries = cache_entries(manifest)?;
    let store = manifest.root.join(".niv/deps");
    let mut removed = 0usize;
    let mut bytes = 0u64;
    for entry in entries.into_iter().filter(|entry| !entry.reachable) {
        let directory = store.join(format!("{}-{}", entry.name, entry.version));
        reject_symlink_if_present(&directory)?;
        fs::remove_dir_all(&directory).map_err(|error| {
            package_error(format!(
                "cannot remove unreachable cache entry '{}': {error}",
                directory.display()
            ))
        })?;
        removed += 1;
        bytes = bytes.saturating_add(entry.bytes);
    }
    Ok((removed, bytes))
}

fn reachable_dependencies(manifest: &Manifest) -> Result<BTreeSet<(String, String)>, NivError> {
    let store = manifest.root.join(".niv/deps");
    let mut pending = manifest
        .dependencies
        .iter()
        .map(|(name, version)| (name.clone(), version.clone()))
        .collect::<Vec<_>>();
    let mut reachable = BTreeSet::new();
    while let Some((name, version)) = pending.pop() {
        if !reachable.insert((name.clone(), version.clone())) {
            continue;
        }
        let dependency = Manifest::load(&store.join(format!("{name}-{version}")))?;
        pending.extend(
            dependency
                .dependencies
                .iter()
                .map(|(name, version)| (name.clone(), version.clone())),
        );
    }
    Ok(reachable)
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
            let contents = crate::source_io::read(&child)
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
            "[package]\nname = \"library\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n\n[capabilities]\nNetwork = \"host:api.example.test;method:GET\"\n",
        )
        .unwrap();
        fs::write(
            dependency_root.join("main.niv"),
            "keep value = 42; expose { value };",
        )
        .unwrap();
        fs::write(
            app_root.join("niv.toml"),
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n\n[dependencies]\nlibrary = \"1.0.0\"\n",
        )
        .unwrap();
        fs::write(app_root.join("main.niv"), "use \"@library\"; library.value").unwrap();

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
        let authority = fs::read_to_string(app_root.join("niv.authority.lock")).unwrap();
        assert!(authority.contains("name = \"app\""));
        assert!(authority.contains("name = \"library\""));
        assert!(authority.contains("capability = \"Network\""));
        assert!(authority.contains("scope = \"host:api.example.test;method:GET\""));
        assert_eq!(
            authority,
            super::installed_authority_lockfile(&app).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
