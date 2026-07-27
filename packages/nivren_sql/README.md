# nivren_sql

Pure, deterministic construction of parameterized SQL `SELECT` queries. Identifiers are restricted to ASCII letters, digits, and underscores and cannot start with a digit. Values are kept in an ordered parameter array and represented only by `?` placeholders, preventing them from being interpolated into SQL text.

This package does not open database connections or claim compatibility with every SQL dialect. Drivers can consume its exported `Query` shape while keeping transport authority in a separate capability-bearing package.
