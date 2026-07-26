use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::NivError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    format: u64,
    edition: String,
    freeze_started: String,
    freeze_ends: String,
    freeze_ends_unix: u64,
    minimum_pilots: usize,
    minimum_pilot_days: u64,
    minimum_conformance_cases: usize,
    edition1_baseline_sha256: String,
    required_files: Vec<String>,
    tier_one_platforms: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pilot {
    format: u64,
    pilot_id: String,
    workload: String,
    toolchain: String,
    started_at: String,
    completed_at: String,
    duration_days: u64,
    outcome: String,
    critical_blockers: Vec<String>,
    evidence: String,
    approved_by: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Audit {
    pub blockers: Vec<String>,
    pub freeze_ends: String,
    pub pilots: usize,
    pub conformance_cases: usize,
}

pub fn audit(root: &Path, now: u64) -> Result<Audit, NivError> {
    let policy: Policy = read_json(&root.join("release/policy.json"), "release policy")?;
    if policy.format != 1 || policy.edition != "1" {
        return Err(release_error("unsupported release policy"));
    }
    let mut blockers = vec![];
    if now < policy.freeze_ends_unix {
        blockers.push(format!(
            "Edition 1 compatibility freeze does not end until {}",
            policy.freeze_ends
        ));
    }
    if policy.freeze_started >= policy.freeze_ends {
        blockers.push("compatibility freeze dates are invalid".into());
    }
    if crate::VERSION != "1.0.0" {
        blockers.push(format!(
            "toolchain version is {}, expected 1.0.0",
            crate::VERSION
        ));
    }
    for relative in &policy.required_files {
        if !root.join(relative).is_file() {
            blockers.push(format!("required release file is missing: {relative}"));
        }
    }

    let baseline_path = root.join("conformance/edition1-baseline.json");
    let baseline_bytes = fs::read(&baseline_path).map_err(|error| {
        release_error(format!(
            "cannot read conformance baseline '{}': {error}",
            baseline_path.display()
        ))
    })?;
    let baseline_digest = encode_hex(&Sha256::digest(&baseline_bytes));
    if baseline_digest != policy.edition1_baseline_sha256 {
        blockers.push("Edition 1 conformance baseline was modified".into());
    }
    let cases: serde_json::Value = serde_json::from_slice(&baseline_bytes)
        .map_err(|error| release_error(format!("invalid conformance baseline: {error}")))?;
    let conformance_cases = cases
        .as_array()
        .ok_or_else(|| release_error("conformance corpus must be a JSON array"))?
        .len();
    if conformance_cases < policy.minimum_conformance_cases {
        blockers.push(format!(
            "conformance corpus has {conformance_cases} cases; {} required",
            policy.minimum_conformance_cases
        ));
    }

    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .map_err(|error| release_error(format!("cannot read CI workflow: {error}")))?;
    for platform in &policy.tier_one_platforms {
        if !ci.contains(platform) {
            blockers.push(format!("tier-one CI platform is missing: {platform}"));
        }
    }

    let mut pilots = 0usize;
    let pilot_directory = root.join("release/pilots");
    for entry in fs::read_dir(&pilot_directory)
        .map_err(|error| release_error(format!("cannot read pilot evidence: {error}")))?
    {
        let entry = entry.map_err(|error| release_error(format!("cannot read pilot: {error}")))?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json")
            || path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".example.json"))
        {
            continue;
        }
        let pilot: Pilot = read_json(&path, "pilot evidence")?;
        if valid_pilot(&pilot, policy.minimum_pilot_days) {
            pilots += 1;
        } else {
            blockers.push(format!("pilot evidence is incomplete: {}", path.display()));
        }
    }
    if pilots < policy.minimum_pilots {
        blockers.push(format!(
            "only {pilots} qualifying production pilots; {} required",
            policy.minimum_pilots
        ));
    }
    Ok(Audit {
        blockers,
        freeze_ends: policy.freeze_ends,
        pilots,
        conformance_cases,
    })
}

fn valid_pilot(pilot: &Pilot, minimum_days: u64) -> bool {
    pilot.format == 1
        && !pilot.pilot_id.trim().is_empty()
        && !pilot.workload.trim().is_empty()
        && pilot.toolchain.starts_with("0.9.")
        && date_shape(&pilot.started_at)
        && date_shape(&pilot.completed_at)
        && pilot.duration_days >= minimum_days
        && pilot.outcome == "pass"
        && pilot.critical_blockers.is_empty()
        && !pilot.evidence.trim().is_empty()
        && !pilot.approved_by.trim().is_empty()
}

fn date_shape(value: &str) -> bool {
    value.len() == 10
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7) {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, label: &str) -> Result<T, NivError> {
    let bytes = fs::read(path).map_err(|error| {
        release_error(format!("cannot read {label} '{}': {error}", path.display()))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| release_error(format!("invalid {label} '{}': {error}", path.display())))
}

fn release_error(message: impl Into<String>) -> NivError {
    NivError::new(message, 1, 1)
}

#[cfg(test)]
mod tests {
    use super::audit;
    use std::path::Path;

    #[test]
    fn repository_release_policy_is_machine_checkable() {
        let audit = audit(Path::new(env!("CARGO_MANIFEST_DIR")), u64::MAX).unwrap();
        assert_eq!(audit.conformance_cases, 27);
        assert_eq!(audit.pilots, 0);
        assert!(
            audit
                .blockers
                .iter()
                .any(|blocker| blocker.contains("expected 1.0.0"))
        );
        assert!(
            audit
                .blockers
                .iter()
                .any(|blocker| blocker.contains("production pilots"))
        );
        assert!(
            audit
                .blockers
                .iter()
                .all(|blocker| !blocker.contains("compatibility freeze"))
        );
    }
}
