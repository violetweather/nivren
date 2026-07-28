# Nivren Language Specification, Edition 4 (language-proof draft)

## 1. Status

This executable draft defines the Checkpoint 1 source surface for Edition 4. It is not a release specification. The Edition 4 conformance corpus and the six programs under `proofs/edition4` are normative for the language-proof checkpoint. Intent-graph optimization and external-effect plan semantics remain gated by Checkpoint 2 and MUST NOT be claimed by a Checkpoint 1 build.

## 2. Identity invariants

Edition 4 source states what it keeps, changes, takes, gives, needs, prepares, performs, starts, waits for, and chooses. Bindings and labeled values use words rather than assignment punctuation. Recoverable failure remains typed. Capabilities remain statically required. Resource and task lifetime remain scoped. Familiar literal, arithmetic, comparison, indexing, and member expressions are retained.

An implementation MUST reject duplicate or unknown derives and duplicate labels. For a callable declared in the same source unit, labels MUST exactly match its parameter or field names in declaration order. A formatter MUST be idempotent.

## 3. Grammar

```ebnf
binding        = "keep", identifier, ["is", type], "set", expression, ";"? ;
mutable        = "change", identifier, ["is", type], "set", expression, ";"? ;
reassignment   = "change", identifier, "to", expression, ";"? ;
function       = "define", identifier, [generic-parameters],
                 ["takes", "{", {parameter}, "}"],
                 ["gives", type, ["or", type]],
                 ["needs", need, {",", need}],
                 "{", {declaration}, "}" ;
parameter      = identifier, "is", type, ("," | ";")? ;
need           = identifier, ["within", string] ;
nominal-type   = "type", identifier, "from", type, ";"? ;
shape          = "shape", identifier, [generic-parameters], "holds", "{",
                 {field}, "}", ["with", derive, {",", derive}] ;
field          = identifier, "is", type, ("," | ";")? ;
choice         = "choice", identifier, [generic-parameters], "holds", "{",
                 case, {case}, "}" ;
case           = "case", identifier, ["carries", type], ("," | ";")? ;
type           = "maybe", type | identifier | "[", type, "]" |
                 identifier, "<", type, {",", type}, ">" ;
preparation    = "prepare", identifier, "as", identifier, "with", labeled-values ;
labeled-call   = postfix, "with", labeled-values ;
labeled-values = "{", {identifier, "set", expression, ("," | ";")?}, "}" ;
selection-arm  = "case", identifier, ["carries", identifier], "=>", expression ;
conditional    = "when", expression, statement, ["otherwise", statement] ;
iteration      = "each", identifier, "in", expression, statement ;
repetition     = "repeat", "while", expression, statement ;
unary          = "perform", unary | edition-three-unary ;
```

The built-in derive names are `Json`, `Compare`, `Display`, `Key`, `Validate`, `Binary`, `DatabaseRow`, and `Arguments`. Derive metadata is part of the checked declaration and gates the corresponding generated operation. `Key` also requires `Compare`. Data derives reject functions, secrets, iterators, resources, handles, tasks, and channels. `DatabaseRow` accepts scalar or nullable-scalar columns (including nominal scalar wrappers), while `Arguments` accepts command-line scalar or nullable-scalar fields. Diagnostics name the derive, field, and unsupported type. A declaration with an explicit derive list cannot use an omitted generated operation. Unadorned Edition 3 shapes retain their structural compatibility behavior while the Edition 4 proof is developed.

Labeled-call names are preserved in the checked syntax tree. Local functions and shapes use their declared parameters or fields; imported module exports retain that metadata and are checked at the call site. Official callables use the same canonical metadata catalog. A labeled call with missing metadata is rejected rather than silently reverting to positional behavior.

Scoped `needs` declarations preserve both the capability and boundary. Capabilities are drawn from the fixed capability vocabulary. Boundaries must be non-empty, bounded, and free of control characters; `Network` boundaries name hosts rather than URLs. This is static Language Proof validation only. Authorization and enforcement belong to the Intent Proof checkpoint.

`gives Value or Problem` denotes the same checked result type represented internally as `Result<Value, Problem>`. `maybe Value` denotes the standard optional type. Edition 4 source MUST NOT require `T?` or `Result<T, E>` spellings.

## 4. Checkpoint 1 lowering

The language-proof compiler lowers the new surface into the existing checked AST, tree interpreter, and bytecode VM. A nominal `type Name from Representation` is provisionally represented by a one-value nominal shape. Labeled calls preserve declaration order and lower to existing calls after label validation.

`prepare name as Shape with { ... }` provisionally constructs an immutable typed plan-shaped value. `perform value` marks the source boundary and evaluates to that value during Checkpoint 1. This provisional lowering exists only to execute grammar proof programs. It does not satisfy the zero-allocation intent graph, inspection, authorization, or effect-ordering requirements of Checkpoint 2.

## 5. Compatibility

Edition 3 remains executable during the proof checkpoint so its regression suite can protect the implementation foundation. Edition 4 does not promise source compatibility with Edition 3, and the final Edition 4 release will not advertise the older grammar as canonical.
