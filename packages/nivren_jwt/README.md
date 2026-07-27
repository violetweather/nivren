# nivren_jwt

Bounded compact JSON Web Tokens with algorithm-pinned HS256 and Ed25519/EdDSA signing and verification. Verification rejects header algorithm confusion, malformed base64url, invalid signatures, and non-JSON payloads before returning canonical payload text. Applications remain responsible for issuer, audience, expiry, nonce, and authorization policy.

`sign_hs256` authenticates with HMAC-SHA-256. `sign_eddsa` keeps its Ed25519 seed in an opaque `SecretKey` and publishes verification through ordinary 32-byte public keys. Both modes canonicalize JSON, use unpadded base64url, require exactly three segments, and authenticate the complete header/payload input before returning payload data.

This package authenticates compact payloads; it deliberately does not infer authorization or time policy. Applications must decode claims into their own shapes and enforce issuer, audience, expiration, and clock-skew rules explicitly.
