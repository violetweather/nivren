# Nivren Edition 3 standard library guide

The normative draft is `spec/STANDARD-LIBRARY-3.md`. Fallible operations return `Result<T, String>`; absence returns `T?`; host effects declare and require capabilities.

## Data

- `std.bytes.from_string`, `from_values`, `to_string`, `length`, `get`, and `slice`
- `std.text.concat(left,right)` returns bounded UTF-8 text through `Result<String,String>` without overloading numeric `+` or coercing values implicitly. `split(value, separator, maximum)` returns at most one million bounded parts, `split_last` separates once from the right for address-like values, and `starts_with` performs an explicit prefix test.
- `std.int.parse` and `std.int.format` provide explicit, range-checked signed decimal conversion.
- `std.float.parse` and `std.float.format` provide explicit finite floating-point conversion and reject NaN or infinity.
- `std.binary` encodes and decodes explicit big- or little-endian `U16`, `U32`, `U64`, `I16`, `I32`, `Int`, and `Float` values. Offset reads are zero-copy, bounds checked, and return typed errors; `concat` caps new buffers at 16 MiB.
- `std.crypto.sha256`, `hmac_sha256`, and `verify_hmac_sha256` provide bounded SHA-256 and constant-time-verified HMAC-SHA-256 protocol primitives.
- `std.crypto.random_bytes(length)` needs `Random` and obtains 1 byte through 1 MiB from the operating system's cryptographic entropy source. Portable guests without an installed entropy host return an explicit typed error.
- `std.crypto.password_hash(password, salt, memory_kib, iterations, lanes)` and `password_verify(password, encoded)` provide Argon2id v=19 password storage. Passwords are capped at 1 MiB, salts at 16 through 64 bytes, memory at 8 KiB per lane through 256 MiB, iterations at 1 through 10, lanes at 1 through 16, and output at 32 bytes. Verification accepts only bounded Argon2id PHC strings and validates attacker-controlled parameters before expensive allocation.
- `std.crypto.key_import(bytes)` copies exactly 32 bytes into an opaque `SecretKey`; `key_generate()` needs `Random` and creates one directly from operating-system entropy. `SecretKey` displays only `<secret-key>`, zeroizes its storage when the final reference is released, and cannot be compared, serialized, transferred into tasks, or used as a collection key.
- `std.crypto.encrypt(key, nonce, associated, plaintext)` and `decrypt(key, nonce, associated, ciphertext)` provide ChaCha20-Poly1305 authenticated encryption and accept only `SecretKey`. Nonces are exactly 12 bytes, associated data at most 1 MiB, and the complete ciphertext at most 16 MiB. Decryption releases no plaintext unless its 16-byte tag authenticates both ciphertext and associated data. Nonce uniqueness per key remains the caller's responsibility; `nivren_aead.seal` provides the safe random-nonce path.
- `std.crypto.ed25519_public(key)`, `ed25519_sign(key, message)`, and `ed25519_verify(public, message, signature)` provide RFC 8032 asymmetric signatures while keeping the 32-byte signing seed inside the opaque, zeroized `SecretKey`. Public keys are exactly 32 bytes, signatures exactly 64 bytes, messages at most 16 MiB, and well-formed signature mismatches return `Ok(no)`.
- `std.json.valid`, `compact`, `pretty`, `parse`, and `stringify`
- `std.json.decode(Shape, source)` derives a strict typed decoder from a shape. It validates required and unexpected fields recursively, including nested shapes, choices, arrays, nullable fields, maps, sets, zoned time, and exact/fixed-width numbers.
- `std.json.read_next(file, maximum)` incrementally reads one newline-delimited JSON value, while `read_next_as(Shape, file, maximum)` validates and returns a typed shape. Both need `FileRead`, return `none` at clean end-of-file, cap each record at 16 MiB, and drain an oversized record before the next read.
- `std.csv.decode(source, headers, delimiter, maximum_rows)` and `encode(rows, headers, delimiter)` exchange bounded tables as `[Map<String,String>]`. Ordered headers make schema and output order explicit. Quoted delimiters, doubled quotes, CRLF/LF records, and quoted newlines are supported; input/output is capped at 16 MiB, fields at 1 MiB, columns at 4,096, and rows at a caller-selected ceiling no greater than one million.
- `std.encoding.hex`/`unhex`, `base64`/`unbase64`, and `base64url`/`unbase64url` provide canonical lowercase hexadecimal, padded RFC 4648 base64, and unpadded URL-safe base64. Encoded text is capped at 16 MiB; source bytes are capped at 8 MiB for hex and 12 MiB for base64 so native allocations remain bounded. Decoders reject odd hex, invalid alphabets, noncanonical padding, and oversized output through typed errors.
- `std.map.single`, `set`, `get`, `contains`, `remove`, `length`, `keys`, and `values`
- `std.set.single`, `add`, `contains`, `remove`, `length`, and `values`
- `std.list.batch`, `transform`, `select`, `fold`, `any`, and `every`; `batch` returns bounded typed groups and composes as a labeled `through` stage
- `std.iter.from`, lazy end-exclusive `range`, `next`, `take`, `skip`, `transform`, `select`, `chain`, and `collect` provide typed single-pass iterator values and adapters. `range(start,end,step)` keeps only cursor state, rejects a zero step, and caps its calculated length at one million. `transform` and `select` are truly lazy: construction invokes no callback, and each downstream request pulls only as far as needed through the shared cursor. Adapter nesting is capped at 1,024 stages. `fold`, `find`, `any`, `every`, and `count` are typed terminal algorithms; predicate terminals short-circuit while preserving the unvisited suffix. Callback capabilities remain transitive, `each` accepts iterators, and materializing operations are capped at one million values.
- `std.bigint.parse`, `from_int`, `format`, and `to_int` support arbitrary-precision signed integers.
- `std.decimal.parse`, `from_int`, `format`, and `to_int` support exact base-10 arithmetic.
- `std.i8`, `i16`, `i32`, `u8`, `u16`, `u32`, and `u64` provide range-checked fixed-width construction, parsing, formatting, and conversion.

Bytes, maps, sets, arrays, strings, shapes, choices, BigInts, Decimals, fixed-width integers, and zoned DateTimes have deterministic JSON representations where applicable. Bare choices encode as their variant string; payload choices encode as a tagged `$variant`/`$value` object so their data is not discarded. Numeric arithmetic uses ordinary operators without implicit coercion; fixed-width and Decimal overflow and zero division fail safely. Map and set updates return new insertion-ordered values. Iterators are stateful, single-pass, non-transferable, and neither stable keys nor serializable values. Invalid byte values, indexes, slices, conversions, schemas, and UTF-8 decoding return errors.

## Files, paths, environment, time, and process

`std.iter.lines(file, maximum_bytes)` turns an open reader into a lazy `Iterator<Result<String,String>>` under `FileRead`. It normalizes CRLF, caps each line at 1 MiB, drains an oversized line before yielding its error, and can continue with the next record without loading the whole file.

`std.iter.tcp_lines(stream, maximum_bytes, timeout_seconds)` provides the same single-pass shape for CRLF-framed TCP protocols under `Network`. Each pull has its own finite timeout, payloads are capped at 64 KiB, oversized frames are drained so iteration can recover, clean EOF ends the iterator, and partial frames or transport errors end it with a typed error.

- `std.files.read` and `exists` need `FileRead`; `std.files.write` needs `FileWrite`.
- `std.files.open_read`, `open_write`, `read_open`, `write_open`, and `close` expose bounded, deterministic `File` resources for `using` scopes.
- `std.files.read_async(path, maximum)` and `write_async(path, contents)` enqueue bounded work and return `Result<Task,String>`. `wait` produces the file result without blocking the caller thread. Payloads are capped at 16 MiB; the shared executor uses 2 through 8 workers with 32 queued jobs each, reports saturation instead of growing without limit, wakes the runtime event loop, and checks cancellation before and after work.
- `std.path.join`, `basename`, and `dirname` are pure.
- `std.env.get` needs `Environment`.
- `std.time.now_zoned(zone)` returns an immutable `DateTime` using bundled IANA timezone data and needs `Time`; `sleep` also needs `Time`.
- `std.time.from_unix`, `parse`, `format`, `in_zone`, `unix`, and `add_seconds` provide checked instant/timezone conversion and arithmetic. `DateTime` is comparable, ordered by instant, and transferable.
- `std.process.run` needs `Process` and executes directly without a shell.

The Edition 2 `std.fs` spelling remains a draft compatibility alias for `std.files`.

## Network and web

`std.web.encode_component` and `decode_component` provide strict RFC 3986 UTF-8 URL-component encoding for OAuth, forms, redirects, and signed requests. They use uppercase percent escapes, preserve `+` literally, reject malformed escapes or invalid UTF-8, and enforce 1 MiB decoded/input and 3 MiB encoded ceilings.

- `std.net.listen`, `accept`, `connect`, `read`, `write`, `write_some`, and `close` need `Network`. `TcpListener` and `TcpStream` are closable resources.
- `std.net.tls_connect(host, port, timeout, options)` creates a distinct closable `TlsStream` using the same mandatory certificate/hostname verification and bounded TLS policy as secure WebSockets. `tls_read_exact_bytes`, `tls_read_line`, `tls_write_ready`, and `tls_close` expose exact framed I/O without allowing encrypted streams to enter plaintext-only APIs.
- `std.net.read_exact_bytes` consumes an exact bounded binary frame; `std.net.read_line` consumes one bounded CRLF line. Neither over-reads the next pipelined frame, and both restore prior timeout policy.
- `std.net.write_some(stream, text, maximum, timeout)` writes at most `maximum` bytes before the deadline and returns the actual byte count. A zero count means backpressure prevented progress; callers retain the unwritten suffix and decide when to retry. The limit is 1 through 16 MiB and the stream's prior timeout is restored.
- `std.net.ready(stream, interest, timeout)` waits on the operating system's cross-platform readiness reactor. Interest is `read`, `write`, or `read_write`; `yes` means the caller should attempt the operation and still handle a permitted spurious readiness event, while `no` means the deadline elapsed.
- `std.web.get` needs `Network` and accepts certificate-verified HTTP/HTTPS.
- `std.web.headers()` creates an empty `Map<String,String>`.
- `std.web.request(method, url, headers, body, timeout, maximum)` returns a response map containing `status`, `body`, and `header:<name>` entries. It preserves non-2xx status instead of translating it into a transport error.
- `std.web.read_request(stream, maximum)` and `respond(stream, status, headers, body)` provide bounded HTTP/1.0 and HTTP/1.1 server framing.
- `std.web.websocket_connect`, `websocket_secure_connect`, `websocket_accept`, `websocket_send`, `websocket_receive`, and `websocket_close` provide RFC 6455 bounded text-message clients and servers. Secure clients verify certificates and hostnames.
- `std.web.websocket_secure_listen(host, port, certificate_pem, private_key_pem, options)` creates a `TlsListener`; `websocket_secure_accept(listener, timeout)` performs the TLS handshake and WebSocket upgrade and returns a managed `WebSocket`. The certificate and PKCS#8/PKCS#1/SEC1 private-key PEM inputs are each capped at 1 MiB, mismatched material is rejected before binding, and accepted sockets use bounded read/write deadlines. `TlsListener` and `WebSocket` are closable resources intended for `using`; `tls_close` is the explicit listener close operation.
- `std.web.tls_options()` returns a safe policy map shared by clients and servers. `minimum_version` accepts `1.2` or `1.3`; `alpn` accepts at most 16 bounded ASCII protocol names. Clients may add up to 1 MiB of explicit trust anchors through `additional_root_pem`. Supplying `client_certificate_pem` and `client_private_key_pem` together presents a bounded mTLS identity; incomplete or mismatched credentials fail before connecting. Servers can set `client_auth` to `required` with a bounded `client_ca_pem` trust bundle; client-only and server-only options are rejected on the wrong side. Public WebPKI roots and hostname verification always remain enabled for clients, and there is no verification-bypass option.

`std.net.ready(stream, interest, timeout)` waits through the operating-system reactor for one TCP stream. `std.net.ready_any(streams, interest, timeout)` registers up to 1024 streams in one poll and returns the lowest ready array index or `none` at the deadline; an empty array returns `none` immediately. Interests are `read`, `write`, or `read_write`, and callers must retry the operation because readiness can be spurious. `std.net.read_ready` waits and then performs one bounded UTF-8 read. `std.net.write_ready` sends an entire value in caller-sized chunks, re-waits under backpressure, and reports timeout with byte progress. TCP and TLS listener accepts use the same readiness mechanism instead of sleep polling.

Timeouts are bounded, ports are validated, TCP/WebSocket text must be UTF-8, HTTP and WebSocket upgrade headers are capped at 64 KiB, bodies and WebSocket messages at 16 MiB, framing and masking are validated, and unsafe managed headers are rejected. `std.web.get` treats non-2xx status as an error; `request` returns the status for application policy. There is no certificate-bypass API. `std.http` is the Edition 2 GET-only alias.

## Tasks and channels

- `std.tasks.spawn`, `await`, `await_for`, `cancel`, `all`, and `race` need `Task`.
- `std.channels.create`, `send`, and `receive` need `Channel`.

Prefer the language forms `start`, `wait`, `together`, and `race`. Tasks are scoped and joined; channels are bounded; only recursively immutable transferable values cross boundaries. Task completion, deadlines, and races use a shared wake-driven runtime event loop rather than millisecond polling. Edition 2 singular namespaces remain draft aliases.

## Scoped locks

- `std.locks.create(value)` creates shared state.
- `std.locks.acquire(lock, timeout)` needs `Task` and returns a `LockGuard` or a timeout error.
- `std.locks.read`, `write`, and `close` need `Task`.

Use an acquired guard with `using`. Release is deterministic across ordinary completion, `give`, `or give`, and runtime errors; closing twice is safe and using a released guard returns a typed error.

## Transactions

- `std.transactions.begin(Map<K,V>)` creates a bounded `Transaction<K,V>` over an immutable snapshot.
- `get`, `set`, and `remove` read or stage insertion-ordered changes while the transaction is open.
- `commit` closes and returns the updated map; `rollback` closes and returns the original map.
- `close` is idempotent and rolls back an open transaction. A `using` scope therefore rolls back unless code explicitly commits first, including across `give`, `or give`, and runtime errors.

Transactions hold at most one million entries, repeat key validation at runtime, charge staged values to the execution memory budget, reject all operations after commit/rollback except idempotent close, and are non-transferable, non-comparable, and non-serializable.

## Atomic integers

- `std.atomics.create(value)` creates a transferable `AtomicInt`.
- `load`, `store`, and `swap` are linearizable.
- `add` returns `Result<Int,String>`, reports checked overflow, and leaves the value unchanged on failure.
- `compare_exchange` returns `Ok(previous)` when it replaces the expected value or `Err(observed)` when another task won.

Atomic operations are sequentially consistent across structured tasks. The portable runtime may implement them with an internal lock while preserving exactly the same behavior. Atomic handles cannot be serialized, compared, or used as collection keys.

## Compression

`std.compression.gzip` and `zlib` accept immutable bytes plus a level from 0 through 9 and return bounded deterministic bytes. Gzip metadata fixes its timestamp at zero. `gunzip` and `unzlib` require a caller-selected maximum output from 1 byte through 16 MiB, stop after at most one byte beyond that ceiling, and return invalid/truncated/oversized streams as typed errors. Compressed inputs and encoded outputs are also capped at 16 MiB.

## Logging and observation

- `std.log.info`, `warn`, and `error` emit leveled text and need `Log`.
- `std.log.event(level, message, Map<String,String>)` emits deterministic JSON and needs `Log`.
- `niv explain [--no-optimize] <program>` emits deterministic `org.nivren.intent.v1` JSON for plans, capabilities, resources, cancellation, retries, timeouts, buffering, blocking, fusion, effect order, target choice, and portability. `niv profile`, `niv coverage`, `niv debug`, and `niv inspect` observe the same project capability, instruction, memory, and call-depth policy as `niv run`. Profile and coverage accept `--json <output>` and emit the stable `org.nivren.observation.v1` schema. `niv inspect <program> <output.jsonl>` flushes versioned `org.nivren.inspect.v1` start, instruction, and completion events as execution happens; it includes operation, location, stack, variable names, metrics, and heap counts but omits source and variable values.
- `niv run --crash-report <output> <program>` writes `org.nivren.crash.v1` after a runtime failure. Reports include the basename, location, and call frames while deliberately omitting source, arguments, environment variables, and local values.

## Native hosts

- `std.host.invoke(name, request) gives Result<String,String>` needs `Native`.
- `std.host.invoke_async(name, request) gives Result<Task,String>` needs `Native, Task`, uses the bounded blocking executor, and joins through the ordinary structured-task APIs.
- `std.host.open(kind, request)` returns an opaque, non-transferable `NativeHandle`; `std.host.call` operates on it and `std.host.close` releases it. All need `Native`.

Embedding hosts install the callback through the stable Rust compiler/runtime facade or `nivren_run_host_utf8` in the C ABI. Operation names are restricted ASCII identifiers and every request/response is capped at 16 MiB. Inputs and outputs are copied UTF-8 buffers, callback allocations have an explicit paired free operation, and structured integrations conventionally exchange JSON. Async invocation checks cancellation before and after a callback and reports executor saturation as typed data; an already-running ABI 2 callback may finish before cancellation is observed. `niv bindgen c` derives ownership-explicit C11/C++17 views from checked shapes and choices. `nivren_run_async_utf8` delivers one owned completion followed by an optional event-loop wake; its opaque handle supports cooperative cancellation and joining. Long-lived identifiers remain inside opaque handles and `using` guarantees a close request. Without an installed host, invocation and opening return typed errors.

## Dynamic C libraries

- `std.native.open(path)` loads a dynamic library as an opaque `NativeLibrary` and returns `Result<NativeLibrary, String>`.
- `std.native.call_int(library, symbol, arguments)` invokes a C function whose zero through six parameters and return value are signed 64-bit integers.
- `std.native.call_float(library, symbol, arguments)` invokes the corresponding all-`double` signature.
- `std.native.call_buffer(library, symbol, input, capacity)` marshals immutable input and initialized bounded output through a fixed pointer/length C ABI, rejects negative or oversized returned lengths, and yields owned `Bytes`.
- `std.native.close(library)` unloads the library; `using` is preferred and closes it on every exit path.

Every operation needs `Native`. A manifest may grant all native access or restrict library opening to a `path:` scope. An authorized library remains opaque, non-transferable, non-serializable, and invalid as a key; symbols never escape the call that resolved them. The caller must ensure that an export really has the selected C ABI signature because crossing this boundary deliberately trusts native code.

## Reflection and generation

- `std.reflect.kind(value)` returns the public runtime type name.
- `std.reflect.fields(shapeValue)` returns a `Result<Map<String,String>,String>` of declared field names to runtime value kinds.
- `std.reflect.schema(ShapeOrChoice)` returns deterministic declaration metadata. `$kind` is `shape` or `choice`, `$name` is the qualified name, shape entries map fields to canonical type schemas, and choice entries map variants to stable declaration ordinals.
- `niv bindgen c` and Rust compiler facade v3 consume the same checked public AST/schema contract. The C ABI remains independently versioned at v3. Generated output is ordinary inspectable source; it receives no access to lexical values, runtime memory, or compiler internals.

## Core globals

`show`, `len`, `type`, `append`, `assert`, `ok`, and `err` remain available. `clock()` and float-returning `std.time.now()` are retained for Edition 2; Edition 3 code should use `std.time.now_zoned(zone)`.
