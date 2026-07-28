# Nivren Bytecode 7

Bytecode 7 is the Edition 4 Intent Proof bundle format. It extends Bytecode 6 with three instructions:

- `Prepare(type)` leaves the typed payload on the stack and records one explicitly materialized immutable plan.
- `Perform` leaves a stored-plan value on the stack and records its visible execution boundary.
- `PerformCall(arity)` has the exact stack and call semantics of `Call(arity)` while also recording the visible execution boundary. It prevents effect-heavy code from paying for a second VM dispatch.

The encoder tags these instructions as 28, 29, and 30. All are bounded by the existing string/count rules. `Prepare` and `Perform` have stack effect zero; `PerformCall(n)` has stack effect `-n`, identical to `Call(n)`. The verifier rejects unknown tags and continues to validate every nested chunk and control-flow join.

Runtime metrics expose plan allocations, perform boundaries, conservative allocation work, and the authorized native-effect sequence. Capability and scoped-authority checks occur before an effect is appended to that sequence. Pure `through` lowering emits ordinary direct-call operations and no plan instruction.

Bytecode 6 and Bytecode 7 are deliberately not interchangeable. All instructions and safety rules not changed here remain as specified by `BYTECODE-6.md`.
