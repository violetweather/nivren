# Nivren SQLite host

This crate is the concrete SQLite implementation for the Edition 4 `nivren_database` package. The default host-runtime CLI installs it for projects that grant `Native`; Nivren source must still declare and receive `Native within "database"` before it can open a driver.

Configurations use `memory://name` for an isolated in-memory database or `sqlite:relative/path.db` for a database rooted under the project's `.nivren/database` directory. Absolute paths, traversal, empty paths, and non-normal components are rejected.

The adapter supports parameterized query and execute operations plus explicit begin, commit, and rollback. It bounds configuration, statements, parameter count, rows, timeout, and the final response. Text must be UTF-8, non-finite floats fail, blobs are represented only by their byte length, and every opaque connection closes through the runtime's deterministic handle contract.

SQLite is compiled with rusqlite's `bundled` feature so supported native builds do not depend on an unknown system SQLite version. This host is unavailable in browser Wasm; portable programs must choose a host-provided remote database adapter there.
