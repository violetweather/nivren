# Security Policy

## Supported versions

The published 0.9 compatibility beta remains supported while Edition 4 is developed locally. Edition 4 is not approved for executing untrusted source, bytecode, packages, registry data, update manifests, or native integrations until Product Proof and the independent audit in `docs/SECURITY_AUDIT_SCOPE.md` pass.

| Version | Security fixes |
| --- | --- |
| 0.9.x beta | Yes |
| 0.10.x Edition 4 candidate | Not published |
| 0.8.x and earlier | No |

## Report a vulnerability

Report suspected vulnerabilities through [GitHub private vulnerability reporting](https://github.com/violetweather/nivren/security/advisories/new). Do not open a public issue for an unpatched vulnerability.

Include affected versions, reproduction steps or a minimal proof of concept, impact, and any suggested mitigation. Remove credentials, personal data, and unrelated secrets from evidence.

The parser, checker, intent model, runtime, package tooling, bytecode verifier, native interface, JIT/AOT backends, Wasm guests, capability policy, registry, updater, installers, desktop bridge, mobile wrappers, and GPU boundary are security boundaries. Reports are evaluated across all supported platforms and coordinated fixes include regression tests and release notes.
