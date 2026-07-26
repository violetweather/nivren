# Public registry security and operations

Nivren's public-registry trust protocol is separate from transport security. HTTPS protects a download in transit; the v1 trust documents let a client authenticate it offline against a pinned registry root key.

## Trusted publishing

The registry root signs a `PublisherAuthorization` binding a publisher Ed25519 key to one publisher name, source repository, CI workflow identity, and expiration. A trusted publishing service validates its CI provider's short-lived OIDC token before issuing or accepting this authorization; long-lived registry passwords are not part of the protocol.

The authorized publisher signs `ReleaseProvenance`, which binds the exact `.nivpkg` SHA-256 and embedded name/version to the publisher, public key, repository, workflow, source commit, and issuance time. Clients require strict Ed25519 verification and exact identity equality with the root authorization.

## Advisories and incident response

The root signs advisories and a short-lived monotonic `RegistryStatus`. Active advisories block listed package versions. Status can revoke publisher keys and freeze package names with a reason. Clients reject expired or future-dated status and accept a caller-supplied minimum generation so a mirror cannot replay pre-incident state. Production clients persist the highest verified generation.

An incident operator should:

1. Freeze the affected package and publish a higher-generation status.
2. Revoke compromised publisher keys.
3. Publish a signed advisory identifying affected versions and severity.
4. Preserve packages, attestations, identity-provider logs, and registry audit logs.
5. Authorize a replacement key only after ownership recovery.
6. Publish fixed versions, then withdraw the advisory or unfreeze only when remediation is verified.
7. Publish a post-incident report and rotate the offline root through a separately documented ceremony if root compromise is suspected.

## Client verification

```text
niv registry verify-release package.nivpkg provenance.json authorization.json status.json advisories.json root.pub UNIX_TIME MINIMUM_GENERATION
```

JSON documents reject unknown fields. Verification checks signatures, expiration, status rollback, revocation/freeze state, provenance identity, commit shape, exact checksum/name/version, and active advisories before returning the decoded package.

## Hostable service

`niv registry envelope` creates the bounded binary publish request (`NIVE`, version, length-prefixed strict JSON provenance/authorization, and `.nivpkg`). `niv registry serve REGISTRY ADDRESS MINIMUM_GENERATION` starts a fixed 16-worker daemon with a bounded queue, 15-second socket deadlines, 64 KiB headers, a 66 MiB body limit, exact `Content-Length`, no transfer encoding, allowlisted GET paths, immutable writes, and serialized publication.

Publish an envelope with `Content-Type: application/vnd.nivren.publish-v1` to `POST /v1/publish`. The daemon loads the pinned root, current status, and advisories from `v1/trust`, performs full release verification, and only then writes package, index, provenance, and authorization documents. A signed envelope is the authorization; replay can only reproduce the identical immutable release.

The service intentionally does not implement TLS or edge rate limiting. Deploy the non-root container from `deploy/registry` behind a hardened reverse proxy and keep the offline root key outside the service host.
