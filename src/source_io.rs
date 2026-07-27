use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::Duration;

/// Reads one stable file snapshot and rejects short/transient provider reads.
pub(crate) fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    for attempt in 0..4 {
        let before = fs::metadata(path)?;
        let bytes = fs::read(path)?;
        let after = fs::metadata(path)?;
        if before.len() == after.len() && after.len() == bytes.len() as u64 {
            return Ok(bytes);
        }
        if attempt < 3 {
            thread::sleep(Duration::from_millis(5 * (attempt + 1)));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!(
            "{} changed or returned a transient short read; retry after the file provider is ready",
            path.display()
        ),
    ))
}

pub(crate) fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "source file is not valid UTF-8"))
}
