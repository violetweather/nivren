# Nivren Language Guide

This guide describes the implemented Nivren 0.9 beta language. Edition 1 is frozen for compatibility testing; `spec/LANGUAGE-1.md` is the normative definition used by the 1.0 release gate.

## Values and types

Nivren has `Int`, `Float`, `String`, `Bool`, `Null`, function, and homogeneous array values. Local bindings infer their type, or declare it explicitly:

```nivren
let name: String = "Nivren"
let primes: [Int] = [2, 3, 5, 7]
var attempts: Int = 0
```

`let` is immutable. `var` permits reassignment, but the new value must retain the inferred or declared type. Arrays are immutable values; `append(array, value)` returns a new array.

`Int` is a signed 64-bit integer. Overflow always raises a source-located error in both debug and optimized builds. `Float` is IEEE-754 binary64. There are no implicit conversions between them.

Types are non-null by default. `T?` explicitly permits `null`, and `nullable ?? fallback` produces a non-null `T`:

```nivren
let label: String? = null
let displayed: String = label ?? "untitled"
```

## Expressions

Arithmetic uses `+`, `-`, `*`, `/`, and `%`. `+` adds numbers or concatenates two strings. Comparisons use `<`, `<=`, `>`, and `>=`. Equality uses `==` and `!=`. Boolean logic uses `and`, `or`, and `!` and short-circuits. Nivren has no implicit truthiness.

Array and string indexing is zero-based. Indices must be non-negative whole numbers and are bounds checked. String indices select Unicode scalar values rather than bytes.

## Control flow

```nivren
if (ready and attempts < 3) {
    print("starting")
} else {
    print("waiting")
}

while (attempts < 3) {
    attempts = attempts + 1
}

for (value in [1, 2, 3]) {
    print(value)
}
```

Blocks and each `for` iteration create lexical scopes. Arrays iterate by element and strings by Unicode scalar value. Semicolons are optional. `//` starts a line comment; `/* ... */` comments may nest.

## Functions

```nivren
fun total(values: [Int]) -> Int {
    var result: Int = 0
    var index: Int = 0
    while (index < len(values)) {
        result = result + values[index]
        index = index + 1
    }
    return result
}
```

Functions are first-class lexical closures and may recurse. Parameter and return annotations are optional during the prototype phase; public functions will require them at the 1.0 stability boundary.

## Records

Records are nominal, immutable structured values. Their declaration also defines a checked positional constructor:

```nivren
record User { name: String, active: Bool }
let user: User = User("Ada", true)
print(user.name)
```

## Sealed enums and matching

Enums define a closed set of values. `match` is an expression and must cover every variant exactly once:

```nivren
enum State { Idle, Running, Done }
let state: State = State.Running
let label: String = match (state) {
    Idle => "idle",
    Running => "running",
    Done => "done"
}
```

## Recoverable errors

Expected failures use `Result<T, E>`. `ok(value)` and `err(error)` construct results, and exhaustive matching safely binds their payloads:

```nivren
fun load(found: Bool) -> Result<String, String> {
    if (found) { return ok("contents") }
    return err("not found")
}

let outcome: Result<String, String> = load(true)
let text: String = match (outcome) {
    Ok(value) => value,
    Err(message) => "error: " + message
}
```

## Built-ins

- `print(value)` writes a value followed by a newline.
- `clock()` returns Unix time in seconds.
- `len(value)` returns the Unicode-scalar length of a string or element count of an array.
- `type(value)` returns its runtime type name.
- `append(array, value)` returns a new array with the value appended.
- `assert(condition, message)` raises a source-located failure when the condition is false.

## Modules and projects

An import creates a namespace from the imported filename. Declarations are private unless explicitly exported:

```nivren
// math.niv
fun double(value: Int) -> Int { return value * 2 }
let privateConstant = 7
export { double }
```

```nivren
// main.niv
import "math.niv"
print(math.double(21))
```

Imports are relative to the importing file, loaded once per module, and may not form cycles. Project builds reject imports outside their root. A project uses `niv.toml`:

```toml
[package]
name = "example"
version = "0.1.0"
entry = "src/main.niv"

[dependencies]
text_utils = "1.2.3"
```

Dependencies use exact versions. `niv install <registry> [project]` verifies and installs the complete graph, then writes a checksum-pinned `niv.lock`. Import a dependency's entry module with `import "@text_utils"`.

## Commands

- `niv run [file.niv|project]` checks and executes a program or project.
- `niv check file.niv|project` performs lexical, syntax, module, name, mutability, arity, and type checks.
- `niv build [project]` verifies a project and writes its deterministic lockfile.
- `niv install registry [project]` installs exact dependency versions and writes their checksums to the lockfile.
- `niv fmt [--check] file|path` applies or verifies source-preserving formatting.
- `niv doc [project]` writes export-aware API documentation.
- `niv migrate --from version file|path` applies an idempotent source migration.
- `niv test [path]` recursively executes files named `*_test.niv`.
- `niv repl` starts an interactive session with persistent global bindings.
- `niv version` prints the toolchain version.
