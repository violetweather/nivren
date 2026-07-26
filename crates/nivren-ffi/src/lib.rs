use std::panic::{AssertUnwindSafe, catch_unwind};

#[repr(C)]
pub struct NivrenBuffer {
    pub data: *mut u8,
    pub length: usize,
    pub capacity: usize,
    pub status: u32,
}

impl NivrenBuffer {
    fn new(bytes: Vec<u8>, status: u32) -> Self {
        let mut bytes = bytes;
        let result = Self {
            data: bytes.as_mut_ptr(),
            length: bytes.len(),
            capacity: bytes.capacity(),
            status,
        };
        std::mem::forget(bytes);
        result
    }

    fn error(message: &str, status: u32) -> Self {
        Self::new(message.as_bytes().to_vec(), status)
    }
}

/// Executes one UTF-8 Nivren source buffer.
///
/// Status 0 is success, 1 is a language error, 2 is invalid host input, and 3
/// is a caught internal panic. The caller owns the returned allocation and must
/// release it exactly once with `nivren_buffer_free`.
///
/// # Safety
///
/// When `length` is nonzero, `source` must point to `length` readable bytes for
/// the duration of this call. The memory may be unaligned and is never retained.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_run_utf8(source: *const u8, length: usize) -> NivrenBuffer {
    if source.is_null() && length != 0 {
        return NivrenBuffer::error("null source pointer with nonzero length", 2);
    }
    let bytes = if length == 0 {
        &[]
    } else {
        // SAFETY: The caller contract requires this pointer/length pair to be
        // readable. We copy/parse it during this call and do not retain it.
        unsafe { std::slice::from_raw_parts(source, length) }
    };
    let source = match std::str::from_utf8(bytes) {
        Ok(source) => source,
        Err(error) => return NivrenBuffer::error(&format!("source is not UTF-8: {error}"), 2),
    };
    match catch_unwind(AssertUnwindSafe(|| nivren::run(source))) {
        Ok(Ok(value)) => NivrenBuffer::new(value.to_string().into_bytes(), 0),
        Ok(Err(errors)) => NivrenBuffer::new(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                .into_bytes(),
            1,
        ),
        Err(_) => NivrenBuffer::error("internal Nivren panic", 3),
    }
}

/// Releases a buffer returned by `nivren_run_utf8`.
///
/// # Safety
///
/// `buffer` must be an unchanged result from `nivren_run_utf8` that has not
/// already been freed. Passing any other pointer/capacity pair is undefined.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_buffer_free(buffer: NivrenBuffer) {
    if buffer.capacity == 0 {
        return;
    }
    // SAFETY: The caller contract requires an unchanged, uniquely owned buffer
    // produced by Nivren. Reconstructing the Vec transfers it back for drop.
    drop(unsafe { Vec::from_raw_parts(buffer.data, buffer.length, buffer.capacity) });
}

#[cfg(test)]
mod tests {
    use super::{NivrenBuffer, nivren_buffer_free, nivren_run_utf8};

    fn output(buffer: &NivrenBuffer) -> String {
        // SAFETY: Tests inspect a live buffer returned by this crate.
        let bytes = unsafe { std::slice::from_raw_parts(buffer.data, buffer.length) };
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn c_boundary_runs_source_and_reports_invalid_input() {
        let source = b"20 + 22";
        // SAFETY: The byte slice remains readable for the call.
        let buffer = unsafe { nivren_run_utf8(source.as_ptr(), source.len()) };
        assert_eq!((buffer.status, output(&buffer)), (0, "42".into()));
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(buffer) };

        // SAFETY: A null pointer is explicitly accepted when length is zero.
        let empty = unsafe { nivren_run_utf8(std::ptr::null(), 0) };
        assert_eq!(empty.status, 0);
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(empty) };

        // SAFETY: The one-byte slice remains readable for the call.
        let invalid = unsafe { nivren_run_utf8([0xff].as_ptr(), 1) };
        assert_eq!(invalid.status, 2);
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(invalid) };
    }
}
