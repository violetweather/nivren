# nivren_aws

Official AWS Signature Version 4 request signing for Nivren Edition 4.

`sign_v4` accepts an already canonical method, URI, query, lowercase header block, signed-header list, and payload. It derives the date/region/service signing key with HMAC-SHA-256 and returns the authorization header plus payload and canonical-request hashes. Network authority remains outside the package: applications attach these values to a bounded certificate-verified `std.web.request`.

Canonical URI/query/header construction is deliberately explicit because service-specific escaping and duplicate-query rules are security-sensitive. The package is tested against AWS's published IAM `ListUsers` Signature Version 4 example. Applications should obtain credentials through an opaque native/cloud key-store adapter rather than hard-code them.
