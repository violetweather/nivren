# Nivren WebAssembly host

Nivren ships portable `wasm32-wasip1` and zero-import `wasm32-unknown-unknown` modules containing the Edition 4 frontend, Bytecode 7 compiler, and portable virtual machine. They are intended for build tools, browser playgrounds, editors, and sandboxed application hosts. The real-host parity suite executes Edition 4 shapes, choices, labeled calls, `prepare`, and `perform` on both targets. The modules do not silently emulate unavailable operating-system facilities: TLS, WebSockets, dynamic libraries, native JIT, and other host-only resources report typed Nivren errors.

## Build and verify

```sh
rustup target add wasm32-wasip1
cargo build -p nivren-wasm --target wasm32-wasip1 --release --locked
node tools/test_wasm_host.mjs

rustup target add wasm32-unknown-unknown
cargo build -p nivren-wasm --target wasm32-unknown-unknown --release --locked
node tools/test_wasm_browser.mjs
```

Release files are `nivren-VERSION-wasm32-wasip1.wasm`, `nivren-VERSION-browser.wasm`, and `nivren-VERSION-browser.mjs`. Release CI builds both modules twice, compares their bytes, exercises real WASI and zero-import browser hosts, checksums every file, and includes them in provenance attestations.

## Browser SDK

```js
import { Nivren } from "./nivren-VERSION-browser.mjs";

const nivren = await Nivren.instantiate("./nivren-VERSION-browser.wasm");
nivren.check("keep answer: Int = 42");
console.log(nivren.run("40 + 2"));
```

`Nivren.check`, `format`, `compile`, and `run` own all allocation/free steps. Language, host-input, and internal failures throw `NivrenError` with the ABI status. The complete static example is under `examples/browser`.

## ABI 1

The module exports `memory`, `nivren_wasm_abi_version`, `nivren_wasm_alloc`, `nivren_wasm_free`, `nivren_wasm_check`, `nivren_wasm_format`, `nivren_wasm_compile`, and `nivren_wasm_run`.

Hosts allocate UTF-8 input with `nivren_wasm_alloc`, copy it into guest memory, call an operation with `(pointer, length)`, and free the input. An operation returns one `u64`:

- bits 63–60: status (`0` success, `1` Nivren diagnostic/runtime error, `2` invalid host input, `3` internal failure)
- bits 59–32: output length
- bits 31–0: output pointer

The host must copy and then free non-empty output with the exact returned pointer and length. Inputs and outputs are limited to 16 MiB. A nonzero-length null pointer is rejected. Exported calls catch panics at the ABI boundary.

`check` returns no bytes on success and UTF-8 diagnostics on failure. `format` returns UTF-8 source. `compile` returns verified NIVB bytecode. `run` checks, compiles, and executes source, returning the displayed value as UTF-8.

The executable integrations are `tools/test_wasm_host.mjs` and `tools/test_wasm_browser.mjs`; the latter tests the public SDK used by `examples/browser`. Both are part of CI.
