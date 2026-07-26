# Nivren Edition 1 Standard Library Specification

The key words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are normative. All APIs are immutable bindings in the `std` namespace. A wrong arity or statically known wrong type is a static error; embedders MUST apply equivalent runtime checks. Fallible host operations return `Result<T, String>` and absence returns `T?`.

## 1. Core globals

- `clock() -> Float` and `std.time.now() -> Float` return seconds since the Unix epoch.
- `len(String|[T]) -> Int` counts Unicode scalar values or array elements.
- `type(T) -> String` returns the Edition 1 display type name.
- `append([T], T) -> [T]` returns a new array and does not modify its input.
- `assert(Bool, String) -> Null` traps with the supplied message when false.
- `ok(T) -> Result<T, E>` and `err(E) -> Result<T, E>` construct result values under contextual typing.

## 2. Files, paths, and environment

- `std.fs.read(String) -> Result<String, String>` reads a complete UTF-8 file.
- `std.fs.write(String, String) -> Result<Null, String>` replaces a file's contents.
- `std.fs.exists(String) -> Bool` tests host path existence.
- `std.path.join(String, String) -> String` joins two native path components.
- `std.path.basename(String) -> String?` and `std.path.dirname(String) -> String?` return a component or absence.
- `std.env.get(String) -> String?` reads without mutating the host environment.

These APIs use native path syntax. I/O errors MUST be returned, not trapped. Invalid UTF-8 file contents MUST be an error.

## 3. Time, processes, and logging

- `std.time.sleep(Float) -> Null` accepts a finite non-negative number of seconds.
- `std.process.run(String, [String]) -> Result<String, String>` executes the program directly without a shell. It returns UTF-8 standard output only for exit status zero; spawn, nonzero exit, and invalid UTF-8 are errors.
- `std.log.info(String)`, `std.log.warn(String)`, and `std.log.error(String)` return `Null` and emit one explicitly leveled line.

## 4. JSON

- `std.json.valid(String) -> Bool`
- `std.json.compact(String) -> Result<String, String>`
- `std.json.pretty(String) -> Result<String, String>`

The parser MUST reject duplicate object keys, invalid number grammar, malformed escapes or surrogate pairs, trailing data, input over 16 MiB, and nesting deeper than 256. Compact and pretty output MUST be deterministic for identical input.

## 5. TCP, HTTP, and TLS

- `std.net.connect(String, Int, Float) -> Result<TcpStream, String>`
- `std.net.read(TcpStream, Int) -> Result<String, String>`
- `std.net.write(TcpStream, String) -> Result<Null, String>`
- `std.net.close(TcpStream) -> Result<Null, String>`
- `std.http.get(String, Float) -> Result<String, String>`

Timeouts MUST be greater than zero and at most 300 seconds. Ports range from 0 through 65,535. A read maximum MUST be non-negative and no greater than 16 MiB; returned data MUST be UTF-8. HTTP accepts only `http` and `https`, limits headers to 64 KiB and bodies to 16 MiB, validates response framing, and returns non-2xx status as an error. HTTPS MUST verify the server name and certificate chain against the toolchain's documented Mozilla root snapshot. Edition 1 provides no certificate-bypass API.

## 6. Tasks and channels

- `std.task.spawn(fun() -> T) -> Task`
- `std.task.await(Task) -> Result<T, String>`
- `std.task.await_for(Task, Float) -> Result<T, String>`
- `std.task.cancel(Task) -> Null`
- `std.channel.create(Int) -> Channel`
- `std.channel.send(Channel, T, Float) -> Result<Null, String>`
- `std.channel.receive(Channel, Float) -> Result<T, String>`

Workers observe cancellation at every bytecode instruction and are joined when their owning task is dropped. A task can be awaited once. Channel capacity MUST be from 0 through 65,536 and channel operations use the same timeout range. Only recursively immutable data can cross a task result or channel; functions, modules, native handles, tasks, and channels MUST be rejected.

## 7. Resource and compatibility rules

Native handles are opaque, cannot be portably compared or serialized, and MUST fail safely after close. Implementations MUST bound all externally sized reads as stated above and MUST NOT expose ambient shell evaluation through the standard library. Removing an API or changing defined behavior requires a new edition; additive APIs may be introduced compatibly.
