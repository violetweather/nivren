# Changelog

## Unreleased

- **Edition 5 is a breaking update.** Edition 2, 3, and 4 sources are no longer supported surfaces: the Edition 5 fix ledger's accepted repairs land directly, with no compatibility fallbacks. Programs written for earlier editions must migrate their spellings. The retained Edition 2/3 black-box conformance suites are removed with this policy.

- Generalized compact bytecode call frames beyond integer-only functions, pre-resolved local loads/stores/definitions to lexical slot indices, specialized VM integer operations, and indexed shape fields for constant-time property access; added lexical-shadowing and record-workload regression coverage.
- Reduced bytecode function-call and loop overhead with borrowed operand-stack arguments, compact integer function/root frames, lock-free finalized JIT dispatch, and inline native argument buffers; added persistence, concurrency, high-arity, recursion, and benchmark regression coverage.
- Added bounded Edition 3 HTTP clients and servers, closable TCP listeners, deterministic file handles, structured JSON values, scoped path/host grants, and shared memory budgets.
- Added a versioned compiler facade and C ABI operations for check, format, compile, run, and capability-gated owned host callbacks.
- Added standalone application executables, native shared/static libraries and header packaging, deterministic SPDX SBOMs, and reproducibility checks for native libraries.
- Added opaque `NativeLibrary` resources with `std.native.open`, bounded primitive C ABI calls, deterministic `using` cleanup, path-scoped grants, and real shared-library tests in both execution engines.
- Added intent-first correction hints, the Nivren style guide, a 17-case Edition 3 black-box corpus, and type-checked API client, Discord, typed streaming JSON, native-host, and web-server examples.
- Added shape- and choice-derived strict JSON codecs, bounded typed NDJSON streaming, lossless exact-number serialization, recursive schema validation, and bytecode v3 schema metadata.
- Added deterministic `niv bindgen c` shape/choice views and C ABI v2 asynchronous completion, cooperative cancellation, joinable handles, and event-loop wake callbacks, with C11/C++17 compilation tests.
- Replaced task completion, deadline, and race polling with a shared cross-platform condition-driven runtime event loop.
- Added certificate-verified secure WebSockets plus safe TLS policy for protocol floors, bounded ALPN, and additional PEM roots without a verification-bypass mode.
- Added `std.net.write_some` for deadline-bounded partial writes with explicit byte progress and backpressure.
- Added `std.net.ready` using the tier-one OS readiness reactor for readable/writable deadlines without sleep-based retry loops.
- Added bounded executor-backed `std.files.read_async` and `write_async` tasks with queue backpressure, cancellation checkpoints, event-loop wakes, and 16 MiB limits.
- Added safe deterministic shape/choice schema reflection, compiler facade v2 generation, and LSP discovery for schema tooling.
- Added typed single-pass `Iterator<T>` values with bounded transform/select/take/skip/next/collect adapters, transitive callback effects, and `each` integration.
- Added bounded generic `Transaction<K,V>` resources with staged map updates, explicit commit/rollback, idempotent close, memory charging, and automatic rollback through `using`.
- Added explicit-endian fixed-width binary encoders, zero-copy bounded offset decoders, and 16 MiB immutable byte concatenation.
- Added cursor-only lazy numeric range iterators with directional steps, exact bounds, and dual-engine single-pass behavior.
- Added bounded SHA-256 and HMAC-SHA-256 primitives with constant-time verification plus the integration-tested official `nivren_crypto` package.
- Added the first five official packages with entry API documentation, byte-identical builds, immutable-registry publication, clean locked installation, combined dual-engine consumption, and a synchronized website catalog.
- Added static user marker protocols with explicit `adopt Protocol for Type`, sealed built-ins, duplicate-adoption rejection, and module-qualified coherence across constrained package APIs.
- Added bounded typed `std.text.concat` and the official `nivren_sql` package for validated identifiers, ordered parameters, and injection-resistant placeholder construction.
- Added explicit bounded `std.int.parse` and canonical `std.int.format` conversion for protocol and data-format implementations.
- Added a capability-visible Redis client with bounded RESP2/RESP3 framing, raw verified TLS, ACL/password AUTH, pipelining, functional pools, MOVED/ASK Cluster redirects, and a dual-engine live matrix across Redis 6.2 through 8.8.

- Began the Edition 3 capability program without publishing it: checked `needs`, manifest capability grants, preferred `files`/`web`/`tasks` namespaces, typed `or give`, `through`, `using`, and `start`/`wait`/`together`/`race`.
- Added inferred generic functions, sealed protocol constraints, higher-order list algorithms, immutable `Bytes`, and persistent insertion-ordered `Map`/`Set` values.
- Added `niv new`, `niv add`, `niv dev`, and `niv ship`, deterministic manifest rendering, structured JSON log events, instruction budgets, call-depth limits, and consistent project policy in run/test/debug/profile/coverage.
- Versioned bytecode to v3 for verified resource, failure-propagation, and shape-schema operations; added Edition 3 language, standard-library, bytecode, and black-box conformance drafts.
- Replaced the obsolete Edition 2 release clock with an explicit machine blocker until the entire capability audit is complete and a new Edition 3 freeze begins.

## 0.10.0-beta.6 — 2026-07-26

- Established Nivren Edition 2 with the distinctive `keep`, `change`, `define`, `gives`, `give`, `when`, `otherwise`, `repeat`, `each`, `within`, `shape`, `choice`, `choose`, `use`, `expose`, `yes`, `no`, `none`, and `show` vocabulary.
- Removed the earlier TypeScript-like prototype spellings and the migration command; Nivren had no users requiring source compatibility.
- Updated the compiler, formatter, language server, conformance baseline, examples, editor grammar, specifications, guides, and website to use one canonical syntax.
- Corrected release artifact collection so all six independently downloaded `nivren-*.zip` archives are checksummed, attested, and published.
- Added guided macOS, Linux, and Windows installers with architecture detection, checksum/provenance verification, versioned installs, optional PATH setup, unattended mode, and optional VS Code integration.

## 0.9.0-beta.3 — 2026-07-26

- Made explicit full garbage collection wait for concurrent marking and deterministically sweep unreachable cycles on every supported architecture.

## 0.9.0-beta.2 — 2026-07-26

- Made Windows release executables reproducible by enabling the MSVC linker's deterministic output mode.

## 0.9.0-beta.1 — 2026-07-26

- Added normative Edition 1 language, bytecode, package, registry, and compatibility specifications.
- Added exact transitive package dependencies, checksum-pinned locks, tamper-checked local and provenance-verified HTTPS installs, and `import "@package"` resolution.
- Added an implementation-independent JSON conformance corpus and black-box CLI runner covering successful behavior and required failures.
- Added an enforced 1.0 release policy, six-platform reproducible build workflow, signed artifact attestations, scheduled RustSec checks, and separate frontend/binary fuzz targets.
- Fixed the concurrent collector's final remark to rescan mutable roots, preventing newly added scope edges from being missed under GC stress.

## 0.8.0 — 2026-07-26

- Added a Cranelift native tier for hot integer functions with VM fallback, checked-overflow parity, isolated executable-memory ownership, and runtime counters.
- Added a published median benchmark suite and CI speedup gate; the initial measured workload improved by 2.085x.
- Replaced the single-generation closure collector with automatic young collections, survivor promotion, concurrent old-generation marking, and a mutation-safe final remark.
- Added strict Ed25519 release provenance, root-authorized CI publisher identities, expiring rollback-resistant registry status, key/package incident controls, and signed advisory enforcement.
- Added a hostable fixed-worker registry daemon, bounded signed publish envelopes, allowlisted artifact serving, immutable verified publication, and a non-root container deployment.

## 0.7.0 — 2026-07-26

- Added a bounded JSON-RPC language server with diagnostics, completion, formatting, and document synchronization.
- Added a first-party VS Code extension with syntax highlighting, language-server startup, reproducible packaging, and dependency auditing in CI.
- Added shared VM execution metrics plus `niv profile` operation counts and `niv coverage` source-line reports.
- Added an interactive source debugger with stepping, line breakpoints, scoped variables, and a reusable VM debug-hook API.
- Added shrinking property tests and a bounded libFuzzer target spanning lexing, parsing, type checking, bytecode compilation, encoding, and decoding.
- Added deterministic traversal-safe `.nivpkg` archives, source-only sandboxed builds, and an immutable SHA-256-verified filesystem registry protocol.

## 0.6.0 — 2026-07-26

- Added typed `std.fs`, `std.path`, `std.env`, `std.time`, `std.process`, and `std.log` namespaces with structured failures.
- Added a bounded strict JSON validator, compactor, and pretty printer with Unicode surrogate support.
- Added timeout-bounded TCP streams and strict HTTP/1.1 framing with certificate-verified HTTPS through pinned Rustls and Mozilla roots.
- Added OS-thread tasks, cooperative cancellation, deadline waits, structured joining, and bounded typed-value channels.
- Converted runtime ownership to synchronized managed handles while preserving precise GC stress behavior.
- Added an isolated C ABI crate, header, panic containment, UTF-8 validation, and explicit allocation ownership.

## 0.5.0 — 2026-07-26

- Replaced default execution with a versioned stack bytecode VM and differential interpreter tests.
- Added control-flow, operand, stack, scope, and nested-chunk verification plus structured disassembly and call-frame traces.
- Added bounded binary bundle encoding and hostile-input decoding checks.
- Made project builds emit self-contained `.nivb` applications that can be checked, run, and disassembled without source files.
- Added a collector-neutral managed-environment interface, precise reachability collection, heap statistics, and instruction-level GC stress mode.

## 0.4.0 — 2026-07-25

- Added relative file modules, namespaced access, explicit exports, cycle detection, and private-by-default visibility.
- Added nominal module-qualified record and enum identities and project-root import confinement.
- Added strict `niv.toml` manifests and deterministic `niv.lock` generation.
- Added project-aware run, check, and build commands.
- Added a source-preserving formatter, export-aware API documentation generator, and idempotent migration engine.

## 0.3.0 — 2026-07-25

- Added nominal immutable records with checked constructors and field access.
- Added sealed enums and exhaustive expression-oriented matching.
- Added explicit `T?` nullability and typed `??` fallback.
- Added typed `Result<T, E>` values with exhaustive payload matching.
- Split numeric values into checked signed 64-bit `Int` and binary64 `Float` types with no implicit conversion.
- Added typed `for` iteration over arrays and Unicode strings.

## 0.2.0 — 2026-07-25

- Added the first executable Nivren interpreter and `niv` command.
- Added lexical scoping, immutable and mutable bindings, control flow, functions, recursion, and closures.
- Added static name, mutability, operator, condition, function-arity, annotation, argument, and return checks.
- Added homogeneous immutable arrays, safe Unicode string indexing, bounds checks, and persistent append.
- Added source-located diagnostics, a persistent REPL, built-in assertions, native test discovery, examples, and language documentation.
