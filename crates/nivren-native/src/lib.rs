//! The only dynamic-library loading and invocation boundary used by the Nivren VM.
//!
//! The safe VM passes only owned paths, checked symbol names, and finite primitive
//! argument lists. Calling foreign code is still an explicit native trust boundary:
//! the caller must ensure that a symbol really has the selected C ABI signature.

use std::path::Path;

use libloading::{Library, Symbol};

const MAX_ARGUMENTS: usize = 6;
const MAX_BUFFER: usize = 16 * 1024 * 1024;

/// An owned dynamic library. Symbols never escape a call, so they cannot outlive it.
pub struct DynamicLibrary {
    library: Library,
}

impl DynamicLibrary {
    /// Loads a library and runs its initialization routines.
    pub fn open(path: &Path) -> Result<Self, String> {
        // SAFETY: Loading a library runs foreign initialization code. Nivren exposes
        // this only through the explicit Native capability; the resulting Library is
        // owned and cannot expose a longer-lived Symbol.
        let library = unsafe { Library::new(path) }.map_err(|error| error.to_string())?;
        Ok(Self { library })
    }

    /// Calls an `extern "C" fn(i64, ...) -> i64` with zero through six arguments.
    pub fn call_int(&self, symbol: &str, arguments: &[i64]) -> Result<i64, String> {
        check_call(symbol, arguments.len())?;
        let name = symbol.as_bytes();
        // SAFETY: The Native-capable caller promises the exported symbol uses the
        // exact signature selected by argument count. The Symbol is invoked while
        // `self.library` is borrowed and is never stored or returned.
        unsafe {
            match arguments {
                [] => load::<unsafe extern "C" fn() -> i64>(&self.library, name).map(|f| f()),
                [a] => load::<unsafe extern "C" fn(i64) -> i64>(&self.library, name).map(|f| f(*a)),
                [a, b] => load::<unsafe extern "C" fn(i64, i64) -> i64>(&self.library, name)
                    .map(|f| f(*a, *b)),
                [a, b, c] => {
                    load::<unsafe extern "C" fn(i64, i64, i64) -> i64>(&self.library, name)
                        .map(|f| f(*a, *b, *c))
                }
                [a, b, c, d] => {
                    load::<unsafe extern "C" fn(i64, i64, i64, i64) -> i64>(&self.library, name)
                        .map(|f| f(*a, *b, *c, *d))
                }
                [a, b, c, d, e] => load::<unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64>(
                    &self.library,
                    name,
                )
                .map(|f| f(*a, *b, *c, *d, *e)),
                [a, b, c, d, e, f] => load::<
                    unsafe extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64,
                >(&self.library, name)
                .map(|call| call(*a, *b, *c, *d, *e, *f)),
                _ => unreachable!("argument count was checked"),
            }
        }
    }

    /// Calls an `extern "C" fn(double, ...) -> double` with zero through six arguments.
    pub fn call_float(&self, symbol: &str, arguments: &[f64]) -> Result<f64, String> {
        check_call(symbol, arguments.len())?;
        let name = symbol.as_bytes();
        // SAFETY: See `call_int`; this selects only finite all-double signatures.
        unsafe {
            match arguments {
                [] => load::<unsafe extern "C" fn() -> f64>(&self.library, name).map(|f| f()),
                [a] => load::<unsafe extern "C" fn(f64) -> f64>(&self.library, name).map(|f| f(*a)),
                [a, b] => load::<unsafe extern "C" fn(f64, f64) -> f64>(&self.library, name)
                    .map(|f| f(*a, *b)),
                [a, b, c] => {
                    load::<unsafe extern "C" fn(f64, f64, f64) -> f64>(&self.library, name)
                        .map(|f| f(*a, *b, *c))
                }
                [a, b, c, d] => {
                    load::<unsafe extern "C" fn(f64, f64, f64, f64) -> f64>(&self.library, name)
                        .map(|f| f(*a, *b, *c, *d))
                }
                [a, b, c, d, e] => load::<unsafe extern "C" fn(f64, f64, f64, f64, f64) -> f64>(
                    &self.library,
                    name,
                )
                .map(|f| f(*a, *b, *c, *d, *e)),
                [a, b, c, d, e, f] => load::<
                    unsafe extern "C" fn(f64, f64, f64, f64, f64, f64) -> f64,
                >(&self.library, name)
                .map(|call| call(*a, *b, *c, *d, *e, *f)),
                _ => unreachable!("argument count was checked"),
            }
        }
    }

    /// Calls `extern "C" fn(const uint8_t*, size_t, uint8_t*, size_t) -> int64_t`.
    ///
    /// The output buffer is initialized, remains owned by Rust, and is truncated to
    /// the returned byte count. A negative return is a foreign error code.
    pub fn call_buffer(
        &self,
        symbol: &str,
        input: &[u8],
        capacity: usize,
    ) -> Result<Vec<u8>, String> {
        check_call(symbol, 0)?;
        if input.len() > MAX_BUFFER || capacity > MAX_BUFFER {
            return Err("native buffers are limited to 16 MiB".into());
        }
        let mut output = vec![0; capacity];
        let function = unsafe {
            load::<unsafe extern "C" fn(*const u8, usize, *mut u8, usize) -> i64>(
                &self.library,
                symbol.as_bytes(),
            )?
        };
        // SAFETY: Both pointers describe live Rust-owned buffers for the exact
        // lengths supplied. The Native-capable caller promises the symbol uses the
        // documented signature and the function cannot retain either pointer.
        let length = unsafe {
            function(
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        if length < 0 {
            return Err(format!("native buffer call returned error code {length}"));
        }
        let length = usize::try_from(length).map_err(|_| "native buffer length is invalid")?;
        if length > output.len() {
            return Err("native buffer call returned more bytes than its capacity".into());
        }
        output.truncate(length);
        Ok(output)
    }
}

fn check_call(symbol: &str, argument_count: usize) -> Result<(), String> {
    if symbol.is_empty() || symbol.len() > 256 || symbol.as_bytes().contains(&0) {
        return Err("native symbol must contain 1 through 256 non-NUL bytes".into());
    }
    if argument_count > MAX_ARGUMENTS {
        return Err(format!(
            "native calls accept at most {MAX_ARGUMENTS} primitive arguments"
        ));
    }
    Ok(())
}

unsafe fn load<'library, T>(
    library: &'library Library,
    name: &[u8],
) -> Result<Symbol<'library, T>, String> {
    // SAFETY: The caller selects T from a finite ABI table and upholds that the
    // library export has that exact type. The returned lifetime is tied to library.
    unsafe { library.get(name) }.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::check_call;

    #[test]
    fn rejects_invalid_symbols_and_unbounded_arity() {
        assert!(check_call("", 0).is_err());
        assert!(check_call("bad\0name", 0).is_err());
        assert!(check_call("ok", 6).is_ok());
        assert!(check_call("too_many", 7).is_err());
    }
}
