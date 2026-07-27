# Nivren Bytecode and Bundle Specification, Version 3

## 1. Conformance

A version-3 decoder MUST reject malformed, truncated, over-limit, unverifiable, or trailing data before execution. All integers are little-endian. A `count` is `u32` and MUST be no greater than 1,000,000. A string is `count` UTF-8 bytes. Recursive chunk nesting MUST NOT exceed 256.

An application bundle is ASCII `NIVB` followed by exactly one chunk. A chunk contains `u16 bytecode_version (= 3)`, `count instruction_count`, then that many instructions. Every instruction begins with `count line`, `count column`, a `u8` opcode, and its operands. Unsupported versions are rejected rather than inferred.

## 2. Literals and operators

Literal tags are `0 Int(i64)`, `1 Float(IEEE-754 u64 bits)`, `2 String`, `3 Bool(exactly 0 or 1)`, and `4 Null`.

Operator tags are `0 +`, `1 -`, `2 *`, `3 /`, `4 %`, `5 !`, `6 ==`, `7 !=`, `8 >`, `9 >=`, `10 <`, `11 <=`. Operators invalid for the containing unary/binary instruction fail verification.

## 3. Instructions

| Opcode | Name | Encoded operands | Abstract effect |
|---:|---|---|---|
| 0 | Constant | literal | push literal |
| 1 | Load | string | push binding |
| 2 | Store | string | assign and retain top |
| 3 | Define | string, bool | bind and retain top |
| 4 | Pop | — | pop |
| 5 | Unary | operator | replace top |
| 6 | Binary | operator | pop two, push result |
| 7 | Jump | count target | jump |
| 8 | JumpIfFalse | count target | inspect Bool top |
| 9 | Call | count arity | pop args/callee, push result |
| 10 | MakeArray | count | replace values with array |
| 11 | Index | — | pop index/collection, push element |
| 12 | Coalesce | count target | jump if top is non-null |
| 13 | Get | string | replace object with member |
| 14 | Print | — | print top, replace with Null |
| 15 | EnterScope | — | enter child scope |
| 16 | ExitScope | — | leave child scope |
| 17 | MakeFunction | string, strings, chunk | push closure |
| 18 | Return | — | return top |
| 19 | DefineRecord | string, field schemas | define/push shape constructor and runtime schema |
| 20 | DefineEnum | string, strings | define/push choice namespace |
| 21 | Match | arms | select arm, push result |
| 22 | DefineModule | string, chunk, strings | execute/push namespace |
| 23 | Iterate | string, chunk | run body for each value |
| 24 | Using | string, chunk | bind resource, run body, always close, push/return body result |
| 25 | Propagate | — | unwrap Ok or return the original Err |

`strings` is a count followed by strings. `field schemas` is a count followed by pairs of field-name string and canonical type-schema string. Canonical schemas preserve named, applied, array, nullable, and result type structure and are runtime data for strict shape-derived codecs. An arm is a variant string, optional-binding tag (`0` or `1` plus string), line, column, and chunk. Nested chunks repeat the version field and MUST be version 3.

## 4. Verification and execution

The VM recursively verifies version, tags, booleans, operators, limits, UTF-8, complete consumption, jump targets, operand depth, lexical-scope depth, control-flow joins, and required exit values. Function, module, iterator, using, and match-arm chunks are verified independently.

`Using` MUST perform cleanup on normal return, early return, propagation, and runtime failure. `Propagate` MUST accept only runtime `Result`; malformed bytecode encountering another value fails safely. Shape construction and JSON decoding MUST observe the same field order and schema metadata in tree and bytecode engines.

Bundles contain no native pointers or host layouts. Optimized execution MUST preserve values, output, errors, call frames, capability checks, resource cleanup, instruction charging, memory limits, and checked arithmetic.
