# nivren_crypto

Bounded cryptographic building blocks for Nivren applications. `fingerprint` uses SHA-256; `sign` and `verify` use HMAC-SHA-256, with verification performed by the audited constant-time backend. Keys are capped at 1 MiB, messages at 16 MiB, and malformed tags return typed errors.

This package intentionally does not provide password hashing, encryption, key generation, or certificate management yet. Applications must not substitute these primitives for those higher-level constructions.
