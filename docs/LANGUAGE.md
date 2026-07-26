# Nivren Language Guide

This guide describes the implemented Nivren 0.10 beta language. Edition 2 is frozen for compatibility testing; `spec/LANGUAGE-2.md` is the normative definition used by the 1.0 release gate.

## Values and types

Nivren has `Int`, `Float`, `String`, `Bool`, `Null`, function, and homogeneous array values. Local bindings infer their type, or declare it explicitly:

```nivren
keep name: String = "Nivren"
keep primes: [Int] = [2, 3, 5, 7]
change attempts: Int = 0
```

`keep` is immutable. `change` permits reassignment, but the new value must retain the inferred or declared type. Arrays are immutable values; `append(array, value)` returns a new array.

`Int` is a signed 64-bit integer. Overflow always raises a source-located error in both debug and optimized builds. `Float` is IEEE-754 binary64. There are no implicit conversions between them.

Types are non-null by default. `T?` explicitly permits `none`, and `nullable ?? fallback` produces a non-null `T`:

```nivren
keep label: String? = none
keep displayed: String = label ?? "untitled"
```

## Expressions

Arithmetic uses `+`, `-`, `*`, `/`, and `%`. `+` adds numbers or concatenates two strings. Comparisons use `<`, `<=`, `>`, and `>=`. Equality uses `==` and `!=`. Boolean logic uses `and`, `or`, and `!` and short-circuits. Nivren has no implicit truthiness.

Array and string indexing is zero-based. Indices must be non-negative whole numbers and are bounds checked. String indices select Unicode scalar values rather than bytes.

## Control flow

```nivren
when ready and attempts < 3 {
    show("starting")
} otherwise {
    show("waiting")
}

repeat attempts < 3 {
    attempts = attempts + 1
}

each value within [1, 2, 3] {
    show(value)
}
```

Blocks and every `each` iteration create lexical scopes. Arrays iterate by element and strings by Unicode scalar value. Semicolons are optional. `//` starts a line comment; `/* ... */` comments may nest.

## Functions

```nivren
define total(values: [Int]) gives Int {
    change result: Int = 0
    change index: Int = 0
    repeat index < len(values) {
        result = result + values[index]
        index = index + 1
    }
    give result
}
```

Functions are first-class lexical closures and may recurse. Parameter and result annotations are optional during the prototype phase; public functions will require them at the 1.0 stability boundary.

## Shapes

Shapes are nominal, immutable structured values. Their declaration also defines a checked positional constructor:

```nivren
shape User { name: String, active: Bool }
keep user: User = User("Ada", yes)
show(user.name)
```

## Sealed choices

Choices define a closed set of values. `choose` is an expression and must cover every variant exactly once:

```nivren
choice State { Idle, Running, Done }
keep state: State = State.Running
keep label: String = choose state {
    Idle => "idle",
    Running => "running",
    Done => "done"
}
```

## Recoverable errors

Expected failures use `Result<T, E>`. `ok(value)` and `err(error)` construct results, and exhaustive choosing safely binds their payloads:

```nivren
define load(found: Bool) gives Result<String, String> {
    when found { give ok("contents") }
    give err("not found")
}

keep outcome: Result<String, String> = load(yes)
keep text: String = choose outcome {
    Ok(value) => value,
    Err(message) => "error: " + message
}
```

## Built-ins

- `show(value)` writes a value followed by a newline.
- `clock()` returns Unix time in seconds.
- `len(value)` returns the Unicode-scalar length of a string or element count of an array.
- `type(value)` returns its runtime type name.
- `append(array, value)` returns a new array with the value appended.
- `assert(condition, message)` raises a source-located failure when the condition is false.

## Modules and projects

A `use` declaration creates a namespace from the used filename. Declarations are private unless explicitly exposed:

```nivren
// math.niv
define double(value: Int) gives Int { give value * 2 }
keep privateConstant = 7
expose { double }
```

```nivren
// main.niv
use "math.niv"
show(math.double(21))
```

Used modules are relative to the file using them, loaded once per module, and may not form cycles. Project builds reject modules outside their root. A project uses `niv.toml`:

```toml
[package]
name = "example"
version = "0.1.0"
entry = "src/main.niv"

[dependencies]
text_utils = "1.2.3"
```

Dependencies use exact versions. `niv install <registry> [project]` verifies and installs the complete graph, then writes a checksum-pinned `niv.lock`. Load a dependency's entry module with `use "@text_utils"`.

## Commands

- `niv run [file.niv|project]` checks and executes a program or project.
- `niv check file.niv|project` performs lexical, syntax, module, name, mutability, arity, and type checks.
- `niv build [project]` verifies a project and writes its deterministic lockfile.
- `niv install registry [project]` installs exact dependency versions and writes their checksums to the lockfile.
- `niv fmt [--check] file|path` applies or verifies source-preserving formatting.
- `niv doc [project]` writes export-aware API documentation.
- `niv test [path]` recursively executes files named `*_test.niv`.
- `niv repl` starts an interactive session with persistent global bindings.
- `niv version` prints the toolchain version.
