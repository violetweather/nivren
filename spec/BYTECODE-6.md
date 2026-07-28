# Nivren Bytecode 6

Bytecode 6 is the Edition 4 Language Proof bundle format. It extends Bytecode 5 shape declarations with an ordered list of checked built-in derives. The list follows the field schema in the `DefineRecord` payload and is encoded with the ordinary bounded string-list representation.

The VM preserves this metadata on the runtime shape schema. Generated derive methods therefore behave identically in the tree interpreter and bytecode VM, including after deterministic bundle encoding and decoding. Unknown derive names are rejected by the source parser before bytecode generation; the bytecode verifier continues to enforce the global item, string, nesting, stack, and control-flow limits.

Bytecode 5 and Bytecode 6 are deliberately not interchangeable. All instructions and safety rules not changed here remain as specified by `BYTECODE-5.md`.
