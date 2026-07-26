# Nivren

Nivren is a new application programming language focused on safety, clarity, and a small coherent core. This repository contains the executable 0.10 Edition 2 beta on the road to 1.0.

Start with `docs/GETTING_STARTED.md` for archive verification, installation, and a first project.

## Build and run

```sh
cargo build --workspace
cargo run -- run examples/hello.niv
cargo run -- test
cargo run -- repl
cargo test --workspace
```

The installed executable is named `niv`:

```sh
niv run program.niv
niv run .
niv check .
niv install /path/to/registry .
niv install --trusted https://registry.example root.pub .
niv build .
niv package .
niv run target/example.nivb
niv disasm target/example.nivb
niv debug examples/hello.niv
niv profile examples/hello.niv
niv coverage examples/hello.niv
niv fmt --check .
niv doc .
niv test [path]
niv repl
```

## Implemented language surface

- Signed 64-bit integers, binary64 floats, strings, booleans, and `none`
- Immutable `keep` and mutable `change` bindings
- Arithmetic, comparison, equality, and short-circuit logic
- `when`/`otherwise`, `repeat`, blocks, and lexical scope
- Functions, recursion, closures, and `give`
- Optional semicolons and nested block comments
- Unicode identifiers and UTF-8 strings
- Source-located lexer, parser, and runtime diagnostics
- Explicit binding, parameter, array, and return type annotations
- Built-in language test discovery with `assert(condition, message)`
- Private-by-default modules with explicit `expose` declarations and namespaced access
- Strict manifests, transitive exact-version dependencies, checksum-pinned deterministic lockfiles, project-root confinement, formatting, and API docs
- Verified versioned bytecode, portable application bundles, call-frame traces, and precise closure-environment collection
- Typed application APIs for files, paths, environment, processes, time, strict JSON, TCP, HTTP, certificate-verified TLS, and logging
- Structured worker tasks, cooperative cancellation and deadlines, bounded channels, and an isolated C embedding ABI
- Incremental builds, LSP and VS Code support, debugging, profiling, coverage, property/fuzz tests, deterministic packages, and private registries
- Built-ins: `clock()`, `len(value)`, `type(value)`, `append(array, value)`, `assert(condition, message)`, `ok(value)`, and `err(value)`

The implementation remains pre-1.0 until the six-platform hosted checks, compatibility-freeze period, and three independent production-pilot gates are complete.

The normative Edition 2 drafts live in `spec/`. The hash-pinned, implementation-independent vectors in `conformance/edition2-baseline.json` are exercised against the external `niv` process by the black-box conformance runner.

## Editor support

The first-party VS Code extension in `editors/vscode` provides syntax highlighting, live diagnostics, completion, and formatting through the built-in language server. Build an installable extension with:

```text
cd editors/vscode
npm ci
npm run package
```

Install the resulting `.vsix` in VS Code. The extension runs `niv lsp`; set **Nivren: Server Path** if `niv` is not on `PATH`.

## Runtime observability

Run `niv profile <file-or-project>` to execute a program and report elapsed time plus bytecode-operation counts. Run `niv coverage <file-or-project>` to execute it and report hit and missed source lines. Both commands also accept self-contained `.nivb` bundles.

Use `niv debug <file-or-project>` for the interactive source debugger. It supports stepping, continuing, line breakpoints, scoped variable listing, individual variable inspection, and clean termination; type `help` at its prompt for the command list.

## Robustness testing

`cargo test --workspace --all-targets` includes deterministic property suites with shrinking for VM equivalence, formatting, and arbitrary frontend input. The `fuzz` package contains the `frontend` libFuzzer target; run it with `cargo +nightly fuzz run frontend`. CI performs bounded fuzz smoke tests and retains crash artifacts.

## Packages and private registries

`niv package [project]` creates a deterministic, source-only `.nivpkg` after compiling the project without running project code or lifecycle scripts. `niv package verify` validates an archive. `niv registry publish` and `niv registry fetch` operate on immutable, checksum-verified local or privately mounted v1 registries. See `docs/PACKAGES.md` for the bounded archive and registry protocol.

## License

Apache License 2.0.
