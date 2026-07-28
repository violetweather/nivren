# nivren_database

Driver-neutral production contracts for connection pools, bounded query/execute/transaction requests, ordered migrations, and cursor-based result pages. Requests and pages use derived strict JSON so C, Rust, host-callback, and remote driver adapters share one typed schema.

The package deliberately does not contain credentials or open sockets. A capability-bearing adapter owns the database connection and must enforce TLS, cancellation, timeout, transaction, and secret-storage policy. `nivren_redis` remains the first fully implemented network driver; SQL adapters consume `nivren_sql.Query` and this package's `DriverRequest`.
