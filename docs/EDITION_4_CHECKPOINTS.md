# Edition 4 checkpoint ledger

This ledger prevents later Edition 4 work from bypassing the agreed proof gates. A checkpoint is complete only when every blocking row is passing. Edition 4 remains local and unpublished until Product Proof passes.

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
- `prepare` and `perform` currently use the explicitly provisional Checkpoint 1 lowering described by the draft specification. They do not count as Intent Proof.
- The first structural formatter pass split `<=` and merged adjacent assignment statements. Operator joining and executable-vs-data block layout were corrected; all 29 Edition 3 examples and six Edition 4 proofs then passed after formatting.

### Final Language Proof gate

- `cargo fmt --all --check`: passing.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passing.
- `cargo test --workspace --all-targets --locked`: passing with only the explicitly ignored live-Redis release test.
- Bench evidence: VM 784.602 ms; tiered 560.310 ms; JIT speedup 1.400x; recursive tree 56.150 ms; recursive VM 36.950 ms; recursive speedup 1.520x; repeated record fields 585.130 ms.
- Stop-and-correct triggers: none remain active. Language Proof is closed and later work may now begin at Intent Proof without weakening this gate.

## Intent Proof — ready to begin

Language Proof passed. Intent Proof has not started.

## Compiler Proof — not started

Blocked on Intent Proof.

## Product Proof — not started

Blocked on Compiler Proof.
