# Nivren 1.0 release audit

Last updated: 2026-07-26

This document maps the 1.0 requirements to reviewable evidence. “Ready locally” means the implementation and local checks exist; it does not substitute for hosted platform results, elapsed compatibility time, or real production use.

## Current decision

**1.0 is not releasable.** `niv release check .` correctly rejects the current tree because the Edition 1 freeze ends on 2027-01-26, the toolchain version is `0.9.0-beta.3`, and zero of three required independent 30-day production pilots are recorded.

## Requirement matrix

| Requirement | Status | Evidence or remaining action |
| --- | --- | --- |
| Normative Edition 1 language, bytecode, package, and standard-library definitions | Ready locally | `spec/LANGUAGE-1.md`, `spec/BYTECODE-1.md`, `spec/PACKAGE-1.md`, and `spec/STANDARD-LIBRARY-1.md` |
| Frozen external conformance corpus | Ready locally | `conformance/edition1-baseline.json`; 27 vectors; SHA-256 pinned in `release/policy.json` and checked by `tests/conformance.rs` |
| Compatibility migrations | Ready locally | `src/migration.rs` and `tests/language.rs` cover supported 0.2–0.9 inputs and idempotence |
| Exact, checksum-pinned dependency resolution | Ready locally | Package resolver tests, lockfile tests, immutable archive checks, and `spec/PACKAGE-1.md` |
| Runtime, VM, JIT, GC, FFI, registry, and tooling test suites | Ready locally | Workspace, language, native, property, FFI, and JIT tests; the latest full local suite passed on 2026-07-26 using Rust 1.85.0 |
| Performance gate | Ready locally | `benches/performance.rs`; latest release gate measured a 1.906x tiered speedup on 2026-07-26 |
| Deterministic distributable archives | Ready locally | `tools/package_release.py` and `tools/test_release_packager.py`; archives use fixed metadata and include the executable, project documents, Edition 1 specifications, locked dependency inventory, and available dependency license/notice files |
| Reproducible native builds | Ready locally, hosted proof pending | `.github/workflows/release.yml` performs two clean builds through the same target path and compares them; the local macOS ARM64 check was byte-identical, but all hosted runners must still pass |
| Six tier-one platform checks | Pending external evidence | `.github/workflows/ci.yml` defines Linux, macOS, and Windows on x64 and ARM64. A hosted repository and successful immutable CI runs are required |
| Dependency security monitoring and fuzzing | Ready locally, hosted scheduling pending | `.github/workflows/security.yml` uses RustSec; `.github/workflows/fuzz.yml` runs both fuzz targets; `.github/dependabot.yml` covers Rust, fuzz, editor, and workflow dependencies. Scheduled executions require a hosted repository |
| Private vulnerability-reporting channel | Pending owner decision | `SECURITY.md` intentionally contains no invented address. Add a monitored security email or enable a documented private reporting facility before public distribution |
| Six-month Edition 1 compatibility freeze | Time-gated | Began 2026-07-26 and cannot pass before 2027-01-26 |
| Three independent 30-day production pilots | Pending real users | Add only independently reviewable records under `release/pilots/`; synthetic records are prohibited |
| Version and immutable signed release | Pending final gate | Change all workspace packages to exactly `1.0.0` only after every other gate passes, then use a matching `v1.0.0` tag and the attested release workflow |

## Local verification snapshot

- Rust minimum supported version: 1.85.0.
- `cargo fmt --all -- --check`: pass.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: pass.
- Full workspace/language/native/property/FFI/JIT suite: pass after the migration and packaging changes.
- Release archive unit tests: 2 pass.
- Two independently created beta archives had identical SHA-256 digests and valid member integrity.
- A clean release-only macOS ARM64 build produced two byte-identical archives with 121 dependency license/inventory entries. The extracted beta ran `niv version` and `examples/hello.niv` successfully; archive SHA-256: `cdcb9e6f7c6b0b9557ed9bdc8d1ef5c3f810fcf6ff99b0fb08263ffb778e75a7`.

## Before publishing 1.0

Create a hosted repository with immutable CI history, configure the private reporting channel, collect real pilot evidence, wait for the compatibility freeze, rerun every gate on all tier-one platforms, review dependency and fuzz findings, and only then promote the version and tag. Any failure resets the affected evidence; it must not be waived by editing `release/policy.json`.
