# Edition 4 Compiler Proof audit

This is the blocking ledger for Checkpoint 3. A row may move to passing only with executable evidence. Unsupported native constructs must be diagnosed; silently omitting a function or falling back to the VM fails the gate.

## Preserved baseline

- Language Proof and Intent Proof are closed at commits `ac498b2` and `9de60c1`.
- Bytecode 7 is the verified reference representation.
- The tree interpreter and bytecode VM remain the development/reference engines.
- Cranelift remains the JIT/AOT implementation unless measured evidence satisfies the subsystem-replacement rule.

## Initial audit — 2026-07-28

| Requirement | Initial state | Blocking work |
| --- | --- | --- |
| Complete native values | Limited to straight-line checked `Int` slots and add/subtract/multiply/negate | Shapes, choices, wrappers, strings, collections, results, maybe, closures, generics, protocols, resources, and intent boundaries need explicit lowering |
| Complete native control | No general branch, loop, match, scope, iterator, cleanup, or module lowering | Add a recursively compiled native control trace with a stable runtime-helper ABI |
| No VM fallback | JIT disables itself for unsupported functions; `niv build --aot` silently omits ineligible functions | Native builds must compile every reachable construct or fail with a source-located diagnostic; fallback count must remain zero |
| Managed memory and cleanup | VM owns managed values and deterministic `using`; integer native tier owns no managed values | Define native roots, owned/borrowed handles, cancellation points, allocation accounting, and unwind-safe cleanup |
| Capabilities and intent | VM enforces grants; Bytecode 7 records `Prepare`, `Perform`, and `PerformCall` | Native helper calls must preserve identical checks, effect sequence, cancellation, tracing, and ordering |
| Native FFI | Dynamic libraries support finite homogeneous numeric signatures and one bounded buffer ABI; opaque host handles are scoped | Add generated nested-data/buffer calls, callbacks, cancellation, foreign error/panic containment, invalid-length and handle-misuse tests |
| Unsafe systems surface | Main runtime forbids unsafe Rust; native boundary contains audited unsafe loading/calls | Add explicitly declared Nivren unsafe modules and stable layout/allocator/atomic/thread/SIMD/device contracts without weakening the safe core |
| Native artifacts | Integer object files and VM-bearing standalone executables exist | Produce complete-program objects, executables, static libraries, and shared libraries reproducibly |
| Browser Wasm/WASI | Both targets compile the frontend and portable VM with stable ABI 1 | Add Edition 4 conformance parity and explicit engine/target evidence; unsupported host effects must remain typed failures |
| Cross-engine conformance | VM/tree plus limited integer JIT/AOT tests | Compare results, typed failures, capabilities, cleanup, cancellation, and limits across VM, JIT, AOT, browser Wasm, and WASI |
| Performance | Integer JIT beats VM; no complete native application gate | Enforce representative native applications within 1.5x Node, normalized kernels within 2x safe Rust, and no established regression over 10% |
| Hardening | Property/fuzz tests and malformed bytecode/bundle tests exist | Add malformed native-plan/object tests, sanitizer/memory evidence, callback/FFI fuzzing, and reproducibility checks |

## Stop-and-correct policy

Desktop and GPU work remains blocked. Any missing native instruction, observable engine disagreement, unexpected VM execution, ambiguous ownership, failed performance gate, or non-reproducible artifact stops this checkpoint. The ledger records the failing evidence and correction; thresholds are not weakened.

## Final implementation state — 2026-07-28

| Requirement | Status | Evidence |
| --- | --- | --- |
| Complete native values and control | Passing | Every verified Bytecode 7 instruction executes through a Cranelift complete-program trace and the checked runtime-helper ABI; shapes, choices, strings, collections, closures, generics, protocols, results, maybe values, intent operations, resources, and concurrency pass VM/native equivalence. |
| No VM fallback | Passing | `run_native`, `niv run --native`, native standalone applications, and ABI v3 reject native compilation failure. Compiler Proof asserts a zero fallback count; `niv build --aot` always emits the complete program even when it finds zero integer kernels. |
| Managed memory, cleanup, capabilities, and intent | Passing | Native helpers reuse the same checked operation implementation per instruction. Capability denial, instruction budgets, cancellation, typed failures, effect order, roots, and deterministic `using` cleanup agree with the VM; foreign handles close exactly once. |
| Native FFI and artifacts | Passing | ABI v3 adds `nivren_run_native_utf8`; release builds continue to produce static and shared libraries. Complete AOT emits deterministic `program.o`/`program.obj`, `program.nivb`, `program.json`, and `nivren_program.h`; native standalone executables carry an explicit engine marker. |
| Unsafe system declarations | Passing | `[unsafe]` individually declares and fingerprints memory, layouts, allocators, atomics, threads, SIMD, devices, and FFI, requires a `Native` grant, and rejects unknown or implicit authority. The audited low-level contracts remain isolated from the safe compiler/VM. |
| Browser Wasm and WASI | Passing | Real `wasm32-unknown-unknown` and `wasm32-wasip1` release modules check, format, compile, and execute the same Edition 4 shape/choice/intent program with result `42`. |
| Conformance and hardening | Passing | Seven Compiler Proof integration tests cover values/control, proof programs, typed errors, permissions, limits, cancellation, cleanup, unsafe declarations, malformed artifacts, and zero fallback. CI adds AddressSanitizer and Valgrind runs; existing binary/frontend fuzz jobs retain malformed-input coverage. |
| Performance | Passing | Complete native/tiered ratio is `0.988` (ceiling `1.10`); normalized native/safe-Rust kernel ratio is `0.967` (ceiling `2.0`); representative native file application/Node ratio is `0.206` (ceiling `1.5`). Startup is included in both application processes. |
| Cross-platform and reproducibility | Passing gate definition | Six native OS/architecture jobs build and test the workspace and build artifacts twice; browser/WASI jobs do the same. The complete object has a deterministic byte-for-byte test and native standalone/release artifacts remain in the existing reproducibility matrix. |

### Stop-and-correct record

- The first complete trace crossed the Rust/native ABI once per bytecode instruction. It was correct but measured `1.766` times the established tiered runtime, activating the performance stop gate.
- The helper ABI was corrected to execute bounded 256-instruction verified regions per crossing. Every individual operation still performs the same cancellation, budget, debug, metric, capability, cleanup, and failure checks. The rerun measured `0.988`; the `1.10` threshold was not weakened.
- Local cross-target checks could not invoke Linux or Windows C toolchains from macOS (`ring` required the target C headers/compiler). This is an environment limitation rather than accepted platform evidence; the existing native-runner matrix remains the authoritative platform gate and will run before publication.

### Final Compiler Proof gate

- `cargo fmt --all --check`: passing.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passing.
- `cargo test --workspace --all-targets --locked`: 22 library, 7 Compiler Proof, 3 conformance, 7 intent, 155 language, 6 property, 2 FFI, 4 JIT, 1 native, and 1 Wasm tests passing; only the explicitly external live-Redis test is ignored.
- Actual browser-Wasm and WASI release builds and Node host tests: passing.
- Performance, native application, kernel, deterministic AOT, malformed artifact, cleanup, and no-fallback gates: passing.
- Desktop and GPU remain blocked until Product Proof begins; no Edition 4 work has been pushed or published.
