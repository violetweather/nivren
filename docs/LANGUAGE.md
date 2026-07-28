# Nivren language guide — Edition 3 draft

Nivren is an intent-first application language: code says what it keeps, changes, needs, starts, waits for, and gives back. The normative draft is `spec/LANGUAGE-3.md`; this guide is the practical tour.

## Values and bindings

```nivren
keep name: String = "Nivren"
keep bytes: Bytes = std.bytes.from_string(name)
keep scores: Map<String, Int> = std.map.single("clarity", 10)
keep tags: Set<String> = std.set.single("safe")
change attempts: Int = 0
```

`keep` is immutable. `change` permits reassignment without changing type. Core values include checked signed `Int`, fixed-width `I8`/`I16`/`I32` and `U8`/`U16`/`U32`/`U64`, binary64 `Float`, exact base-10 `Decimal`, arbitrary-precision signed `BigInt`, Unicode `String`, `Bool`, `none`, homogeneous arrays, immutable `Bytes`, persistent `Map<K,V>` and `Set<T>`, nullable `T?`, and `Result<T,E>`. There is no implicit truthiness or numeric conversion.

## Functions, generics, and protocols

```nivren
define identity<Value>(value: Value) gives Value { give value }

define add<Value: Number>(left: Value, right: Value) gives Value {
    give left + right
}
```

Generic arguments are inferred per call. Constraints are visible and checked. The sealed draft protocols are `Comparable`, `Number`, `Ordered`, `Iterable`, `Closable`, and `Sendable`.

Applications and packages can declare marker protocols or protocols with required behavior:

```nivren
protocol Named {
    define name(value: Self) gives String
}
shape User { name: String }
define user_name(value: User) gives String { give value.name }
adopt Named for User { name = user_name }

define present<Value: Named>(value: Value) gives String {
    give Named.name(value)
}
```

Protocols and adoptions are module scoped and use fully qualified identities. Every required member is mapped exactly once to a signature- and capability-compatible function. `Protocol.member(value)` is statically constrained and dispatches coherently by nominal receiver type in both execution engines. Each protocol/type pair can be adopted once; one package must own either the protocol or type, preventing conflicting third-party implementations. Built-in safety protocols stay sealed. Empty protocols remain available as lightweight semantic categories.

Functions are lexical closures. Public APIs should annotate every parameter, result, generic constraint, and capability.

## Pipelines and collections

`through` passes the current value as the first argument of the next stage:

```nivren
define double(value: Int) gives Int { give value * 2 }
define even(value: Int) gives Bool { give value % 2 == 0 }

keep values: [Int] = [1, 2, 3, 4]
    through std.list.transform(double)
    through std.list.select(even)
```

List algorithms include `transform`, `select`, `fold`, `any`, and `every`. Callback types and callback capabilities are checked.

Edition 4 preserves pipelines in the checked tree. Pure stages fuse to the same operations as direct calls and allocate no runtime plan; effectful stages retain source order. Labeled pipeline stages omit only the piped first value:

```nivren
define batches takes {} gives [[Int]] or String {
    give [1, 2, 3, 4, 5]
        through std.list.batch with { size set 2 }
}
```

`batch` rejects zero or oversized batches and returns bounded arrays. Structured tasks supply parallel/race stages, while lazy iterators and bounded channels provide streaming and backpressure. `prepare` materializes an immutable typed plan, and `perform` is the visible external-effect boundary. `niv explain` reports allocation, capability/resource flow, ordering, cancellation, buffering, blocking, fusion, target choice, and portability.

## Decisions, loops, shapes, and choices

```nivren
when attempts < 3 { show("ready") } otherwise { show("waiting") }
repeat attempts < 3 { attempts = attempts + 1 }
each value within [1, 2, 3] { show(value) }

shape User { name: String, active: Bool }
choice State { Idle, Running, Failed(String), Done }
shape Pair<Left, Right> { left: Left, right: Right }
choice Maybe<Value> { Some(Value), None }

keep state = State.Failed("network unavailable")
keep label = choose state {
    Idle => "idle",
    Running => "running",
    Failed(problem) => problem,
    Done => "done"
}
```

Shapes are immutable nominal records. Shapes and choices may declare inferred, constrained type parameters; `Pair<String, Int>` and `Maybe<Int>` remain nominally distinct from other applications. Choices are sealed and may give a variant one typed payload. Payload variants are one-argument constructors, bare variants are values, and `choose` must exhaustively bind exactly the variants that carry data. Choices may refer to themselves through a payload, such as `Array([Response])`. Blocks and each iteration create lexical scopes. Semicolons are optional; block comments nest.

## Typed failure

Use `choose` when each outcome needs explicit behavior. Use `or give` when the current function should return the same typed error unchanged:

```nivren
define load(path: String) gives Result<String, String> needs FileRead {
    give std.files.read(path)
}

define configuration() gives Result<String, String> needs FileRead {
    keep text: String = load("app.json") or give
    give ok(text)
}
```

`or give` only accepts `Result` and only appears inside a function returning a compatible `Result`, so failure propagation remains statically visible.

## Checked capabilities and project permissions

Effects belong in function signatures:

```nivren
define fetch(url: String) gives Result<String, String> needs Network {
    give std.web.get(url, 10.0)
}
```

Calls propagate `needs` transitively, including callbacks and started tasks. A project separately grants runtime authority:

```toml
[capabilities]
FileRead = "allow"
Network = "host:api.example.com"

[limits]
instructions = "1000000"
memory_bytes = "67108864"
```

The current capabilities are `FileRead`, `FileWrite`, `Environment`, `Time`, `Process`, `Network`, `Task`, `Channel`, `Log`, `Native`, and `Random`. Filesystem grants may use `path:<directory-or-file>` and network grants may use `host:<name>` or `host:*.example.com`; `allow` remains the explicit whole-capability grant. A declaration explains an effect; a manifest grant authorizes it. `Random` controls operating-system entropy, while deterministic cryptographic verification remains capability-free. Runtime policy, shared task-tree instruction budgets, conservative memory budgets, and call-depth limits are enforced again during execution.

## Structured concurrency

```nivren
define first() gives Int { give 20 }
define second() gives Int { give 22 }

keep joined = together [start first, start second]
keep quickest = race [start first, start second]
keep one = wait start first
```

`start`, `wait`, `together`, and `race` are the preferred forms. Tasks are owned, cancellation-aware, joined on drop, and cannot silently outlive their owner. Only `Sendable` values cross task and channel boundaries.

## Deterministic resources

```nivren
define load(path: String) gives Result<String, String> needs FileRead {
    keep opened = std.files.open_read(path)
    using file = opened or give {
        give std.files.read_open(file, 1048576)
    }
}
```

`using` closes `File`, `TcpListener`, `TlsListener`, `TcpStream`, `WebSocket`, `LockGuard`, `NativeHandle`, `NativeLibrary`, and transaction values on normal completion, `give`, `or give`, and runtime failure. Closing is idempotent where the resource protocol permits it, and bounded operations fail safely after close. Resource handles are neither transferable task/channel values nor stable map/set keys.

## Modules and projects

Declarations are private unless exposed:

```nivren
// math.niv
define double(value: Int) gives Int { give value * 2 }
expose { double }
```

`use "math.niv"` creates the `math` namespace. `use "@package"` loads an exact, declared dependency. Modules are loaded once, cannot cycle, and cannot escape the project root.

The everyday project path is:

```text
niv new my-app
cd my-app
niv add package 1.2.3
niv dev
niv test
niv ship
```

`ship` checks and builds the project, runs its tests, generates `target/doc/api.md`, creates a deterministic package, and emits a directly executable standalone application that embeds verified bytecode plus its capability/resource policy. It does not publish externally.

## Native integration and compiler tooling

`std.host.invoke` is the capability-gated escape hatch used by embedding applications and generated bindings. `std.host.open`, `call`, and `close` add opaque `NativeHandle` ownership for long-lived foreign resources, with deterministic `using` cleanup. The stable compiler facade checks, formats, compiles, documents, generates C schema views, and executes source without exposing compiler internals. C ABI version 2 provides the same check/format/compile/run operations, an owned-buffer host callback contract, and an asynchronous completion/cancellation/wake bridge. Shared and static libraries and `nivren.h` ship beside every supported native release.

Direct C libraries use `std.native.open`, `call_int`, `call_float`, and `close`. `NativeLibrary` is an opaque closable resource: symbols never escape a call, primitive call arity is capped at six, and `using` cleanup prevents use after unload. These calls require `needs Native` because loading or invoking foreign code trusts its initializers and declared C signatures. Project policy may restrict opening to an approved path.

`std.reflect.schema(User)` inspects a shape or choice declaration through deterministic string metadata rather than runtime object layout. It is safe, fallible, and side-effect free. Compiler facade v3 and `niv bindgen c` use checked public declarations and emit ordinary inspectable source; Nivren has no hidden unhygienic text-substitution macro phase.

`Iterator<T>` is a typed single-pass value. Build one from a snapshot with `std.iter.from` or from a lazy end-exclusive numeric source with `std.iter.range(start, end, step)`; adapt it with `transform`, `select`, `skip`, `take`, or `chain`; then consume it with `next`, `collect`, `count`, `fold`, `find`, `any`, `every`, or `each value within iterator`. Range stores only cursor state, while query terminals short-circuit and leave the unvisited suffix available. Consumption is explicit: adapters drain their input, iterators cannot cross task/channel boundaries, and work is bounded to one million values per call.

See `docs/STYLE_GUIDE.md` for the conformance-tested idiomatic style.
