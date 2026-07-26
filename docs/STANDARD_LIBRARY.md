# Nivren Edition 1 Standard Library Guide

The normative contract is `spec/STANDARD-LIBRARY-1.md`; this guide summarizes it for application authors.

The application library is available through the typed `std` namespace. Operations that can fail return `Result<T, String>`; absence uses nullable values. APIs validate argument types both statically and at runtime.

## Files, paths, and environment

- `std.fs.read(path: String) -> Result<String, String>` reads a UTF-8 file.
- `std.fs.write(path: String, contents: String) -> Result<Null, String>` replaces a file.
- `std.fs.exists(path: String) -> Bool` tests path existence.
- `std.path.join(left: String, right: String) -> String` joins native path components.
- `std.path.basename(path: String) -> String?` and `std.path.dirname(path: String) -> String?` inspect components without sentinel strings.
- `std.env.get(name: String) -> String?` reads an environment variable without mutating the host environment.

## Time, processes, and logging

- `std.time.now() -> Float` returns Unix time in seconds.
- `std.time.sleep(seconds: Float) -> Null` accepts finite non-negative durations.
- `std.process.run(program: String, arguments: [String]) -> Result<String, String>` executes without a shell and returns standard output only for a successful exit.
- `std.log.info`, `std.log.warn`, and `std.log.error` emit explicitly leveled string messages.

## JSON

- `std.json.valid(source: String) -> Bool`
- `std.json.compact(source: String) -> Result<String, String>`
- `std.json.pretty(source: String) -> Result<String, String>`

The parser rejects duplicate object keys, invalid number grammar, malformed escapes and surrogate pairs, trailing data, inputs over 16 MiB, and nesting deeper than 256 levels.

## TCP, HTTP, and TLS

- `std.net.connect(host: String, port: Int, timeout: Float) -> Result<TcpStream, String>`
- `std.net.read(stream: TcpStream, maximum: Int) -> Result<String, String>`
- `std.net.write(stream: TcpStream, contents: String) -> Result<Null, String>`
- `std.net.close(stream: TcpStream) -> Result<Null, String>`
- `std.http.get(url: String, timeout: Float) -> Result<String, String>`

Timeouts must be greater than zero and no more than 300 seconds. TCP reads are capped at 16 MiB and require UTF-8. HTTP accepts `http` and `https`, caps headers at 64 KiB and bodies at 16 MiB, validates response framing, and treats non-2xx status as an error. HTTPS verifies hostnames and chains against the pinned Mozilla root set; there is no certificate-bypass API.

## Tasks and channels

- `std.task.spawn(function: fun() -> T) -> Task`
- `std.task.await(task: Task) -> Result<T, String>`
- `std.task.await_for(task: Task, timeout: Float) -> Result<T, String>`
- `std.task.cancel(task: Task) -> Null`
- `std.channel.create(capacity: Int) -> Channel`
- `std.channel.send(channel: Channel, value: T, timeout: Float) -> Result<Null, String>`
- `std.channel.receive(channel: Channel, timeout: Float) -> Result<T, String>`

Workers use OS threads, observe cancellation at every bytecode instruction, and are joined when their owning task is dropped. Channel capacities are bounded at 65,536. Only immutable transferable data may cross task results or channels; functions, modules, native resources, tasks, and channels are rejected.

## C embedding ABI

The `nivren-ffi` workspace crate builds static and dynamic libraries and publishes `crates/nivren-ffi/include/nivren.h`. The main VM forbids unsafe Rust. Raw-pointer conversion is confined to two documented ABI functions that validate nullability and UTF-8, contain panics, and return an explicitly freed owned buffer.
