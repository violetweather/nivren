use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::NivError;

pub const MANIFEST_NAME: &str = "niv.toml";
pub const LOCKFILE_NAME: &str = "niv.lock";
pub const AUTHORITY_LOCKFILE_NAME: &str = "niv.authority.lock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub root: PathBuf,
    pub name: String,
    pub version: String,
    pub entry: PathBuf,
    pub dependencies: BTreeMap<String, String>,
    pub capabilities: BTreeSet<String>,
    pub capability_scopes: BTreeMap<String, String>,
    pub unsafe_modules: BTreeSet<String>,
    pub instruction_limit: Option<u64>,
    pub memory_limit: Option<u64>,
    /// Declared `payload_bytes` limit: the Edition 5 named override for the
    /// 16 MiB default payload cap, honored by interpreter-owned bounds.
    pub payload_limit: Option<u64>,
    /// The declared language edition (`edition = "5"` under `[package]`).
    /// Edition 4 remains the default; Edition 5 opts into the strict gates,
    /// including the trusted-module rule for the project's own scripts.
    pub edition: u8,
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
        let source = crate::source_io::read_to_string(&manifest_path).map_err(|error| {
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
        let mut capabilities = BTreeSet::new();
        let mut capability_scopes = BTreeMap::new();
        let mut unsafe_modules = BTreeSet::new();
        let mut instruction_limit = None;
        let mut memory_limit = None;
        let mut payload_limit = None;
        for (index, raw_line) in source.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_string();
                if !matches!(
                    section.as_str(),
                    "package" | "dependencies" | "capabilities" | "unsafe" | "limits"
                ) {
                    return Err(project_error(
                        format!("unknown manifest section [{section}]"),
                        line_number,
                    ));
                }
                continue;
            }
            if !matches!(
                section.as_str(),
                "package" | "dependencies" | "capabilities" | "unsafe" | "limits"
            ) {
                return Err(project_error(
                    "manifest values must be inside [package], [dependencies], [capabilities], [unsafe], or [limits]",
                    line_number,
                ));
            }
            let (key, raw_value) = line
                .split_once('=')
                .ok_or_else(|| project_error("expected a key = \"value\" pair", line_number))?;
            let key = key.trim();
            if section == "package" && !matches!(key, "name" | "version" | "entry" | "edition") {
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
            } else if section == "dependencies" {
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
            } else if section == "capabilities" {
                if !known_capability(key) {
                    return Err(project_error(
                        format!("unknown capability '{key}'"),
                        line_number,
                    ));
                }
                let valid_grant = value == "allow"
                    || matches!(key, "FileRead" | "FileWrite")
                        && value
                            .strip_prefix("path:")
                            .is_some_and(|scope| !scope.is_empty())
                    || key == "Network" && valid_composed_scope(&value, "host", &["method"])
                    || key == "Environment"
                        && (value
                            .strip_prefix("name:")
                            .is_some_and(|scope| !scope.is_empty())
                            || value
                                .strip_prefix("prefix:")
                                .is_some_and(|scope| !scope.is_empty()))
                    || key == "Process" && valid_composed_scope(&value, "command", &["arg0"])
                    || key == "Native"
                        && (value
                            .strip_prefix("path:")
                            .is_some_and(|scope| !scope.is_empty())
                            || value
                                .strip_prefix("kind:")
                                .is_some_and(|scope| !scope.is_empty()));
                if !valid_grant {
                    return Err(project_error(
                        format!(
                            "capability '{key}' must be \"allow\"{}",
                            match key {
                                "FileRead" | "FileWrite" =>
                                    " or a path scope such as \"path:./data\"",
                                "Network" =>
                                    " or a host/method scope such as \"host:api.example.com,*.cdn.example.com;method:GET,POST\"",
                                "Environment" =>
                                    " or a name/prefix scope such as \"name:HOME\" or \"prefix:NIVREN_\"",
                                "Process" =>
                                    " or a command/first-argument scope such as \"command:git;arg0:status\"",
                                "Native" =>
                                    " or a path/kind scope such as \"path:./native\" or \"kind:database\"",
                                _ => "",
                            }
                        ),
                        line_number,
                    ));
                }
                if !capabilities.insert(key.to_string()) {
                    return Err(project_error(
                        format!("duplicate capability '{key}'"),
                        line_number,
                    ));
                }
                if value != "allow" {
                    capability_scopes.insert(key.to_string(), value);
                }
            } else if section == "unsafe" {
                if !matches!(
                    key,
                    "memory"
                        | "layouts"
                        | "allocators"
                        | "atomics"
                        | "threads"
                        | "simd"
                        | "devices"
                        | "ffi"
                ) {
                    return Err(project_error(
                        format!("unknown unsafe module '{key}'"),
                        line_number,
                    ));
                }
                if value != "allow" {
                    return Err(project_error(
                        format!("unsafe module '{key}' must be explicitly set to \"allow\""),
                        line_number,
                    ));
                }
                if !unsafe_modules.insert(key.to_string()) {
                    return Err(project_error(
                        format!("duplicate unsafe module '{key}'"),
                        line_number,
                    ));
                }
            } else {
                if !matches!(key, "instructions" | "memory_bytes" | "payload_bytes") {
                    return Err(project_error(format!("unknown limit '{key}'"), line_number));
                }
                let parsed = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        project_error(format!("{key} must be a positive integer"), line_number)
                    })?;
                if key == "payload_bytes" && !(1_024..=268_435_456).contains(&parsed) {
                    return Err(project_error(
                        "payload_bytes must be from 1024 through 268435456",
                        line_number,
                    ));
                }
                let slot = if key == "instructions" {
                    &mut instruction_limit
                } else if key == "memory_bytes" {
                    &mut memory_limit
                } else {
                    &mut payload_limit
                };
                if slot.replace(parsed).is_some() {
                    return Err(project_error(
                        format!("duplicate limit '{key}'"),
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
        if !unsafe_modules.is_empty() && !capabilities.contains("Native") {
            return Err(project_error(
                "declared unsafe modules require an explicit Native capability grant",
                1,
            ));
        }
        let version = required(&values, "version")?;
        if !valid_version(&version) {
            return Err(project_error(
                "package version must have the form major.minor.patch",
                1,
            ));
        }
        let edition = match values.get("edition").map(String::as_str) {
            None | Some("4") => 4,
            Some("5") => 5,
            Some(other) => {
                return Err(project_error(
                    format!("unknown edition '{other}'; declare edition = \"4\" or \"5\""),
                    1,
                ));
            }
        };
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
            capabilities,
            capability_scopes,
            unsafe_modules,
            instruction_limit,
            memory_limit,
            payload_limit,
            edition,
        })
    }

    pub fn entry_path(&self) -> PathBuf {
        self.root.join(&self.entry)
    }

    pub fn add_dependency(&mut self, name: &str, version: &str) -> Result<(), NivError> {
        if !valid_dependency_name(name) {
            return Err(project_error(
                "dependency names must be valid Nivren identifiers",
                1,
            ));
        }
        if name == self.name {
            return Err(project_error("a package cannot depend on itself", 1));
        }
        if !valid_version(version) {
            return Err(project_error(
                format!("dependency '{name}' must use an exact major.minor.patch version"),
                1,
            ));
        }
        self.dependencies.insert(name.into(), version.into());
        Ok(())
    }

    pub fn source(&self) -> String {
        let mut output = format!(
            "[package]\nname = \"{}\"\nversion = \"{}\"\nentry = \"{}\"\n{}",
            self.name,
            self.version,
            self.entry.to_string_lossy(),
            if self.edition >= 5 {
                format!("edition = \"{}\"\n", self.edition)
            } else {
                String::new()
            }
        );
        if !self.dependencies.is_empty() {
            output.push_str("\n[dependencies]\n");
            for (name, version) in &self.dependencies {
                output.push_str(&format!("{name} = \"{version}\"\n"));
            }
        }
        if !self.capabilities.is_empty() {
            output.push_str("\n[capabilities]\n");
            for capability in &self.capabilities {
                let grant = self
                    .capability_scopes
                    .get(capability)
                    .map_or("allow", String::as_str);
                output.push_str(&format!("{capability} = \"{grant}\"\n"));
            }
        }
        if !self.unsafe_modules.is_empty() {
            output.push_str("\n[unsafe]\n");
            for module in &self.unsafe_modules {
                output.push_str(&format!("{module} = \"allow\"\n"));
            }
        }
        if self.instruction_limit.is_some()
            || self.memory_limit.is_some()
            || self.payload_limit.is_some()
        {
            output.push_str("\n[limits]\n");
            if let Some(instructions) = self.instruction_limit {
                output.push_str(&format!("instructions = \"{instructions}\"\n"));
            }
            if let Some(memory) = self.memory_limit {
                output.push_str(&format!("memory_bytes = \"{memory}\"\n"));
            }
            if let Some(payload) = self.payload_limit {
                output.push_str(&format!("payload_bytes = \"{payload}\"\n"));
            }
        }
        output
    }

    pub fn lockfile(&self) -> String {
        self.resolved_lockfile(&BTreeMap::new())
    }

    /// The capability scopes as the runtime must apply them: a relative
    /// `path:` scope is anchored to this manifest's directory, so a grant
    /// means the same directory no matter where `niv` was started. The
    /// manifest itself keeps the scopes as written, so published metadata
    /// and authority locks stay deterministic across machines.
    pub fn anchored_capability_scopes(&self) -> BTreeMap<String, String> {
        self.capability_scopes
            .iter()
            .map(|(capability, scope)| {
                let anchored = match scope.strip_prefix("path:") {
                    Some(path) if Path::new(path).is_relative() => {
                        format!("path:{}", self.root.join(path).display())
                    }
                    _ => scope.clone(),
                };
                (capability.clone(), anchored)
            })
            .collect()
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
        for capability in &self.capabilities {
            hash_part(&mut digest, capability.as_bytes());
            if let Some(scope) = self.capability_scopes.get(capability) {
                hash_part(&mut digest, scope.as_bytes());
            }
        }
        for module in &self.unsafe_modules {
            hash_part(&mut digest, b"unsafe");
            hash_part(&mut digest, module.as_bytes());
        }
        if let Some(instructions) = self.instruction_limit {
            hash_part(&mut digest, &instructions.to_le_bytes());
        }
        if let Some(memory) = self.memory_limit {
            hash_part(&mut digest, &memory.to_le_bytes());
        }
        for file in files {
            let relative = file
                .strip_prefix(&self.root)
                .map_err(|_| project_error("fingerprinted source escaped the project root", 1))?;
            let relative = relative
                .to_str()
                .ok_or_else(|| project_error("source path is not valid UTF-8", 1))?;
            let contents = crate::source_io::read(&file).map_err(|error| {
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

fn known_capability(name: &str) -> bool {
    matches!(
        name,
        "FileRead"
            | "FileWrite"
            | "Environment"
            | "Time"
            | "Process"
            | "Network"
            | "Task"
            | "Channel"
            | "Log"
            | "Random"
            | "Native"
    )
}

fn valid_composed_scope(value: &str, required: &str, optional: &[&str]) -> bool {
    let mut seen = BTreeSet::new();
    for clause in value.split(';') {
        let Some((kind, choices)) = clause.split_once(':') else {
            return false;
        };
        if (kind != required && !optional.contains(&kind))
            || !seen.insert(kind)
            || choices.is_empty()
            || choices.split(',').any(str::is_empty)
        {
            return false;
        }
    }
    seen.contains(required)
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
