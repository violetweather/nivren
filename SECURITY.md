# Security Policy

## Supported versions

Nivren is currently an Edition 2 compatibility beta and is not yet approved for executing untrusted source, bytecode, package, or registry content.

| Version | Security fixes |
| --- | --- |
| 0.9.x beta | Yes |
| 0.8.x and earlier | No |

## Report a vulnerability

Report suspected vulnerabilities through [GitHub private vulnerability reporting](https://github.com/violetweather/nivren/security/advisories/new). Do not open a public issue for an unpatched vulnerability.

Include affected versions, reproduction steps or a minimal proof of concept, impact, and any suggested mitigation. Remove credentials, personal data, and unrelated secrets from evidence.

The parser, checker, runtime, package tooling, bytecode verifier, native interface, JIT, and registry are security boundaries. Reports are evaluated across all supported platforms and coordinated fixes will include regression tests and release notes.
