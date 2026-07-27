# Getting started with Nivren

Nivren 0.10 is a pre-1.0 Edition 3 development build. The repository is intentionally not being published again until the full roadmap, documentation, installers, website, and validation gates agree.

## Install a release build

The guided installer selects the correct archive, verifies its checksum, installs `niv`, optionally updates your user PATH, and can install the VS Code extension.

macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -fsSLO https://raw.githubusercontent.com/violetweather/nivren/main/install/install.sh
sh install.sh
```

Windows PowerShell:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/violetweather/nivren/main/install/install.ps1 -OutFile install.ps1
Set-ExecutionPolicy -Scope Process Bypass
.\install.ps1
```

Use `--yes` on macOS/Linux or `-Yes` on Windows for recommended unattended choices. Until the next release is published, these URLs install the last public build rather than the local Edition 3 work described here.

The installer writes a private ownership marker before it will remove anything. Uninstall with `sh install.sh --uninstall` on macOS/Linux or `.\install.ps1 -Uninstall` on Windows. The Unix flow removes only its verified managed command link and the exact PATH block it recorded; both installers refuse an unmarked or unsafe root.

## Create a project

```text
niv new hello-nivren
cd hello-nivren
niv dev
niv test
```

`niv new` creates the manifest, `src/main.niv`, and a first test. Replace the entry point with:

```nivren
define double(value: Int) gives Int { give value * 2 }
define even(value: Int) gives Bool { give value % 2 == 0 }

keep values = [1, 2, 3, 4]
    through std.list.transform(double)
    through std.list.select(even)

show(values)
```

Run `niv ship` when the project is ready. It checks and builds the project, runs its tests, generates `target/doc/api.md`, creates a deterministic `.nivpkg`, and emits a directly executable standalone application under `target/`; it does not upload anything.

## Add capabilities deliberately

Effects are visible in source and authorized in the project manifest. For a file-reading program:

```nivren
define load(path: String) gives Result<String, String> needs FileRead {
    give std.files.read(path)
}

define main() gives Result<Null, String> needs FileRead {
    keep text = load("message.txt") or give
    show(text)
    give ok(none)
}

main()
```

Add this to `niv.toml`:

```toml
[capabilities]
FileRead = "path:./data"

[limits]
instructions = "1000000"
memory_bytes = "67108864"
```

A missing `needs` is a check error. A missing or out-of-scope manifest grant is a runtime denial. `FileRead`/`FileWrite` and native libraries accept `path:` scopes; `Network` accepts exact or wildcard host alternatives plus an optional HTTP method constraint, such as `host:api.example.com,*.cdn.example.com;method:GET,POST`; `Environment` accepts `name:` or `prefix:`; `Process` accepts command alternatives plus an optional exact first argument, such as `command:git;arg0:status`; native host handles accept exact `kind:`. Every composed clause must pass. `allow` deliberately authorizes the whole capability. Shared instruction and memory budgets stop runaway work and are also applied by project tests, debugging, profiling, coverage, inspection, tasks, and standalone applications.

## Build a service

`examples/web_server.niv` shows the complete bounded server path: `std.net.listen`, a `using`-owned listener, deadline-bound `accept`, `std.web.read_request`, and managed `std.web.respond`. `examples/api_client.niv` uses the general certificate-verified request API, while `examples/discord_bot.niv` is an explained real-world integration.

## Embed Nivren

Release archives include a shared library, static library, and `nivren.h`. ABI version 2 can check, format, compile, or run UTF-8 source. `nivren_run_host_utf8` adds an owned callback/free pair exposed through the `Native` capability; `nivren_run_async_utf8` adds cooperative cancellation, one owned completion, and an event-loop wake callback. `niv bindgen c schema.niv output.h` derives deterministic C11/C++17 data views from checked shapes and choices. Rust build tools and editors can use `nivren::compiler::Compiler`, whose facade version is independent of internal modules. See `docs/EMBEDDING.md` for ownership and lifecycle rules.

Programs that need an existing C library can use `std.native.open(path)` inside a function that declares `needs Native`, keep the opaque `NativeLibrary` inside `using`, and call a bounded primitive signature with `std.native.call_int` or `call_float`. A project can replace unrestricted `Native = "allow"` with a `path:` grant for the library location. This boundary deliberately trusts the library and selected export signature.

## Add a dependency

```text
niv add text_utils 1.2.3
niv install /path/to/registry
```

Dependencies use exact versions and checksum-pinned lockfiles. Import the installed entry module with `use "@text_utils"`.

## Editor and help

The first-party VS Code extension provides Edition 3 highlighting, diagnostics, completion, and formatting through `niv lsp`. Run `niv help` for the complete command list. See `docs/LANGUAGE.md`, `docs/STANDARD_LIBRARY.md`, and the normative Edition 3 drafts in `spec/` next.
