# nivren_database

Production contracts and a capability-scoped host adapter for connection pools, bounded query/execute/transaction requests, ordered migrations, and cursor-based result pages. Requests and pages use derived strict JSON so C, Rust, host-callback, and remote driver adapters share one typed schema.

`open_driver`, `query_driver`, and `execute_driver` declare `Native within "database"`, exchange bounded JSON through opaque owned handles, and close deterministically through `using`. A concrete PostgreSQL, MySQL, SQLite, or managed-service host still owns TLS, cancellation, timeout, connection-string secrecy, and transaction policy. `nivren_redis` remains the first fully implemented network driver; SQL adapters consume `nivren_sql.Query` and this package's `DriverRequest`.
