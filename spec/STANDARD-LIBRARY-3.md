# Nivren Edition 3 Standard Library Specification (working draft)

All APIs are immutable members of `std`. Wrong arity or statically known wrong types are static errors; embedding boundaries MUST repeat validation. Fallible operations return `Result<T, String>`, absence returns `T?`, and effectful calls carry the listed capability.

## Preferred namespaces

Edition 3 canonically uses `std.files`, `std.web`, `std.tasks`, and `std.channels`. This namespace decision is final for the 1.x line. The Edition 2 spellings `std.fs`, `std.http`, `std.task`, and `std.channel` are compatibility aliases: they MUST behave identically throughout 1.x, MUST NOT receive alias-only APIs, and MAY be removed only by a future major edition with an automated formatter rewrite. Diagnostics, generated documentation, completion ranking, and all new examples MUST prefer the canonical spellings.

## Core and data

- `len(String|[T]) gives Int`, `type(T) gives String`, `append([T], T) gives [T]`
- `assert(Bool, String) gives Null`
- `ok(T) gives Result<T, E>` and `err(E) gives Result<T, E>`
- `std.json.valid`, `compact`, and `pretty` retain Edition 2 bounded deterministic behavior.
- `std.json.parse(String) gives Result<Value, String>` maps JSON objects to insertion-ordered `Map<String,Value>` and arrays to immutable arrays.
- `std.json.stringify(Value) gives Result<String,String>` accepts JSON-representable Nivren primitives, arrays, string-keyed maps, shapes, choices, exact/fixed-width numbers, and zoned DateTimes; it MUST reject non-finite floats and unsupported handles. `BigInt` and `Decimal` use lossless JSON strings. Bare choices use their variant string; payload choices use an object with exactly `$variant` and `$value`, recursively encoding the payload. Shapes use their declared field names.
- `std.json.decode(S, String) gives Result<S,String>`, where `S` is a shape constructor, derives a strict decoder from the declared shape. The checker MUST infer the nominal result type. Runtime decoding MUST reject missing and unexpected fields, out-of-range numbers, unknown choices, and type mismatches, recursively through nested shapes and collections.
- `std.json.read_next(File, Int) gives Result<Value?,String>` and `read_next_as(S, File, Int) gives Result<S?,String>` need `FileRead` and consume newline-delimited JSON incrementally. The byte limit MUST be 1 through 16 MiB, memory use MUST remain bounded by that limit plus a fixed read buffer, clean end-of-file returns `none`, CRLF is accepted, malformed UTF-8/JSON is data error, and an oversized record MUST be drained through its newline so the next record remains readable.
- `std.iter.lines(File, Int) gives Result<Iterator<Result<String,String>>,String>` needs `FileRead` and lazily consumes one bounded text line at a time. The byte limit MUST be 1 through 1 MiB. CRLF is normalized, EOF ends the iterator, malformed UTF-8 or an oversized line yields one `Err` element, and an oversized line MUST be drained so the following line remains readable. The file remains owned by its `using` scope.
- `std.iter.tcp_lines(TcpStream, Int, Float) gives Result<Iterator<Result<String,String>>,String>` needs `Network` and lazily consumes CRLF-framed UTF-8 lines. Payload limits MUST be 1 through 65,536 bytes and the positive finite timeout applies independently to each pull. A bounded oversized frame MUST be drained before yielding `Err`; clean EOF yields no further element, while a timeout, malformed UTF-8, or partial final frame yields one `Err` and ends the iterator. The stream remains owned by its `using` scope.
- `std.csv.decode(String,[String],String,Int) gives Result<[Map<String,String>],String>` parses a headerless CSV table using caller-declared ordered headers, a one-byte ASCII delimiter, and a row ceiling from 1 through 1,000,000. `encode([Map<String,String>],[String],String)` emits canonical CRLF records in the declared order and requires each row's keys to match exactly. Both MUST support quoted delimiters, doubled quotes, and quoted newlines; reject duplicate/empty headers and inconsistent widths; cap headers at 4,096, each header at 1 KiB, each field at 1 MiB, and total input/output at 16 MiB.
- `std.encoding.hex(Bytes)` and `unhex(String)` exchange canonical lowercase hexadecimal; decode MAY accept uppercase but MUST reject odd length or non-hex digits. `base64`/`unbase64` use padded RFC 4648 standard encoding, while `base64url`/`unbase64url` use its unpadded URL-safe alphabet. All return typed errors, cap encoded text at 16 MiB, and cap decoded/base64 source bytes at 12 MiB or hex source bytes at 8 MiB.

### Exact numbers

- `BigInt` is an arbitrary-precision signed integer. `std.bigint.parse`, `from_int`, `format`, and `to_int` construct and convert it; conversion to `Int` is range checked.
- `Decimal` is a base-10 fixed-precision number for financial and human-scale exact arithmetic. `std.decimal.parse`, `from_int`, `format`, and `to_int` construct and convert it; fractional or out-of-range conversion fails.
- Matching `BigInt` and matching `Decimal` operands support `+`, `-`, `*`, `/`, `%`, unary `-`, equality, and ordering. Decimal overflow and division/remainder by zero MUST be runtime errors. Numeric types never coerce implicitly.
- `I8`, `I16`, `I32`, `U8`, `U16`, `U32`, and `U64` are distinct fixed-width integers. Each lowercase namespace provides `from_int`, `parse`, `format`, and checked `to_int`; parsing is required to represent `U64` values above `Int::MAX`.
- Matching fixed-width operands support checked arithmetic, equality, and ordering. Signed types support unary `-`; unsigned negation is a static error. Crossing widths or signedness requires an explicit string/`Int` conversion path and never coerces.

### Bytes

- `std.bytes.from_string(String) gives Bytes`
- `std.bytes.from_values([Int]) gives Result<Bytes, String>`; every value MUST be 0 through 255.
- `std.bytes.to_string(Bytes) gives Result<String, String>` uses strict UTF-8.
- `std.bytes.length(Bytes) gives Int`
- `std.bytes.get(Bytes, Int) gives Result<Int, String>`
- `std.bytes.slice(Bytes, Int, Int) gives Result<Bytes, String>` uses a start-inclusive, end-exclusive range.
- `std.text.concat(String,String) gives Result<String,String>` explicitly concatenates UTF-8 text and refuses output beyond 16 MiB. Numeric `+` remains numeric and values never stringify implicitly.
- `std.text.split(String,String,Int) gives Result<[String],String>` returns at most the caller's 1 through 1,000,000 part limit and rejects an empty separator. `split_last(String,String)` returns exactly the text before and after the final separator or an error. `starts_with(String,String) gives Bool` performs an explicit prefix test.
- `std.int.parse(String) gives Result<Int,String>` accepts canonical signed decimal text within the `Int` range and bounds input to 20 bytes. `std.int.format(Int) gives String` emits canonical decimal text. Numeric/text conversion is never implicit.
- `std.float.parse(String) gives Result<Float,String>` and `std.float.format(Float) gives Result<String,String>` accept and emit finite floating-point values; NaN and infinity are rejected.

### Binary codecs

- `std.binary.u16_be/le`, `u32_be/le`, `u64_be/le`, `i16_be/le`, and `i32_be/le` encode the matching fixed-width value into exactly 2, 4, or 8 bytes. `int_be/le` and `float_be/le` encode exactly 8 bytes.
- Every encoder has a matching `read_` decoder taking `(Bytes, offset: Int)` and returning `Result<T,String>`. Reads inspect the immutable input without copying it, reject negative or overflowing offsets, and never read a partial value.
- The `_be` and `_le` suffix is mandatory so protocol byte order remains visible at every boundary. There is no platform-native-order API.
- `std.binary.concat(Bytes, Bytes) gives Result<Bytes,String>` creates a new immutable value and refuses output beyond 16 MiB.

### Cryptography

- `std.crypto.sha256(Bytes) gives Result<Bytes,String>` returns a 32-byte SHA-256 digest and caps input at 16 MiB.
- `std.crypto.hmac_sha256(Bytes,Bytes) gives Result<Bytes,String>` signs a bounded message with a bounded key and returns a 32-byte HMAC-SHA-256 tag.
- `std.crypto.verify_hmac_sha256(Bytes,Bytes,Bytes) gives Result<Bool,String>` requires an exact 32-byte tag and verifies it with the audited backend's constant-time comparison.
- `std.crypto.random_bytes(Int) gives Result<Bytes,String>` needs `Random`. Length MUST be 1 through 1 MiB. Host runtimes MUST use the operating system cryptographic random source; a portable guest without host entropy MUST return a typed availability error and MUST NOT substitute a predictable generator.
- `std.crypto.password_hash(String,Bytes,Int,Int,Int) gives Result<String,String>` produces an Argon2id v=19 PHC string with a 32-byte output. Its parameters are password, salt, memory KiB, iterations, and lanes. Passwords MUST be at most 1 MiB, salts 16 through 64 bytes, memory at least 8 KiB per lane and at most 262,144 KiB, iterations 1 through 10, and lanes 1 through 16.
- `std.crypto.password_verify(String,String) gives Result<Bool,String>` accepts only Argon2id v=19 PHC strings no longer than 1,024 bytes. It MUST parse and enforce the same parameter ceilings before performing expensive work. A well-formed password mismatch returns `Ok(no)`; malformed or excessive input returns a typed error.
- `SecretKey` is an opaque 32-byte symmetric-key value. It MUST redact display/debug output, zeroize its owned storage when the last reference is released, and be non-comparable, non-serializable, non-transferable, and invalid as a collection key. `std.crypto.key_import(Bytes) gives Result<SecretKey,String>` copies exactly 32 bytes into opaque storage. `key_generate() gives Result<SecretKey,String>` needs `Random` and MUST construct the key directly from the host cryptographic entropy source.
- `std.crypto.encrypt(SecretKey,Bytes,Bytes,Bytes) gives Result<Bytes,String>` and matching `decrypt` implement ChaCha20-Poly1305. Remaining parameters are nonce, associated data, and plaintext/ciphertext. Nonces MUST be exactly 12 bytes, associated data at most 1 MiB, and output at most 16 MiB including the 16-byte tag. Authentication failure MUST return a typed error without exposing unauthenticated plaintext. The primitive MUST NOT silently generate, truncate, pad, or reuse nonce material.
- `std.crypto.ed25519_public(SecretKey) gives Result<Bytes,String>` derives the 32-byte public key from an opaque 32-byte seed. `ed25519_sign(SecretKey,Bytes)` returns a 64-byte deterministic Ed25519 signature, and `ed25519_verify(Bytes,Bytes,Bytes)` accepts an exact 32-byte public key and 64-byte signature and returns `Ok(no)` for a well-formed mismatch. Messages are capped at 16 MiB. Secret seeds remain opaque and zeroized; public keys and signatures are ordinary immutable bytes. Implementations MUST match RFC 8032 and MUST NOT accept noncanonical signatures.
- Hash and HMAC remain low-level protocol primitives. Higher-level APIs MUST name the construction they provide and keep entropy authority visible.

### Persistent maps and sets

- `std.map.single(K: Comparable, V) gives Map<K, V>`
- `std.map.set(Map<K,V>, K, V) gives Map<K,V>`
- `std.map.get(Map<K,V>, K) gives V?`
- `std.map.contains`, `remove`, `length`, `keys`, and `values`
- `std.set.single(T: Comparable) gives Set<T>`
- `std.set.add`, `contains`, `remove`, `length`, and `values`

Updates preserve the original collection. Enumeration follows insertion order. Replacing a map value does not move its key; removing and adding it again places it last.

### Iterators

- `std.iter.from([T]) gives Iterator<T>` creates a single-pass iterator over an immutable snapshot.
- `std.iter.range(Int, Int, Int) gives Result<Iterator<Int>,String>` creates a lazy, end-exclusive numeric source. The step is nonzero, its sign selects direction, and the calculated length is capped at 1,000,000 without allocating those values in advance.
- `next(Iterator<T>) gives T?` advances once and returns `none` after exhaustion.
- `take(Iterator<T>, Int)` consumes at most the requested prefix and returns a new iterator over that prefix. `skip(Iterator<T>, Int)` consumes at most the requested prefix and returns the same shared cursor at its new position.
- `transform(Iterator<T>, define(T) gives U) gives Iterator<U>` and `select(Iterator<T>, define(T) gives Bool) gives Iterator<T>` are lazy single-pass stages. Constructing a stage MUST NOT invoke its callback. A downstream request invokes only the callbacks required to produce that request, and all wrappers share the upstream cursor. Callback failures propagate at the requesting operation; callback capabilities are transitive.
- `collect(Iterator<T>) gives [T]` consumes the remainder. `each value within iterator` has the same consumption behavior.

Iterator values are stateful and non-transferable, non-comparable, non-serializable, and invalid as map/set keys. Counts must be nonnegative, materializing operations MUST refuse more than 1,000,000 values, and lazy callback nesting MUST refuse more than 1,024 stages. Lazy sources and stages store only cursor/source/callback state until consumed.

### Safe reflection

- `std.reflect.kind(T) gives String` exposes only the public type name.
- `std.reflect.fields(T) gives Result<Map<String,String>,String>` accepts shape values and exposes declared field names plus their public runtime kinds.
- `std.reflect.schema(S) gives Result<Map<String,String>,String>` accepts shape or choice constructors. It MUST return deterministic `$kind` and `$name` entries plus canonical field schemas or stable choice ordinals. It MUST NOT expose addresses, object layout, lexical values, private runtime state, or compiler implementation types.

Generators consuming this metadata or the public checked AST MUST emit inspectable ordinary source/artifacts, use deterministic name conversion, detect collisions, and preserve explicit ownership at foreign boundaries. Compile-time generation may not observe mutable runtime state.

## Files, environment, time, process, and logs

- `std.files.read(String) gives Result<String, String>` needs `FileRead`.
- `std.files.write(String, String) gives Result<Null, String>` needs `FileWrite`.
- `std.files.exists(String) gives Bool` needs `FileRead`.
- `std.files.open_read(String) gives Result<File,String>` and `read_open(File, Int)` need `FileRead`.
- `std.files.open_write(String) gives Result<File,String>` and `write_open(File, String)` need `FileWrite`.
- `std.files.close(File) gives Result<Null,String>` is idempotent. `using` MUST close a live file across every scope-exit path.
- `std.files.read_async(String,Int)` and `write_async(String,String)` MUST return `Result<Task,String>`, use a bounded shared executor, cap each payload at 16 MiB, make queue saturation a typed error, wake task completion without polling, and check cooperative cancellation before and after blocking work. Awaiting the task yields the file value or error.
- `std.env.get(String) gives String?` needs `Environment`.
- `std.time.now_zoned(String) gives Result<DateTime,String>` uses the bundled IANA timezone database and needs `Time`; `sleep(Float)` also needs `Time`.
- `std.time.from_unix(Int, String)`, `parse(String)`, `format(DateTime)`, `in_zone(DateTime,String)`, `unix(DateTime)`, and `add_seconds(DateTime,Int)` are deterministic and return typed errors for invalid zones, text, ranges, or arithmetic overflow.
- `std.time.now() gives Float` remains the Edition 2 compatibility clock. Edition 3 APIs SHOULD use `DateTime` when an instant leaves a single expression.
- `std.process.run(String, [String]) gives Result<String, String>` needs `Process` and never invokes a shell.
- `std.log.info`, `warn`, and `error` need `Log`.
- `std.path.join`, `basename`, and `dirname` are pure lexical/native-path operations.

## Network and web

`std.web.encode_component(String) gives Result<String,String>` applies RFC 3986 UTF-8 percent encoding, preserving only ASCII letters, digits, `-`, `.`, `_`, and `~`, and emitting uppercase hexadecimal. `decode_component` requires complete hexadecimal escapes, preserves `+` literally, and returns invalid UTF-8 as a typed error. Decoded/input text is capped at 1 MiB and encoded text at 3 MiB.

- `std.net.listen(String, Int) gives Result<TcpListener,String>` and `accept(TcpListener, Float)` need `Network`.
- `std.net.connect(String, Int, Float) gives Result<TcpStream, String>` needs `Network`.
- `std.net.tls_connect(String, Int, Float, Map<String,String>) gives Result<TlsStream,String>` needs `Network` and MUST complete a certificate- and hostname-verified handshake before succeeding. `tls_read_exact_bytes`, `tls_read_line`, `tls_write_ready`, and `tls_close` MUST retain the corresponding TCP bounds, deadlines, framing guarantees, and idempotent close behavior. No verification-bypass option is permitted.
- `std.net.read`, `write`, and `close` need `Network` and retain Edition 2 limits.
- `std.net.read_exact_bytes(TcpStream,Int,Float) gives Result<Bytes,String>` consumes exactly the requested bounded byte count or fails without over-reading. `read_line(TcpStream,Int,Float)` consumes one CRLF-terminated UTF-8 line, excludes the terminator, caps it at 64 KiB, and leaves subsequent pipelined bytes untouched. Both restore the stream's previous timeout policy.
- `std.net.write_some(TcpStream,String,Int,Float) gives Result<Int,String>` MUST write no more than the caller's bounded byte limit, MUST return actual progress, MUST return zero for deadline backpressure, and MUST preserve the stream's prior timeout policy.
- `std.net.ready(TcpStream,String,Float) gives Result<Bool,String>` MUST use an OS readiness facility on every tier-one platform. It accepts `read`, `write`, or `read_write`; false denotes deadline expiry and true is a readiness hint, so subsequent I/O MUST remain prepared for spurious readiness.
- `std.web.get(String, Float) gives Result<String, String>` needs `Network`.
- `std.web.headers() gives Map<String,String>` creates an empty header map.
- `std.web.request(String, String, Map<String,String>, String, Float, Int) gives Result<Map<String,String>,String>` needs `Network`; the response map MUST include `status` and `body` and MAY include normalized `header:<name>` entries.
- `std.web.read_request(TcpStream, Int)` and `respond(TcpStream, Int, Map<String,String>, String)` need `Network` and implement bounded HTTP/1.x server framing.
- `std.web.websocket_connect(String, Int, String, Float) gives Result<WebSocket,String>` performs a version-13 client upgrade on an explicitly connected host, port, and path.
- `std.web.websocket_secure_connect(String, Int, String, Float, Map<String,String>)` performs the same upgrade over certificate- and hostname-verified TLS. `tls_options()` MUST default to TLS 1.2 or newer and public WebPKI roots. Implementations MAY accept a TLS 1.3 floor, bounded ALPN names, and bounded additional PEM roots, but MUST NOT expose certificate or hostname verification bypass.
- `std.web.websocket_accept(TcpStream, Map<String,String>) gives Result<WebSocket,String>` validates a request returned by `read_request` and performs the server upgrade.
- `std.web.websocket_send(WebSocket, String)`, `websocket_receive(WebSocket, Int)`, and `websocket_close(WebSocket)` need `Network`. Client frames MUST use cryptographically random masks; server frames MUST be unmasked. Receivers MUST validate masking, framing, control frames, UTF-8, fragmentation, and the caller's limit.

TLS verification, URL restrictions, header/body limits, timeout ranges, and response-framing validation are unchanged from Edition 2. A certificate-bypass API is forbidden from the safe standard library.

## Tasks and channels

- `std.tasks.spawn(callable) gives Task`, `await`, `await_for`, `cancel`, `all`, and `race` need `Task`.
- `std.channels.create`, `send`, and `receive` need `Channel`.

The word forms `start`, `wait`, `together`, and `race` lower to these APIs. The callable passed to `spawn` contributes its own capabilities to the call site. Channel capacity, deadlines, ownership, cancellation, joining, and transferable-value rules are normative as described in `LANGUAGE-3.md`. Implementations MUST provide a wake-driven completion/deadline mechanism and MUST NOT implement task waits or races as fixed-interval busy polling.

### Locks

- `std.locks.create(T) gives Lock` creates shared state without acquiring it.
- `std.locks.acquire(Lock, Float) gives Result<LockGuard,String>` needs `Task`, uses a bounded timeout, and returns an exclusive guard.
- `std.locks.read(LockGuard) gives Result<T,String>` and `write(LockGuard,T)` need `Task`.
- `std.locks.close(LockGuard)` needs `Task` and is idempotent. `using` MUST release a live guard across every exit path.

`Lock` is transferable only when its contained value is transferable. `LockGuard` is closable but never transferable or comparable. Operations after release return typed errors.

### Atomic integers

`std.atomics.create(Int) gives AtomicInt` creates a transferable, opaque, linearizable signed 64-bit atomic value. `load`, `store`, and `swap` return the observed values implied by their names. `add(AtomicInt,Int) gives Result<Int,String>` performs checked addition and returns the value before the update; overflow changes nothing. `compare_exchange(AtomicInt,expected,replacement) gives Result<Int,Int>` returns `Ok(previous)` after an update or `Err(observed)` without one.

All operations use sequentially consistent semantics across Nivren structured tasks. `AtomicInt` is `Sendable` but not comparable, serializable, reflectable as storage, or valid as a map/set key. Implementations without lock-free 64-bit atomics MUST preserve the same semantics with an internal lock.

### Compression

`std.compression.gzip(Bytes,Int)` and `zlib(Bytes,Int)` return `Result<Bytes,String>`. Levels MUST be 0 through 9. Input and output MUST each be capped at 16 MiB. Gzip output MUST use a zero timestamp and no environment-dependent filename/comment, making equal input and level byte-identical within a release.

`gunzip(Bytes,Int)` and `unzlib(Bytes,Int)` return `Result<Bytes,String>`. The second argument is a mandatory maximum decompressed size from 1 through 16 MiB. Decoders MUST read no more than maximum plus one byte before reporting an oversize stream. Invalid, truncated, and oversized streams are typed errors and MUST NOT return partial output.

### Transactions

- `std.transactions.begin(Map<K,V>) gives Transaction<K,V>` snapshots an insertion-ordered map.
- `get`, `set`, and `remove` operate only while open and return typed errors after close.
- `commit(Transaction<K,V>) gives Result<Map<K,V>,String>` returns staged state and closes committed.
- `rollback(Transaction<K,V>) gives Result<Map<K,V>,String>` returns original state and closes rolled back.
- `close(Transaction<K,V>)` is idempotent and MUST roll back when still open. `using` applies this close rule across every exit path.

Transaction keys repeat immutable-comparable validation at runtime, staged allocations count against the shared memory budget, entry count is capped at 1,000,000, and transaction handles are never transferable, comparable, serializable, or stable keys.

## Resource and security rules

Every host-sized input is bounded. Host failures are data unless the contract identifies a runtime misuse. Native handles are opaque, cannot be serialized, and fail safely after close. Project capability grants are enforced again at runtime and are never inferred from source declarations.

Filesystem and dynamic-library capabilities MAY be scoped with `path:` and network capability MAY be scoped with `host:`; path resolution MUST resist parent/symlink escapes and wildcard hosts MUST match subdomains rather than the registrable suffix itself. Network grants MAY list comma-separated host alternatives and append an AND-composed `method:` clause containing comma-separated HTTP methods. `Environment` accepts an exact `name:` or `prefix:` grant. `Process` accepts comma-separated `command:` alternatives and MAY append an AND-composed exact `arg0:` clause. Empty, duplicate, unknown, or missing required clauses MUST be rejected while loading the manifest. Native host handles accept an exact `kind:` grant. A resource created by an authorized operation remains usable for bounded I/O and cleanup. Instruction and memory budgets are shared by the complete structured task tree.

## Native host bridge

`std.native.open(String)` MUST require `Native` and return `Result<NativeLibrary,String>`. A `NativeLibrary` MUST own its loader lifetime, MUST be closable, and MUST NOT be transferable, serializable, comparable, or usable as a stable key. Implementations MUST reject calls after close and MUST NOT expose a resolved symbol beyond the dynamic extent of its call.

`std.native.call_int(NativeLibrary,String,[Int])` and `std.native.call_float(NativeLibrary,String,[Float])` MUST support only C ABI signatures with zero through six homogeneous primitive arguments and a same-typed return. Implementations MUST reject empty, NUL-containing, or oversized symbol names. Matching the actual export signature is an explicit native trust obligation. `using` MUST close the library on every scope exit.

`std.native.call_buffer(NativeLibrary,String,Bytes,Int) gives Result<Bytes,String>` MUST call the C ABI signature `int64_t function(const uint8_t *input, size_t input_length, uint8_t *output, size_t output_capacity)`. Input and initialized output buffers remain runtime-owned and valid only for the call. Capacity MUST be 0 through 16 MiB. A negative return is an error code; a return larger than capacity MUST be rejected; otherwise the result contains exactly the returned byte count. The foreign function MUST NOT retain either pointer.

`std.host.invoke(String, String) gives Result<String,String>` needs `Native`. It MUST return an error when no host is installed. An embedding implementation MUST copy callback buffers before invoking the paired host free operation and MUST NOT retain borrowed host pointers. The UTF-8 request/response contract is the stable primitive on which generated typed bindings can layer JSON or other inspected schemas.

`std.host.invoke_async(String, String) gives Result<Task,String>` needs `Native, Task`. It MUST submit the same operation to the runtime's bounded blocking executor and return immediately with a structured task or a saturation error. Waiting produces `Result<String,String>`. Cancellation MUST be checked before and after the host callback; because ABI 2 host callbacks are synchronous, an already-running callback is allowed to finish before the cancelled task becomes observable. A future ABI may negotiate active host cancellation without changing this language contract.

Host operation names MUST contain 1 through 128 ASCII letters, digits, `-`, `_`, or `.`. Every request and response is limited to 16 MiB; oversize payloads MUST fail without exposing a partial response.

`std.host.open(kind, request) gives Result<NativeHandle,String>`, `call(handle, operation, request)`, and `close(handle)` also need `Native`. The host returns an opaque identifier no larger than 1,024 bytes. Calls receive a JSON envelope containing the identifier and request; identifiers are never exposed to Nivren code. `using` MUST request `nivren.handle.close` exactly once on every scope exit. Failed closes remain retryable, explicit double-close is safe, and the runtime performs a best-effort release if an embedding drops a live handle.

The stable C embedding ABI MUST copy source before asynchronous return, deliver exactly one owned completion, provide cooperative cancellation and joinable opaque handles, and invoke an optional wake callback only after completion returns. Generated schema views MUST use explicit widths and pointer/length ownership, compile as C11 and C++17, and never expose compiler or runtime object layouts.
