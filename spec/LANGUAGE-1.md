# Nivren Language Specification, Edition 1 (0.9 draft)

## 1. Status and terminology

This document is the normative definition of Nivren Edition 1 source behavior. “MUST”, “MUST NOT”, “SHOULD”, and “MAY” have their RFC 2119 meanings. `docs/LANGUAGE.md` is explanatory; where it differs from this document, this document governs.

A conforming implementation MUST accept every valid program described here, reject every constraint violation before execution unless identified as a runtime error, and satisfy the observable behavior in the edition-1 conformance suite. Resource exhaustion MAY terminate translation or execution with a diagnostic rather than continue unsafely.

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

Reserved words are `and`, `else`, `enum`, `export`, `false`, `for`, `fun`, `if`, `import`, `in`, `let`, `match`, `null`, `or`, `print`, `record`, `return`, `true`, `var`, and `while`.

## 3. Syntactic grammar

The following EBNF is normative. `;?` means an optional semicolon.

```ebnf
program       = { declaration } ;
declaration   = binding | function | record | enum | import | export | statement ;
binding       = ("let" | "var"), identifier, [":", type], "=", expression, ";"? ;
function      = "fun", identifier, "(", [parameters], ")",
                ["->", type], "{", {declaration}, "}" ;
parameters    = parameter, {",", parameter} ;
parameter     = identifier, [":", type] ;
record        = "record", identifier, "{", [fields], "}" ;
fields        = field, {("," | ";"), field}, ["," | ";"] ;
field         = identifier, ":", type ;
enum          = "enum", identifier, "{", variants, "}" ;
variants      = identifier, {("," | ";"), identifier}, ["," | ";"] ;
import        = "import", string, ";"? ;
export        = "export", "{", identifier, {",", identifier}, "}", ";"? ;

type          = (identifier | "[", type, "]" |
                "Result", "<", type, ",", type, ">"), ["?"] ;

statement     = print | return | if | while | for | block | expression, ";"? ;
print         = "print", ("(", expression, ")" | expression), ";"? ;
return        = "return", [expression], ";"? ;
if            = "if", "(", expression, ")", statement,
                ["else", statement] ;
while         = "while", "(", expression, ")", statement ;
for           = "for", "(", identifier, "in", expression, ")", statement ;
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
primary       = integer | float | string | "true" | "false" | "null" |
                identifier | array | match | "(", expression, ")" ;
array         = "[", [arguments], "]" ;
match         = "match", "(", expression, ")", "{", match-arms, "}" ;
match-arms    = match-arm, {("," | ";"), match-arm}, ["," | ";"] ;
match-arm     = identifier, ["(", identifier, ")"], "=>", expression ;
```

Functions and calls MUST contain no more than 255 parameters/arguments. An enum and export list MUST be nonempty. Only a bare identifier is a legal assignment target.

## 4. Values and types

Edition 1 has `Int`, `Float`, `String`, `Bool`, `Null`, `[T]`, `T?`, `Result<T,E>`, nominal records, sealed enums, functions, modules, tasks, channels, and opaque standard-library handles.

- `Int` is two’s-complement signed 64-bit. Addition, subtraction, multiplication, division, remainder, and negation MUST detect overflow. Division/remainder by zero is a runtime error.
- `Float` is IEEE-754 binary64. No implicit numeric conversions exist.
- `String` is an immutable sequence of Unicode scalar values.
- `[T]` is an immutable homogeneous sequence. Empty arrays initially have unknown element type and MUST acquire a compatible contextual type before an operation requiring `T`.
- `T?` contains either a `T` or `null`. Plain `T` excludes `null`.
- Records and enums are nominal; same-shaped declarations are not interchangeable.

Bindings infer the initializer type unless annotated. `let` cannot be assigned after initialization; `var` can be assigned only a value compatible with its established type. References are lexically scoped. Duplicate declarations in one scope and unresolved names are static errors.

Function parameter and return annotations MAY be omitted in the 0.9 draft, in which case unknown types are checked wherever subsequently constrained. Edition 1 public exported functions SHOULD provide complete annotations; 1.0 makes that requirement mandatory.

## 5. Operators and evaluation

Operands evaluate left-to-right except `and`, `or`, and `??`, which short-circuit.

- `+` accepts two `Int`, two `Float`, or two `String` values.
- `-`, `*`, `/`, `%`, `<`, `<=`, `>`, `>=` accept two values of the same numeric type.
- `!` accepts `Bool`; unary `-` accepts `Int` or `Float`.
- `and` and `or` accept `Bool` and return `Bool`.
- `==` and `!=` compare values structurally where their types are compatible. Function/native/handle identity has no portable equality guarantee.
- `nullable ?? fallback` requires a `T?` left side and compatible `T` fallback and returns `T`.

Conditions MUST be `Bool`; there is no truthiness. Array/string indexes MUST be nonnegative `Int`; string indexing counts Unicode scalars. Out-of-bounds indexing is a runtime error.

Arguments evaluate left-to-right before invocation. Closures capture their lexical environment. `return` exits the nearest function; a top-level return is invalid. A function reaching its closing brace returns `null`.

## 6. Control flow and matching

Blocks create lexical scopes. Each `for` iteration creates a fresh scope and immutable iteration binding. Arrays iterate in index order; strings iterate by Unicode scalar.

An enum match MUST contain exactly one arm for every declared variant and no unknown variant. Enum arms do not take payload bindings. `Result<T,E>` matching MUST contain exactly `Ok(binding)` and `Err(binding)`. The selected arm alone evaluates, in a child scope containing its payload binding where applicable. All arm values MUST have compatible result types.

## 7. Records, enums, and modules

A record declaration creates a nominal type and positional constructor in field declaration order. Construction MUST have exactly the field count/types. Fields are immutable.

An enum declaration creates a namespace whose members are the declared singleton variants. Variants are referenced as `Type.Variant`.

`import "path.niv"` resolves relative to the importing file. The namespace is the imported filename stem. `import "@name"` resolves an installed, declared package dependency and uses `name` as its namespace. Imports MUST remain inside the project root (whose `.niv` dependency store is part of that confinement), be loaded once, and form an acyclic graph. Module declarations are private unless named in exactly one `export { ... }` declaration. Exporting an undeclared or duplicate name is a static error.

## 8. Projects and observable execution

A project contains strict `niv.toml` package metadata as specified by `spec/PACKAGE-1.md`. Translation order MUST NOT change observable behavior or deterministic artifacts. `print` writes the display representation followed by LF to standard output. Diagnostics go to standard error and include source path, one-based line/column, severity, and message; wording beyond conformance-required substrings is not normative.

Static errors prevent execution. Runtime errors terminate the current top-level execution with a nonzero status and source-located call frames. Optimized/JIT execution MUST be behaviorally identical to verified bytecode execution.

## 9. Compatibility

The Edition 1 grammar and semantics freeze begins with 0.9. Changes that reject previously valid Edition 1 source or alter its defined observable behavior require a new edition or a documented defect correction with migration. Additive standard-library APIs and diagnostic improvements are compatible. Bytecode and package compatibility are governed separately by their embedded format versions.
