# Edition 4 checkpoint ledger

This ledger prevents a stable Edition 4 release from bypassing the agreed proof gates. A checkpoint is complete only when every blocking row is passing. Working evaluation builds may be published as explicitly labeled beta prereleases, but they do not close Product Proof or authorize a stable release.

## Language Proof — passed 2026-07-28

| Evidence | Status | Current result |
| --- | --- | --- |
| Edition 3 baseline preserved | Passing | Full workspace matrix: 18 library tests, 3 conformance suites, 154 language tests passing with one live-Redis test intentionally ignored, 4 property tests, benchmark execution, 2 FFI tests, 3 JIT tests, 1 native test, and 1 Wasm test |
| Edition 4 grammar executes | Passing | Tree interpreter and Bytecode 6 VM execute bindings, vertical functions, results, nominal wrappers, shapes, choices, labeled calls, generated derives, preparation syntax, control flow, protocols, and scoped-needs syntax |
| Black-box conformance | Passing | `conformance/edition4-language-proof.json` passes alongside retained Edition 2 and Edition 3 vectors |
| Six proof programs | Passing | Every program under `proofs/edition4` checks before and after canonical formatting; network/native proof programs remain static at Language Proof as specified |
| Labeled argument safety | Passing | Labels survive the AST; local declarations, imported exports, core functions, compatibility aliases, and every typed standard-library function validate exact canonical names/order; catalog coverage and arity are checked automatically |
| Derive behavior | Passing | All eight derives enforce dependencies/field constraints and generate labeled methods; JSON, comparison, display, key, validation, deterministic binary, strict row, and argument decoding agree in the tree and Bytecode 6 engines |
| Scoped source capabilities | Passing for Language Proof | Fixed capability vocabulary, bounded scope metadata, host-only Network validation, and generated documentation pass; runtime authorization remains gated by Intent Proof |
| Canonical formatter | Passing | Spacing, clauses, structural line layout, four-space indentation, comments, strings, and operators are deterministic; compact/vertical equivalence, arbitrary-source idempotence, 29 Edition 3 examples, and six Edition 4 proof programs pass after formatting |
| Usability corpus | Passing | Twelve paired tasks type-check at a measured 1.069 median token ratio, below the 1.15 limit; six executable maintenance cases add no conceptual steps and reduce recorded ambiguous choices |
| Actionable diagnostics | Passing for implemented forms | Canonical bindings, nominal wrappers, shapes, choices, preparation, pipelines, scopes, protocols, adoptions, derives, and label order have correction-oriented assertions |
| Clippy warnings denied | Passing | The complete workspace passes Rust 1.97 clippy with warnings denied |

### Stop-and-correct record

- The first lexer revision made `set` and `from` globally reserved and broke existing package APIs and a valid local binding. They were corrected to contextual uses before work continued.
- Labeled calls initially discarded their names after parsing. Same-unit callable metadata and exact canonical-order validation were added before the proof corpus was accepted.
- `prepare` and `perform` used provisional Checkpoint 1 lowering when Language Proof closed. Intent Proof subsequently replaced that lowering with preserved AST boundaries and Bytecode 7 `Prepare`, `Perform`, and fused `PerformCall` semantics without reopening or weakening the Language Proof gate.
- The first structural formatter pass split `<=` and merged adjacent assignment statements. Operator joining and executable-vs-data block layout were corrected; all 29 Edition 3 examples and six Edition 4 proofs then passed after formatting.

### Final Language Proof gate

- `cargo fmt --all --check`: passing.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passing.
- `cargo test --workspace --all-targets --locked`: passing with only the explicitly ignored live-Redis release test.
- Bench evidence: VM 784.602 ms; tiered 560.310 ms; JIT speedup 1.400x; recursive tree 56.150 ms; recursive VM 36.950 ms; recursive speedup 1.520x; repeated record fields 585.130 ms.
- Stop-and-correct triggers: none remain active. Language Proof is closed and later work may now begin at Intent Proof without weakening this gate.

## Intent Proof — passed 2026-07-28

| Evidence | Status | Current result |
| --- | --- | --- |
| Typed intent graph | Passing | `prepare`, `perform`, and `through` survive parsing; `org.nivren.intent.v1` records typed operation, allocation, authority, resources, cancellation, retries, timeout policy, buffering, blocking, fusion, target, effect order, and portability decisions |
| Pure zero-allocation lowering | Passing | Pure pipelines compile to the same ordinary operations as direct calls; property and operation-equivalence tests report zero pure runtime plan allocations |
| Explicit plans and portability | Passing | Bytecode 7 records one immutable materialized plan per `prepare`; only self-contained literal data plans produce `org.nivren.portable-plan.v1` serialization, while variables, handles, callbacks, secrets, authority, and effects are rejected |
| Visible effect boundary | Passing | Direct `perform` calls fuse into `PerformCall` without an extra dispatch; stored plans use `Perform`; graph validation rejects effect calls outside a visible boundary |
| Authority and effect order | Passing | Native capability and scope authorization completes before an effect enters runtime metrics; denied effects leave no sequence entry; authorized file/environment effects retain source order |
| Pipeline execution model | Passing | Verified pure fusion, bounded `std.list.batch`, structured parallel/race tasks, lazy streams, bounded channels, backpressure, and serial effect stages are exercised without plan allocation for pure work |
| `niv explain` | Passing | File and project commands support optimized and `--no-optimize` graphs; output is deterministic, validates canonically, and matches a reviewed snapshot |
| Files, HTTP, database, concurrency | Passing | Each domain appears with its real checker-owned capability/resource metadata; runtime integration tests cover bounded I/O, transactions, tasks, channels, slow-consumer backpressure, cancellation, cleanup, and injected failure |
| Throughput and latency | Passing | Release gate ratios versus direct forms: files 0.864, loopback HTTP 0.856, database 0.973, concurrency 0.989; every result is below the 1.10 ceiling |
| Memory | Passing | Conservative allocation-work ratio is 1.000 for all four performance domains; pure plans allocate zero runtime plan objects |
| Optimization equivalence | Passing | Generated programs execute identically with optimized and non-optimized intent graphs; direct and pipeline pure bytecode operations are equivalent |
| Fuzz/property safety | Passing | 512-case arbitrary-source schedules cannot panic graph construction; generated graphs validate; denied generated effects never execute or enter the effect sequence |
| Baseline preservation | Passing | Complete workspace matrix passes with 22 library tests, 3 conformance suites, 7 intent integration tests, 154 language tests plus one intentionally ignored live-Redis test, 6 property tests, both benchmark programs, 2 FFI, 3 JIT, 1 native, and 1 Wasm test |

### Stop-and-correct record

- The first runtime lowering emitted a separate `Perform` instruction after each call. Files, HTTP, and database passed, but bounded-channel concurrency regressed by 23.8%, activating the performance stop gate.
- The compiler was corrected to emit one fused `PerformCall` instruction with identical call stack behavior and visible-boundary metrics. The rerun produced a 0.989 concurrency ratio. The 10% threshold was not relaxed.
- Labeled pipeline stages were initially validated as complete calls before the pipeline inserted their first value. Parser suffix validation plus canonical first-label insertion corrected this without allowing incomplete ordinary calls.
- A generated denied-effect property initially called its effectful wrapper outside `perform`. Graph validation stopped the matrix; the property was corrected and the full suite rerun.

### Final Intent Proof gate

- `cargo fmt --all --check`: passing.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passing.
- `cargo test --workspace --all-targets --locked`: passing with only the explicitly ignored live-Redis release test.
- `NIVREN_INTENT_BENCH_GATE=1 cargo bench --bench intent_proof --locked`: passing all four time and memory ratios.
- Deterministic explain snapshot, optimized/non-optimized properties, unauthorized-effect properties, batching/parallel/backpressure tests, Bytecode 7 bundle verification, and existing cleanup/cancellation failure suites: passing.
- Stop-and-correct triggers: none remain active. Intent Proof is closed and Compiler Proof may now begin without weakening this gate.

## Compiler Proof — passed 2026-07-28

Complete-program Cranelift control now covers every verified Bytecode 7 construct through one checked native helper ABI, with no VM fallback. VM/native results and typed failures agree for Edition 4 proof programs, shapes, choices, collections, closures, generics, protocols, intent, structured concurrency, capabilities, limits, cancellation, and deterministic resource cleanup. Complete deterministic AOT objects, native standalone executables, ABI v3 static/shared embedding libraries, browser-Wasm, and WASI are covered by executable gates.

The initial per-instruction trace failed performance at `1.766` times the tiered baseline. Bounded verified helper regions corrected the crossing cost to `0.988`; the threshold remained `1.10`. The safe-Rust kernel ratio is `0.967` against a `2.0` ceiling, and the native application ratio is `0.206` against Node with a `1.5` ceiling. Full evidence and the stop-and-correct record are in `docs/EDITION_4_COMPILER_PROOF_AUDIT.md`.

## Product Proof — not started

Compiler Proof passed. Product work may begin. Beta prereleases may collect real installation and usage evidence while the Product Proof matrix remains open; stable publication stays blocked until that matrix passes.
