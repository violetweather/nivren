# Production registry operations and recovery

This runbook describes the supported v1 registry daemon. A hosted beta registry remains unavailable until its clean deployment, backup/restore, monitoring, and incident exercise produce Product Proof evidence.

## Deploy

1. Put the registry behind a maintained TLS terminator. Expose only `GET` allowlisted content, `GET /healthz`, signed `POST /v1/publish`, and root-signed `POST /v1/admin`.
2. Run the daemon as a dedicated non-root account with a private, durable registry volume. Keep root and publisher secret keys outside the host.
3. Install `v1/trust/root.pub`, a current signed `status.json`, and signed `advisories.json`. Start with a configured minimum status/admin generation from independently retained state.
4. Back up immutable packages, index metadata, ownership, provenance, authorizations, admin audit records, trust documents, generations, and identity-provider logs. Test restoration into an isolated host before admitting traffic.
5. Monitor health, queue saturation, disk space, TLS expiry, status expiry, failed signatures, replay attempts, recovery markers, publication latency, and checksum/integrity scans.

## Publish and administer

- Accept releases only through a verified publish envelope. An identical replay is idempotent; different bytes for one identity fail.
- Create yank/unyank actions offline with `niv registry sign-admin`, inspect them with `verify-admin`, then POST over authenticated operator transport. Every action needs a unique increasing generation and a bounded public-safe reason.
- Retain the signed request, response, immutable audit record, registry generation, operator approval, and incident/ticket identifier.
- Never delete a yanked package. Preserve bytes for existing locks, investigation, and advisories.

## Recover a pending admin action

`v1/admin/pending.json` means a signed operation may have stopped between atomic files. The daemon refuses further administrative changes while it exists.

1. Remove the registry from write traffic and snapshot the entire volume.
2. Verify the pending document against the separately retained root public key and confirm its generation is not older than `v1/trust/admin-generation`.
3. Compare the target index state, `v1/admin/GENERATION.json`, and persisted generation with the signed action. Do not edit any of them manually.
4. Run `niv registry recover-admin REGISTRY NOW_UNIX MINIMUM_GENERATION`. Recovery re-verifies the signature and validity window, idempotently applies the exact state, writes or checks the immutable audit record, advances the generation, and removes only that pending marker.
5. Run search/fetch behavior checks, an integrity scan, and backup. Restore traffic only after a second operator reviews the retained evidence.

If verification fails, the audit record differs, the pending generation is older than persisted state, or the package bytes/index identity are inconsistent, keep the service read-only and treat it as a security incident.

## Root or publisher key incident

Freeze affected packages in a newly signed registry status, advance its generation, preserve all evidence, revoke publisher keys, and issue advisories as needed. A suspected root compromise stops publication and administration entirely; rotate trust through the separately documented out-of-band recovery authority before clients accept a new root.

## Disaster recovery evidence

A qualifying release drill restores a clean host from backup, validates every immutable checksum and signature, proves minimum generations did not roll back, exercises publish/search/fetch/yank/unyank/pending recovery, confirms old locks remain retrievable according to policy, and records artifact SHA-256 values in the Product Proof receipt.
