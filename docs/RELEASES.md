# Nivren release policy

Release tags are `vMAJOR.MINOR.PATCH` and must exactly match every workspace package version. CI builds each native executable twice on Linux, macOS, and Windows for x64 and ARM64 and rejects byte differences. It also reproducibly builds and executes the `wasm32-wasip1` compiler/runtime guest against a real Node WASI host and a zero-import `wasm32-unknown-unknown` guest through the public browser SDK. Each native platform artifact is a deterministic ZIP containing the executable, guided installers, project notices, changelog, user README, current Edition 3 and retained Edition 2 specifications, bytecode/package/WASM specifications, `Cargo.lock`, a compiled-dependency inventory, and available dependency license/notice files. CI also packages the first-party VS Code extension. It publishes SHA-256 checksums and signed build-provenance attestations for every archive, WASM module, browser SDK, and VSIX.

The earlier Edition 2 freeze is no longer sufficient for 1.0 because Edition 3 intentionally changes the pre-user language surface. A new Edition 3 freeze date begins only after its syntax, standard library, bytecode, conformance corpus, and capability audit are complete. At least three independent production pilots must then each run for 30 days without a critical blocker. Pilot evidence belongs in `release/pilots/*.json`; synthetic records never count.

Run `niv release check [repository]` for the same local gate used by the release workflow. A passing result checks the freeze date, pilot evidence, normative files, conformance-corpus floor, all six explicit CI runner labels, and an exact `1.0.0` toolchain version.

Release attestations can be verified with `gh attestation verify --repo OWNER/REPOSITORY <artifact>`. Verify `SHA256SUMS` as well and obtain registry root keys through a separate trusted channel.

To inspect an archive before installation, list its members and confirm it contains a single `nivren-VERSION-PLATFORM` directory. Install the executable from that directory's `bin` folder somewhere on `PATH`; retain the accompanying notices and specifications when redistributing it.
