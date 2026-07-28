# Nivren Language Specification, Edition 4 (intent-proof draft)

## 1. Status

This executable draft defines the passed Language Proof and Intent Proof source semantics for Edition 4. It is not a release specification. The Edition 4 conformance corpus, six application proofs, intent snapshot, and checkpoint ledger are normative evidence. Complete native compilation remains gated by Checkpoint 3.

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

Derives generate labeled shape methods: `to_json`/`from_json`, `compare`, `display`, `key`, `validate`, `to_binary`/`from_binary`, `from_row`, and `from_arguments`, respectively. JSON and binary methods return typed failures; the binary Language Proof representation is the deterministic UTF-8 JSON representation and remains versionable. `from_row` consumes a strict JSON row object. `from_arguments` accepts `--name=value` entries, rejects duplicates and unexpected or missing fields, and recognizes nullable fields plus checked string, boolean, and numeric values. Generated methods preserve identical metadata and behavior in the tree interpreter and Bytecode 7 VM.

Labeled-call names are preserved in the checked syntax tree. Local functions and shapes use their declared parameters or fields; imported module exports retain that metadata and are checked at the call site. Official callables use the same canonical metadata catalog. A labeled call with missing metadata is rejected rather than silently reverting to positional behavior.

The canonical formatter removes layout-only blank lines, emits four-space block indentation, places structural braces deterministically, keeps executable statements on separate lines, and compacts fields or labeled values within their data boundary before applying the structural layout. Function `takes`, `gives`, and `needs` clauses receive deterministic clause breaks. Strings and nested block or line comments are never interpreted as structure. Formatting is idempotent, preserves comments, and maps compact and vertical spellings of the same checked token sequence to one representation.

Scoped `needs` declarations preserve both the capability and boundary. Capabilities are drawn from the fixed capability vocabulary. Boundaries must be non-empty, bounded, and free of control characters; `Network` boundaries name hosts rather than URLs. The runtime enforces these grants before an effect enters the authorized effect sequence, and `niv explain` reports the capability and resource flow.

`gives Value or Problem` denotes the same checked result type represented internally as `Result<Value, Problem>`. `maybe Value` denotes the standard optional type. Edition 4 source MUST NOT require `T?` or `Result<T, E>` spellings.

## 4. Intent semantics

The compiler preserves `prepare`, `perform`, and `through` in the checked tree. A nominal `type Name from Representation` is represented by a one-value nominal shape. Labeled calls preserve declaration order; a labeled pipeline stage receives the pipeline value as its canonical first label.

`prepare name as Shape with { ... }` materializes one immutable, typed, record-backed plan. The Bytecode 7 `Prepare` marker makes this allocation visible to runtime metrics. A prepared plan is portable only when inspection proves it contains data rather than a handle, secret, callback, local authority, or effect. Tooling MUST refuse to describe any other plan as serializable.

`perform expression` is the visible external-effect boundary. Direct calls lower to one fused `PerformCall` instruction, preserving the boundary without a second VM dispatch; performing a stored plan uses `Perform`. Capability and scope authorization occurs before the runtime records or executes the effect. Denied effects therefore cannot enter the effect sequence.

`through` preserves source-to-sink order. Pure stages lower to the same optimized operations as their direct-call form and allocate zero runtime plans. Verified pure stages may be fused. Effectful stages remain serial, preserve typed failures, cancellation, cleanup and tracing, and may not be reordered. `std.list.batch` supplies bounded batching, structured task operations supply parallel/race stages, iterators and bounded channels provide streaming and backpressure.

`niv explain` emits deterministic `org.nivren.intent.v1` JSON. It reports allocation, capabilities, resources, cancellation, retries, timeout policy, buffering, blocking, target choice, fusion, source effect order, and portability. The graph rejects non-canonical metadata, reordered effects, pure-plan allocation, and effects outside `perform`.

## 5. Compatibility

Edition 3 remains executable during the proof checkpoint so its regression suite can protect the implementation foundation. Edition 4 does not promise source compatibility with Edition 3, and the final Edition 4 release will not advertise the older grammar as canonical.
