# Nivren language guide — Edition 4 beta candidate

Nivren is an intent-first application language: source says what it keeps, changes, takes, gives, needs, prepares, performs, starts, waits for, and chooses. The executable candidate specification is `spec/LANGUAGE-4-DRAFT.md`; the conformance corpus and proof programs test the behavior described here.

## Values and bindings

```nivren
keep name is String set "Nivren"
keep bytes set std.bytes.from_string with { value set name }
keep scores set std.map.single with { key set "clarity" value set 10 }
keep tags set std.set.single with { value set "safe" }
change attempts is Int set 0
change attempts to attempts + 1
```

`keep` introduces an immutable fact. `change` introduces state whose value can later change without changing type. Core values include checked signed `Int`, fixed-width integers, `Float`, exact `Decimal`, `BigInt`, Unicode `String`, `Bool`, `none`, arrays, immutable `Bytes`, persistent maps and sets, `maybe Value`, and `Value or Problem`. There is no implicit truthiness or numeric conversion.

## Functions, labeled calls, and nominal data

```nivren
type UserId from Int

shape User holds {
    id is UserId
    name is String
    email is maybe String
} with Json, Compare, Display, Validate

define greeting
takes {
    user is User
    prefix is String
}
gives String
{
    give prefix + ", " + user.name
}

keep text set greeting with {
    user set User with {
        id set UserId with { value set 7 }
        name set "Mira"
        email set none
    }
    prefix set "Hello"
}
```

Function parameters and shape fields use `name is Type`. Calls use `with { name set value }`, so arguments remain identifiable after refactoring. Labels are checked for spelling, order, omissions, and duplicates. A nominal `type Name from Representation` prevents accidentally mixing values that share a representation.

Generic arguments are inferred and may be constrained by protocols. Protocol declarations and adoptions are module-scoped, coherent, and statically dispatched. Built-in safety protocols remain sealed, and a package must own the protocol or the adopted type.

Shapes are immutable nominal records. Choices are sealed alternatives:

```nivren
choice State holds {
    case Idle
    case Running
    case Failed carries String
    case Done
}

keep state set State.Failed("network unavailable")
keep label set choose state {
    case Idle => "idle"
    case Running => "running"
    case Failed carries problem => problem
    case Done => "done"
}
```

`choose` must cover every case exactly once. The built-in derives are `Json`, `Compare`, `Display`, `Key`, `Validate`, `Binary`, `DatabaseRow`, and `Arguments`. Derive eligibility is checked from field types; unsupported resources, secrets, callbacks, tasks, and handles are rejected with a field-specific diagnostic.

## Decisions, loops, and pipelines

```nivren
change total set 0
each value within [1, 2, 3, 4] {
    when value % 2 == 0 {
        change total to total + value
    }
}

repeat total < 10 {
    change total to total + 1
}

keep batches set [1, 2, 3, 4, 5]
    through std.list.batch with { size set 2 }
```

`when`/`otherwise`, `each`/`within`, and `repeat` keep control flow explicit. `through` passes the current value as the first parameter of the next stage; the remaining parameters stay labeled. Pure stages lower without runtime plan allocation. Effectful stages preserve source order, typed failure, cleanup, cancellation, and tracing.

## Typed failure

Expected failure appears in the signature as `gives Value or Problem`:

```nivren
define configuration
gives String or String
needs FileRead
{
    keep text set perform std.files.read with { path set "app.json" } or give
    give ok(text)
}
```

Use `or give` when the current function should propagate the same problem unchanged. Use `choose` when the function can recover, translate, log, or fall back. There are no exceptions, implicit nulls, or unchecked result extraction.

## Intent and visible effects

```nivren
shape FetchPlan holds {
    url is String
    timeout is Float
} with Display, Validate

define fetch
takes { plan is FetchPlan }
gives String or String
needs Network within "api.example.com"
{
    give perform std.web.get with {
        url set plan.url
        timeout set plan.timeout
    }
}

prepare request as FetchPlan with {
    url set "https://api.example.com/users"
    timeout set 5.0
}

perform fetch with { plan set perform request }
```

`prepare` creates an immutable typed plan. `perform` is the visible boundary for external work and explicitly stored plans. `needs` states the required authority; `within` narrows it at the source boundary. Pure computation remains direct and allocation-free. Only portable plans made entirely of data may be serialized.

`niv explain program.niv` emits a deterministic intent graph showing capabilities, resources, allocation, effect order, cancellation, retries, timeouts, buffering, blocking, fusion, target selection, and portability.

## Project authority

A declaration explains an effect. The project manifest separately authorizes it:

```toml
[capabilities]
FileRead = "path:./data"
Network = "host:api.example.com;method:GET"

[limits]
instructions = "1000000"
memory_bytes = "67108864"
```

Capabilities include `FileRead`, `FileWrite`, `Environment`, `Time`, `Process`, `Network`, `Task`, `Channel`, `Log`, `Native`, and `Random`. Filesystem, host, environment, process, and native grants can be scoped. Runtime policy, shared task-tree instruction budgets, memory budgets, and call-depth limits are enforced during execution.

## Structured concurrency and resources

```nivren
define first gives Int { give 20 }
define second gives Int { give 22 }

keep joined set together [start first, start second]
keep quickest set race [start first, start second]
keep one set wait start first
```

Tasks are owned, cancellation-aware, joined on drop, and cannot silently outlive their scope. Only `Sendable` values cross task and channel boundaries. Bounded channels provide backpressure.

Own closeable resources with `using`:

```nivren
define load
takes { path is String }
gives String or String
needs FileRead
{
    keep opened set perform std.files.open_read with { path set path }
    using file = opened or give {
        give perform std.files.read_open with { file set file maximum set 1048576 }
    }
}
```

`using` closes files, listeners, streams, WebSockets, locks, native handles and libraries, and transactions on normal completion, `give`, propagated failure, and runtime failure. Cleanup is deterministic and resources cannot be serialized or used as stable keys.

## Modules, projects, and tooling

Declarations are private unless exposed. `use "math.niv"` creates a namespace; `use "@package"` loads an exact locked dependency. Modules load once, cannot cycle, and cannot escape the project root.

```text
niv new my-app
cd my-app
niv add package 1.2.3
niv dev
niv test
niv explain src/main.niv
niv ship
```

`ship` checks and builds the project, runs tests, generates API documentation, creates a deterministic package, and emits a standalone application. It does not publish externally.

The formatter owns the canonical spelling and layout. `niv fmt` is idempotent. The LSP, debugger adapter, VM, JIT, native AOT, browser Wasm, and WASI paths consume the same checked language model; the Product Proof ledger records which distribution and platform gates still need external evidence.

## Native and unsafe boundaries

`std.host.invoke` is the capability-gated embedding escape hatch. `std.native.open` and bounded primitive/buffer calls support dynamic C libraries through opaque owned handles. Generated C11/C++17 bindings and the stable compiler facade use checked public declarations. Native access requires `needs Native` and an explicit project grant because library initializers and declared C signatures cross the safe-language boundary.

Declared unsafe modules contain raw memory, stable layout, allocators, atomics, threads, SIMD, device access, and unchecked FFI. Unsafe authority never appears implicitly in safe modules. Nivren has no syntax macros or unrestricted runtime reflection; schema reflection is deterministic declaration metadata.

See `docs/STYLE_GUIDE.md`, `docs/STANDARD_LIBRARY.md`, and `docs/UNSAFE_MODULES.md` for the idiomatic, library, and systems-level references.
