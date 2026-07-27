# Nivren

Nivren is an intent-first application programming language focused on safety, clarity, visible effects, and a coherent path from a first script to production software. This repository contains the local 0.10 Edition 3 development line on the road to 1.0.

Start with `docs/GETTING_STARTED.md` for archive verification, installation, and a first project.

The guided installers under `install/` detect the correct binary, verify it, retain its documentation, and can configure PATH and VS Code automatically. Manual archives remain available for fully controlled installs.

## Build and run

```sh
cargo build --workspace
cargo run -- new my-app
cargo run -- dev my-app
cargo run -- test
cargo run -- repl
cargo test --workspace
```

The installed executable is named `niv`:

```sh
niv new my-app
niv add package 1.2.3 my-app
niv dev my-app
niv test my-app/tests/niv
niv test --snapshots my-app/tests/niv
niv ship my-app
niv run program.niv
niv run .
niv check .
niv install /path/to/registry .
niv install --trusted https://registry.example root.pub .
niv build .
niv build --standalone .
niv build --aot .
niv package .
niv run target/example.nivb
niv disasm target/example.nivb
niv debug examples/hello.niv
niv profile examples/hello.niv
niv inspect examples/hello.niv events.jsonl
niv coverage examples/hello.niv
niv fmt --check .
niv doc .
niv test [path]
niv repl
```

## Implemented language surface

- Signed 64-bit integers, binary64 floats, Unicode strings, immutable bytes with explicit-endian binary codecs, booleans, and `none`
- Immutable `keep` and mutable `change` bindings
- Arithmetic, comparison, equality, and short-circuit logic
- `when`/`otherwise`, `repeat`, blocks, and lexical scope
- Functions, recursion, closures, generic inference, sealed safety constraints, required-member protocols with coherent dispatch, and `give`
- Checked `needs` capabilities—including explicit operating-system entropy through `Random`—plus scoped path/host project grants and shared instruction/memory limits
- Typed `or give` failure propagation and exhaustive `choose`
- Readable `through` pipelines, generic persistent arrays/maps/sets, and bounded typed single-pass iterator adapters
- Scoped `using` files/listeners/streams/locks/transactions, transferable checked atomic integers, and `start`/`wait`/`together`/`race` structured concurrency
- Optional semicolons and nested block comments
- Unicode identifiers and UTF-8 strings
- Source-located lexer, parser, and runtime diagnostics
- Explicit binding, parameter, array, and return type annotations
- Built-in language test discovery with `assert(condition, message)`
- Private-by-default modules with explicit `expose` declarations and namespaced access
- Strict manifests, transitive exact-version dependencies, checksum-pinned deterministic lockfiles, project-root confinement, formatting, and API docs
- Verified versioned bytecode, portable application bundles, call-frame traces, and precise closure-environment collection
- Typed application APIs for bounded file handles, paths, environment, processes, time, shape-derived JSON plus bounded typed data streams, TCP clients/listeners, HTTP clients/servers, certificate-verified TLS clients and certificate/key-configured secure WebSocket servers, and logging
- Structured worker tasks, cooperative cancellation and deadlines, bounded channels, multi-stream OS readiness plus backpressure-aware adapters, a versioned compiler facade, schema-driven C11/C++17 bindings, capability/path-gated dynamic C libraries, bounded asynchronous host operations, and an isolated C ABI with owned native callbacks plus async event-loop wakeups
- Incremental builds, standalone executables, deterministic native AOT objects for eligible pure typed functions, portable WASI and zero-import browser compiler/runtime guests with a JavaScript SDK, shared/static embedding libraries, SBOM-bearing release archives, LSP and VS Code support, debugging, profiling, coverage, property/fuzz tests, deterministic packages, and private registries
- An integration-tested official package catalog for bounded SHA-256/HMAC, Argon2id password storage, secure random keys, random-nonce ChaCha20-Poly1305 authenticated encryption, compact HS256 JWT authentication, AWS Signature Version 4, W3C tracing, Prometheus metrics, deterministic compression, explicit-schema CSV and typed columnar tables, dense matrices, PCM16 audio, escaped SVG interfaces, descriptive statistics, parameterized SQL, Redis RESP2/RESP3 with TLS/AUTH/pipelines/pools/Cluster redirects, Discord REST, typed testing, pure routing, and structured validation, with generated public API docs and semantic compatibility rules
- Safe declaration reflection and compiler facade v2 generation APIs that emit inspectable source instead of hidden text macros
- Built-ins: `clock()`, `len(value)`, `type(value)`, `append(array, value)`, `assert(condition, message)`, `ok(value)`, and `err(value)`

The implementation remains pre-1.0 until every open capability-audit row has executable evidence and the full platform, security, performance, compatibility, installer, documentation, and production-pilot gates pass together. Local Edition 3 work is not published until those gates are complete.

The supported OCI recipe under `containers/` builds a minimal non-root image with verified-TLS certificate roots. CI runs its default command on a read-only filesystem; no image is published before the complete 1.0 gate.

The normative Edition 2 baseline and Edition 3 working drafts live in `spec/`. Implementation-independent Edition 2 and Edition 3 vectors in `conformance/` are exercised against the external `niv` process by the black-box conformance runner.

## Editor support

The first-party VS Code extension in `editors/vscode` provides syntax highlighting, live diagnostics, completion, formatting, and Unicode-correct rename through the built-in language server. Its bounded workspace index covers up to 4,096 Nivren files and 16 MiB of source, skips generated/dependency trees and symlinks, and lets rename update exposed declarations plus qualified references in open or closed importing modules without touching unrelated same-named bindings. The guided installer offers to install the release VSIX automatically when the `code` command is available. To build it yourself:

```text
cd editors/vscode
npm ci
npm run package
```

Install the resulting `.vsix` in VS Code. The extension runs `niv lsp`; set **Nivren: Server Path** if `niv` is not on `PATH`.

## Runtime observability

Run `niv profile <file-or-project>` to execute a program and report elapsed time plus bytecode-operation counts. Run `niv coverage <file-or-project>` to execute it and report hit and missed source lines. Both commands also accept self-contained `.nivb` bundles.

Use `niv debug <file-or-project>` for the interactive source debugger. It supports stepping, continuing, line breakpoints, scoped variable listing, individual variable inspection, and clean termination; type `help` at its prompt for the command list.

Use `niv inspect <file-or-project> <output.jsonl>` for a flush-on-every-step `org.nivren.inspect.v1` event stream suitable for live viewers and operational tooling. It reports locations, operations, stack depth, variable names, final metrics, and heap counters while deliberately omitting source and variable values.

## Robustness testing

`cargo test --workspace --all-targets` includes deterministic property suites with shrinking for VM equivalence, formatting, and arbitrary frontend input. The `fuzz` package contains the `frontend` libFuzzer target; run it with `cargo +nightly fuzz run frontend`. CI performs bounded fuzz smoke tests and retains crash artifacts.

## Packages and private registries

`niv package [project]` creates a deterministic, source-only `.nivpkg` after compiling the project without running project code or lifecycle scripts. `niv package verify` validates an archive. `niv registry publish` and `niv registry fetch` operate on immutable, checksum-verified local or privately mounted v1 registries. See `docs/PACKAGES.md` for the bounded archive and registry protocol.

## License

Apache License 2.0.
