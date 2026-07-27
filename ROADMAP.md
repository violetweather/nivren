# Road to Nivren 1.0

Nivren does not use the 1.0 label merely because programs execute. Each milestone must pass its tests, documentation review, and compatibility gate.

## 0.2 — Typed interpreter core (complete)

- Lexer, parser, AST, lexical closures, control flow, immutable arrays, and source diagnostics
- Local type inference plus binding, parameter, array, and return annotations
- Command-line runner, checker, REPL, language-native tests, and Rust implementation tests

## 0.3 — Data and error model (complete)

- Records, sealed enums, exhaustive pattern matching, nullable types, and typed `Result`
- Signed 64-bit integers with checked overflow and distinct binary64 floats
- Typed `each` iteration over immutable arrays and Unicode strings

## 0.4 — Modules and projects (complete)

- Explicit modules/imports and private-by-default visibility
- `niv.toml`, deterministic builds, dependency graph, and lockfile format
- Source-preserving formatter and documentation generator

## 0.5 — Bytecode VM (complete)

- Versioned bytecode, verifier, disassembler, interpreter, stack traces, and debug metadata
- Precise managed heap with a collector-neutral interface and GC stress mode
- Self-contained application bundles

## 0.6 — Application standard library (complete)

- Files, paths, processes, environment, time, JSON, sockets, HTTP, TLS, logging, and structured errors
- Structured async tasks, cancellation, deadlines, channels, and blocking executor
- Safe C ABI boundary with explicit unsafe regions

## 0.7 — Developer tooling (complete)

- Incremental compiler, language server, VS Code extension, debugger, profiler, coverage, property tests, and fuzzing
- Sandboxed package builds and local/private registry protocol

Status: all listed features are implemented, documented, and verified, including source-only sandboxed package builds and the immutable local/private registry v1 protocol.

## 0.8 — Performance and public registry (complete)

- Tiered JIT, concurrent generational collection, published benchmark suite, and performance gates
- Provenance-backed public package registry, trusted publishing, advisories, and incident response

Status: the native JIT tier, concurrent generational closure collection, benchmark/performance gates, signed provenance, trusted-publisher authorization, advisories, incident controls, and hostable bounded registry service are implemented and verified.

## 0.10 — Edition 2 compatibility beta (historical baseline)

- Normative language/bytecode/package specifications and independent conformance suite
- Six tier-1 OS/architecture combinations, reproducible signed artifacts, and design-partner production pilots
- Six-month source-compatibility freeze

Status: Edition 2 remains a tested historical baseline. Its old freeze no longer qualifies Nivren for 1.0 because Edition 3 intentionally redesigns the pre-user language surface.

## 0.11 — Edition 3 capability program (in progress)

- Nivren identity: `needs`, `or give`, `through`, structured-concurrency words, `using`, preferred namespaces, and `niv new/add/dev/test/ship`
- Reusable core: inferred generics, sealed protocol constraints, higher-order list algorithms, bytes, and persistent maps/sets
- Runtime policy: scoped path/host grants, shared instruction and memory budgets, call-depth limits, scoped file/listener/stream cleanup, and policy-aware run/test/debug/profile/coverage/standalone apps
- Platform foundation: bounded HTTP/TLS clients and servers, TCP listeners, structured JSON, stable compiler/C APIs, owned native host callbacks, standalone executables, shared/static libraries, and SPDX-bearing release archives
- Normative Edition 3 language/standard-library/bytecode drafts and external-process conformance vectors
- Every capability, domain, ecosystem, target, tooling, distribution, and hardening row below completed to the acceptance rule

Status: these foundations are implemented locally and tested. `docs/CAPABILITY_AUDIT.md` remains authoritative for work such as the production event loop/WebSockets, deeper generic protocols/reflection/data types, additional targets/domains, ecosystem breadth, external audits, and pilots. No GitHub or website publication occurs until the entire program and final validation matrix pass.

## 1.0 — Stable production release

- All conformance, security, performance, platform, documentation, registry, and production-pilot gates pass
- Every row in `docs/CAPABILITY_AUDIT.md` has complete executable evidence and no required-completion item remains
- Strict SemVer and editions begin; first annual LTS receives three years of fixes

## Capability program — No artificial ceiling

These capabilities are required by the current no-artificial-ceiling program. Syntax still must earn its place through prototypes, user testing, specification review, and an Edition boundary; specialized capabilities should remain packages when core syntax would weaken clarity.

### Capability foundation

- A broad native interface: stable C ABI imports, generated bindings, dynamic libraries, memory-safe foreign handles, callbacks, and async interoperability with existing C and Rust libraries
- A serious public package ecosystem covering TLS, WebSockets, databases, web servers, cryptography, GUI, audio/video, compression, cloud services, chat platforms, and testing
- Production cross-platform async I/O with an event loop, cancellation, timers, sockets, files, backpressure, and high-level HTTP and WebSocket clients and servers
- Reusable abstractions through parametric types, protocols, generic constraints, iterators, and collection algorithms without template-heavy syntax
- Deterministic resource management for files, sockets, locks, transactions, native handles, and other values that must be released promptly alongside the managed heap
- A complete data model for byte buffers, fixed-width signed and unsigned numbers, decimal and arbitrary-precision arithmetic, Unicode text, dates/time zones, serialization, streaming, and large-data processing
- Safe reflection and schema inspection, plus narrowly scoped hygienic compile-time generation for bindings, serializers, queries, and repetitive declarations; generated code must remain inspectable and toolable
- Explicit systems escape hatches for unsafe memory, SIMD, atomics, threads, devices, embedded targets, and no-runtime environments
- Native ahead-of-time binaries, WebAssembly, shared and static libraries, mobile platforms, and carefully evaluated GPU targets
- First-class browser, server, desktop, mobile, game/media, data-science/ML, automation, embedded, and systems-development paths with maintained reference applications for each supported domain
- A versioned compiler-as-a-library and tooling protocol so build systems, editors, notebooks, language servers, debuggers, profilers, documentation tools, and deployment platforms do not depend on compiler internals
- Application distribution through self-contained binaries, libraries, containers, WebAssembly components, installers, code signing, update channels, and reproducible deployment metadata
- Production observability with structured logging, metrics, traces, crash reports, source maps, runtime inspection, and stable hooks for monitoring systems
- Testing at every level: unit, integration, snapshot, property, fuzz, benchmark, concurrency, compatibility, platform, and end-to-end deployment tests with deterministic test controls
- Industrial tooling for incremental builds, workspaces, refactoring, debugger integration, tracing, fuzzing, hosted documentation, compatibility testing, and long-term reproducibility
- A hardened runtime and specification with sandboxing, resource limits, independent implementations, extensive fuzzing, external security audits, and sustained compatibility evidence

Capability coverage is not complete merely because a low-level primitive exists. Each supported domain requires an idiomatic safe API, a small end-to-end example, reference documentation, actionable diagnostics, editor support, cross-platform tests, performance expectations, and a maintained deployment story. Features that cannot meet Nivren's clarity and predictability standards remain packages or explicit escape hatches rather than expanding the core language.

### Recognizable Nivren identity

- Keep one intent-first vocabulary: `keep`/`change`, `define`/`give`, `when`/`otherwise`, `each`/`within`, and `shape`/`choice`/`choose`, without accumulating synonyms
- Keep checked capability declarations with `needs FileRead` so effects are visible, transitive, and enforceable by project policy
- Keep typed `or give` failure propagation while preserving `choose` for explicit, exhaustive handling
- Keep the readable `through` pipeline only while it improves real programs without weakening precedence or tooling
- Keep `start`, `wait`, `together`, and `race` scoped; spawned work must not silently outlive its owner
- Keep `niv new`, `niv add`, `niv dev`, `niv test`, and `niv ship` consistent around one standard layout and minimal configuration
- Give diagnostics a distinctive intent-first voice: explain what the program attempted, show the relevant types or values, and suggest a concrete correction
- Maintain short, stable, batteries-included namespaces such as `web`, `json`, `files`, `time`, `tasks`, and `process`, leaving specialized integrations to packages
- Enforce a punctuation budget: prefer familiar structure and meaningful words over cryptic operators or ornamental syntax
- Publish and conformance-test an idiomatic style guide so Nivren code remains recognizable across projects and teams
