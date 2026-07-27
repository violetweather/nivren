#![allow(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};

const MAXIMUM: usize = 16 * 1024 * 1024;

fn result_word(status: u8, length: u32, pointer: u32) -> u64 {
    debug_assert!(status <= 0x0f);
    debug_assert!(length <= 0x0fff_ffff);
    (u64::from(status) << 60) | (u64::from(length) << 32) | u64::from(pointer)
}

fn pack(bytes: Vec<u8>, status: u8) -> u64 {
    let mut bytes = bytes.into_boxed_slice();
    if bytes.len() > MAXIMUM {
        return pack(b"WebAssembly result exceeds 16 MiB".to_vec(), 3);
    }
    let length = bytes.len() as u32;
    let pointer = bytes.as_mut_ptr() as usize as u32;
    std::mem::forget(bytes);
    result_word(status, length, pointer)
}

unsafe fn source(pointer: u32, length: u32) -> Result<String, u64> {
    if length as usize > MAXIMUM {
        return Err(pack(b"WebAssembly input exceeds 16 MiB".to_vec(), 2));
    }
    if pointer == 0 && length != 0 {
        return Err(pack(b"null source pointer with nonzero length".to_vec(), 2));
    }
    let bytes = if length == 0 {
        &[]
    } else {
        // SAFETY: The host contract requires this range to remain readable for the call.
        unsafe { std::slice::from_raw_parts(pointer as *const u8, length as usize) }
    };
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|error| pack(error.to_string().into_bytes(), 2))
}

fn diagnostics(errors: &[nivren::compiler::Diagnostic]) -> Vec<u8> {
    errors
        .iter()
        .map(|error| format!("{}:{}: {}", error.line, error.column, error.message))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

#[unsafe(no_mangle)]
pub extern "C" fn nivren_wasm_abi_version() -> u32 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn nivren_wasm_alloc(length: u32) -> u32 {
    if length == 0 || length as usize > MAXIMUM {
        return 0;
    }
    let mut bytes = vec![0u8; length as usize].into_boxed_slice();
    let pointer = bytes.as_mut_ptr() as usize as u32;
    std::mem::forget(bytes);
    pointer
}

#[unsafe(no_mangle)]
/// Releases one output or input buffer allocated by this module.
///
/// # Safety
/// The pointer and exact length must describe a live allocation returned by
/// `nivren_wasm_alloc` or packed in a Nivren result, and may be freed once.
pub unsafe extern "C" fn nivren_wasm_free(pointer: u32, length: u32) {
    if pointer == 0 || length == 0 || length as usize > MAXIMUM {
        return;
    }
    let slice = std::ptr::slice_from_raw_parts_mut(pointer as *mut u8, length as usize);
    // SAFETY: The pointer and exact length must come from an exported Nivren allocation.
    drop(unsafe { Box::from_raw(slice) });
}

#[unsafe(no_mangle)]
/// Checks UTF-8 Nivren source without executing it.
///
/// # Safety
/// The pointer and length must describe readable guest memory for this call.
pub unsafe extern "C" fn nivren_wasm_check(pointer: u32, length: u32) -> u64 {
    let source = match unsafe { source(pointer, length) } {
        Ok(source) => source,
        Err(error) => return error,
    };
    match catch_unwind(AssertUnwindSafe(|| {
        nivren::compiler::Compiler::new().check(&source)
    })) {
        Ok(Ok(())) => pack(Vec::new(), 0),
        Ok(Err(errors)) => pack(diagnostics(&errors), 1),
        Err(_) => pack(b"internal Nivren panic".to_vec(), 3),
    }
}

#[unsafe(no_mangle)]
/// Formats UTF-8 Nivren source.
///
/// # Safety
/// The pointer and length must describe readable guest memory for this call.
pub unsafe extern "C" fn nivren_wasm_format(pointer: u32, length: u32) -> u64 {
    let source = match unsafe { source(pointer, length) } {
        Ok(source) => source,
        Err(error) => return error,
    };
    match catch_unwind(AssertUnwindSafe(|| {
        nivren::compiler::Compiler::new().format(&source)
    })) {
        Ok(value) => pack(value.into_bytes(), 0),
        Err(_) => pack(b"internal Nivren panic".to_vec(), 3),
    }
}

#[unsafe(no_mangle)]
/// Compiles UTF-8 Nivren source to verified bytecode.
///
/// # Safety
/// The pointer and length must describe readable guest memory for this call.
pub unsafe extern "C" fn nivren_wasm_compile(pointer: u32, length: u32) -> u64 {
    let source = match unsafe { source(pointer, length) } {
        Ok(source) => source,
        Err(error) => return error,
    };
    match catch_unwind(AssertUnwindSafe(|| {
        nivren::compiler::Compiler::new().compile(&source)
    })) {
        Ok(Ok(artifact)) => pack(artifact.bytes, 0),
        Ok(Err(errors)) => pack(diagnostics(&errors), 1),
        Err(_) => pack(b"internal Nivren panic".to_vec(), 3),
    }
}

#[unsafe(no_mangle)]
/// Checks, compiles, and executes UTF-8 Nivren source in the portable VM.
///
/// # Safety
/// The pointer and length must describe readable guest memory for this call.
pub unsafe extern "C" fn nivren_wasm_run(pointer: u32, length: u32) -> u64 {
    let source = match unsafe { source(pointer, length) } {
        Ok(source) => source,
        Err(error) => return error,
    };
    match catch_unwind(AssertUnwindSafe(|| nivren::run(&source))) {
        Ok(Ok(value)) => pack(value.to_string().into_bytes(), 0),
        Ok(Err(errors)) => pack(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                .into_bytes(),
            1,
        ),
        Err(_) => pack(b"internal Nivren panic".to_vec(), 3),
    }
}

#[cfg(test)]
mod tests {
    use super::result_word;

    #[test]
    fn result_layout_preserves_maximum_length_and_status() {
        let word = result_word(3, 16 * 1024 * 1024, 0xdead_beef);
        assert_eq!(word >> 60, 3);
        assert_eq!((word >> 32) & 0x0fff_ffff, 16 * 1024 * 1024);
        assert_eq!(word & 0xffff_ffff, 0xdead_beef);
    }
}
