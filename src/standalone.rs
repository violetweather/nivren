use crate::error::NivError;

const MAGIC: &[u8; 8] = b"NIVAPP3\0";
const TRAILER: usize = 24;
const MAX_MANIFEST: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedApplication {
    pub executable_length: usize,
    pub bundle: Vec<u8>,
    pub manifest: String,
}

pub fn attach(executable: &[u8], bundle: &[u8], manifest: &str) -> Result<Vec<u8>, NivError> {
    let base = extract(executable)
        .map(|application| application.executable_length)
        .unwrap_or(executable.len());
    if manifest.len() > MAX_MANIFEST {
        return Err(error("standalone manifest exceeds 1 MiB"));
    }
    crate::bundle::decode(bundle)?;
    let bundle_length = u64::try_from(bundle.len()).map_err(|_| error("bundle is too large"))?;
    let manifest_length =
        u64::try_from(manifest.len()).map_err(|_| error("manifest is too large"))?;
    let mut output = Vec::with_capacity(
        base.saturating_add(bundle.len())
            .saturating_add(manifest.len())
            .saturating_add(TRAILER),
    );
    output.extend_from_slice(&executable[..base]);
    output.extend_from_slice(bundle);
    output.extend_from_slice(manifest.as_bytes());
    output.extend_from_slice(&bundle_length.to_le_bytes());
    output.extend_from_slice(&manifest_length.to_le_bytes());
    output.extend_from_slice(MAGIC);
    Ok(output)
}

pub fn extract(executable: &[u8]) -> Option<EmbeddedApplication> {
    if executable.len() < TRAILER || &executable[executable.len() - MAGIC.len()..] != MAGIC {
        return None;
    }
    let lengths = executable.len() - TRAILER;
    let bundle_length = u64::from_le_bytes(executable[lengths..lengths + 8].try_into().ok()?);
    let manifest_length =
        u64::from_le_bytes(executable[lengths + 8..lengths + 16].try_into().ok()?);
    let bundle_length = usize::try_from(bundle_length).ok()?;
    let manifest_length = usize::try_from(manifest_length).ok()?;
    if manifest_length > MAX_MANIFEST {
        return None;
    }
    let payload_length = bundle_length.checked_add(manifest_length)?;
    let executable_length = lengths.checked_sub(payload_length)?;
    let bundle_end = executable_length.checked_add(bundle_length)?;
    let manifest_end = bundle_end.checked_add(manifest_length)?;
    let bundle = executable[executable_length..bundle_end].to_vec();
    if crate::bundle::decode(&bundle).is_err() {
        return None;
    }
    let manifest = std::str::from_utf8(&executable[bundle_end..manifest_end])
        .ok()?
        .to_string();
    Some(EmbeddedApplication {
        executable_length,
        bundle,
        manifest,
    })
}

fn error(message: impl Into<String>) -> NivError {
    NivError::new(message, 1, 1)
}

#[cfg(test)]
mod tests {
    use super::{attach, extract};

    #[test]
    fn applications_attach_replace_validate_and_extract() {
        let chunk = crate::bytecode::compile(
            &crate::parser::parse(crate::lexer::scan("40 + 2").unwrap()).unwrap(),
        )
        .unwrap();
        let bundle = crate::bundle::encode(&chunk).unwrap();
        let first = attach(b"executable", &bundle, "manifest one").unwrap();
        let extracted = extract(&first).unwrap();
        assert_eq!(extracted.executable_length, 10);
        assert_eq!(extracted.bundle, bundle);
        assert_eq!(extracted.manifest, "manifest one");
        let replaced = attach(&first, &bundle, "manifest two").unwrap();
        let extracted = extract(&replaced).unwrap();
        assert_eq!(extracted.executable_length, 10);
        assert_eq!(extracted.manifest, "manifest two");
        assert!(extract(b"ordinary executable").is_none());
        let mut corrupted = replaced;
        corrupted[10] ^= 0xff;
        assert!(extract(&corrupted).is_none());
    }
}
