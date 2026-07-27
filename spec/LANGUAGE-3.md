# Nivren Language Specification, Edition 3 (working draft)

## 1. Status

This document is the normative source-language definition for the Edition 3 work toward Nivren 1.0. `MUST`, `MUST NOT`, `SHOULD`, and `MAY` have their RFC 2119 meanings. Edition 2 remains documented separately. An Edition 3 implementation MUST reject static violations before execution, verify bytecode before execution, and agree with the Edition 3 black-box conformance suite.

## 2. Design invariants

Nivren uses one intent-first vocabulary. Bindings are `keep` or `change`; functions are `define` and exit with `give`; decisions are `when`/`otherwise`; iteration is `each`/`within`; data declarations are `shape` and `choice`; exhaustive selection is `choose`. New constructs MUST use words when a word communicates intent more clearly than punctuation.

There is no truthiness, implicit numeric conversion, unchecked integer overflow, implicit ambient capability, unscoped task lifetime, or implicit fallthrough from a failed `Result`.

## 3. Lexical additions

Edition 3 adds the reserved words `needs`, `through`, `start`, `wait`, `together`, `race`, `using`, `protocol`, `adopt`, and `for`. The Edition 2 lexical rules for UTF-8, identifiers, comments, strings, numeric literals, source positions, and nesting limits otherwise apply.

## 4. Grammar additions

The following productions replace or extend their Edition 2 counterparts.

```ebnf
function       = "define", identifier, [generic-parameters], "(", [parameters], ")",
                 ["gives", type], ["needs", capability, {",", capability}],
                 "{", {declaration}, "}" ;
generic-parameters = "<", generic-parameter, {",", generic-parameter}, ">" ;
generic-parameter  = identifier, [":", protocol] ;
protocol-declaration = "protocol", identifier,
                 ["{", protocol-member, {protocol-member}, "}"], ";"? ;
protocol-member = "define", identifier, "(", parameters, ")",
                 "gives", type, ["needs", capability, {",", capability}], ";"? ;
protocol-adoption  = "adopt", identifier, "for", type,
                 ["{", protocol-mapping, {("," | ";"), protocol-mapping}, "}"], ";"? ;
protocol-mapping = identifier, "=", identifier ;
shape          = "shape", identifier, [generic-parameters], "{", field,
                 {("," | ";"), field}, ["," | ";"], "}" ;
choice         = "choice", identifier, [generic-parameters], "{", variant,
                 {("," | ";"), variant}, ["," | ";"], "}" ;
field          = identifier, ":", type ;
variant        = identifier, ["(", type, ")"] ;
type           = (identifier | "[", type, "]" |
                 identifier, "<", type, {",", type}, ">"), ["?"] ;
statement      = using | show | give | when | repeat | each | block | expression, ";"? ;
using          = "using", identifier, "=", expression, statement ;
coalesce       = pipeline, ["??", coalesce] ;
pipeline       = or, {"through", pipeline-stage} ;
pipeline-stage = identifier | member-expression | call-expression ;
or             = and, {"or", ("give" | and)} ;
unary          = ("!" | "-" | "start" | "wait" | "together" | "race"), unary | postfix ;
```

`or give` terminates the nearest function with the original `Err` value. It is valid only within a function returning `Result<_, E>`, its operand MUST be `Result<T, E2>`, and `E2` MUST be compatible with `E`. Its expression type is `T`.

`value through transform` is equivalent to `transform(value)`. `value through transform(a, b)` is equivalent to `transform(value, a, b)`. The input is evaluated once and before explicit stage arguments.

`start callable`, `wait task`, `together tasks`, and `race tasks` have the typed semantics specified in `STANDARD-LIBRARY-3.md`.

## 5. Types and generic protocols

Edition 3 includes Edition 2 types plus immutable `Bytes`, `Map<K, V>`, and `Set<T>`. `Task`, `Channel`, and `TcpStream` are nameable opaque types. Generic function parameters are inferred independently at each call. Every occurrence of one generic parameter in a call MUST resolve compatibly.

The initial sealed protocols are:

- `Comparable`: stable structural equality and suitability as a map key or set element.
- `Number`: `Int` or `Float`, supporting checked/defined arithmetic for that type.
- `Ordered`: values with a deterministic total language ordering.
- `Iterable`: values that produce a finite or streaming sequence through an iterator API.
- `Closable`: resources eligible for deterministic `using` cleanup.
- `Sendable`: values safe to transfer between tasks.

A type argument that does not satisfy its declared protocol is a static error. Protocol names are part of the edition surface; unknown protocols are errors.

Edition 3 protocols use `protocol Name` and `adopt Name for Type`. A protocol MAY be an empty semantic marker or declare required `define` signatures. Each required member MUST take `Self` as its first parameter. An adoption of a protocol with members MUST map every member exactly once to a compatible in-scope function; parameter, result, and capability requirements are checked. Calls use `Protocol.member(receiver, ...)` and are statically available only when the receiver satisfies that protocol. Runtime dispatch uses the qualified protocol identity, member, and nominal receiver type; missing dispatch is a checked runtime error rather than an unchecked call.

Protocols and adoptions are module scoped. An adoption is unique for its fully qualified `(protocol, type)` pair, and at most one applied form of a generic nominal type may occupy the same erased runtime dispatch slot. The orphan rule requires the adopting package to own either the protocol or the nominal type. Built-in safety protocols are sealed and cannot be adopted explicitly. These rules provide coherence across separately authored packages.

`Map` and `Set` are persistent values: update operations return a new collection and MUST NOT modify the input. Keys/elements MUST satisfy `Comparable`. Iteration order is insertion order until a future edition explicitly introduces another collection with different ordering.

`Bytes` is an immutable byte sequence. Byte indexes and slice endpoints use nonnegative `Int` values. Text decoding is explicit and fallible.

### Payload choices

A sealed `choice` variant is either a value or a one-argument constructor:

```nivren
choice Response {
    Text(String),
    Integer(Int),
    Array([Response]),
    Null
}
```

`Response.Null` is a value. `Response.Text("ready")` and `Response.Array(values)` construct values with exactly one statically checked payload. Payload variants MUST be called with one compatible value; bare variants MUST NOT be called. A `choose` arm for a payload variant MUST bind that payload, while an arm for a bare variant MUST NOT bind one. Selection remains exhaustive and evaluates only the selected arm. Recursive payloads are permitted through nominal choice or shape references and remain subject to runtime depth and resource limits at consuming boundaries.

Shapes and choices accept the same type-parameter and protocol-constraint syntax as functions:

```nivren
shape Pair<Left, Right> { left: Left, right: Right }
choice Maybe<Value> { Some(Value), None }
```

Constructor calls infer type arguments from their values. Applied annotations such as `Pair<String, Int>` and `Maybe<Int>` MUST supply exactly the declared number of arguments and satisfy every declared constraint. Applied nominal types with the same declaration but incompatible arguments are distinct static types. Runtime representation MAY erase type arguments after successful checking, but observable tree, bytecode, reflection, serialization, and FFI behavior MUST remain type safe.

## 6. Checked capabilities

A function that performs an effect declares it after its result type:

```nivren
define load(path: String) gives Result<String, String> needs FileRead {
    give std.files.read(path)
}
```

Capability requirements are part of function types and propagate through direct calls and spawned callables. Calling an effectful function without declaring every required capability is a static error. The sealed capabilities are `FileRead`, `FileWrite`, `Environment`, `Time`, `Process`, `Network`, `Task`, `Channel`, `Log`, `Native`, and `Random`.

Projects grant ambient execution permission separately in `niv.toml`:

```toml
[capabilities]
FileRead = "allow"
```

Missing project grants MUST be rejected at the runtime boundary even for verified bytecode. Standalone snippets and embedding hosts MAY supply their own explicit policy; the default embedding policy is documented by the embedding API.

## 7. Structured concurrency

`start` creates a task from a zero-argument callable. A task has one owner and may be awaited once. `wait` joins one task. `together` joins all tasks in array order and returns their values in that order. `race` returns the lowest-index completed task when completion is observed, cancels all losers, and joins them before leaving the scope.

Dropping a task requests cancellation and joins it. Cancellation is observed at bytecode instruction boundaries. No task may silently outlive its owner. Only `Sendable` values cross a task or channel boundary.

## 8. Deterministic resources

`using name = resource statement` evaluates the resource once, binds it immutably in a child scope, executes the statement, and closes the resource exactly once when the scope exits. Cleanup occurs on normal completion, `give`, `or give`, and runtime error. If both the body and close fail, the body failure is primary. Closing a resource requires its associated capability.

Edition 3 defines this protocol for `TcpStream`, files, locks, transactions, native handles, and dynamic libraries. Every closable resource MUST adopt the same observable rules.

## 9. Execution and compatibility

Interpreter, verified bytecode, bundles, and optimized execution MUST have identical observable behavior. Edition 3 bytecode uses format version 4, including shape schema and payload-choice metadata. A loader MUST reject unsupported versions rather than guess.

Edition 3 is not frozen while this document is marked working draft. A compatibility freeze requires an Edition 3 conformance corpus, supported-platform results, security/resource-limit evidence, and the release gates in `ROADMAP.md`.
