use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::NivError;

pub const MANIFEST_NAME: &str = "niv.toml";
pub const LOCKFILE_NAME: &str = "niv.lock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub root: PathBuf,
    pub name: String,
    pub version: String,
    pub entry: PathBuf,
    pub dependencies: BTreeMap<String, String>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, NivError> {
        let manifest_path = if path.is_dir() {
            path.join(MANIFEST_NAME)
        } else {
            path.to_path_buf()
        };
        let root = manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|error| project_error(format!("cannot resolve project: {error}"), 1))?;
        let source = fs::read_to_string(&manifest_path).map_err(|error| {
            project_error(
                format!("cannot read {}: {error}", manifest_path.display()),
                1,
            )
        })?;
        Self::parse(&source, root)
    }

    pub fn parse(source: &str, root: PathBuf) -> Result<Self, NivError> {
        let mut section = String::new();
        let mut values = BTreeMap::new();
        let mut dependencies = BTreeMap::new();
        for (index, raw_line) in source.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_string();
                if !matches!(section.as_str(), "package" | "dependencies") {
                    return Err(project_error(
                        format!("unknown manifest section [{section}]"),
                        line_number,
                    ));
                }
                continue;
            }
            if !matches!(section.as_str(), "package" | "dependencies") {
                return Err(project_error(
                    "manifest values must be inside [package] or [dependencies]",
                    line_number,
                ));
            }
            let (key, raw_value) = line
                .split_once('=')
                .ok_or_else(|| project_error("expected a key = \"value\" pair", line_number))?;
            let key = key.trim();
            if section == "package" && !matches!(key, "name" | "version" | "entry") {
                return Err(project_error(
                    format!("unknown package key '{key}'"),
                    line_number,
                ));
            }
            let value = quoted(raw_value.trim()).ok_or_else(|| {
                project_error(format!("'{key}' must be a quoted string"), line_number)
            })?;
            if section == "package" {
                if values.insert(key.to_string(), value).is_some() {
                    return Err(project_error(
                        format!("duplicate package key '{key}'"),
                        line_number,
                    ));
                }
            } else {
                if !valid_dependency_name(key) {
                    return Err(project_error(
                        "dependency names must be valid Nivren identifiers",
                        line_number,
                    ));
                }
                if !valid_version(&value) {
                    return Err(project_error(
                        format!("dependency '{key}' must use an exact major.minor.patch version"),
                        line_number,
                    ));
                }
                if dependencies.insert(key.to_string(), value).is_some() {
                    return Err(project_error(
                        format!("duplicate dependency '{key}'"),
                        line_number,
                    ));
                }
            }
        }

        let name = required(&values, "name")?;
        if !valid_name(&name) {
            return Err(project_error(
                "package name must use lowercase ASCII letters, digits, '-' or '_'",
                1,
            ));
        }
        if dependencies.contains_key(&name) {
            return Err(project_error("a package cannot depend on itself", 1));
        }
        let version = required(&values, "version")?;
        if !valid_version(&version) {
            return Err(project_error(
                "package version must have the form major.minor.patch",
                1,
            ));
        }
        let entry = PathBuf::from(required(&values, "entry")?);
        if entry.is_absolute()
            || entry
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(project_error(
                "package entry must be a relative path within the project",
                1,
            ));
        }
        Ok(Self {
            root,
            name,
            version,
            entry,
            dependencies,
        })
    }

    pub fn entry_path(&self) -> PathBuf {
        self.root.join(&self.entry)
    }

    pub fn lockfile(&self) -> String {
        self.resolved_lockfile(&BTreeMap::new())
    }

    pub fn resolved_lockfile(&self, resolved: &BTreeMap<(String, String), String>) -> String {
        let mut output = format!(
            "# This file is generated by Nivren.\nformat = 1\n\n[[package]]\nname = \"{}\"\nversion = \"{}\"\n",
            self.name, self.version
        );
        for ((name, version), sha256) in resolved {
            output.push_str(&format!(
                "\n[[dependency]]\nname = \"{name}\"\nversion = \"{version}\"\nsha256 = \"{sha256}\"\n"
            ));
        }
        output
    }

    pub fn fingerprint(&self) -> Result<String, NivError> {
        let mut files = vec![];
        collect_sources(&self.root, &mut files)
            .map_err(|error| project_error(format!("cannot fingerprint project: {error}"), 1))?;
        files.sort();
        let mut digest = Sha256::new();
        hash_part(&mut digest, b"Nivren project fingerprint v1");
        hash_part(&mut digest, crate::VERSION.as_bytes());
        hash_part(&mut digest, self.name.as_bytes());
        hash_part(&mut digest, self.version.as_bytes());
        hash_part(&mut digest, self.entry.to_string_lossy().as_bytes());
        for (name, version) in &self.dependencies {
            hash_part(&mut digest, name.as_bytes());
            hash_part(&mut digest, version.as_bytes());
        }
        for file in files {
            let relative = file
                .strip_prefix(&self.root)
                .map_err(|_| project_error("fingerprinted source escaped the project root", 1))?;
            let relative = relative
                .to_str()
                .ok_or_else(|| project_error("source path is not valid UTF-8", 1))?;
            let contents = fs::read(&file).map_err(|error| {
                project_error(format!("cannot read {}: {error}", file.display()), 1)
            })?;
            hash_part(&mut digest, relative.as_bytes());
            hash_part(&mut digest, &contents);
        }
        let bytes = digest.finalize();
        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut output, "{byte:02x}").unwrap();
        }
        Ok(output)
    }
}

fn hash_part(digest: &mut Sha256, value: &[u8]) {
    digest.update(u64::try_from(value.len()).unwrap().to_le_bytes());
    digest.update(value);
}

fn collect_sources(path: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            if child
                .file_name()
                .is_some_and(|name| name != "target" && name != ".niv")
            {
                collect_sources(&child, files)?;
            }
        } else if child
            .extension()
            .is_some_and(|extension| extension == "niv")
        {
            files.push(child);
        }
    }
    Ok(())
}

fn quoted(value: &str) -> Option<String> {
    value
        .strip_prefix('"')?
        .strip_suffix('"')
        .filter(|inner| !inner.contains('"'))
        .map(str::to_string)
}

fn required(values: &BTreeMap<String, String>, key: &str) -> Result<String, NivError> {
    values
        .get(key)
        .cloned()
        .ok_or_else(|| project_error(format!("missing package key '{key}'"), 1))
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
}

fn valid_dependency_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn valid_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == &"0" || !part.starts_with('0'))
        })
}

fn project_error(message: impl Into<String>, line: usize) -> NivError {
    NivError::new(message, line, 1)
}
