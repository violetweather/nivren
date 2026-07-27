# nivren_secrets

Official password hashing and secure random key generation for Nivren Edition 3.

Passwords use Argon2id v=19 with a 19 MiB memory cost, two iterations, one lane, a 32-byte output, and a fresh 16-byte operating-system salt. Verification accepts only bounded Argon2id PHC strings and rejects excessive attacker-controlled resource parameters before allocating. `random_key` requires the visible `Random` capability and is capped at 1 MiB.

`hash_password_with_salt` exists for deterministic migrations and tests. Applications must supply a unique cryptographically random salt; ordinary password storage should use `hash_password`.
