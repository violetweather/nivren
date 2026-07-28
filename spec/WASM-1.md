# Nivren WASM ABI 1

This document specifies the stable Nivren WebAssembly embedding ABI version 1.

## Target and exports

Distribution targets are `wasm32-wasip1` for WASI Preview 1 hosts and `wasm32-unknown-unknown` for zero-import browser hosts. A conforming module exports linear `memory` and these unmangled functions:

```text
nivren_wasm_abi_version() -> u32
nivren_wasm_alloc(length: u32) -> u32
nivren_wasm_free(pointer: u32, length: u32)
nivren_wasm_check(pointer: u32, length: u32) -> u64
nivren_wasm_format(pointer: u32, length: u32) -> u64
nivren_wasm_compile(pointer: u32, length: u32) -> u64
nivren_wasm_run(pointer: u32, length: u32) -> u64
```

`nivren_wasm_abi_version` returns `1`. Inputs are UTF-8. `alloc` returns zero for a zero or unsupported length. A successful nonzero allocation is owned by the host until passed exactly once to `free`. No input range may be changed during an operation.

## Results

Operation results pack status in bits 63–60, byte length in bits 59–32, and a guest-memory pointer in bits 31–0. Status values are success `0`, language diagnostic/runtime error `1`, invalid host input `2`, and internal failure `3`; values 4–15 are reserved. Non-empty results are guest allocations transferred to the host and must be released exactly once. Empty results have length zero and require no free.

An implementation accepts no input or result larger than 16 MiB. It rejects nonzero-length null input, invalid UTF-8, and ranges the host cannot lawfully make readable. It contains panics at the exported boundary.

`check` produces empty success or UTF-8 diagnostics. `format` produces canonical UTF-8 source for the declared edition, including Edition 4. `compile` produces verified NIVB bytecode. `run` checks, compiles, and executes source in the portable VM and produces the value display as UTF-8.

## Capabilities

The portable VM preserves normal Nivren capability checking and resource budgets. Facilities absent from the WASI guest—currently native JIT, dynamic libraries, TLS, and WebSockets—must fail explicitly and must not downgrade security or claim success. Hosts may further restrict WASI imports.

The browser distribution MUST have an empty WebAssembly import list. Its JavaScript SDK MUST validate ABI version 1, enforce the same 16 MiB ceiling before allocation, copy every result before freeing it, free every non-empty allocation exactly once, and surface nonzero statuses as typed JavaScript errors.
