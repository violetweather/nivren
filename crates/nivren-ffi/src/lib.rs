use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

pub const NIVREN_ABI_VERSION: u32 = 3;

#[repr(C)]
pub struct NivrenBuffer {
    pub data: *mut u8,
    pub length: usize,
    pub capacity: usize,
    pub status: u32,
}

pub type NivrenHostCallback = unsafe extern "C" fn(
    name: *const u8,
    name_length: usize,
    request: *const u8,
    request_length: usize,
    context: *mut c_void,
) -> NivrenBuffer;
pub type NivrenHostFree = unsafe extern "C" fn(buffer: NivrenBuffer, context: *mut c_void);
pub type NivrenAsyncComplete = unsafe extern "C" fn(buffer: NivrenBuffer, context: *mut c_void);
pub type NivrenWake = unsafe extern "C" fn(context: *mut c_void);

/// Opaque, uniquely owned asynchronous execution handle.
pub struct NivrenAsyncRun {
    cancellation: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
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

#[unsafe(no_mangle)]
pub extern "C" fn nivren_abi_version() -> u32 {
    NIVREN_ABI_VERSION
}

unsafe fn decode_source<'a>(source: *const u8, length: usize) -> Result<&'a str, NivrenBuffer> {
    if source.is_null() && length != 0 {
        return Err(NivrenBuffer::error(
            "null source pointer with nonzero length",
            2,
        ));
    }
    let bytes = if length == 0 {
        &[]
    } else {
        // SAFETY: Each exported caller contract requires this range to be
        // readable for the call and no exported function retains the slice.
        unsafe { std::slice::from_raw_parts(source, length) }
    };
    std::str::from_utf8(bytes)
        .map_err(|error| NivrenBuffer::error(&format!("source is not UTF-8: {error}"), 2))
}

fn diagnostics(errors: &[nivren::compiler::Diagnostic]) -> Vec<u8> {
    errors
        .iter()
        .map(|error| format!("{}:{}: {}", error.line, error.column, error.message))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

/// Checks one UTF-8 source buffer without executing it.
///
/// # Safety
/// `source` follows the same pointer and lifetime contract as
/// `nivren_run_utf8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_check_utf8(source: *const u8, length: usize) -> NivrenBuffer {
    let source = match unsafe { decode_source(source, length) } {
        Ok(source) => source,
        Err(error) => return error,
    };
    match catch_unwind(AssertUnwindSafe(|| {
        nivren::compiler::Compiler::new().check(source)
    })) {
        Ok(Ok(())) => NivrenBuffer::new(Vec::new(), 0),
        Ok(Err(errors)) => NivrenBuffer::new(diagnostics(&errors), 1),
        Err(_) => NivrenBuffer::error("internal Nivren panic", 3),
    }
}

/// Formats one UTF-8 source buffer.
///
/// # Safety
/// `source` follows the same pointer and lifetime contract as
/// `nivren_run_utf8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_format_utf8(source: *const u8, length: usize) -> NivrenBuffer {
    let source = match unsafe { decode_source(source, length) } {
        Ok(source) => source,
        Err(error) => return error,
    };
    match catch_unwind(AssertUnwindSafe(|| {
        nivren::compiler::Compiler::new().format(source)
    })) {
        Ok(formatted) => NivrenBuffer::new(formatted.into_bytes(), 0),
        Err(_) => NivrenBuffer::error("internal Nivren panic", 3),
    }
}

/// Compiles one UTF-8 source buffer to verified, versioned bytecode.
///
/// # Safety
/// `source` follows the same pointer and lifetime contract as
/// `nivren_run_utf8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_compile_utf8(source: *const u8, length: usize) -> NivrenBuffer {
    let source = match unsafe { decode_source(source, length) } {
        Ok(source) => source,
        Err(error) => return error,
    };
    match catch_unwind(AssertUnwindSafe(|| {
        nivren::compiler::Compiler::new().compile(source)
    })) {
        Ok(Ok(artifact)) => NivrenBuffer::new(artifact.bytes, 0),
        Ok(Err(errors)) => NivrenBuffer::new(diagnostics(&errors), 1),
        Err(_) => NivrenBuffer::error("internal Nivren panic", 3),
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
    let source = match unsafe { decode_source(source, length) } {
        Ok(source) => source,
        Err(error) => return error,
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

/// Checks, compiles, and executes source through the complete-program native
/// Cranelift tier. This entry point never silently redirects to the VM.
///
/// # Safety
///
/// `source` follows the same pointer and lifetime contract as
/// `nivren_run_utf8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_run_native_utf8(source: *const u8, length: usize) -> NivrenBuffer {
    let source = match unsafe { decode_source(source, length) } {
        Ok(source) => source,
        Err(error) => return error,
    };
    match catch_unwind(AssertUnwindSafe(
        || -> Result<_, Vec<nivren::error::NivError>> {
            let tokens = nivren::lexer::scan(source)?;
            let program = nivren::parser::parse(tokens)?;
            nivren::typecheck::check(&program)?;
            let chunk = nivren::bytecode::compile(&program)?;
            nivren::runtime::Interpreter::new()
                .run_native(&chunk)
                .map_err(|error| vec![error])
        },
    )) {
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

/// Executes source with one capability-gated native host callback.
///
/// Nivren calls `callback` for `std.host.invoke(name, request)`. It copies the
/// returned bytes immediately and then calls `free_callback` exactly once.
/// Callback status 0 is success; any other status becomes a Nivren `Err`.
///
/// # Safety
///
/// Source follows `nivren_run_utf8`. Callback pointers must remain valid for
/// this synchronous call. Returned callback buffers must remain readable until
/// `free_callback` is invoked. `context` is passed through unchanged.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_run_host_utf8(
    source_pointer: *const u8,
    length: usize,
    callback: Option<NivrenHostCallback>,
    free_callback: Option<NivrenHostFree>,
    context: *mut c_void,
) -> NivrenBuffer {
    let source = match unsafe { decode_source(source_pointer, length) } {
        Ok(source) => source,
        Err(error) => return error,
    };
    let (Some(callback), Some(free_callback)) = (callback, free_callback) else {
        return NivrenBuffer::error("host callback and free callback are required", 2);
    };
    let context_address = context as usize;
    match catch_unwind(AssertUnwindSafe(|| {
        let tokens = nivren::lexer::scan(source)?;
        let program = nivren::parser::parse(tokens)?;
        nivren::typecheck::check(&program)?;
        let chunk = nivren::bytecode::compile(&program)?;
        let host = move |name: &str, request: &str| {
            // SAFETY: The exported function contract keeps callbacks and the
            // opaque context valid for this synchronous execution.
            let returned = unsafe {
                callback(
                    name.as_ptr(),
                    name.len(),
                    request.as_ptr(),
                    request.len(),
                    context_address as *mut c_void,
                )
            };
            let invalid = returned.data.is_null() && returned.length != 0;
            let bytes = if invalid || returned.length == 0 {
                Vec::new()
            } else {
                // SAFETY: The callback contract provides a readable returned
                // range until its paired free callback is invoked below.
                unsafe { std::slice::from_raw_parts(returned.data, returned.length) }.to_vec()
            };
            let status = returned.status;
            // SAFETY: This is exactly the unchanged callback-owned buffer.
            unsafe { free_callback(returned, context_address as *mut c_void) };
            if invalid {
                return Err("native host returned a null buffer with nonzero length".into());
            }
            let text = String::from_utf8(bytes)
                .map_err(|error| format!("native host response is not UTF-8: {error}"))?;
            if status == 0 { Ok(text) } else { Err(text) }
        };
        nivren::runtime::Interpreter::new()
            .with_host_callback(host)
            .run_bytecode(&chunk)
            .map_err(|error| vec![error])
    })) {
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

/// Starts one Nivren program on a managed worker and returns immediately.
///
/// `complete` receives exactly one owned `NivrenBuffer`; it must eventually
/// release that buffer with `nivren_buffer_free`. After completion returns,
/// `wake` is called when supplied so an embedding event loop can schedule its
/// consumer without polling. Cancellation is cooperative between verified VM
/// instructions. The returned handle must be released with
/// `nivren_async_run_free`, which also joins the worker.
///
/// # Safety
///
/// Source bytes and callback pointers must satisfy the synchronous ABI
/// contracts for this call. Source is copied before return. `context` and both
/// callbacks must remain valid until `nivren_async_run_free` returns. Freeing a
/// handle from inside its own completion or wake callback is forbidden.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_run_async_utf8(
    source: *const u8,
    length: usize,
    complete: Option<NivrenAsyncComplete>,
    wake: Option<NivrenWake>,
    context: *mut c_void,
) -> *mut NivrenAsyncRun {
    let Some(complete) = complete else {
        return std::ptr::null_mut();
    };
    let source = if source.is_null() && length != 0 {
        let buffer = NivrenBuffer::error("null source pointer with nonzero length", 2);
        // SAFETY: The caller contract keeps the callback and context valid.
        unsafe { complete(buffer, context) };
        if let Some(wake) = wake {
            // SAFETY: The caller contract keeps the callback and context valid.
            unsafe { wake(context) };
        }
        return std::ptr::null_mut();
    } else if length == 0 {
        Vec::new()
    } else {
        // SAFETY: The caller provides a readable source range for this call;
        // copying it here ensures the worker never retains caller memory.
        unsafe { std::slice::from_raw_parts(source, length) }.to_vec()
    };
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = cancellation.clone();
    let context_address = context as usize;
    let worker = std::thread::Builder::new()
        .name("nivren-async".into())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                let source = std::str::from_utf8(&source)
                    .map_err(|error| format!("source is not UTF-8: {error}"))?;
                let tokens = nivren::lexer::scan(source).map_err(|errors| {
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n")
                })?;
                let program = nivren::parser::parse(tokens).map_err(|errors| {
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n")
                })?;
                nivren::typecheck::check(&program).map_err(|errors| {
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n")
                })?;
                let chunk = nivren::bytecode::compile(&program).map_err(|errors| {
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n")
                })?;
                nivren::runtime::Interpreter::new()
                    .with_cancellation(worker_cancellation)
                    .run_bytecode(&chunk)
                    .map(|value| value.to_string())
                    .map_err(|error| error.to_string())
            }));
            let buffer = match result {
                Ok(Ok(value)) => NivrenBuffer::new(value.into_bytes(), 0),
                Ok(Err(error)) => NivrenBuffer::error(&error, 1),
                Err(_) => NivrenBuffer::error("internal Nivren panic", 3),
            };
            // SAFETY: The exported function contract keeps callbacks and the
            // opaque context valid until the worker is joined.
            unsafe { complete(buffer, context_address as *mut c_void) };
            if let Some(wake) = wake {
                // SAFETY: Same callback lifetime contract as above.
                unsafe { wake(context_address as *mut c_void) };
            }
        });
    let worker = match worker {
        Ok(worker) => worker,
        Err(error) => {
            let buffer = NivrenBuffer::error(&format!("could not start Nivren worker: {error}"), 3);
            // SAFETY: The caller contract keeps the callback and context valid.
            unsafe { complete(buffer, context) };
            if let Some(wake) = wake {
                // SAFETY: The caller contract keeps the callback and context valid.
                unsafe { wake(context) };
            }
            return std::ptr::null_mut();
        }
    };
    Box::into_raw(Box::new(NivrenAsyncRun {
        cancellation,
        worker: Some(worker),
    }))
}

/// Requests cooperative cancellation of an asynchronous execution.
///
/// # Safety
/// `run` must be null or a live handle returned by `nivren_run_async_utf8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_async_run_cancel(run: *mut NivrenAsyncRun) {
    if let Some(run) = unsafe { run.as_ref() } {
        run.cancellation.store(true, Ordering::Release);
    }
}

/// Returns 1 after the worker has completed, otherwise 0.
///
/// # Safety
/// `run` must be null or a live handle returned by `nivren_run_async_utf8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_async_run_finished(run: *const NivrenAsyncRun) -> u32 {
    unsafe { run.as_ref() }
        .and_then(|run| run.worker.as_ref())
        .is_some_and(JoinHandle::is_finished) as u32
}

/// Cancels, joins, and releases an asynchronous execution handle.
///
/// # Safety
/// `run` must be null or a uniquely owned live handle returned by
/// `nivren_run_async_utf8`. It must not be called from that handle's callbacks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nivren_async_run_free(run: *mut NivrenAsyncRun) {
    if run.is_null() {
        return;
    }
    // SAFETY: The caller transfers unique ownership of its live handle.
    let mut run = unsafe { Box::from_raw(run) };
    run.cancellation.store(true, Ordering::Release);
    if let Some(worker) = run.worker.take() {
        let _ = worker.join();
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
    use super::{
        NivrenBuffer, nivren_abi_version, nivren_async_run_cancel, nivren_async_run_finished,
        nivren_async_run_free, nivren_buffer_free, nivren_check_utf8, nivren_compile_utf8,
        nivren_format_utf8, nivren_run_async_utf8, nivren_run_host_utf8, nivren_run_native_utf8,
        nivren_run_utf8,
    };
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::time::Duration;

    struct AsyncContext {
        sender: Sender<(u32, String)>,
        wakes: AtomicUsize,
    }

    unsafe extern "C" fn host(
        name: *const u8,
        name_length: usize,
        request: *const u8,
        request_length: usize,
        _: *mut c_void,
    ) -> NivrenBuffer {
        // SAFETY: Nivren supplies readable callback slices for this call.
        let name = unsafe { std::slice::from_raw_parts(name, name_length) };
        // SAFETY: Nivren supplies readable callback slices for this call.
        let request = unsafe { std::slice::from_raw_parts(request, request_length) };
        let mut response = name.to_vec();
        response.push(b':');
        response.extend_from_slice(request);
        NivrenBuffer::new(response, 0)
    }

    unsafe extern "C" fn host_free(buffer: NivrenBuffer, context: *mut c_void) {
        // SAFETY: The test context points to its live counter.
        unsafe { *(context.cast::<usize>()) += 1 };
        // SAFETY: This is the unchanged callback buffer returned above.
        unsafe { nivren_buffer_free(buffer) };
    }

    unsafe extern "C" fn invalid_host_buffer(
        _: *const u8,
        _: usize,
        _: *const u8,
        _: usize,
        _: *mut c_void,
    ) -> NivrenBuffer {
        NivrenBuffer {
            data: std::ptr::null_mut(),
            length: 1,
            capacity: 0,
            status: 0,
        }
    }

    unsafe extern "C" fn count_only_free(_: NivrenBuffer, context: *mut c_void) {
        // SAFETY: The test keeps its counter alive for the synchronous call.
        unsafe { *(context.cast::<usize>()) += 1 };
    }

    unsafe extern "C" fn nested_host_buffer(
        _: *const u8,
        _: usize,
        _: *const u8,
        _: usize,
        _: *mut c_void,
    ) -> NivrenBuffer {
        NivrenBuffer::new(br#"{"name":"Nivren","values":[20,22]}"#.to_vec(), 0)
    }

    unsafe extern "C" fn async_complete(buffer: NivrenBuffer, context: *mut c_void) {
        // SAFETY: The async test keeps this context alive through handle join.
        let context = unsafe { &*context.cast::<AsyncContext>() };
        let status = buffer.status;
        let text = output(&buffer);
        // SAFETY: Completion owns the unchanged Nivren result buffer.
        unsafe { nivren_buffer_free(buffer) };
        context.sender.send((status, text)).unwrap();
    }

    unsafe extern "C" fn async_wake(context: *mut c_void) {
        // SAFETY: The async test keeps this context alive through handle join.
        let context = unsafe { &*context.cast::<AsyncContext>() };
        context.wakes.fetch_add(1, Ordering::AcqRel);
    }

    fn output(buffer: &NivrenBuffer) -> String {
        // SAFETY: Tests inspect a live buffer returned by this crate.
        let bytes = unsafe { std::slice::from_raw_parts(buffer.data, buffer.length) };
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn c_boundary_runs_source_and_reports_invalid_input() {
        assert_eq!(nivren_abi_version(), 3);
        let source = b"20 + 22";
        // SAFETY: The byte slice remains readable for the call.
        let buffer = unsafe { nivren_run_utf8(source.as_ptr(), source.len()) };
        assert_eq!((buffer.status, output(&buffer)), (0, "42".into()));
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(buffer) };

        // SAFETY: The byte slice remains readable for the native call.
        let native = unsafe { nivren_run_native_utf8(source.as_ptr(), source.len()) };
        assert_eq!((native.status, output(&native)), (0, "42".into()));
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(native) };

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

        let invalid_source = b"keep value: Int = true";
        // SAFETY: The byte slice remains readable for the call.
        let checked = unsafe { nivren_check_utf8(invalid_source.as_ptr(), invalid_source.len()) };
        assert_eq!(checked.status, 1);
        assert!(!output(&checked).is_empty());
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(checked) };

        let messy = b"when true {\nkeep value=42\n}";
        // SAFETY: The byte slice remains readable for the call.
        let formatted = unsafe { nivren_format_utf8(messy.as_ptr(), messy.len()) };
        assert_eq!(formatted.status, 0);
        assert!(output(&formatted).contains("    keep value = 42"));
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(formatted) };

        // SAFETY: The byte slice remains readable for the call.
        let compiled = unsafe { nivren_compile_utf8(source.as_ptr(), source.len()) };
        assert_eq!(compiled.status, 0);
        // SAFETY: Tests inspect a live buffer returned by this crate.
        let compiled_bytes = unsafe { std::slice::from_raw_parts(compiled.data, compiled.length) };
        assert!(compiled_bytes.starts_with(b"NIVB"));
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(compiled) };

        let host_source = b"define call takes { } gives Result<String, String> needs Native { give std.host.invoke(\"echo\", \"hello\") } call()";
        let mut frees = 0usize;
        // SAFETY: Callback functions and the counter context remain live.
        let hosted = unsafe {
            nivren_run_host_utf8(
                host_source.as_ptr(),
                host_source.len(),
                Some(host),
                Some(host_free),
                (&mut frees as *mut usize).cast(),
            )
        };
        assert_eq!(
            (hosted.status, output(&hosted)),
            (0, "Ok(echo:hello)".into())
        );
        assert_eq!(frees, 1);
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(hosted) };

        let nested_source = br#"
shape Reply { name is String, values is [Int] }
define read takes { } gives Result<String, String> needs Native {
    keep response set std.host.invoke("nested", "request") or give
    keep reply set std.json.decode(Reply, response) or give
    give ok(reply.name)
}
read()
"#;
        let mut nested_frees = 0usize;
        // SAFETY: Callback functions and the counter remain live for the call.
        let nested = unsafe {
            nivren_run_host_utf8(
                nested_source.as_ptr(),
                nested_source.len(),
                Some(nested_host_buffer),
                Some(host_free),
                (&mut nested_frees as *mut usize).cast(),
            )
        };
        assert_eq!((nested.status, output(&nested)), (0, "Ok(Nivren)".into()));
        assert_eq!(nested_frees, 1);
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(nested) };

        let mut invalid_frees = 0usize;
        // SAFETY: Callback functions and the counter remain live for the call.
        let invalid_callback = unsafe {
            nivren_run_host_utf8(
                host_source.as_ptr(),
                host_source.len(),
                Some(invalid_host_buffer),
                Some(count_only_free),
                (&mut invalid_frees as *mut usize).cast(),
            )
        };
        assert_eq!(invalid_callback.status, 0);
        assert!(output(&invalid_callback).contains("null buffer with nonzero length"));
        assert_eq!(invalid_frees, 1);
        // SAFETY: This is the unchanged, not-yet-freed returned buffer.
        unsafe { nivren_buffer_free(invalid_callback) };
    }

    #[test]
    fn async_c_boundary_completes_wakes_and_cancels() {
        let (sender, receiver) = mpsc::channel();
        let context = Box::new(AsyncContext {
            sender,
            wakes: AtomicUsize::new(0),
        });
        let source = b"20 + 22";
        // SAFETY: Source, callbacks, and boxed context remain live until join.
        let run = unsafe {
            nivren_run_async_utf8(
                source.as_ptr(),
                source.len(),
                Some(async_complete),
                Some(async_wake),
                (&*context as *const AsyncContext).cast_mut().cast(),
            )
        };
        assert!(!run.is_null());
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            (0, "42".into())
        );
        // SAFETY: `run` is a live uniquely owned handle.
        unsafe { nivren_async_run_free(run) };
        assert_eq!(context.wakes.load(Ordering::Acquire), 1);

        let (sender, receiver) = mpsc::channel();
        let context = Box::new(AsyncContext {
            sender,
            wakes: AtomicUsize::new(0),
        });
        let source = b"define spin takes { } gives Nothing { repeat yes { none } give none } keep worker set start spin wait worker";
        // SAFETY: Source, callbacks, and boxed context remain live until join.
        let run = unsafe {
            nivren_run_async_utf8(
                source.as_ptr(),
                source.len(),
                Some(async_complete),
                Some(async_wake),
                (&*context as *const AsyncContext).cast_mut().cast(),
            )
        };
        assert!(!run.is_null());
        std::thread::sleep(Duration::from_millis(10));
        // SAFETY: `run` remains live until the free below.
        unsafe { nivren_async_run_cancel(run) };
        let (status, message) = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(status, 1);
        assert!(message.contains("task cancelled"));
        for _ in 0..2_000 {
            // SAFETY: `run` remains live throughout this polling loop.
            if unsafe { nivren_async_run_finished(run) } == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        // SAFETY: `run` is a live uniquely owned handle.
        unsafe {
            assert_eq!(nivren_async_run_finished(run), 1);
            nivren_async_run_free(run);
        }
        assert_eq!(context.wakes.load(Ordering::Acquire), 1);
    }
}
