# Nivren 1.0 release audit

Audit date: 2026-07-26

**1.0 is not releasable and no repository/site publication is authorized yet.** Edition 3 is a working draft. Its bounded async TCP/HTTP/TLS/WebSocket foundation and twenty-two-package baseline are implemented, but `docs/CAPABILITY_AUDIT.md` still records material gaps in complete-language AOT, platform/domain breadth, hosted ecosystem operations, systems escape hatches, deployment/signing, independent audits, usability trials, and production pilots.

## Evidence currently passing locally

- Edition 2 and Edition 3 black-box conformance suites execute the external `niv` binary.
- Workspace unit, language, property, FFI, JIT, and documentation tests pass.
- Strict workspace Clippy passes with warnings denied.
- Edition 3 has checked capabilities plus runtime project grants, typed `or give`, `through`, structured concurrency words, generic protocols and collections, immutable bytes, deterministic `using`, unified project commands, instruction budgets, call-depth limits, bounded WebSocket text clients/servers over plain or configured verified TLS, stable JSON observation export, and privacy-safe crash reports.
- Bytecode version 5 is bounded, recursively verified, bundle-tested, and documents `Using`, `Propagate`, canonical shape schemas, payload-choice metadata, and coherent protocol dispatch.
- Deterministic package, registry trust, checksum, provenance, advisory, installer, and native release foundations remain in the tree.
- Reproducible WASI Preview 1 and zero-import browser compiler/runtime guests execute check, diagnostics, formatting, bytecode compilation, and Edition 3 programs through a bounded owned-memory ABI. The public browser SDK passes the same real-module test.
- Bounded asynchronous native host operations return ordinary structured tasks; transferable sequentially consistent atomic integers pass four-task contention in both engines.
- Twenty-two official packages, including deterministic bounded gzip/zlib compression, explicit-schema CSV tables, descriptive statistics and matrices, RGB raster and PCM16 audio interchange, cloud signing, OIDC/PKCE, tracing/metrics, capability-checked random keys, bounded Argon2id password storage, random-nonce ChaCha20-Poly1305 authenticated encryption, and algorithm-pinned HS256/Ed25519 JWT authentication, rebuild, document, publish, install, lock, import, and execute together in both engines.
- A 34.6 MB OCI image builds locally, runs the CLI from a read-only filesystem as non-root UID/GID 10001, includes certificate roots, and has tag-gated amd64/arm64 provenance/SBOM publication automation.

## Blocking gates

Every row in `docs/CAPABILITY_AUDIT.md` must have an idiomatic API, executable example, reference documentation, actionable diagnostics, automated tests, resource/performance expectations, and a supported distribution path. In addition:

- all supported OS/architecture jobs must pass from clean runners twice with byte-identical artifacts;
- installer, PATH, editor, package, registry, upgrade, and uninstall flows must pass on clean machines;
- Edition 3 must complete its compatibility freeze and independent production pilots;
- fuzz, hostile-bundle/package, quota, sandbox, one-way and mutual TLS/network, and supply-chain suites must pass;
- external security findings must be resolved or explicitly release-blocking;
- the language docs, generated API docs, examples, editor, installers, release archives, and website must describe the exact shipped behavior;
- only after all gates pass may the language repository, release artifacts, and website be published.

Past beta release attempts are historical evidence only. Successful partial jobs do not satisfy this audit, and failed/missing release objects must not be represented as shipped versions.
