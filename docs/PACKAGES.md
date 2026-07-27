# Nivren package and private registry protocol v1

Nivren packages are deterministic, source-only `.nivpkg` archives. Creating a package first compiles and type-checks the project, verifies the installed dependency graph, then archives `niv.toml`, the checksum-pinned `niv.lock`, and project-owned `.niv` files in bytewise path order.

Package builds are sandboxed by construction: Nivren has no lifecycle or compiler-plugin scripts, package creation never runs the program, imports cannot escape the project root, symlinked source entries are rejected, and `target`, `.niv`, plus `.git` are excluded.

## Dependencies and locks

`[dependencies]` maps Nivren identifiers to exact versions. `niv install <registry> [project]` resolves the complete transitive graph, verifies registry metadata and package archives, installs versioned packages below `.niv/deps`, and writes SHA-256 identities to `niv.lock`. Packages are loaded with `use "@name"`. Builds reject stale locks, missing packages, altered archives, and installed source that differs from its archive.

Use `niv install --trusted https://registry.example root.pub [project]` for a public registry. This mode requires verified HTTPS and a separately obtained root key, verifies signed publishing provenance/status/advisories for every package, and persists the highest registry generation to prevent rollback.

## Archive format

All integers are unsigned little-endian. Strings are UTF-8 prefixed by a `u32` byte length.

1. Four-byte magic `NIVP`
2. `u16` format version, currently `1`
3. Package name string
4. Semantic `major.minor.patch` version string
5. `u32` file count
6. For every file: normalized relative path string, `u64` content length, content bytes

Readers reject unknown versions, trailing bytes, duplicate or non-normalized paths, absolute paths, parent traversal, backslashes, invalid UTF-8 metadata, identity mismatches, more than 4,096 files, files over 16 MiB, and archives over 64 MiB.

## Filesystem registry

A local or privately mounted registry has this immutable layout:

```text
v1/
  index/<name>/<version>.json
  packages/<name>/<version>.nivpkg
```

Index metadata contains `format`, `name`, `version`, lowercase SHA-256, and byte size. Publishing the same content is idempotent. Reusing a name/version for different content or metadata is rejected. Fetch verifies the metadata identity, bounded size, exact byte size, SHA-256, archive structure, and embedded manifest identity before extraction.

## Commands

```text
niv package [project]
niv install /path/to/registry [project]
niv install --trusted https://registry.example root.pub [project]
niv package verify target/name-version.nivpkg
niv registry search web /path/to/registry
niv registry publish target/name-version.nivpkg /path/to/registry
niv registry fetch name 1.2.3 /path/to/registry ./vendor/name
```

Extraction requires a destination that does not exist. Files are written to a sibling temporary directory and committed with one rename; failures clean up the temporary sandbox.

Registry search is case-insensitive and deterministic. It returns package names in ascending order and stable semantic versions newest-first, ignores symlinked index entries, limits output to 100 packages, and is available from a hosted registry at `GET /v1/search/<query>`.

## Official packages

The source, tests, compatibility rules, and release acceptance matrix for Nivren-maintained packages are documented in `docs/OFFICIAL_PACKAGES.md`. Official identities are importable Nivren identifiers such as `nivren_validation`; hyphens are not accepted in dependency keys or `use "@package"` paths. Generated entry-module docs include only names from `expose`.
