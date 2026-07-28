# Edition 4 Product Proof audit

This is the blocking ledger for Checkpoint 4. Product Proof closes only when every promised application, developer, distribution, documentation, platform, and security surface has executable evidence. Nothing in this ledger authorizes publication.

## Preserved baseline

- Language Proof passed at `ac498b2`.
- Intent Proof passed at `9de60c1`.
- Compiler Proof passed at `4b8b04f`.
- Bytecode 7, the VM, complete native control, complete AOT objects, ABI v3, browser Wasm, and WASI are frozen foundations unless a Product Proof test exposes a blocking defect.
- Existing packages, registry, installers, release workflows, documentation, and site are extended rather than replaced.

## Initial audit — 2026-07-28

| Product area | Existing foundation | Blocking completion work |
| --- | --- | --- |
| HTTP, WebSocket, and TLS | Bounded clients/servers, routing package, certificate-verified TLS, mTLS, streaming files/lines/JSON, and dual-engine loopback tests | Authentication/middleware recipes, body/database iterator integration, released-platform evidence, and production checklists |
| Database and data | Parameterized SQL package, transactions, typed rows/JSON/CSV/columnar packages, Redis TLS/pools/Cluster | Driver adapter contract, pooling/migration/query-stream reference service, additional driver fixture, and typed Edition 4 package docs |
| Cryptography and automation | Hash/HMAC, Argon2id, AEAD, JWT/EdDSA, OIDC, AWS signing, secrets, filesystem, process, statistics, and testing foundations | Key-store adapter boundary, automation recipes, capability/resource guidance, and package-by-package production evidence |
| Observability and realtime | Structured logs, metrics, W3C trace context, OTLP/HTTP export, crash reports, inspector, profiler, WebSockets, and Discord REST | Unified telemetry recipe, retry/rate-limit/event-command foundations, native/native-error evidence, and complete Edition 4 docs |
| Desktop | ABI v3 embedding library, browser SDK, native handles, generated bindings, standalone executables | Native host contract, system-webview bridge, typed message boundary, packaging/signing/update metadata, example, tests, and support policy |
| Mobile | C ABI and portable runtime can be embedded | Experimental iOS and Android SDK headers/wrappers, lifecycle/cancellation contract, examples, packaging, and tests |
| GPU compute | Matrices, columnar data, fixed-width numbers, browser Wasm, and declared SIMD boundary | Checked portable compute API, validated WGSL generation, explicit limits/capabilities, required vectorized CPU fallback, examples, tests, and experimental support policy |
| CLI and workspaces | `new`, `add`, `dev`, `check`, `test`, `build`, `explain`, `ship`, formatter, docs, profile, coverage, debugger, package, registry | First-class `bench`; workspace manifests; incremental multi-package graph; property/fuzz/compatibility commands; deterministic-time test UX; complete help/docs |
| Editor and debugging | LSP, workspace rename, VS Code, source maps, debug hook, inspection/profile/coverage JSON | Debug Adapter Protocol endpoint, compatibility tests, richer async/memory/I/O profiling evidence, and Edition 4 editor/docs conversion |
| Registry | Immutable package artifacts, search, checksums, signed provenance, advisories, trusted HTTPS install, rollback-generation protection | Ownership records, yanking, capability audit metadata, signed immutable contents, production deployment/recovery runbook, and hosted-operation evidence |
| Offline and authority locking | Exact dependencies, deterministic content hashes, complete lockfile verification, cached local registry installs | Explicit offline command/mode, authority lockfile surface, cache management, workspaces, and cold/warm install tests |
| Install and update | Guided Windows/macOS/Linux installers, architecture detection, checksums/attestations, PATH/VS Code setup, safe uninstall | Stable/beta/nightly channels, pinning, signed update, rollback, install receipts, clean-system matrix, and recovery tests |
| Artifacts and supply chain | Reproducible native/Wasm builds, archives, static/shared libraries, containers, SBOM, checksums, GitHub attestations | Channel manifests, signatures independent of transport, rollback metadata, desktop/mobile bundles, complete artifact verifier, and clean-runner evidence |
| Packages and examples | Twenty-two tested official packages and real application examples | Edition 4-only conversion, complete per-package references/recipes/failures/capabilities/performance notes, aggregate promised-capability coverage, and additional desktop/mobile/GPU packages |
| Documentation | Language, standard library, embedding, AOT, Wasm, packages, installer, testing, performance, and release references | Remove stale Edition 3 behavior claims, add progressive CLI/HTTP/database/realtime/desktop/FFI/Wasm/GPU path, compile every snippet, and verify every download/version/link |
| Website | Existing multi-route Nivren site with docs explorer, package catalog/detail routes, install/download pages, examples, benchmarks, responsive design, tests, and Sites configuration | Complete Edition 4 content/data synchronization, new platform/tooling pages, generated package detail depth, verified local downloads/versions/links, accessibility/build tests, and coordinated unpublished release state |
| Platform and security evidence | Six native release jobs, Wasm jobs, fuzz schedules, RustSec, ASan/Valgrind gate, TLS/network suites, hostile artifact tests | Complete Product Proof matrix, installer clean machines, desktop/GPU fallbacks, audit scope checklist, critical/high finding closure, and independent audit evidence |

## Stop-and-correct policy

- Desktop, mobile, or GPU additions may not weaken Compiler Proof or introduce VM fallback.
- A feature remains experimental when clean-platform or independent-audit evidence is unavailable; documentation must say so.
- Registry, updater, installer, signing, or recovery ambiguity blocks publication.
- Documentation or website claims without executable evidence block publication.
- Benchmarks must remain reproducible and include limitations.
- Failed usability, security, performance, or platform evidence changes the product design before release criteria are reconsidered.

## Product Proof progress — local, unpublished

| Slice | State | Executable evidence | Remaining gate |
| --- | --- | --- | --- |
| Official application foundations | In progress | `nivren_database`, `nivren_desktop`, and `nivren_gpu` are Edition 4 packages with package tests; the aggregate release fixture builds, documents, publishes, installs, imports, and executes all 25 official packages together | Concrete database driver, native system-webview host/package, mobile wrappers, and real GPU host/fallback matrix |
| Daily CLI | In progress | `niv bench` performs isolated warmups/samples and exports `org.nivren.benchmark.v1`; new project templates use Edition 4; targeted CLI tests and warnings-denied clippy pass | Workspaces, property/fuzz/compatibility UX, deterministic time, and broader profiler evidence |
| Debugging | In progress | `niv dap` implements bounded framed initialize/launch/breakpoint/thread/stack/scope/variable/continue/disconnect requests and has protocol framing tests | Editor launch integration, pause/resume execution semantics, and multi-editor compatibility evidence |
| Registry and offline | In progress | Signed publication claims immutable first-publisher ownership; metadata records capability scopes/unsafe modules; yank/unyank affects fetch/search without deleting artifacts; offline cache relocking detects tampering | Hosted operations/recovery, signed administrative yank flow, cache management, and cold/warm clean-runner evidence |
| Package website | In progress | Local site now presents 25 package guides; each route includes authority, bounds, failures, performance, and production checks; lint, TypeScript, production build, and 14 rendered-route tests pass | Complete Edition 4 site conversion, runnable snippet verification, version/download synchronization, and accessibility/link evidence |

The gate remains open. These passing slices do not authorize a release or GitHub publication.
