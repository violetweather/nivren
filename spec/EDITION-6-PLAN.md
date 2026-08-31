# Edition 6 plan: the runtime edition, ending at 1.0

## 1. Premise

Edition 5 finished the language and froze it: its finality clause forbids new syntax, keywords, operators, and capability names forever. Edition 6 therefore changes nothing about how Nivren reads. It is the edition where the machine underneath catches up to the language on top, and it ends with the 1.0 release.

Two goals, one release:

1. **Close the performance gap with Node.js on every workload class** — not only the rows Nivren already wins, but the ones it would lose today: shape-heavy work, large JSON, allocation churn, and warmed long-running services.
2. **Close the Product Proof** — the project's own published 1.0 gates — plus the two things that cannot be coded: an independent security audit and field time with real users.

Everything lands through the established discipline: dual-engine tests, conformance vectors, fuzzing, CI performance gates, and published benchmarks that report losses as plainly as wins.

## 2. Phase B — measure first (the benchmark build-out)

No optimization lands before the measurement exists. The Nivren-versus-Node suite grows from seven rows to a full probe:

- **B1. Rescale the compute rows.** fibonacci(35)-class recursion and hundred-million-step loops, so sustained compute dominates and startup stops flattering the ratios.
- **B2. Add the adversarial rows.** A shape-construction loop, a megabyte-scale typed JSON transform, and an allocation-churn workload. These are expected to lose at first. They are published anyway, under the former-limits pattern that made the page credible.
- **B3. Add a warmed service row.** An HTTP echo service measured for requests per second and p99 latency against Node on identical hardware — Node's home turf, measured hot, after warmup, in one long-lived process.
- **B4. Add a concurrency row and a memory column.** Task spawn/join and bounded-channel throughput; peak memory measured on Windows and Linux, not only macOS.

Exit gate: the expanded report is committed, the site publishes it including every losing row, and the harness runs from one command.

## 3. Phase M — the memory generation

The single biggest gap is value representation, not compilation. Ordered smallest risk first:

- **M1. Shared field-name tables.** A shape's field names are stored once at declaration; constructing a value stops copying every name. Expected to remove most of the shape-row cost on its own.
- **M2. JSON without copies.** Parse and encode large documents with borrowed slices and preallocated buffers instead of per-node values.
- **M3. Packed values.** Small values (integers, booleans, floats) stop being heap-touching boxes; reference-count traffic disappears from hot paths. The widest, riskiest change in the codebase; it lands alone, behind the full gate set, with nothing else moving.
- **M4. Nursery allocation.** Short-lived values allocate from a bump region and die in bulk, replacing system-allocator calls in churn-heavy code.

Exit gate: the shape and JSON rows beat Node; the allocation-churn row is within 2× of Node or better, with the honest number published either way.

## 4. Phase N — the native generation

Edition 5's engine work (escape-aware slots, block-based Cranelift codegen, whole-program integer plans with direct native calls) is the seed. Phase N grows it to the whole language:

- **N1. Shapes in native code.** The checker already proves field types; integer-only shapes compile to raw native slots. Extends the existing kind-analysis pattern.
- **N2. Floats, booleans, and text handles in plans.** The plan vocabulary grows until typical hot functions compile whole.
- **N3. Ahead-of-time whole-program output.** `niv ship` gains a mode that emits a true machine-code binary with no embedded interpreter for programs the planner fully covers, with graceful fallback to the bundled-VM form otherwise.
- **N4. The self-hosting proof.** `niv check`'s front half, written in Nivren, shipped as a maintained example and checked against its own source in CI — the credibility milestone a frozen-syntax language owes itself.

Exit gate: the warmed service row and every compute row are green; an AOT-shipped binary passes the conformance vectors.

## 5. Phase P — Product Proof closure

The remaining published 1.0 gates, closed one by one, each with the evidence the docs already demand:

- Desktop system-webview hosts released for macOS, Windows, and Linux, with origin, CSP, allowlist, packaging, signing, notarization, and updater-recovery evidence.
- Mobile embedding out of experimental: Xcode and Android NDK builds, lifecycle and cancellation tests, device examples, per-ABI packaging.
- PostgreSQL and MySQL hosts joining the bundled SQLite host, with streaming, pooling, and migration reference services.
- A real WebGPU host behind the existing checked plans, with the GPU-unavailable fallback matrix.
- Debugger completion: real pause/resume/step over the DAP, with multi-editor evidence.
- Signed production channels live: public channel manifests, the signing-recovery drill, and clean-platform update/rollback evidence.
- Accessibility and link checks across the site and docs.

Exit gate: the Product Proof checkpoint counter the site reports reads complete.

## 6. Phase E — the ecosystem opens

- **E1. Hosted registry.** The registry v1 protocol goes live on a public host with the trusted-publication flow (Ed25519 roots, provenance, advisories) already specified in PACKAGE-2.
- **E2. Outside publishing.** Account issuance, publisher authorization, and yank/advisory operations documented and drilled.
- **E3. The 25 official packages published to the live registry** and installed from it in CI.

Exit gate: a stranger can publish a package and another stranger can depend on it, with signatures verified end to end.

## 7. Phase A — assurance, then 1.0

- **A1. Independent security audit** of the runtime, capability system, package pipeline, and installers. External, paid, calendar-bound. Every critical and high finding closed with a test.
- **A2. Soak.** A release-candidate period with the registry live and real users on the betas; the bar is that incoming reports stay boring for several consecutive weeks.
- **A3. 1.0 ships as Edition 6.** The language spec (frozen), the runtime evidence, the platform matrix, the registry, and the audit report, one release.

## 8. Sequencing and versioning

Phases B → M → N run in order (measure, then memory, then native) because each depends on the previous one's truth. Phase P runs in parallel with M and N — it is platform work, not engine work. Phase E begins once P's signed-channel work exists. Phase A is last by definition.

Each landing is a beta: memory generation as 0.11.0-beta.1 onward, native generation following, then release candidates. Every beta goes through the full pipeline proven in Edition 5: green CI, tagged release, published artifacts, site updated with re-verified snippets and benchmarks the same day.

## 9. Non-goals, stated so they stay non-goals

- No new syntax, keywords, operators, or capability names — Edition 5's finality clause is the contract, and Edition 6 is its first proof.
- No compatibility breaks: Edition 5 source runs unchanged on every Edition 6 beta and on 1.0.
- No benchmark rows quietly removed because they lose. Rows are removed only by replacing them with harder ones.
