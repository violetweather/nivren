# Getting started with Nivren

Nivren 0.9 is a compatibility beta. Use it to evaluate Edition 1 and report problems, but do not run untrusted `.niv`, `.nivb`, or `.nivpkg` files yet.

## Install a release archive

Download the ZIP for your operating system and architecture together with `SHA256SUMS`. Verify the checksum and the GitHub build attestation before extracting it; see `docs/RELEASES.md` for the verification command.

Each ZIP contains one directory with this layout:

```text
nivren-VERSION-PLATFORM/
  bin/niv              # niv.exe on Windows
  Cargo.lock
  LICENSE
  README.md
  SECURITY.md
  THIRD_PARTY.md
  licenses/
  spec/
```

On Linux or macOS, copy `bin/niv` into a directory already on `PATH`, such as a user-owned `~/.local/bin`, then open a new terminal:

```sh
install -d "$HOME/.local/bin"
install -m 755 nivren-VERSION-PLATFORM/bin/niv "$HOME/.local/bin/niv"
niv version
```

On Windows, create a user-owned folder such as `%LOCALAPPDATA%\Nivren\bin`, copy `bin\niv.exe` there, add that folder to your user `Path`, open a new terminal, and run:

```powershell
niv version
```

You can also run the executable directly from the extracted `bin` directory without installing it.

## Run your first program

Create `hello.niv`:

```nivren
let language: String = "Nivren"
let values: [Int] = [2, 3, 5, 7]

fun sum(items: [Int]) -> Int {
    var total: Int = 0
    for (value in items) {
        total = total + value
    }
    return total
}

print("Hello, " + language + "!")
print(sum(values))
```

Check it without executing it, then run it:

```sh
niv check hello.niv
niv run hello.niv
```

The program prints:

```text
Hello, Nivren!
17
```

The same source is included as `examples/getting_started.niv` in the repository.

## Create a project

A project has a strict `niv.toml` manifest and a source entry point:

```text
hello-project/
  niv.toml
  src/main.niv
```

Use this manifest:

```toml
[package]
name = "hello-project"
version = "0.1.0"
entry = "src/main.niv"

[dependencies]
```

Place the first program in `src/main.niv`, then run these commands from `hello-project`:

```sh
niv check .
niv run .
niv build .
```

The build creates a portable verified bytecode bundle under `target/`. The complete language guide is in `docs/LANGUAGE.md`; the Edition 1 specification in `spec/LANGUAGE-1.md` is normative when the two differ.

## Editor support and help

The VS Code extension provides highlighting, diagnostics, completion, and formatting. See the editor section in `README.md`. Run `niv help` for the complete command list and consult `SECURITY.md` before reporting a suspected vulnerability.
