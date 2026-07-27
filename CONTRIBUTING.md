# Contributing

Nivren is company-led open source under Apache-2.0. Changes should include a focused rationale, tests for every behavioral change, and documentation for public behavior.

Before submitting a change, run:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --locked
cargo run -- test
cargo check --manifest-path fuzz/Cargo.toml --locked
(cd editors/vscode && npm ci && npm audit --audit-level=high && npm run check && npm run compile)
```

Use Rust 1.88.0, the minimum supported toolchain. Changes to `Cargo.toml`, the VS Code package, or fuzz dependencies must commit the corresponding lockfile. Before a release, also run the release benchmark gate and `niv release check .`; the latter is expected to remain blocked until its date and real-pilot conditions are satisfied.

Language changes require an RFC describing motivation, semantics, rejected alternatives, compatibility impact, diagnostics, and conformance tests. Implementation behavior is not a substitute for an accepted language rule.
