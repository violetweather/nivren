# Product Proof evidence receipts

`niv release check` reads one JSON receipt for every gate named by `release/policy.json`. Receipts are produced by the corresponding clean workflow or independent assessor; they are never copied from example data or marked passing by hand merely to unblock a release.

Every receipt uses format 1:

```json
{
  "format": 1,
  "gate": "platform-matrix",
  "status": "pass",
  "completed_at_unix": 1800000000,
  "run_id": "github-actions-run-or-audit-id",
  "independent": false,
  "artifacts": [
    {
      "name": "retained-result-or-report",
      "sha256": "64-lowercase-hex-digits"
    }
  ]
}
```

The checker rejects a wrong gate, non-passing status, future or stale completion time, missing run ID, empty artifact list, malformed digest, or a non-independent receipt for a gate that requires independence. Release operations retain the referenced artifacts and provenance beside the final build.
