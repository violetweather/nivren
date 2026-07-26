# Third-party components

Nivren's TLS implementation uses Rustls 0.23, rustls-webpki, Ring, and webpki-roots. `Cargo.lock` is authoritative for exact resolved versions. Rustls provides the TLS protocol implementation; webpki-roots supplies Mozilla trust anchors. Their license texts and notices are included by their source distributions and must accompany binary redistribution as required.

The native execution tier uses Cranelift 0.121.2 components from the Bytecode Alliance under Apache-2.0 with the LLVM exception. The JIT boundary is isolated in `crates/nivren-jit`; exact transitive versions remain pinned by `Cargo.lock`.

Public-registry trust documents use ed25519-dalek 3.0 and curve25519-dalek 5.0 under BSD-3-Clause. Nivren uses strict signature verification and domain-separated canonical messages; `Cargo.lock` pins the complete cryptographic dependency graph.

Official binary archives include `Cargo.lock` plus a generated `licenses/INDEX.txt` and every license, copying, and notice file supplied by the third-party Rust packages compiled into that platform build. The index retains the declared SPDX expression and explicitly flags any package that supplied no license file. Those archive files, rather than this architectural overview, are the redistribution inventory for a particular build.
