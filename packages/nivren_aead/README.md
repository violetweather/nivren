# nivren_aead

Official authenticated encryption for Nivren Edition 4 using ChaCha20-Poly1305.

`generate_key` creates an opaque, redacted `SecretKey` through the visible `Random` capability. `import_key` copies exactly 32 bytes into the same zeroized key storage for protocol and key-store integration. Secret keys cannot be compared, serialized, used as collection keys, or transferred into tasks.

`seal` creates a fresh 12-byte nonce through `Random` and returns a `Sealed` value containing the nonce and authenticated ciphertext. `unseal` authenticates both ciphertext and caller-supplied context before releasing plaintext. Associated data is capped at 1 MiB, and plaintext/ciphertext is capped at 16 MiB including the 16-byte tag.

Nonces must never repeat with the same key. `seal_with_nonce` exists for deterministic protocols and tests; ordinary application encryption should call `seal`.
