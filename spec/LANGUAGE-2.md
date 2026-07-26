# Nivren Language Specification, Edition 2 (0.10 draft)

## 1. Status and terminology

This document is the normative definition of Nivren Edition 2 source behavior. “MUST”, “MUST NOT”, “SHOULD”, and “MAY” have their RFC 2119 meanings. `docs/LANGUAGE.md` is explanatory; where it differs from this document, this document governs.

A conforming implementation MUST accept every valid program described here, reject every constraint violation before execution unless identified as a runtime error, and satisfy the observable behavior in the Edition 2 conformance suite. Resource exhaustion MAY terminate translation or execution with a diagnostic rather than continue unsafely.

## 2. Source text and lexical grammar

Source is UTF-8. A byte-order mark is not part of the grammar. Lines are separated by LF; implementations MAY accept CRLF and treat CR before LF as whitespace. Diagnostic lines and columns are one-based Unicode scalar positions.

```ebnf
identifier-start    = "_" | Unicode Alphabetic ;
identifier-continue = identifier-start | ASCII digit ;
identifier          = identifier-start, { identifier-continue } ;
integer             = ASCII digit, { ASCII digit } ;
float               = integer, ".", ASCII digit, { ASCII digit } ;
string              = '"', { string-character | escape }, '"' ;
escape              = "\\n" | "\\r" | "\\t" | '\\"' | "\\\\" |
                      "\\", any Unicode scalar ;
line-comment        = "//", { scalar other than LF } ;
block-comment       = "/*", { scalar | block-comment }, "*/" ;
```

Whitespace and comments separate tokens and are otherwise insignificant. Block comments nest. Strings may contain literal newlines. The five specified escapes map to LF, CR, tab, quotation mark, and backslash; for compatibility, a backslash before another scalar yields that scalar. Unterminated strings/comments and integers outside signed 64-bit range MUST be diagnosed.

Reserved words are `and`, `change`, `choice`, `choose`, `define`, `each`, `expose`, `give`, `gives`, `keep`, `no`, `none`, `or`, `otherwise`, `repeat`, `shape`, `show`, `use`, `when`, `within`, and `yes`.

## 3. Syntactic grammar

The following EBNF is normative. `;?` means an optional semicolon.

```ebnf
program       = { declaration } ;
declaration   = binding | function | shape | choice | use | expose | statement ;
binding       = ("keep" | "change"), identifier, [":", type], "=", expression, ";"? ;
function      = "define", identifier, "(", [parameters], ")",
                ["gives", type], "{", {declaration}, "}" ;
parameters    = parameter, {",", parameter} ;
parameter     = identifier, [":", type] ;
shape         = "shape", identifier, "{", [fields], "}" ;
fields        = field, {("," | ";"), field}, ["," | ";"] ;
field         = identifier, ":", type ;
choice        = "choice", identifier, "{", variants, "}" ;
variants      = identifier, {("," | ";"), identifier}, ["," | ";"] ;
use           = "use", string, ";"? ;
expose        = "expose", "{", identifier, {",", identifier}, "}", ";"? ;

type          = (identifier | "[", type, "]" |
                "Result", "<", type, ",", type, ">"), ["?"] ;

statement     = show | give | when | repeat | each | block | expression, ";"? ;
show          = "show", ("(", expression, ")" | expression), ";"? ;
give          = "give", [expression], ";"? ;
when          = "when", expression, statement,
                ["otherwise", statement] ;
repeat        = "repeat", expression, statement ;
each          = "each", identifier, "within", expression, statement ;
block         = "{", {declaration}, "}" ;

expression    = assignment ;
assignment    = coalesce, ["=", assignment] ;
coalesce      = or, ["??", coalesce] ;
or            = and, {"or", and} ;
and           = equality, {"and", equality} ;
equality      = comparison, {("==" | "!="), comparison} ;
comparison    = term, {("<" | "<=" | ">" | ">="), term} ;
term          = factor, {("+" | "-"), factor} ;
factor        = unary, {("*" | "/" | "%"), unary} ;
unary         = ("!" | "-"), unary | postfix ;
postfix       = primary, { call | index | member } ;
call          = "(", [arguments], ")" ;
arguments     = expression, {",", expression} ;
index         = "[", expression, "]" ;
member        = ".", identifier ;
primary       = integer | float | string | "yes" | "no" | "none" |
                identifier | array | choose | "(", expression, ")" ;
array         = "[", [arguments], "]" ;
choose        = "choose", expression, "{", choose-arms, "}" ;
choose-arms   = choose-arm, {("," | ";"), choose-arm}, ["," | ";"] ;
choose-arm    = identifier, ["(", identifier, ")"], "=>", expression ;
```

Functions and calls MUST contain no more than 255 parameters/arguments. A choice and expose list MUST be nonempty. Only a bare identifier is a legal assignment target.

## 4. Values and types

Edition 2 has `Int`, `Float`, `String`, `Bool`, `Null`, `[T]`, `T?`, `Result<T,E>`, nominal shapes, sealed choices, functions, modules, tasks, channels, and opaque standard-library handles.

- `Int` is two’s-complement signed 64-bit. Addition, subtraction, multiplication, division, remainder, and negation MUST detect overflow. Division/remainder by zero is a runtime error.
- `Float` is IEEE-754 binary64. No implicit numeric conversions exist.
- `String` is an immutable sequence of Unicode scalar values.
- `[T]` is an immutable homogeneous sequence. Empty arrays initially have unknown element type and MUST acquire a compatible contextual type before an operation requiring `T`.
- `T?` contains either a `T` or `none`. Plain `T` excludes `none`.
- Records and enums are nominal; same-shaped declarations are not interchangeable.

Bindings infer the initializer type unless annotated. `keep` cannot be assigned after initialization; `change` can be assigned only a value compatible with its established type. References are lexically scoped. Duplicate declarations in one scope and unresolved names are static errors.

Function parameter and result annotations MAY be omitted in the 0.10 draft, in which case unknown types are checked wherever subsequently constrained. Edition 2 public exposed functions SHOULD provide complete annotations; 1.0 makes that requirement mandatory.

## 5. Operators and evaluation

Operands evaluate left-to-right except `and`, `or`, and `??`, which short-circuit.

- `+` accepts two `Int`, two `Float`, or two `String` values.
- `-`, `*`, `/`, `%`, `<`, `<=`, `>`, `>=` accept two values of the same numeric type.
- `!` accepts `Bool`; unary `-` accepts `Int` or `Float`.
- `and` and `or` accept `Bool` and return `Bool`.
- `==` and `!=` compare values structurally where their types are compatible. Function/native/handle identity has no portable equality guarantee.
- `nullable ?? fallback` requires a `T?` left side and compatible `T` fallback and returns `T`.

Conditions MUST be `Bool`; there is no truthiness. Array/string indexes MUST be nonnegative `Int`; string indexing counts Unicode scalars. Out-of-bounds indexing is a runtime error.

Arguments evaluate left-to-right before invocation. Closures capture their lexical environment. `give` exits the nearest function; a top-level `give` is invalid. A function reaching its closing brace returns `none`.

## 6. Control flow and matching

Blocks create lexical scopes. Each `each` iteration creates a fresh scope and immutable iteration binding. Arrays iterate in index order; strings iterate by Unicode scalar.

A choice selection MUST contain exactly one arm for every declared variant and no unknown variant. Choice arms do not take payload bindings. Choosing a `Result<T,E>` MUST contain exactly `Ok(binding)` and `Err(binding)`. The selected arm alone evaluates, in a child scope containing its payload binding where applicable. All arm values MUST have compatible result types.

## 7. Shapes, choices, and modules

A shape declaration creates a nominal type and positional constructor in field declaration order. Construction MUST have exactly the field count/types. Fields are immutable.

A choice declaration creates a namespace whose members are the declared singleton variants. Variants are referenced as `Type.Variant`.

`use "path.niv"` resolves relative to the using file. The namespace is the used filename stem. `use "@name"` resolves an installed, declared package dependency and uses `name` as its namespace. Used modules MUST remain inside the project root (whose `.niv` dependency store is part of that confinement), be loaded once, and form an acyclic graph. Module declarations are private unless named in exactly one `expose { ... }` declaration. Exposing an undeclared or duplicate name is a static error.

## 8. Projects and observable execution

A project contains strict `niv.toml` package metadata as specified by `spec/PACKAGE-1.md`. Translation order MUST NOT change observable behavior or deterministic artifacts. `show` writes the display representation followed by LF to standard output; booleans display as `yes` or `no`, and absence displays as `none`. Diagnostics go to standard error and include source path, one-based line/column, severity, and message; wording beyond conformance-required substrings is not normative.

Static errors prevent execution. Runtime errors terminate the current top-level execution with a nonzero status and source-located call frames. Optimized/JIT execution MUST be behaviorally identical to verified bytecode execution.

## 9. Compatibility

The Edition 2 grammar and semantics freeze begins with 0.10. Changes that reject previously valid Edition 2 source or alter its defined observable behavior require a new edition. Because Nivren had no users before Edition 2, earlier prototype spellings have no compatibility or migration guarantee. Additive standard-library APIs and diagnostic improvements are compatible. Bytecode and package compatibility are governed separately by their embedded format versions.
