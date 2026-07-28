# Experimental Nivren mobile embedding SDKs

These wrappers expose ABI v3 to iOS and Android without exposing Rust ownership or raw Nivren buffers to application code. They are experimental Product Proof inputs, not supported mobile releases.

## iOS

`ios/NivrenMobile.swift` accepts a Swift `String`, passes its exact UTF-8 bytes to `nivren_run_utf8` or `nivren_run_native_utf8`, copies the bounded result, and always calls `nivren_buffer_free`. Build the existing static `nivren_ffi` library for the selected Apple target, expose `crates/nivren-ffi/include/nivren.h` through the supplied `CNivren` module map, and link it into the application target.

## Android

`android/NivrenMobile.kt` converts Kotlin text to an exact UTF-8 `ByteArray`. `android/nivren_mobile_jni.c` copies that bounded array into native memory, calls ABI v3, converts the owned result back to a JVM byte array, frees every native allocation, and raises a Java exception for typed/compiler/host failures. Build the C shim and `nivren_ffi` with the Android NDK, then package the matching `libnivren_mobile.so` for each ABI.

## Lifecycle and security

- Reject ABI versions below 3 and inputs/results above 16 MiB.
- Run compilation and execution away from the UI thread.
- Use `nivren_run_async_utf8` for cooperative cancellation in a platform host; a production wrapper must map app/background lifecycle to cancel/join/free before mobile support can graduate.
- Treat Native host callbacks as an explicit authority boundary. Never pass platform secrets through logs or untrusted web content.
- Test low-memory termination, background/foreground transitions, cancellation, Unicode, malformed source, and repeated initialization on real devices.

Edition 4 does not promise full native mobile UI frameworks. These SDKs embed the compiler/runtime into a host application; the host still owns platform UI, signing, sandbox entitlements, networking, and store packaging.
