# Nivren release policy

Release tags are `vMAJOR.MINOR.PATCH` and must exactly match every workspace package version. CI builds each native executable twice on Linux, macOS, and Windows for x64 and ARM64 and rejects byte differences. Each platform artifact is a deterministic ZIP containing the executable, guided installers, project notices, changelog, user README, the Edition 2 language and standard-library specifications, the bytecode and package format specifications, `Cargo.lock`, a compiled-dependency inventory, and all license/notice files supplied by those dependency packages. CI also packages the first-party VS Code extension. It publishes SHA-256 checksums and signed build-provenance attestations for every archive and the VSIX.

Edition 2 entered its compatibility freeze on 2026-07-26. A 1.0 release is prohibited before 2027-01-26 and before at least three independent production pilots have each run for 30 days without a critical blocker. Pilot evidence belongs in `release/pilots/*.json`; the example file is never counted. Do not create synthetic records—each record requires an independently reviewable evidence reference and named release reviewer.

Run `niv release check [repository]` for the same local gate used by the release workflow. A passing result checks the freeze date, pilot evidence, normative files, conformance-corpus floor, all six explicit CI runner labels, and an exact `1.0.0` toolchain version.

Release attestations can be verified with `gh attestation verify --repo OWNER/REPOSITORY <artifact>`. Verify `SHA256SUMS` as well and obtain registry root keys through a separate trusted channel.

To inspect an archive before installation, list its members and confirm it contains a single `nivren-VERSION-PLATFORM` directory. Install the executable from that directory's `bin` folder somewhere on `PATH`; retain the accompanying notices and specifications when redistributing it.
