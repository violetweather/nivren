# Nivren Package and Registry Specification, Version 1

## 1. Manifest

`niv.toml` is UTF-8 and contains exactly one `[package]` section plus an optional `[dependencies]` section. The package section's required, unique quoted-string keys are `name`, `version`, and `entry`. Each dependency key is a Nivren identifier and each value is an exact canonical `major.minor.patch` version. Unknown sections/keys, duplicate dependencies, and self-dependencies are errors.

- `name` is nonempty lowercase ASCII letters, digits, `-`, or `_`.
- `version` is canonical `major.minor.patch` decimal with no leading zero except `0`.
- `entry` is a relative path without a parent component and MUST resolve inside the canonical project root.

Relative modules discovered from the entry MUST remain inside that root. `use "@name"` resolves the declared dependency's entry module and exposes it as namespace `name`. Dependency packages use their own manifests when resolving transitive `use` declarations.

`niv.lock` format 1 is deterministic and generated. It starts with `format = 1` and one `[[package]]` name/version record, followed in bytewise `(name, version)` order by the complete transitive graph as `[[dependency]]` records containing exact name, version, and lowercase SHA-256 package-archive digest. Implementations MUST NOT use map iteration order, timestamps, or host paths in it. A build MUST reject missing/stale locks, undeclared package uses, missing archives, identity/checksum mismatches, and installed source that differs from its locked archive.

`niv install <registry> [project]` resolves the exact graph from registry v1 metadata, verifies every package, installs it below `.niv/deps/<name>-<version>`, and atomically replaces the lockfile. Installers MUST reject symlinked state/store/package paths and MUST NOT execute package code.

For a remote public registry, `niv install --trusted <https-registry> <root-key> [project]` additionally MUST require certificate-verified HTTPS, match the registry's advertised root to the separately trusted key, and verify publisher authorization, release provenance, current signed status, and every advisory before installing. The client persists the greatest verified status generation and MUST reject rollback, stale status, revocation, freezes, advisories, wrong package identity, or an authorization/provenance mismatch.

## 2. `.nivpkg` archive

All integers are unsigned little-endian. Strings are `u32` byte length followed by UTF-8.

```text
4 bytes  "NIVP"
u16      format (= 1)
string   package name
string   package version
u32      file count
repeat file count:
  string normalized relative path
  u64    content length
  bytes  content
```

Files MUST be bytewise path sorted for canonical encoding. The archive MUST contain `niv.toml`, generated `niv.lock`, and required `.niv` sources. The outer identity MUST exactly equal the embedded manifest.

Readers MUST reject unknown versions, trailing data, duplicates, invalid UTF-8 metadata, empty/absolute/parent/current/backslash paths, strings over 4,096 bytes, more than 4,096 files, one file over 16 MiB, or an archive/content total over 64 MiB. Extraction MUST write only below a new destination and SHOULD commit by atomic directory rename.

Packagers MUST NOT execute project code, lifecycle scripts, compiler plugins, or follow symlinks. `.git`, `.niv`, and `target` are excluded.

## 3. Filesystem registry

Version 1 layout is:

```text
v1/index/<name>/<version>.json
v1/packages/<name>/<version>.nivpkg
v1/provenance/<name>/<version>.json
v1/authorizations/<publisher>.json
v1/trust/root.pub
v1/trust/status.json
v1/trust/advisories.json
```

Index JSON records format 1, name, version, lowercase SHA-256, and byte size. A name/version is immutable. Identical publication is idempotent; differing reuse MUST fail. Fetch MUST verify size, checksum, archive constraints, and embedded identity.

## 4. Trusted publication

Trust JSON uses the strict schemas represented by `PublisherAuthorization`, `ReleaseProvenance`, `RegistryStatus`, and `Advisory`. Unknown fields are errors. Signatures are strict Ed25519 over length-prefixed, domain-separated canonical fields—not serialized JSON bytes.

The registry root authorizes one publisher key plus repository/workflow identity and expiry. The publisher attests exact package SHA-256, name/version, the same identity, source commit, and issuance. Clients MUST verify both signatures, equality of authorized identity, expiration/future skew, package bytes, active advisories, revoked keys, frozen names, and a nondecreasing status generation.

## 5. Publish envelope and HTTP profile

The binary envelope is `NIVE`, `u16` version 1, `u32` provenance JSON length, `u32` authorization JSON length, `u64` package length, then those bytes in order. Each JSON document is at most 1 MiB; the complete envelope is at most 66 MiB and contains exactly one valid `.nivpkg`.

The HTTP profile uses HTTP/1.1 with exact `Content-Length`, no `Transfer-Encoding`, and `Content-Type: application/vnd.nivren.publish-v1` for `POST /v1/publish`. Successful publication returns 201. Artifact GET paths use the registry layout. Servers MUST reject ambiguous encoding/traversal and bound headers, body, time, concurrency, and queued work. Production deployment MUST add authenticated TLS and edge abuse controls; signature verification remains mandatory even with TLS.
