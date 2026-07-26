# Changelog

## Unreleased

## 0.10.0-beta.5 — 2026-07-26

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
