# nivren_routing

Edition 4 routing and request-policy primitives for bounded HTTP services. The package keeps exact and `:parameter` route selection, required headers, bearer presence checks, body ceilings, and response construction pure and independently testable while `std.web` owns sockets and TLS.

`RequestPolicy` is deliberately not a complete authentication system: applications still verify tokens, enforce issuer/audience/expiry, implement authorization, and protect secrets. Parameterized routes accept at most 128 path parts; request and response bodies remain capped at 16 MiB.
