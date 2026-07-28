# Edition 4 independent security audit scope

Edition 4 Beta cannot ship until an assessor independent of the implementation records a passing `security-audit` evidence receipt and every critical or high finding is resolved. A report may redact exploit details, but it must identify the audited commit, artifacts, platforms, methods, finding severities, retest results, and assessor.

## Required boundaries

The audit covers:

- lexer, parser, formatter, type checker, derives, module loading, diagnostics, and malicious source input;
- Bytecode 7 decoding, verification, VM execution, JIT/AOT inputs, malformed native and Wasm artifacts, and resource limits;
- intent graphs, `prepare` portability, `perform` ordering, capability propagation, scoped authority, cancellation, cleanup, and plan inspection;
- managed memory, structured tasks, channels, iterators, files, sockets, TLS, HTTP, WebSockets, databases, and secret handling;
- dynamic libraries, generated bindings, compiler facade, C ABI ownership, callbacks, opaque handles, unsafe modules, desktop bridge, mobile wrappers, and GPU buffers;
- package archives, dependency/authority locks, cache, registry ownership/yanking/advisories, signed provenance, update channels, installers, rollback, uninstall, and signing-key recovery;
- LSP, debugger adapter, profiler, crash reports, documentation examples, browser/WASI guests, desktop packaging, containers, and CI/release workflows.

## Minimum methods

The assessor must combine manual design review with automated dependency review, static analysis, property testing, coverage-guided fuzzing, hostile artifact corpora, sanitizer or memory-check execution, permission-boundary tests, protocol/network tests, and supply-chain/recovery exercises. Findings must state exploitability and affected support tiers rather than relying only on scanner severity.

## Pass conditions

- No unresolved critical or high finding.
- Medium and low findings have an owner, remediation or explicit beta limitation, and public-safe disclosure text.
- Fixed findings have independent retest evidence against the release candidate.
- The audited commit and every delivered artifact are named by SHA-256.
- The evidence receipt is marked independent, is no older than the release policy permits, and points to retained audit artifacts.

The repository's automated tests are implementation evidence, not an independent audit. A project maintainer cannot self-approve this gate.
