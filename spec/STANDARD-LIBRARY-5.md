# Nivren Edition 5 Standard Library Specification (design draft)

## 1. Status

This design draft accompanies `spec/LANGUAGE-5-DRAFT.md` and completes it. Everything in the Edition 3 standard-library specification and the Edition 4 additions carries forward unchanged except where this document adds, tightens, or removes surface. All APIs remain immutable members of `std`. Wrong arity or statically known wrong types are static errors; embedding boundaries MUST repeat validation. Fallible operations give a typed failure, absence gives `maybe T`, and effectful calls carry the listed capability.

Finality: after Freeze Proof, the namespace set and every contract in this document are frozen. Minor releases MAY add new functions under strict SemVer; they MUST NOT remove, rename, or change the behavior of anything specified here.

Every numeric bound in this document is a frozen **default** under the declared-limits policy of `LANGUAGE-5-DRAFT.md` section 15.3: an application's root manifest may declare a different concrete value for a named limit within its published range, and unconfigured programs keep the defaults forever.

## 2. Namespaces

The canonical namespaces are `std.files`, `std.web`, `std.tasks`, and `std.channels`, plus the topical namespaces this document names. The Edition 2 compatibility aliases `std.fs`, `std.http`, `std.task`, and `std.channel` are removed, exercising the removal clause of the Edition 3 specification; `niv fix` rewrites every alias reference to its canonical spelling. The namespace set is closed: new capability areas ship as packages, not as new `std` namespaces.

## 3. Numbers

Edition 5 completes the numeric tower described in `LANGUAGE-5-DRAFT.md` section 10.

- `std.uint.parse`, `format`, `from_int`, and checked `to_int` construct and convert the new core `UInt`. Parsing accepts canonical unsigned decimal text within the `UInt` range and bounds input to 20 bytes. `std.uint.wrapping_add`, `wrapping_sub`, and `wrapping_mul` provide explicit wrap-around; ordinary `UInt` arithmetic is checked and overflow is a runtime error.
- `I128` and `U128` join the fixed-width family with the same lowercase-namespace members as the existing widths: `from_int`, `parse`, `format`, and checked `to_int`. Parsing input is bounded at 40 bytes. Matching-width operands support checked arithmetic, equality, and ordering exactly as the existing fixed-width types do; crossing widths or signedness never coerces.
- The sealed protocol `Number` covers `Int`, `UInt`, `Float`, `BigInt`, `Decimal`, and every fixed-width type. The sealed refinements `Integer` (all integer forms) and `Exact` (all non-float forms) support generic constraints. Sealed numeric protocols cannot be adopted by user types.
- Every numeric namespace gains `min`, `max`, and `abs` (`abs` is absent from unsigned namespaces). Checked semantics apply: `abs` of the minimum signed value is a typed error.

## 4. Text

`std.text` grows from four functions to a complete bounded set. All functions operate on UTF-8 `String` values, give typed failures where listed, and cap every output at 16 MiB. Position and count arguments index Unicode scalar values, not bytes.

- `contains(String, String) gives Bool`, `ends_with(String, String) gives Bool`, and the existing `starts_with` complete the test family. An empty needle is rejected for `contains`.
- `index_of(String, String) gives maybe Int` returns the scalar position of the first occurrence.
- `slice(String, Int, Int) gives String or String` uses a start-inclusive, end-exclusive scalar range and rejects out-of-range or reversed bounds.
- `replace(String, String, String, Int) gives String or String` replaces at most the caller's 1 through 1,000,000 occurrence limit and rejects an empty needle.
- `trim(String)`, `trim_start(String)`, and `trim_end(String)` remove Unicode whitespace and always succeed.
- `to_upper(String) gives String or String` and `to_lower(String)` apply locale-independent Unicode default case mapping. There is no locale-sensitive casing in `std`; locale behavior is package territory.
- `join([String], String) gives String or String` concatenates with a separator under the output cap. `lines(String) gives [String]` splits on `\n`, normalizing CRLF, and caps at 1,000,000 lines.
- `repeat(String, Int) gives String or String`, `pad_start(String, Int, String)`, and `pad_end(String, Int, String)` build fixed-width text; the pad unit MUST be exactly one scalar. These are the formatting primitives — the `text` literal deliberately has no format mini-language, so width and padding stay in named functions.
- `codepoints(String) gives Iterator<Int>` and `graphemes(String) gives Iterator<String>` iterate scalar values and extended grapheme clusters. Both are lazy, single-pass, and subject to the standard iterator limits. "Grapheme" means one user-perceived character.
- `std.text.concat` and the existing `split`/`split_last` are unchanged.

## 5. Display

`Display` becomes an adoptable protocol with the single required member `display`, which takes no arguments and gives `String`. It is the contract behind `text` literal holes and diagnostic rendering.

- Built-in adoptions: `String` (identity), `Bool`, `Int`, `UInt`, every fixed-width type, `BigInt`, `Decimal`, `Float` (canonical finite formatting; NaN and infinity render as typed runtime errors at the hole), and `DateTime` (canonical zoned format).
- Arrays, maps, sets, iterators, tasks, channels, resources, secrets, and handles do not adopt `Display` and MUST be rejected in `text` holes; collections render through explicit user code. `SecretKey` and every secret-bearing type MUST never adopt `Display`.
- A user `display` implementation MUST be effect-free at the boundary; the checker rejects `needs` on `display`. Output above 16 MiB is a typed error at the hole.
- The `Display` derive continues to generate `display` for shapes; a hand-written adoption and a derive on the same type conflict and are rejected.

## 6. Iteration

The `Iterate<Item>` protocol from `LANGUAGE-5-DRAFT.md` section 11 has the single required member `advance`, taking no arguments and giving `maybe Item`. `each`, `through` stream stages, and every `std.iter` adapter accept adopters.

- `std.iter.from_iterate(value) gives Iterator<Item>` wraps an adopter explicitly for use with the adapter functions.
- `std.iter.indexed(Iterator<T>) gives Iterator<Indexed<T>>` pairs each element with its zero-based position using the standard shape `Indexed<T> holds { index is Int, value is T }`.
- All existing limits are frozen: single-pass, non-transferable, non-comparable, at most 1,024 lazy stages, at most 1,000,000 materialized values. `advance` implementations MUST be effect-free at the boundary; the checker rejects `needs` on `advance`.

## 7. Time

- `std.time.monotonic() gives Float` needs `Time` and returns seconds from an arbitrary fixed origin that never goes backward within a process. It is the elapsed-time and benchmarking clock.
- The Edition 2 compatibility wall clock `std.time.now() gives Float` and the `clock()` built-in are removed. Wall-clock instants use `std.time.now_zoned`; elapsed time uses `monotonic`. `niv fix` rewrites `clock()` and `std.time.now()` calls to `std.time.monotonic()`, which preserves every elapsed-time use; a rewritten call whose result was compared to a Unix timestamp is flagged for review rather than silently rewritten.
- `std.time.year`, `month`, `day`, `hour`, `minute`, `second`, and `weekday` give the named calendar field of a `DateTime` as `Int` in its own zone; `weekday` gives 1 (Monday) through 7 (Sunday), following ISO 8601.
- `std.time.difference_seconds(DateTime, DateTime) gives Int or String` gives the signed whole-second difference and fails on overflow. Existing `DateTime` construction, parsing, formatting, zone conversion, and arithmetic are unchanged.

## 8. Reflection

`std.reflect` keeps exactly three functions and completes `schema`:

- `schema` now also accepts generic shape and choice constructors and function values. Generic results include deterministic `$parameters` and `$constraints` entries naming each parameter and its protocol constraints. Function results include deterministic `$takes` (parameter names and canonical type strings in declaration order), `$gives`, `$fails` (present only for `or` results), `$needs` (capability names with their `within` scopes), and `$promises` (active promise clauses in canonical form) entries.
- The permanent prohibitions are unchanged: no addresses, no object layout, no lexical values, no private runtime state, no compiler implementation types. No fourth reflection function will be added in any edition.

## 9. Source builders

`std.source` is the compile-time construction API consumed by `generate` bodies. It mirrors the grammar of `LANGUAGE-5-DRAFT.md` section 3 one production to one builder; there is no text or token input anywhere in the namespace.

- Declaration builders: `source.shape`, `source.choice`, `source.nominal_type`, `source.function`, `source.binding`, and `source.adoption` each give `source.Declaration or String` from labeled, typed parts.
- Statement and expression builders cover bindings, reassignment, calls with labels, `when`, `choose` arms with patterns, `each`, `repeat`, `stop`, `skip`, `using`, `perform`, `through`, `text` literals, literals, and member/index access. Every identifier passes through `source.name(String)`, which validates a single well-formed Nivren identifier and rejects keywords, whitespace, and control characters — this rule is what makes text pasting impossible.
- Builders validate eagerly: an invalid part fails the builder call with a diagnostic-quality message, never at expansion insertion.
- Bounds: one `expand` MUST NOT insert more than 1,024 declarations, and generator recursion depth through `expand` MUST NOT exceed 8. Both limits are frozen.
- Every produced declaration carries its generator provenance for `niv expand`, diagnostics, coverage, and the debugger.

## 10. GPU

`std.gpu` is the minimal accelerator surface under the new `Gpu` capability. It dispatches named, versioned compute kernels over bounded byte buffers; it is not a graphics API, and shader source never crosses the boundary as text from ordinary programs.

- `std.gpu.available() gives Bool` needs `Gpu` and reports whether any adapter usable under the current grants exists.
- `std.gpu.open(String) gives GpuDevice or String` needs `Gpu` scoped `within` an adapter boundary. `GpuDevice` is closable and never transferable, comparable, serializable, or a stable key. `using` MUST close a live device on every exit path.
- `std.gpu.run(GpuDevice, String, [Bytes], Int) gives [Bytes] or String` dispatches one named built-in kernel with input buffers and a caller-declared output ceiling. Kernel names follow the host-operation name rules; each buffer and each output is capped at 16 MiB; total buffers per dispatch are capped at 16. Dispatch is synchronous and cancellable at buffer boundaries.
- When no adapter is available, `open` fails with a typed availability error and MUST NOT substitute a silent CPU emulation; a host MAY offer an explicit CPU adapter name so fallback stays visible in the capability scope.
- The built-in kernel catalog, its versioning, and custom-kernel packaging are host-bridge concerns specified with the embedding ABI, not language surface.

## 11. Plans

`std.plans` moves portable plans between programs, per `LANGUAGE-5-DRAFT.md` section 13.3. Both functions are pure and need no capability; moving the bytes uses the ordinary file, channel, and network APIs under their own capabilities.

- `std.plans.encode(plan) gives Bytes or String` serializes a compiler-proven portable plan as versioned `org.nivren.portable-plan.v1` bytes, capped at 16 MiB. Encoding a non-portable plan value is a static error where provable and a typed error otherwise. Equal plans encode to equal bytes within a release.
- `std.plans.decode(S, Bytes) gives S or String`, where `S` is a plan shape constructor, validates strictly: unknown format versions, mismatched shapes, missing or unexpected fields, out-of-range numbers, and any embedded handle, secret, callback, or authority marker are typed errors. A decoded plan carries zero authority; performing it is gated entirely by the performing host's grants, promises, and declared limits.

## 12. Removals

Removed by this edition, each with a `niv fix` rewrite: the `std.fs`, `std.http`, `std.task`, and `std.channel` aliases; the `clock()` built-in; and the `std.time.now()` compatibility clock. No other function is removed, and after Freeze Proof none ever will be.

## 13. Resource and security rules

The Edition 3 resource and security rules apply unchanged to every new API in this document: every host-sized input is bounded, host failures are data unless the contract identifies runtime misuse, handles are opaque and fail safely after close, and project capability grants are enforced again at runtime. `Gpu` grants use `within` adapter boundaries with the same manifest validation rules as existing scoped capabilities.

Two Edition 5 policies extend these rules. Declared limits (`LANGUAGE-5-DRAFT.md` section 15.3) let only the application's root manifest change a bound, within published ranges, visibly in `niv explain` and the authority diff. Trusted modules (`LANGUAGE-5-DRAFT.md` section 15.2) are the only callers the checker accepts for `std.native`, `std.host`, and the unchecked systems APIs; their exposed surfaces remain ordinary safe, capability-declared APIs.
