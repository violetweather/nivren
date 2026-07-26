# Registry deployment

Build from the repository root:

```text
docker build -f deploy/registry/Dockerfile -t nivren-registry:0.10-beta.1 .
docker run --read-only --cap-drop=ALL --security-opt=no-new-privileges \
  --mount type=bind,src=/srv/nivren-registry,dst=/registry \
  -p 127.0.0.1:8080:8080 nivren-registry:0.10-beta.1
```

Provision `/srv/nivren-registry/v1/trust/root.pub`, `status.json`, and `advisories.json` before starting. The image runs as UID/GID 10001 and only the registry volume is writable.

Terminate TLS and apply IP/account rate limits at a mature reverse proxy. Forward only HTTP/1.1, cap requests at 66 MiB, disable request buffering to disk if the proxy cannot protect temporary storage, and expose `GET /healthz` for health checks. Back up the immutable package/index/provenance trees and signed trust documents with object versioning. Never place the offline registry root secret on this host.

The default container rejects status generations below 1. Increase the final `ENTRYPOINT` argument during incident response and persist the highest accepted generation in deployment configuration to prevent rollback.
