# Edition 4 checkpoint ledger

This ledger prevents later Edition 4 work from bypassing the agreed proof gates. A checkpoint is complete only when every blocking row is passing. Edition 4 remains local and unpublished until Product Proof passes.

## Language Proof — in progress

| Evidence | Status | Current result |
| --- | --- | --- |
| Edition 3 baseline preserved | Passing | Full workspace matrix: 17 library tests, 3 conformance suites, 146 language tests passing with one live-Redis test intentionally ignored, 3 property tests, benchmark execution, FFI, JIT, native, and Wasm tests |
| Edition 4 grammar executes | Passing, provisional | Tree interpreter and bytecode VM execute bindings, vertical functions, results, nominal wrappers, shapes, choices, labeled calls, preparation syntax, control flow, and scoped-needs syntax |
| Black-box conformance | Passing, provisional | `conformance/edition4-language-proof.json` |
| Six proof programs | Passing, static | All programs under `proofs/edition4` check; network/native proof programs are not executed during Language Proof |
| Labeled argument safety | Partial | Labels survive the AST; same-unit functions/shapes and imported module exports validate exact names/order; complete standard-library metadata remains open |
| Derive behavior | Partial | All eight names, dependencies, field constraints, deterministic diagnostics, Compare/Json operation gates, and dual-engine positive evidence pass; generated Validate/Binary/DatabaseRow/Arguments entry points remain open |
| Scoped source capabilities | Passing for Language Proof | Fixed capability vocabulary, bounded scope metadata, host-only Network validation, and generated documentation pass; runtime authorization remains gated by Intent Proof |
| Canonical formatter | Partial | Edition 4 spacing, indentation, comments, strings, and operators have deterministic output; example and arbitrary-source idempotence properties pass; canonical line breaking remains open |
| Usability corpus | Passing, provisional | Twelve paired tasks type-check and remain within the 1.15 median token budget; maintenance-choice evidence remains open |
| Clippy warnings denied | Passing | The complete workspace passes Rust 1.97 clippy with warnings denied |

### Stop-and-correct record

- The first lexer revision made `set` and `from` globally reserved and broke existing package APIs and a valid local binding. They were corrected to contextual uses before work continued.
- Labeled calls initially discarded their names after parsing. Same-unit callable metadata and exact canonical-order validation were added before the proof corpus was accepted.
- `prepare` and `perform` currently use the explicitly provisional Checkpoint 1 lowering described by the draft specification. They do not count as Intent Proof.

### Remaining Language Proof blockers

1. Preserve label metadata through modules and provide complete official-callable labels.
2. Implement generated behavior and constraints for all built-in derives.
3. Implement a comment-preserving canonical Edition 4 formatter.
4. Complete maintenance-edit evidence and actionable diagnostic coverage.
5. Rerun the entire checkpoint matrix and record final measurements.

## Intent Proof — not started

Blocked on Language Proof.

## Compiler Proof — not started

Blocked on Intent Proof.

## Product Proof — not started

Blocked on Compiler Proof.
