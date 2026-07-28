# Official Nivren packages

Official packages extend the standard library without compiler privilege, lifecycle scripts, or hidden native code. Their source, tests, generated API reference, deterministic archive, and compatibility status live with the language release.

## Initial 1.0 set

| Package | Purpose | Public surface |
| --- | --- | --- |
| `nivren_aead` | Opaque-key, random-nonce ChaCha20-Poly1305 authenticated encryption | `Sealed`, `import_key`, `generate_key`, `seal`, `seal_with_nonce`, `unseal` |
| `nivren_aws` | Explicit AWS Signature Version 4 request authentication | `Signature`, `sign_v4` |
| `nivren_columnar` | Typed immutable columnar tables | `Column`, `Table`, `table`, `select` |
| `nivren_compression` | Deterministic bounded gzip/zlib compression | `gzip`, `gunzip`, `zlib`, `unzlib`, `gzip_text`, `gunzip_text` |
| `nivren_csv` | Bounded explicit-schema CSV tables and file helpers | `decode`, `encode`, `decode_with`, `encode_with`, `read`, `write` |
| `nivren_database` | Bounded driver/pool/migration/result contracts, a capability-scoped host adapter, and a bundled rooted SQLite implementation | `PoolConfig`, `DriverRequest`, `Migration`, `QueryPage`, `validate_pool`, `validate_request`, `validate_migrations`, `encode_request`, `decode_page`, `open_driver`, `query_driver`, `execute_driver` |
| `nivren_desktop` | System-webview window, typed bridge, signed update contracts, and a scoped host adapter | `Window`, `BridgeMessage`, `UpdateManifest`, `validate_window`, `encode_message`, `validate_update`, `open_host`, `send_bridge`, `stage_update` |
| `nivren_testing` | Typed assertions and deterministic concurrency gates | `Gate`, `expect_equal`, `expect_yes`, `expect_no`, `gate`, `open`, `pass`, `checkpoint` |
| `nivren_crypto` | Bounded SHA-256 fingerprints and HMAC authentication | `fingerprint`, `sign`, `verify` |
| `nivren_sql` | Injection-resistant parameterized SQL construction | `Query`, `identifier`, `select`, `where_equal` |
| `nivren_stats` | Deterministic small-data descriptive statistics | `sum`, `mean`, `variance`, `minimum`, `maximum`, `normalize` |
| `nivren_redis` | RESP2/RESP3, verified TLS, AUTH, pipelines, functional pools, and bounded Cluster redirects | `Response`, `Connection`, `Pool`, `Client`, `command`, `open`, `open_secure`, `authenticate`, `pipeline`, `pool`, `client`, `execute` |
| `nivren_routing` | Exact/parameter routes, request policies, bearer presence, and bounded responses | `Route`, `RouteMatch`, `RequestContext`, `RequestPolicy`, `Response`, `route`, `matches`, `first_match`, `match_route`, `first_parameterized_match`, `validate_request`, `response` |
| `nivren_validation` | Structured field validation | `Violation`, `required`, `positive`, `range` |
| `nivren_discord` | Bounded REST/command payloads, retry/rate-limit decisions, and typed Gateway plans/events | `Message`, `Command`, `RetryPolicy`, `RetryDecision`, `GatewayPlan`, `GatewayEvent`, `message_body`, `bot_headers`, `validate_command`, `command_body`, `retry_decision`, `validate_gateway`, `identify_body`, `decode_event`, `send_message` |
| `nivren_gpu` | Portable checked WGSL compute, CPU fallback, and scoped device-host adapter | `ComputeLimits`, `AddPlan`, `ComputeArtifact`, `VectorResult`, `validate_plan`, `add_cpu`, `add_wgsl`, `compile_add`, `execute_gpu` |
| `nivren_image` | Bounded RGB raster images and canonical binary PPM interchange | `Image`, `image`, `encode_ppm`, `decode_ppm` |
| `nivren_jwt` | Algorithm-pinned compact HS256 and Ed25519/EdDSA JSON Web Tokens | `sign_hs256`, `verify_hs256`, `sign_eddsa`, `verify_eddsa` |
| `nivren_matrix` | Bounded dense matrix operations | `Matrix`, `matrix`, `at`, `add`, `multiply`, `transpose` |
| `nivren_metrics` | Prometheus/OpenMetrics text exposition | `Sample`, `sample`, `encode` |
| `nivren_oidc` | OIDC authorization-code/PKCE and explicit core-claim policy | `Authorization`, `CoreClaims`, `pkce_challenge`, `authorization_url`, `validate_claims`, `validate_id_claims` |
| `nivren_secrets` | Argon2id password storage and OS-backed random keys | `random_key`, `hash_password`, `hash_password_with_salt`, `verify_password` |
| `nivren_svg` | Deterministic escaped vector interfaces | `Canvas`, `canvas`, `add`, `rect`, `text`, `render` |
| `nivren_trace` | W3C Trace Context plus bounded OTLP/HTTP JSON export | `Context`, `OtlpAttribute`, `OtlpSpan`, `context`, `fresh`, `child`, `parse`, `traceparent`, `headers`, `otlp_attribute`, `otlp_span`, `encode_otlp_json`, `export_otlp_json` |
| `nivren_wav` | Canonical bounded PCM16 audio | `Audio`, `encode_pcm16`, `decode_pcm16` |

Install exact versions and import their entry modules:

```text
niv add nivren_validation 1.0.0
niv install /path/to/registry
```

```nivren
use "@nivren_validation"

keep port set nivren_validation.range with { field set "port" value set 443 minimum set 1 maximum set 65535 }
```

## Compatibility policy

- Package versions follow semantic versioning. Removing or changing an exposed declaration, parameter/result type, capability requirement, error shape, or deterministic behavior requires a major version.
- A minor release may add exposed declarations or broaden accepted input without changing existing successful results. Patch releases correct defects without expanding authority.
- Every official release declares an exact minimum Nivren edition and is tested on all six tier-one OS/architecture jobs against the oldest and current supported compiler lines.
- Deprecations remain documented and functional for at least one minor line before a major removal. Security fixes may reject previously accepted unsafe input, with an advisory and compatibility note.
- Official packages contain no install/build scripts. Capability requirements remain visible in `.niv` declarations and are authorized by the consuming project.
- A release is publishable only when its package tests, generated API docs, byte-identical rebuild, registry publish/fetch, clean install, lock verification, and a combined consumer run pass.

The repository integration suite builds all twenty-five official packages twice, publishes them to a temporary immutable registry, installs them into a clean consumer, and runs that consumer in both execution engines. The public website mirrors the same package names, versions, install commands, and exposed APIs.

The Redis release matrix additionally runs RESP3 negotiation and a SET/GET round trip through both engines against Redis 6.2, 7.2, 7.4, 8.0, 8.2, 8.4, 8.6, and 8.8. Run it with `release/test-redis-matrix.sh`; Docker images are disposable and persistence is disabled.
