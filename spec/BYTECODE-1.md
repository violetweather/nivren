# Nivren Bytecode and Bundle Specification, Version 1

## 1. Conformance

A version-1 decoder MUST reject malformed, truncated, over-limit, unverifiable, or trailing data before execution. All integers below are little-endian. A `count` is `u32` and MUST be no greater than 1,000,000. A string is `count` UTF-8 bytes. Recursive chunk nesting MUST NOT exceed 256.

## 2. Bundle and chunk

An application bundle is ASCII `NIVB` followed by exactly one chunk and no trailing byte. A chunk is:

```text
u16 bytecode_version (= 1)
count instruction_count
instruction[instruction_count]
```

Every instruction begins with `count line`, `count column`, then a `u8` opcode and its operands. Line and column are source debug metadata and SHOULD be nonzero for compiler output.

## 3. Literals and operators

A literal is a tag plus payload:

| Tag | Value | Payload |
|---:|---|---|
| 0 | Int | `i64` |
| 1 | Float | IEEE-754 binary64 bits as `u64` |
| 2 | String | string |
| 3 | Bool | `u8`, exactly 0 or 1 |
| 4 | Null | none |

Operator tags are `0 +`, `1 -`, `2 *`, `3 /`, `4 %`, `5 !`, `6 ==`, `7 !=`, `8 >`, `9 >=`, `10 <`, `11 <=`. An operator not valid for the containing `Unary`/`Binary` instruction MUST fail verification.

## 4. Instructions

| Opcode | Name | Encoded operands | Abstract effect |
|---:|---|---|---|
| 0 | Constant | literal | push literal |
| 1 | Load | string name | push resolved binding |
| 2 | Store | string name | assign top, retain top |
| 3 | Define | string name, bool mutable | bind top, retain top |
| 4 | Pop | — | pop |
| 5 | Unary | operator | pop one, push result |
| 6 | Binary | operator | pop right/left, push result |
| 7 | Jump | count target | set instruction pointer |
| 8 | JumpIfFalse | count target | inspect Bool top; jump if false |
| 9 | Call | count arity | pop arity args and callee; push result |
| 10 | MakeArray | count length | pop length values; push array |
| 11 | Index | — | pop index/collection; push element |
| 12 | Coalesce | count target | jump if top is non-null |
| 13 | Get | string member | pop object; push member |
| 14 | Print | — | pop value, print with LF, push Null |
| 15 | EnterScope | — | enter lexical child scope |
| 16 | ExitScope | — | leave lexical scope |
| 17 | MakeFunction | string name, strings params, chunk body | push closure |
| 18 | Return | — | pop and return from current chunk call |
| 19 | DefineRecord | string name, strings fields | define constructor/type, push it |
| 20 | DefineEnum | string name, strings variants | define enum namespace, push it |
| 21 | Match | count arms, arm records | pop subject, run selected arm, push result |
| 22 | DefineModule | string name, chunk body, strings exports | execute module, push namespace |
| 23 | Iterate | string binding, chunk body | pop iterable, run body per element, push last/Null |

`strings` is a count followed by that many strings. A match arm is variant string, a `u8` optional-binding tag (0 none, 1 followed by string), line count, column count, then a chunk.

## 5. Verification

Before execution a conforming VM MUST recursively verify:

- version, tags, booleans, operators, limits, UTF-8, and complete consumption;
- all jump/coalesce targets are within the containing chunk or its legal end boundary;
- every reachable instruction has sufficient operand-stack depth;
- `ExitScope` never underflows the lexical scope depth;
- all control-flow edges entering one instruction agree on stack and scope depth;
- every reachable normal chunk exit has a value available where required;
- nested function, module, iterator, and arm chunks independently satisfy these rules.

Verification is not a substitute for source type checking. A hostile but structurally typed-mismatched bundle MUST still fail safely with a runtime diagnostic, never memory unsafety.

## 6. Execution compatibility

Bytecode version 1 is portable across supported OS/architectures. Native pointers, endianness, and host object layouts MUST NOT appear in a bundle. Optimizers MAY replace instruction sequences only if source-visible values, output, errors, call frames, and checked arithmetic remain equivalent. Unknown future bytecode versions MUST be rejected rather than guessed.
