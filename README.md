# Nivren

Nivren is an intent-first application programming language. The source says what a
program keeps, changes, needs, and gives — and the compiler holds it to that.

This is **Nivren 1.0.0**: Edition 6, the runtime edition, stable. The grammar is
frozen for good, and the compatibility promise is unconditional — code you write
today runs unchanged on every later release.

- Website: <https://violetweather.github.io/nivren-site>
- Documentation: <https://violetweather.github.io/nivren-site/docs>
- Package registry: <https://violetweather.github.io/nivren-registry>
- Benchmarks (wins and losses, all published): <https://violetweather.github.io/nivren-site/benchmarks>

## Install

**macOS / Linux**

```sh
curl --proto '=https' --tlsv1.2 -fsSLO https://raw.githubusercontent.com/violetweather/nivren/main/install/install.sh
sh install.sh
```

**Windows (PowerShell)**

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/violetweather/nivren/main/install/install.ps1 -OutFile install.ps1
Set-ExecutionPolicy -Scope Process Bypass
.\install.ps1
```

The guided installer verifies every download against its published SHA-256
checksum, keeps a recovery version for rollback, and can set up PATH and
VS Code for you. Prefer a manual archive? Grab one from the
[downloads page](https://violetweather.github.io/nivren-site/downloads) and
verify it with `SHA256SUMS`.

## Hello, Nivren

Create `hello.niv`:

```text
define greet
takes { name is String }
gives String
{
    give "Hello, " + name + "!"
}

show(greet with { name set "world" })
```

Run it:

```sh
niv run hello.niv
```

Or start a real project — one command path from new to shipped:

```sh
niv new my-app
cd my-app
niv dev       # check + run
niv test      # run the tests
niv ship      # check, test, document, package, and emit a standalone executable
```

## What makes it Nivren

- **Visible authority.** A function that touches the network says
  `needs Network`, and the project grants exactly which hosts. No ambient
  permissions, ever.
- **No exceptions, no nulls.** Failure is a typed value; `or give` forwards it,
  `choose` handles every case, and the compiler checks both.
- **Structured concurrency.** Tasks cannot outlive their scope; channels are
  bounded; cleanup runs on success, failure, and crash alike.
- **Native speed where it counts.** Edition 6 compiles whole integer programs to
  machine code and `niv build --aot` emits a relocatable native object.
- **One toolchain in one binary.** Formatter, language server, debugger,
  profiler, coverage, docs, benchmarks, and package manager — all inside `niv`.
- **A live signed registry.** All 25 official packages install with client-side
  Ed25519 verification: `niv add nivren_stats 1.0.0`, then
  `niv install --trusted https://violetweather.github.io/nivren-registry ./nivren-root.pub`.

Run `niv help` for the full command list, or browse the
[examples](https://violetweather.github.io/nivren-site/examples).

## Building from source

Requires Rust 1.88 or newer.

```sh
git clone https://github.com/violetweather/nivren.git
cd nivren
cargo build --release --workspace
./target/release/niv version
```

`cargo test --workspace --all-targets` runs the full dual-engine suite.

## Learn more

- `docs/GETTING_STARTED.md` — install, verify, and write a first project
- `docs/LANGUAGE.md` and `spec/LANGUAGE-5-DRAFT.md` — the language and its frozen grammar
- `docs/PACKAGES.md` and `docs/REGISTRY_SECURITY.md` — packages and the signed registry
- `docs/AOT.md` — native ahead-of-time objects
- `docs/EMBEDDING.md` — the C ABI, and Swift/Kotlin wrappers (experimental)
- `docs/RELEASES.md` — the release policy and the machine-checked 1.0 gate
- `CONTRIBUTING.md` — how to contribute
- `SECURITY.md` — how to report a vulnerability

Upgrading from a pre-1.0 edition? Retired spellings stop with a diagnostic that
names the current form; `spec/EDITION-5-FIX-LEDGER.md` records every removal and
its rationale.

## License

[Apache-2.0](LICENSE)
