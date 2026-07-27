# Nivren Bytecode 4

Bytecode 4 is the portable stack format for Nivren Edition 3. It retains the instruction semantics, limits, recursive verification rules, and little-endian bundle encoding specified by Bytecode 3, with one incompatible extension to choice declarations.

`DefineEnum` now stores, in order:

1. the qualified choice name;
2. the complete ordered variant-name list; and
3. the ordered subset of variant names that require one payload.

The verifier rejects unsupported bytecode versions before execution. A Bytecode 3 decoder must not accept a Bytecode 4 bundle, and a Bytecode 4 decoder must not reinterpret Bytecode 3 bytes. Payload constructors have arity one; bare variants have arity zero and are values rather than constructors. Both tree and bytecode execution preserve the payload for exhaustive `choose` binding.

All length, nesting, instruction, string, and allocation bounds from `BYTECODE-3.md` remain normative.
