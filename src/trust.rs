use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::NivError;
use crate::package::Package;

const ENVELOPE_MAGIC: &[u8; 4] = b"NIVE";
const MAX_DOCUMENT_SIZE: usize = 1024 * 1024;
const MAX_ENVELOPE_SIZE: usize = 66 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherAuthorization {
    pub publisher: String,
    pub public_key: String,
    pub repository: String,
    pub workflow: String,
    pub expires_at: u64,
    /// Package names this publisher may release: exact names, `prefix*`
    /// patterns, or `*` for every package. An empty set authorizes nothing.
    #[serde(default)]
    pub packages: BTreeSet<String>,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseProvenance {
    pub package: String,
    pub version: String,
    pub sha256: String,
    pub publisher: String,
    pub public_key: String,
    pub repository: String,
    pub workflow: String,
    pub commit: String,
    pub issued_at: u64,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryStatus {
    pub generation: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub revoked_keys: BTreeSet<String>,
    pub frozen_packages: BTreeMap<String, String>,
    /// SHA-256 of the canonical advisory list served beside this status, so a
    /// host cannot drop or trim advisories while serving a valid status.
    #[serde(default)]
    pub advisories_sha256: String,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Advisory {
    pub id: String,
    pub package: String,
    pub affected_versions: BTreeSet<String>,
    pub severity: String,
    pub summary: String,
    pub withdrawn: bool,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryAdminAction {
    pub format: u16,
    pub action: String,
    pub package: String,
    pub version: String,
    pub generation: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub reason: String,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishEnvelope {
    pub package: Vec<u8>,
    pub provenance: ReleaseProvenance,
    pub authorization: PublisherAuthorization,
}

impl PublishEnvelope {
    pub fn encode(&self) -> Result<Vec<u8>, NivError> {
        let provenance = serde_json::to_vec(&self.provenance)
            .map_err(|error| trust_error(format!("cannot encode provenance: {error}")))?;
        let authorization = serde_json::to_vec(&self.authorization)
            .map_err(|error| trust_error(format!("cannot encode authorization: {error}")))?;
        if provenance.len() > MAX_DOCUMENT_SIZE || authorization.len() > MAX_DOCUMENT_SIZE {
            return Err(trust_error("publish document exceeds 1 MiB"));
        }
        Package::decode(&self.package)?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(ENVELOPE_MAGIC);
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(
            &u32::try_from(provenance.len())
                .map_err(|_| trust_error("provenance length overflow"))?
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &u32::try_from(authorization.len())
                .map_err(|_| trust_error("authorization length overflow"))?
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &u64::try_from(self.package.len())
                .map_err(|_| trust_error("package length overflow"))?
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&provenance);
        bytes.extend_from_slice(&authorization);
        bytes.extend_from_slice(&self.package);
        if bytes.len() > MAX_ENVELOPE_SIZE {
            return Err(trust_error("publish envelope exceeds 66 MiB"));
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, NivError> {
        if bytes.len() > MAX_ENVELOPE_SIZE {
            return Err(trust_error("publish envelope exceeds 66 MiB"));
        }
        let mut reader = EnvelopeReader { bytes, offset: 0 };
        if reader.take(4)? != ENVELOPE_MAGIC {
            return Err(trust_error("invalid publish envelope magic"));
        }
        if reader.u16()? != 1 {
            return Err(trust_error("unsupported publish envelope version"));
        }
        let provenance_length = reader.length_u32()?;
        let authorization_length = reader.length_u32()?;
        let package_length = reader.length_u64()?;
        if provenance_length > MAX_DOCUMENT_SIZE || authorization_length > MAX_DOCUMENT_SIZE {
            return Err(trust_error("publish document exceeds 1 MiB"));
        }
        let provenance =
            serde_json::from_slice::<ReleaseProvenance>(reader.take(provenance_length)?)
                .map_err(|error| trust_error(format!("invalid provenance: {error}")))?;
        let authorization =
            serde_json::from_slice::<PublisherAuthorization>(reader.take(authorization_length)?)
                .map_err(|error| trust_error(format!("invalid authorization: {error}")))?;
        let package = reader.take(package_length)?.to_vec();
        if reader.offset != bytes.len() {
            return Err(trust_error("trailing publish envelope data"));
        }
        Package::decode(&package)?;
        Ok(Self {
            package,
            provenance,
            authorization,
        })
    }
}

pub fn authorize_publisher(
    root_secret: [u8; 32],
    publisher: String,
    public_key: String,
    repository: String,
    workflow: String,
    expires_at: u64,
    packages: BTreeSet<String>,
) -> Result<PublisherAuthorization, NivError> {
    decode_key(&public_key)?;
    if packages.is_empty()
        || packages
            .iter()
            .any(|pattern| !valid_package_pattern(pattern))
    {
        return Err(trust_error(
            "authorized package list must name packages, prefix* patterns, or *",
        ));
    }
    let mut authorization = PublisherAuthorization {
        publisher,
        public_key,
        repository,
        workflow,
        expires_at,
        packages,
        signature: String::new(),
    };
    authorization.signature = sign(&root_secret, &authorization_bytes(&authorization));
    Ok(authorization)
}

pub fn attest_release(
    publisher_secret: [u8; 32],
    package_bytes: &[u8],
    publisher: String,
    repository: String,
    workflow: String,
    commit: String,
    issued_at: u64,
) -> Result<ReleaseProvenance, NivError> {
    if commit.len() < 7
        || commit.len() > 128
        || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(trust_error(
            "commit must be a 7-128 character hexadecimal identifier",
        ));
    }
    let package = Package::decode(package_bytes)?;
    let key = SigningKey::from_bytes(&publisher_secret);
    let mut provenance = ReleaseProvenance {
        package: package.name,
        version: package.version,
        sha256: sha256(package_bytes),
        publisher,
        public_key: encode_hex(key.verifying_key().as_bytes()),
        repository,
        workflow,
        commit,
        issued_at,
        signature: String::new(),
    };
    provenance.signature = sign(&publisher_secret, &provenance_bytes(&provenance));
    Ok(provenance)
}

pub fn sign_status(root_secret: [u8; 32], mut status: RegistryStatus) -> RegistryStatus {
    status.signature.clear();
    status.signature = sign(&root_secret, &status_bytes(&status));
    status
}

pub fn sign_advisory(root_secret: [u8; 32], mut advisory: Advisory) -> Advisory {
    advisory.signature.clear();
    advisory.signature = sign(&root_secret, &advisory_bytes(&advisory));
    advisory
}

pub fn sign_admin_action(
    root_secret: [u8; 32],
    mut action: RegistryAdminAction,
) -> Result<RegistryAdminAction, NivError> {
    validate_admin_action(&action)?;
    action.signature.clear();
    action.signature = sign(&root_secret, &admin_action_bytes(&action));
    Ok(action)
}

pub fn verify_admin_action(
    action: &RegistryAdminAction,
    root_public_key: [u8; 32],
    now: u64,
    minimum_generation: u64,
) -> Result<(), NivError> {
    validate_admin_action(action)?;
    verify(
        &root_public_key,
        &admin_action_bytes(action),
        &action.signature,
    )?;
    if action.generation <= minimum_generation {
        return Err(trust_error(
            "registry admin generation was replayed or rolled back",
        ));
    }
    if action.issued_at > now.saturating_add(300) || action.expires_at < now {
        return Err(trust_error(
            "registry admin action is stale or future-dated",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn verify_release(
    package_bytes: &[u8],
    provenance: &ReleaseProvenance,
    authorization: &PublisherAuthorization,
    status: &RegistryStatus,
    advisories: &[Advisory],
    root_public_key: [u8; 32],
    now: u64,
    minimum_status_generation: u64,
) -> Result<Package, NivError> {
    if !valid_publisher(&authorization.publisher) || !valid_publisher(&provenance.publisher) {
        return Err(trust_error(
            "publisher name is not a safe registry identifier",
        ));
    }
    verify(
        &root_public_key,
        &authorization_bytes(authorization),
        &authorization.signature,
    )?;
    validate_status(status)?;
    verify(&root_public_key, &status_bytes(status), &status.signature)?;
    if status.generation < minimum_status_generation {
        return Err(trust_error("registry status generation was rolled back"));
    }
    if status.advisories_sha256 != advisories_sha256(advisories) {
        return Err(trust_error(
            "registry advisory list does not match the signed status",
        ));
    }
    if status.expires_at < now || status.issued_at > now.saturating_add(300) {
        return Err(trust_error("registry status is stale or future-dated"));
    }
    if authorization.expires_at < now {
        return Err(trust_error("publisher authorization has expired"));
    }
    if provenance.issued_at > now.saturating_add(300) {
        return Err(trust_error(
            "release provenance is dated too far in the future",
        ));
    }
    if provenance.publisher != authorization.publisher
        || provenance.public_key != authorization.public_key
        || provenance.repository != authorization.repository
        || provenance.workflow != authorization.workflow
    {
        return Err(trust_error("release identity is not authorized"));
    }
    if !authorization_allows_package(&authorization.packages, &provenance.package) {
        return Err(trust_error(
            "publisher is not authorized to release this package",
        ));
    }
    if status.revoked_keys.contains(&provenance.public_key) {
        return Err(trust_error("publisher key has been revoked"));
    }
    if let Some(reason) = status.frozen_packages.get(&provenance.package) {
        return Err(trust_error(format!("package is frozen: {reason}")));
    }
    let publisher_key = decode_key(&provenance.public_key)?;
    verify(
        &publisher_key,
        &provenance_bytes(provenance),
        &provenance.signature,
    )?;
    let package = Package::decode(package_bytes)?;
    if package.name != provenance.package
        || package.version != provenance.version
        || sha256(package_bytes) != provenance.sha256
    {
        return Err(trust_error("release provenance does not match the package"));
    }
    for advisory in advisories {
        validate_advisory(advisory)?;
        verify(
            &root_public_key,
            &advisory_bytes(advisory),
            &advisory.signature,
        )?;
        if !advisory.withdrawn
            && advisory.package == package.name
            && advisory.affected_versions.contains(&package.version)
        {
            return Err(trust_error(format!(
                "release is blocked by advisory {} ({}): {}",
                advisory.id, advisory.severity, advisory.summary
            )));
        }
    }
    Ok(package)
}

fn valid_publisher(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value.len() <= 128
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub fn public_key(secret: [u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(&secret).verifying_key().to_bytes()
}

pub fn parse_public_key(value: &str) -> Result<[u8; 32], NivError> {
    decode_key(value.trim())
}

pub fn parse_secret_key(value: &str) -> Result<[u8; 32], NivError> {
    decode_hex(value.trim(), "secret key")
}

fn authorization_bytes(value: &PublisherAuthorization) -> Vec<u8> {
    let packages = canonical_list(value.packages.iter().map(String::as_bytes));
    canonical(
        b"nivren.publisher-authorization.v2",
        &[
            value.publisher.as_bytes(),
            value.public_key.as_bytes(),
            value.repository.as_bytes(),
            value.workflow.as_bytes(),
            &value.expires_at.to_le_bytes(),
            &packages,
        ],
    )
}

fn valid_package_pattern(pattern: &str) -> bool {
    pattern == "*"
        || pattern
            .strip_suffix('*')
            .map_or(valid_publisher(pattern), |prefix| {
                !prefix.is_empty() && valid_publisher(prefix)
            })
}

/// Whether an authorization's package list covers `package`.
pub fn authorization_allows_package(patterns: &BTreeSet<String>, package: &str) -> bool {
    valid_publisher(package)
        && patterns.iter().any(|pattern| {
            pattern == "*"
                || pattern
                    .strip_suffix('*')
                    .map_or(pattern == package, |prefix| package.starts_with(prefix))
        })
}

fn provenance_bytes(value: &ReleaseProvenance) -> Vec<u8> {
    canonical(
        b"nivren.release-provenance.v1",
        &[
            value.package.as_bytes(),
            value.version.as_bytes(),
            value.sha256.as_bytes(),
            value.publisher.as_bytes(),
            value.public_key.as_bytes(),
            value.repository.as_bytes(),
            value.workflow.as_bytes(),
            value.commit.as_bytes(),
            &value.issued_at.to_le_bytes(),
        ],
    )
}

// Collections are encoded as a length-prefixed count followed by
// length-prefixed elements. Joining elements with a separator byte let two
// different sets (`["A", "B"]` and `["A\0B"]`) sign to identical bytes, which
// turned a signed revocation or advisory into a forgeable one.
fn status_bytes(value: &RegistryStatus) -> Vec<u8> {
    let revoked = canonical_list(value.revoked_keys.iter().map(String::as_bytes));
    let frozen = value
        .frozen_packages
        .iter()
        .map(|(package, reason)| canonical(b"frozen", &[package.as_bytes(), reason.as_bytes()]))
        .collect::<Vec<_>>();
    let frozen = canonical_list(frozen.iter().map(Vec::as_slice));
    canonical(
        b"nivren.registry-status.v2",
        &[
            &value.generation.to_le_bytes(),
            &value.issued_at.to_le_bytes(),
            &value.expires_at.to_le_bytes(),
            &revoked,
            &frozen,
            value.advisories_sha256.as_bytes(),
        ],
    )
}

fn advisory_bytes(value: &Advisory) -> Vec<u8> {
    let affected = canonical_list(value.affected_versions.iter().map(String::as_bytes));
    canonical(
        b"nivren.advisory.v2",
        &[
            value.id.as_bytes(),
            value.package.as_bytes(),
            &affected,
            value.severity.as_bytes(),
            value.summary.as_bytes(),
            &[u8::from(value.withdrawn)],
        ],
    )
}

/// The digest a signed status commits to for the advisory list served with it.
pub fn advisories_sha256(advisories: &[Advisory]) -> String {
    let items = advisories.iter().map(advisory_bytes).collect::<Vec<_>>();
    let mut bytes = canonical(b"nivren.advisory-list.v1", &[]);
    bytes.extend(canonical_list(items.iter().map(Vec::as_slice)));
    sha256(&bytes)
}

fn canonical_list<'a>(items: impl Iterator<Item = &'a [u8]>) -> Vec<u8> {
    let items = items.collect::<Vec<_>>();
    let mut bytes = Vec::new();
    append(
        &mut bytes,
        &u64::try_from(items.len()).unwrap().to_le_bytes(),
    );
    for item in items {
        append(&mut bytes, item);
    }
    bytes
}

fn safe_text(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

fn validate_status(value: &RegistryStatus) -> Result<(), NivError> {
    if value
        .revoked_keys
        .iter()
        .any(|key| decode_key(key).is_err())
        || value
            .frozen_packages
            .iter()
            .any(|(package, reason)| !valid_publisher(package) || !safe_text(reason, 1024))
        || value.advisories_sha256.len() != 64
        || !value
            .advisories_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(trust_error("registry status contains invalid entries"));
    }
    Ok(())
}

fn validate_advisory(value: &Advisory) -> Result<(), NivError> {
    if !safe_text(&value.id, 128)
        || !valid_publisher(&value.package)
        || value
            .affected_versions
            .iter()
            .any(|version| !safe_text(version, 64))
        || !safe_text(&value.severity, 64)
        || value.summary.len() > 4096
        || value
            .summary
            .chars()
            .any(|character| character.is_control() && character != '\n')
    {
        return Err(trust_error("advisory contains invalid entries"));
    }
    Ok(())
}

fn admin_action_bytes(value: &RegistryAdminAction) -> Vec<u8> {
    canonical(
        b"nivren.registry-admin-action.v1",
        &[
            &value.format.to_le_bytes(),
            value.action.as_bytes(),
            value.package.as_bytes(),
            value.version.as_bytes(),
            &value.generation.to_le_bytes(),
            &value.issued_at.to_le_bytes(),
            &value.expires_at.to_le_bytes(),
            value.reason.as_bytes(),
        ],
    )
}

fn validate_admin_action(value: &RegistryAdminAction) -> Result<(), NivError> {
    if value.format != 1 || !matches!(value.action.as_str(), "yank" | "unyank") {
        return Err(trust_error("invalid registry admin action"));
    }
    if !valid_publisher(&value.package)
        || value.version.len() > 64
        || value.version.split('.').count() != 3
        || !value
            .version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return Err(trust_error("registry admin package identity is invalid"));
    }
    if value.generation == 0
        || value.issued_at > value.expires_at
        || value.reason.trim().is_empty()
        || value.reason.len() > 1024
        || value.reason.chars().any(char::is_control)
    {
        return Err(trust_error("registry admin bounds are invalid"));
    }
    Ok(())
}

fn canonical(domain: &[u8], fields: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    append(&mut bytes, domain);
    for field in fields {
        append(&mut bytes, field);
    }
    bytes
}

fn append(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&u64::try_from(value.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(value);
}

fn sign(secret: &[u8; 32], message: &[u8]) -> String {
    encode_hex(&SigningKey::from_bytes(secret).sign(message).to_bytes())
}

fn verify(public: &[u8; 32], message: &[u8], signature: &str) -> Result<(), NivError> {
    let key =
        VerifyingKey::from_bytes(public).map_err(|_| trust_error("invalid Ed25519 public key"))?;
    let signature = decode_hex::<64>(signature, "signature")?;
    key.verify_strict(message, &Signature::from_bytes(&signature))
        .map_err(|_| trust_error("signature verification failed"))
}

fn decode_key(value: &str) -> Result<[u8; 32], NivError> {
    decode_hex(value, "public key")
}

fn decode_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N], NivError> {
    if !value.is_ascii() || value.len() != N * 2 {
        return Err(trust_error(format!("invalid {label} length")));
    }
    let mut bytes = [0; N];
    for (index, output) in bytes.iter_mut().enumerate() {
        *output = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| trust_error(format!("invalid hexadecimal {label}")))?;
    }
    Ok(bytes)
}

pub fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

fn sha256(bytes: &[u8]) -> String {
    encode_hex(&Sha256::digest(bytes))
}

struct EnvelopeReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> EnvelopeReader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], NivError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| trust_error("publish envelope offset overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| trust_error("truncated publish envelope"))?;
        self.offset = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, NivError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| trust_error("truncated publish envelope"))?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn length_u32(&mut self) -> Result<usize, NivError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| trust_error("truncated publish envelope"))?;
        Ok(u32::from_le_bytes(bytes) as usize)
    }

    fn length_u64(&mut self) -> Result<usize, NivError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| trust_error("truncated publish envelope"))?;
        usize::try_from(u64::from_le_bytes(bytes))
            .map_err(|_| trust_error("publish envelope length exceeds platform range"))
    }
}

fn trust_error(message: impl Into<String>) -> NivError {
    NivError::new(message, 1, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(revoked: &[&str]) -> RegistryStatus {
        RegistryStatus {
            generation: 1,
            issued_at: 1,
            expires_at: 2,
            revoked_keys: revoked.iter().map(|key| (*key).to_string()).collect(),
            frozen_packages: BTreeMap::new(),
            advisories_sha256: advisories_sha256(&[]),
            signature: String::new(),
        }
    }

    fn advisory(affected: &[&str]) -> Advisory {
        Advisory {
            id: "NIV-1".into(),
            package: "library".into(),
            affected_versions: affected.iter().map(|value| (*value).to_string()).collect(),
            severity: "high".into(),
            summary: "test".into(),
            withdrawn: false,
            signature: String::new(),
        }
    }

    #[test]
    fn set_elements_cannot_collide_through_separator_bytes() {
        let key_a = "a".repeat(64);
        let key_b = "b".repeat(64);
        let merged = format!("{key_a}\0{key_b}");
        assert_ne!(
            status_bytes(&status(&[&key_a, &key_b])),
            status_bytes(&status(&[&merged]))
        );
        assert_ne!(
            advisory_bytes(&advisory(&["1.0.0", "1.0.1"])),
            advisory_bytes(&advisory(&["1.0.0\u{0}1.0.1"]))
        );
    }

    #[test]
    fn statuses_and_advisories_with_control_bytes_are_rejected() {
        let key_a = "a".repeat(64);
        assert!(validate_status(&status(&[&key_a])).is_ok());
        assert!(validate_status(&status(&[&format!("{key_a}\0{key_a}")])).is_err());
        let mut bad_digest = status(&[]);
        bad_digest.advisories_sha256.clear();
        assert!(validate_status(&bad_digest).is_err());
        assert!(validate_advisory(&advisory(&["1.0.0"])).is_ok());
        assert!(validate_advisory(&advisory(&["1.0.0\u{0}1.0.1"])).is_err());
    }

    #[test]
    fn authorizations_bind_publishers_to_packages() {
        let exact = BTreeSet::from(["nivren_stats".to_string()]);
        assert!(authorization_allows_package(&exact, "nivren_stats"));
        assert!(!authorization_allows_package(&exact, "nivren_stat"));
        assert!(!authorization_allows_package(&exact, "nivren_stats_extra"));
        let prefix = BTreeSet::from(["nivren_*".to_string()]);
        assert!(authorization_allows_package(&prefix, "nivren_aead"));
        assert!(!authorization_allows_package(&prefix, "registry_drill"));
        let any = BTreeSet::from(["*".to_string()]);
        assert!(authorization_allows_package(&any, "registry_drill"));
        assert!(!authorization_allows_package(
            &BTreeSet::new(),
            "nivren_aead"
        ));
        assert!(!authorization_allows_package(&any, "../escape"));
        assert!(
            authorize_publisher(
                [1u8; 32],
                "team".into(),
                encode_hex(&public_key([2u8; 32])),
                "example/repo".into(),
                "release.yml".into(),
                10,
                BTreeSet::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn advisory_list_digest_tracks_every_entry() {
        let one = advisory(&["1.0.0"]);
        let mut withdrawn = one.clone();
        withdrawn.withdrawn = true;
        assert_ne!(advisories_sha256(&[]), advisories_sha256(&[one.clone()]));
        assert_ne!(
            advisories_sha256(&[one.clone()]),
            advisories_sha256(&[withdrawn])
        );
        assert_eq!(advisories_sha256(&[one.clone()]), advisories_sha256(&[one]));
    }
}
