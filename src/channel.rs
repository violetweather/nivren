use std::collections::BTreeMap;
use std::fmt::Write as _;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::error::NivError;

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelManifest {
    pub format: u64,
    pub channel: String,
    pub version: String,
    pub generation: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub base_url: String,
    pub assets: BTreeMap<String, String>,
    pub signature: String,
}

impl ChannelManifest {
    pub fn sign(&mut self, secret_hex: &str) -> Result<(), NivError> {
        validate_unsigned(self)?;
        let secret = decode_hex::<32>(secret_hex, "channel signing key")?;
        self.signature.clear();
        self.signature = encode_hex(
            &SigningKey::from_bytes(&secret)
                .sign(&canonical(self)?)
                .to_bytes(),
        );
        Ok(())
    }

    pub fn verify(
        &self,
        public_hex: &str,
        now: u64,
        minimum_generation: u64,
        expected_channel: Option<&str>,
    ) -> Result<(), NivError> {
        validate_unsigned(self)?;
        // A validly signed nightly manifest served at the stable URL would
        // otherwise move stable installs onto nightly builds.
        if expected_channel.is_some_and(|expected| expected != self.channel) {
            return Err(channel_error(
                "channel manifest names a different channel than the one requested",
            ));
        }
        if self.generation < minimum_generation {
            return Err(channel_error(
                "channel manifest generation is below the trusted minimum",
            ));
        }
        if now < self.issued_at || now > self.expires_at {
            return Err(channel_error(
                "channel manifest is not valid at the requested time",
            ));
        }
        let public = decode_hex::<32>(public_hex, "channel public key")?;
        let key = VerifyingKey::from_bytes(&public)
            .map_err(|_| channel_error("invalid Ed25519 channel public key"))?;
        let signature = decode_hex::<64>(&self.signature, "channel signature")?;
        key.verify_strict(&canonical(self)?, &Signature::from_bytes(&signature))
            .map_err(|_| channel_error("channel manifest signature verification failed"))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, NivError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(channel_error("channel manifest exceeds 1 MiB"));
        }
        serde_json::from_slice(bytes)
            .map_err(|error| channel_error(format!("invalid channel manifest: {error}")))
    }

    pub fn encode(&self) -> Result<Vec<u8>, NivError> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| channel_error(format!("cannot encode channel manifest: {error}")))?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(channel_error("channel manifest exceeds 1 MiB"));
        }
        Ok(bytes)
    }
}

fn validate_unsigned(value: &ChannelManifest) -> Result<(), NivError> {
    if value.format != 1 || !matches!(value.channel.as_str(), "stable" | "beta" | "nightly") {
        return Err(channel_error("unsupported release channel manifest"));
    }
    if !safe_component(&value.version)
        || value.generation == 0
        || value.issued_at >= value.expires_at
    {
        return Err(channel_error(
            "invalid release channel identity or validity window",
        ));
    }
    if !value.base_url.starts_with("https://")
        || value.base_url.ends_with('/')
        || value.base_url.len() > 4096
    {
        return Err(channel_error(
            "channel base URL must be bounded HTTPS without a trailing slash",
        ));
    }
    if value.assets.is_empty() || value.assets.len() > 64 {
        return Err(channel_error(
            "channel manifest must contain 1 through 64 assets",
        ));
    }
    for (name, digest) in &value.assets {
        if !safe_component(name) || decode_hex::<32>(digest, "asset SHA-256").is_err() {
            return Err(channel_error("channel asset identity or digest is invalid"));
        }
    }
    Ok(())
}

fn canonical(value: &ChannelManifest) -> Result<Vec<u8>, NivError> {
    let mut unsigned = value.clone();
    unsigned.signature.clear();
    serde_json::to_vec(&unsigned)
        .map_err(|error| channel_error(format!("cannot canonicalize channel manifest: {error}")))
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn decode_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N], NivError> {
    if !value.is_ascii() || value.len() != N * 2 {
        return Err(channel_error(format!("invalid {label} length")));
    }
    let mut bytes = [0; N];
    for (index, output) in bytes.iter_mut().enumerate() {
        *output = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| channel_error(format!("invalid hexadecimal {label}")))?;
    }
    Ok(bytes)
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

fn channel_error(message: impl Into<String>) -> NivError {
    NivError::new(message, 1, 1)
}

#[cfg(test)]
mod tests {
    use super::ChannelManifest;
    use ed25519_dalek::SigningKey;
    use std::collections::BTreeMap;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn signed_channels_are_canonical_expiring_and_rollback_safe() {
        let secret = [7u8; 32];
        let public = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
        let mut manifest = ChannelManifest {
            format: 1,
            channel: "beta".into(),
            version: "0.10.0-beta.7".into(),
            generation: 7,
            issued_at: 100,
            expires_at: 200,
            base_url: "https://github.com/violetweather/nivren/releases/download/v0.10.0-beta.7"
                .into(),
            assets: BTreeMap::from([(
                "nivren-v0.10.0-beta.7-linux-x64.zip".into(),
                "00".repeat(32),
            )]),
            signature: String::new(),
        };
        manifest.sign(&hex(&secret)).unwrap();
        manifest.verify(&hex(&public), 150, 7, None).unwrap();
        manifest
            .verify(&hex(&public), 150, 7, Some(manifest.channel.as_str()))
            .unwrap();
        assert!(
            manifest
                .verify(&hex(&public), 150, 7, Some("definitely-other"))
                .is_err()
        );
        assert_eq!(
            ChannelManifest::decode(&manifest.encode().unwrap()).unwrap(),
            manifest
        );
        assert!(manifest.verify(&hex(&public), 201, 7, None).is_err());
        assert!(manifest.verify(&hex(&public), 150, 8, None).is_err());
        manifest.version = "0.10.0-beta.8".into();
        assert!(manifest.verify(&hex(&public), 150, 7, None).is_err());
    }
}
