# Nivren Package and Registry Specification, Version 2

Version 2 is the Edition 5 manifest and registry contract. It supersedes `spec/PACKAGE-1.md` and resolves ledger row 16: `[capabilities]`, `[unsafe]`, and `[limits]` are legal, fully specified sections, the `edition` key is part of the package identity, and the grant-string forms that Edition 4 shipped are now normative.

Everything in PACKAGE-1 §2–§5 (the `.nivpkg` archive, the filesystem registry, trusted publication, and the publish envelope and HTTP profile) is carried forward unchanged. Only §1, the manifest, is restated here.

## 1. Manifest

`niv.toml` is UTF-8 and contains one `[package]` section plus optional `[dependencies]`, `[capabilities]`, `[unsafe]`, and `[limits]` sections. Every value is a quoted string. Unknown sections, unknown keys, and duplicate keys in any section are errors.

### 1.1 `[package]`

Required keys `name`, `version`, `entry`; optional key `edition`.

- `name` is nonempty lowercase ASCII letters, digits, `-`, or `_`.
- `version` is canonical `major.minor.patch` decimal with no leading zero except `0`.
- `entry` is a relative path without a parent component and MUST resolve inside the canonical project root.
- `edition` is `"4"` or `"5"`; when absent, the project is edition 4. Edition 5 turns on the Edition 5 language rules for every source in the project, including the trusted-module gate with no script grandfathering. The manifest writer emits `edition = "5"` for edition-5 projects and omits the key for edition 4.

### 1.2 `[dependencies]`

Each key is a Nivren identifier naming a package; each value is an exact canonical `major.minor.patch` version. Duplicate dependencies and self-dependencies are errors. `use "@name"` resolves the declared dependency's entry module and exposes it as namespace `name` unless the use declares `as other_name`.

### 1.3 `[capabilities]`

Each key is one capability from the twelve-word vocabulary: `FileRead`, `FileWrite`, `Environment`, `Time`, `Process`, `Network`, `Task`, `Channel`, `Log`, `Native`, `Random`, `Gpu`. Each value is a grant string. The grant language has two forms, both normative:

- `"allow"` grants the capability without a scope. Every capability accepts it.
- A scope string narrows the grant. Only these capabilities accept scopes, with these shapes:
  - `FileRead`, `FileWrite`: `"path:<relative-path>"` — for example `"path:./data"`.
  - `Network`: `"host:<host-list>"` optionally composed with `";method:<method-list>"` — for example `"host:api.example.com,*.cdn.example.com;method:GET,POST"`.
  - `Environment`: `"name:<variable>"` or `"prefix:<prefix>"` — for example `"name:HOME"` or `"prefix:NIVREN_"`.
  - `Process`: `"command:<program>"` optionally composed with `";arg0:<first-argument>"` — for example `"command:git;arg0:status"`.
  - `Native`: `"path:<relative-path>"` or `"kind:<kind>"` — for example `"path:./native"` or `"kind:database"`.

An empty scope, a scope on a capability that takes none, and any other value are errors. A scoped grant is enforced at the single runtime capability gate: an effect call outside the scope stops the program with an authority error.

### 1.4 `[unsafe]`

Each key is one of the eight unsafe modules: `memory`, `layouts`, `allocators`, `atomics`, `threads`, `simd`, `devices`, `ffi`. The only legal value is `"allow"`. An unsafe module that is not listed is unavailable to every source in the project.

### 1.5 `[limits]`

Each key is one of `instructions`, `memory_bytes`, `payload_bytes`; each value is a positive integer written as a quoted string. `payload_bytes` MUST be from 1024 through 268435456 and bounds the byte size of any single effect payload. `instructions` and `memory_bytes` bound the interpreter instruction budget and heap. `niv authority report` prints the declared limits beside the granted capabilities.

### 1.6 Lockfile

`niv.lock` format 1 is unchanged from PACKAGE-1: deterministic, generated, starting with `format = 1` and one `[[package]]` record, followed in bytewise `(name, version)` order by the transitive graph as `[[dependency]]` records with exact name, version, and lowercase SHA-256 archive digest. A build MUST reject missing or stale locks, undeclared package uses, missing archives, identity or checksum mismatches, and installed source that differs from its locked archive.

### 1.7 Authority lock

`niv install` MUST diff the union of dependency-declared capabilities, unsafe modules, and limits against the recorded authority lock and stop on any growth (`guard_authority_lock`); the diff prints one `+`/`-` line per changed grant so the growth is reviewable before it is accepted.
