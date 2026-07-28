# Embedding Nivren

Nivren ships `nivren.h` plus shared and static libraries on every tier-one native release. ABI version 3 preserves the existing UTF-8 compiler/run surface and adds complete-program native execution without VM fallback.

## Schema-driven C and C++ views

Declare the messages shared with a host using ordinary Nivren shapes and choices, then generate a deterministic header:

```text
niv bindgen c messages.niv generated/messages.h
```

```nivren
choice Role { Admin, Member }
shape Address { city: String, postal: U32 }
shape User { name: String, role: Role, address: Address?, tags: [String] }
```

The header uses fixed-width C integers, `(pointer, length)` borrowed views for strings and bytes, typed enums for choices, forward-declared immutable pointers for nested shapes, explicit `has_value` fields for nullable values, and `(pointer, length)` array views. `BigInt`, `Decimal`, and `DateTime` remain lossless string views. Shapes that cannot have a direct C layout use an explicit JSON view rather than an implicit or platform-dependent representation.

Generated headers are C11 and C++17 compatible, deterministic for the same checked source, and contain no allocation or hidden ownership. The caller owns backing storage and keeps every view alive for the receiving call. Release CI compiles the public and generated headers with warnings as errors on all six tier-one OS/architecture jobs.

## Synchronous execution

`nivren_check_utf8`, `nivren_format_utf8`, `nivren_compile_utf8`, and `nivren_run_utf8` copy no caller input after returning. Results are owned `NivrenBuffer` values and must be released exactly once with `nivren_buffer_free`.

`nivren_run_native_utf8` checks and compiles the same source, then executes every verified bytecode construct through Cranelift native control. Unsupported compilation is a checked status-1 result; it never redirects to `nivren_run_utf8`. The direct integer JIT remains an optimized kernel tier inside native execution.

`nivren_run_host_utf8` installs a synchronous host callback for `std.host.invoke`, `std.host.open`, `std.host.call`, and `std.host.close`. Nivren copies a callback response before invoking its paired host free callback. Long-lived native identifiers remain opaque inside `NativeHandle` and are deterministically released by `using`.

Nivren programs can load a C dynamic library directly with `std.native.open`. `NativeLibrary` owns the loader handle, keeps every resolved symbol scoped to one call, and is deterministically unloaded by `using` or `std.native.close`. Edition 4 exposes finite all-`int64_t` and all-`double` signatures of zero through six arguments plus a fixed pointer/length buffer ABI. `call_buffer` lends immutable input and initialized bounded output only for the duration of the call, validates the returned length, and copies out owned `Bytes`. The operation is capability-gated and deliberately trusts that the library export matches the selected ABI; generated shape/choice views remain the inspectable typed schema layer.

## Async execution and wakeup

`nivren_run_async_utf8` copies source immediately, starts a managed worker, and returns an opaque `NivrenAsyncRun*`. Its completion callback receives exactly one owned result buffer. After completion returns, the optional wake callback runs so a GUI, server, or custom event loop can schedule its consumer without polling.

```c
NivrenAsyncRun *run = nivren_run_async_utf8(
    source, source_length, on_complete, wake_loop, context
);
```

- `nivren_async_run_cancel` requests cooperative cancellation between verified VM instructions.
- `nivren_async_run_finished` is a nonblocking status probe for hosts that need one.
- `nivren_async_run_free` requests cancellation, joins the worker, and releases the handle.
- The context and callbacks must remain live until `nivren_async_run_free` returns.
- A callback must not free its own run handle; enqueue completion, return, then free it from the awakened event-loop thread.
- Completion owns its `NivrenBuffer` and must eventually call `nivren_buffer_free` exactly once.

This bridge is deliberately small: it integrates Nivren execution with an existing host loop without prescribing a particular C, C++, Rust, GUI, or server runtime. Blocking native operations remain bounded by their API timeouts; cancellation is cooperative rather than thread termination.

## Status and compatibility

`nivren_abi_version()` returns `3`. Status `0` is success, `1` is a checked language/runtime error, `2` is invalid host input, and `3` is a caught internal panic. New ABI versions add symbols without changing the layout or ownership of existing versioned contracts; consumers should feature-detect the reported version before calling newer symbols.
