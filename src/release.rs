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
    release_track: String,
    expected_version: String,
    product_proof_complete: bool,
    completed_checkpoints: Vec<String>,
    minimum_conformance_cases: usize,
    baseline_path: String,
    baseline_sha256: String,
    required_files: Vec<String>,
    tier_one_platforms: Vec<String>,
    required_evidence: Vec<EvidenceRequirement>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceRequirement {
    gate: String,
    path: String,
    maximum_age_seconds: u64,
    independent: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    format: u64,
    gate: String,
    status: String,
    completed_at_unix: u64,
    run_id: String,
    independent: bool,
    artifacts: Vec<EvidenceArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceArtifact {
    name: String,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Audit {
    pub blockers: Vec<String>,
    pub edition: String,
    pub release_track: String,
    pub evidence_passed: usize,
    pub evidence_required: usize,
    pub conformance_cases: usize,
}

pub fn audit(root: &Path, now: u64) -> Result<Audit, NivError> {
    let policy: Policy = read_json(&root.join("release/policy.json"), "release policy")?;
    let supported = matches!(
        (
            policy.format,
            policy.edition.as_str(),
            policy.release_track.as_str()
        ),
        (3, "4", "beta") | (4, "5", "stable" | "beta")
    );
    if !supported {
        return Err(release_error("unsupported release policy"));
    }
    let mut blockers = vec![];
    if !policy.product_proof_complete {
        blockers.push(format!(
            "Edition {} Product Proof is not complete",
            policy.edition
        ));
    }
    for checkpoint in ["language", "intent", "compiler", "product"] {
        if !policy
            .completed_checkpoints
            .iter()
            .any(|completed| completed == checkpoint)
        {
            blockers.push(format!("checkpoint has not passed: {checkpoint}"));
        }
    }
    if crate::VERSION != policy.expected_version {
        blockers.push(format!(
            "toolchain version is {}, expected {}",
            crate::VERSION,
            policy.expected_version
        ));
    }
    for relative in &policy.required_files {
        if !root.join(relative).is_file() {
            blockers.push(format!("required release file is missing: {relative}"));
        }
    }

    let baseline_path = root.join(&policy.baseline_path);
    let baseline_bytes = fs::read(&baseline_path).map_err(|error| {
        release_error(format!(
            "cannot read conformance baseline '{}': {error}",
            baseline_path.display()
        ))
    })?;
    let baseline_digest = encode_hex(&Sha256::digest(&baseline_bytes));
    if baseline_digest != policy.baseline_sha256 {
        blockers.push(format!(
            "Edition {} conformance baseline was modified",
            policy.edition
        ));
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

    let mut evidence_passed = 0;
    for requirement in &policy.required_evidence {
        let path = root.join(&requirement.path);
        if !path.is_file() {
            blockers.push(format!(
                "required Product Proof evidence is missing: {} ({})",
                requirement.gate, requirement.path
            ));
            continue;
        }
        let evidence: Evidence = read_json(&path, "Product Proof evidence")?;
        let mut valid = true;
        if evidence.format != 1 || evidence.gate != requirement.gate || evidence.status != "pass" {
            blockers.push(format!(
                "Product Proof evidence did not pass: {}",
                requirement.gate
            ));
            valid = false;
        }
        if evidence.completed_at_unix > now
            || now.saturating_sub(evidence.completed_at_unix) > requirement.maximum_age_seconds
        {
            blockers.push(format!(
                "Product Proof evidence is stale: {}",
                requirement.gate
            ));
            valid = false;
        }
        if evidence.run_id.trim().is_empty() || evidence.artifacts.is_empty() {
            blockers.push(format!(
                "Product Proof evidence lacks a run or artifact: {}",
                requirement.gate
            ));
            valid = false;
        }
        if requirement.independent && !evidence.independent {
            blockers.push(format!(
                "Product Proof evidence must be independent: {}",
                requirement.gate
            ));
            valid = false;
        }
        for artifact in &evidence.artifacts {
            if artifact.name.trim().is_empty() || !valid_sha256(&artifact.sha256) {
                blockers.push(format!(
                    "Product Proof evidence has an invalid artifact: {}",
                    requirement.gate
                ));
                valid = false;
                break;
            }
        }
        if valid {
            evidence_passed += 1;
        }
    }

    Ok(Audit {
        blockers,
        edition: policy.edition,
        release_track: policy.release_track,
        evidence_passed,
        evidence_required: policy.required_evidence.len(),
        conformance_cases,
    })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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
    fn repository_stable_release_policy_is_machine_checkable() {
        // Evidence freshness is relative to the receipts committed in the
        // repository, so the audit clock is pinned just after they were
        // recorded rather than to the wall clock.
        let policy: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("release/evidence/platform-matrix.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let recorded = policy["completed_at_unix"].as_u64().unwrap();
        let audit = audit(Path::new(env!("CARGO_MANIFEST_DIR")), recorded + 3_600).unwrap();
        assert_eq!(audit.edition, "5");
        assert_eq!(audit.release_track, "stable");
        assert_eq!(audit.conformance_cases, 14);
        assert_eq!(audit.evidence_passed, 5);
        assert_eq!(audit.evidence_required, 5);
        assert_eq!(audit.blockers, Vec::<String>::new());
    }

    #[test]
    fn stale_or_missing_evidence_blocks_the_stable_gate() {
        let audit = audit(Path::new(env!("CARGO_MANIFEST_DIR")), 1_900_000_000).unwrap();
        assert!(
            audit
                .blockers
                .iter()
                .any(|blocker| blocker.contains("stale"))
        );
    }
}
