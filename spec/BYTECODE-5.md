# Nivren Bytecode 5

Bytecode 5 is the portable stack format for Nivren Edition 3. It retains all Bytecode 4 instruction semantics, hostile-input bounds, recursive verification, and little-endian bundle encoding, with two instructions required by protocol members.

| ID | Instruction | Operands | Effect |
| --- | --- | --- | --- |
| 26 | `DefineProtocol` | protocol name, ordered member names | define and push an immutable protocol namespace |
| 27 | `AdoptProtocol` | protocol name, adopted type schema, ordered member/implementation-name pairs | install one checked coherent dispatch table and push confirmation |

The source checker proves member completeness, function-signature compatibility, capability compatibility, the orphan rule, and one runtime-coherent adoption per nominal type before emitting either instruction. Execution resolves a protocol member using the qualified protocol identity, member name, and receiver's nominal runtime type. A missing mapping is a runtime error rather than an unchecked call.

The verifier rejects unsupported bytecode versions before execution. Bytecode 4 and Bytecode 5 bundles are not interchangeable. All limits and choice-constructor rules from `BYTECODE-4.md` remain normative.
