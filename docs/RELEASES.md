# Nivren release policy

Release tags are `vMAJOR.MINOR.PATCH` and must exactly match every workspace package version. CI builds each native executable twice on Linux, macOS, and Windows for x64 and ARM64 and rejects byte differences. It also reproducibly builds and executes the `wasm32-wasip1` compiler/runtime guest against a real Node WASI host and a zero-import `wasm32-unknown-unknown` guest through the public browser SDK. Each native platform artifact is a deterministic ZIP containing the executable, guided installers, project notices, changelog, user README, current Edition 4 and retained Edition 2/3 specifications, bytecode/package/WASM specifications, `Cargo.lock`, a compiled-dependency inventory, and available dependency license/notice files. CI also packages the first-party VS Code extension. It publishes SHA-256 checksums and signed build-provenance attestations for every archive, WASM module, browser SDK, and VSIX.

Edition 4 Beta is checkpoint-gated. Language, Intent, Compiler, and Product Proof evidence must pass before a coordinated language/site release. The later 1.0 decision additionally requires independent production pilots and a compatibility freeze; pilot evidence belongs in `release/pilots/*.json`, and synthetic records never count.

Run `niv release check [repository]` for the machine-enforced Edition 4 Beta gate. It verifies the frozen Edition 4 corpus, required release files and platform jobs, checkpoint state, and fresh named evidence receipts for the platform, installer, artifact, application, docs/site, independent security, and independent signing-recovery gates. Missing or stale evidence fails closed. The later 1.0 decision additionally requires the production pilots and compatibility freeze described above.

## Signed update channels

Stable, beta, and nightly pointers use `ChannelManifest` format 1. Each manifest names one bounded HTTPS release base, an immutable version, a strictly increasing generation, a validity window, and the exact SHA-256 digest of every offered asset. The manifest is canonicalized and signed with a dedicated offline Ed25519 channel key, independently of GitHub transport provenance.

Release automation signs an unsigned manifest with `niv release sign-channel <manifest.json> <secret-key-file> <signed.json>`. Verification uses `niv release verify-channel <signed.json> <public-key-file> <unix-time> <minimum-generation>`. Expired, future, malformed, tampered, or rollback-generation manifests fail closed. Channel signing keys are read only from explicit files and are never stored in the repository.

Install receipts retain the highest trusted generation per channel. Switching channels is explicit; pinning a version disables automatic movement until the pin is removed. A signing-key incident freezes the affected channel, publishes a separately authorized recovery notice, rotates the channel key, and increments generation without reusing an earlier value.

Release attestations can be verified with `gh attestation verify --repo OWNER/REPOSITORY <artifact>`. Verify `SHA256SUMS` as well and obtain registry root keys through a separate trusted channel.

To inspect an archive before installation, list its members and confirm it contains a single `nivren-VERSION-PLATFORM` directory. Install the executable from that directory's `bin` folder somewhere on `PATH`; retain the accompanying notices and specifications when redistributing it.
