# nivren_oidc

Explicit OpenID Connect authorization-code foundations: RFC 7636 S256 PKCE challenges, deterministic HTTPS authorization URLs, strict typed core-claim decoding, and issuer/audience/nonce/expiry/issued-at validation with bounded clock skew.

Token signature verification stays deliberately separate: verify an algorithm-pinned token with `nivren_jwt.verify_eddsa` or another trusted verifier, then pass the authenticated payload to `validate_id_claims`. Discovery documents, key rotation, refresh tokens, and provider-specific claims remain adapter responsibilities.
