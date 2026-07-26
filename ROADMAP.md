# Road to Nivren 1.0

Nivren does not use the 1.0 label merely because programs execute. Each milestone must pass its tests, documentation review, and compatibility gate.

## 0.2 — Typed interpreter core (complete)

- Lexer, parser, AST, lexical closures, control flow, immutable arrays, and source diagnostics
- Local type inference plus binding, parameter, array, and return annotations
- Command-line runner, checker, REPL, language-native tests, and Rust implementation tests

## 0.3 — Data and error model (complete)

- Records, sealed enums, exhaustive pattern matching, nullable types, and typed `Result`
- Signed 64-bit integers with checked overflow and distinct binary64 floats
- Typed `for` iteration over immutable arrays and Unicode strings

## 0.4 — Modules and projects (complete)

- Explicit modules/imports and private-by-default visibility
- `niv.toml`, deterministic builds, dependency graph, and lockfile format
- Source-preserving formatter, documentation generator, and migration framework

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

## 0.9 — Compatibility beta

- Normative language/bytecode/package specifications and independent conformance suite
- Six tier-1 OS/architecture combinations, reproducible signed artifacts, automated migrations, and design-partner production pilots
- Six-month source-compatibility freeze

Status: the Edition 1 language, bytecode, package, and standard-library specifications; 27-case black-box conformance corpus; exact transitive dependency resolver; automated 0.2–0.9 migrations; six-platform CI definition; deterministic self-contained release archives; reproducible attested release workflow; fuzz/security schedules; and machine-enforced freeze/pilot gate are implemented locally. Completion still requires successful CI evidence on all six native runners and three real 30-day production pilots; the compatibility clock ends 2027-01-26.

## 1.0 — Stable production release

- All conformance, security, performance, platform, documentation, migration, registry, and production-pilot gates pass
- Strict SemVer and editions begin; first annual LTS receives three years of fixes
