# nivren_trace

Bounded W3C Trace Context parsing, formatting, propagation headers, and OS-random parent/child identifiers. Version, widths, flags, hexadecimal encoding, and forbidden all-zero identifiers are validated explicitly.

The package also creates canonical OTLP/HTTP JSON for one bounded span with string attributes and exports it through an explicit caller-supplied endpoint, headers, timeout, and `Network` capability. Span names, attribute counts and sizes, Int64 nanosecond strings, response bodies, and endpoint lengths have fixed ceilings; transport success remains visible as the returned HTTP status.
