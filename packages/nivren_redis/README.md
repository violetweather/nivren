# nivren_redis

A capability-visible Redis client written in ordinary Nivren. Commands are UTF-8 byte-counted, limited to 1,024 parts, and encoded without implicit coercion. Network functions declare `needs Network`.

`Connection` represents plain TCP or certificate- and hostname-verified TLS. `authenticate` supports password-only and ACL username/password AUTH. `pipeline` sends up to 4,096 bounded commands in one write and incrementally reads exactly the corresponding responses. Immutable `Pool` and `Lease` values make connection checkout explicit without hidden mutable ownership.

`Response` covers RESP2 plus bounded RESP3 booleans, doubles, big numbers, blob errors, verbatim values, recursive arrays, maps, sets, pushes, and null. Framing never consumes bytes from the next response. Collections are bounded to one million entries, nesting to 128 levels, and caller-controlled frames to 16 MiB.

`Client` and `execute` follow bounded Redis Cluster `MOVED` and `ASK` redirects, including `ASKING`, IPv4, hostnames, bracketed IPv6, optional authentication, and TLS policy reuse. The reproducible Docker matrix runs HELLO 3, SET, and GET through both Nivren engines against every currently supported Redis line: 6.2, 7.2, 7.4, 8.0, 8.2, 8.4, 8.6, and 8.8.
