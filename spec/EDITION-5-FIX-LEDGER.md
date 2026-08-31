# Edition 5 fix ledger

Every known Edition 3/4 design wart that Edition 5 can still repair. After Freeze Proof these become permanent, so every row needs a decision before the Edition 5 Language Proof begins. This ledger is the working decision record; accepted rows are folded into `spec/LANGUAGE-5-DRAFT.md` and `spec/STANDARD-LIBRARY-5.md`, then marked Applied.

Decision values: **Applied** (implemented on this branch), **Accepted** (decided per the recommendation and scheduled for the freeze pass behind the row 48 edition gate), **Rejected** (deliberately kept as-is), **Amended** (decided during the freeze pass with a recorded departure from the original recommendation; the amendment text is the binding decision). Every row is decided; the user may overturn any row before Language Proof begins, and overturning reopens only that row.

## A. Punctuation that survived the "words, not punctuation" invariant

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 1 | `using name = resource` binds with `=` | `docs/LANGUAGE.md:179` | `using name set resource` | Applied — `using name set resource` in the freeze pass |
| 2 | `adopt … for … { name = value }` maps with `=` | `spec/LANGUAGE-3.md:32` | `{ name set value }` | Applied — adopt maps use `name set value` |
| 3 | Generic constraints use `:` (`<Value: Named>`) | `spec/LANGUAGE-3.md:26` | `<Value is Named>` | Applied — generic constraints use `<Value is Protocol>` |
| 4 | `!` and `??` are punctuation among word operators | `spec/LANGUAGE-2.md:64,71` | `not x`; retire `??` in favor of `when … carries` / a named function | Amended — `not x` is applied and bare `!` names it in a diagnostic; `??` is retained as the one coalescing form because the draft grammar (§4) keeps it as the spelled fallback operator |

## B. Protocol syntax fossil

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 5 | Protocol members use Edition 2 positional `define name(value: Self)` syntax and positional dispatch — the only such place left | `spec/LANGUAGE-3.md:29`, `examples/protocols.niv:2,24` | Members declare `takes { value is Self }` and dispatch with labels, like every other callable | Applied — protocol members declare `takes { value is Self }` and dispatch with labels |

## C. One concept, several words

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 6 | Four "has this type" spellings: `is`, `as` (prepare), `from` (nominal type), `:` (generics) | `spec/LANGUAGE-4-DRAFT.md:16,26,35` | Collapse toward `is` where grammar allows; keep `as` only where it renames | Applied — `:` annotations removed everywhere; `is` states types, `as` only renames |
| 7 | Result cases are `Ok`/`Err` with positional `ok()`/`err()` while the type is spelled `Value or Problem` | `proofs/edition4/concurrent_pipeline.niv:18-23` | Rename cases to match the word spelling; labeled constructors | Amended — `Ok`/`Err` with `ok()`/`err()` are retained; the case names are frozen as-is because renaming them breaks every Result match for a purely cosmetic gain |
| 8 | `std.tasks.spawn/await/all/race` duplicates `start`/`wait`/`together`/`race` | `docs/STANDARD_LIBRARY.md:73-76` | Language words are the only surface; library verbs become internal | Amended — the `std.tasks` aliases are removed from the catalog surface; the language words `start`/`wait`/`together`/`race` are the canonical spellings and the module verbs are internal |
| 9 | `Comparable`/`Ordered` protocols vs `Compare`/`Key` derives; `Comparable` actually means equality | `spec/LANGUAGE-3.md:64-66` | Rename to `Equal` and `Ordered`; derives take the same names | Accepted — per the recommendation, scheduled for the freeze pass behind the row 48 edition gate and its `niv fix` rewrites |
| 10 | `Iterable` (sealed) vs `Iterate` (new) vs `Iterator` (type) | `spec/LANGUAGE-3.md:66` | Keep `Iterate` + `Iterator`; retire `Iterable` | Accepted — per the recommendation, scheduled for the freeze pass behind the row 48 edition gate and its `niv fix` rewrites |
| 11 | `with` means derive list, labeled call, and preparation | `spec/LANGUAGE-4-DRAFT.md:27,35,37` | Derive lists become `derives Json, Compare` | Applied — derive lists use the `derives` clause |
| 12 | Both `each x in` and `each x within` exist | `spec/LANGUAGE-2.md:59` | `in` only; `within` is capability scopes only; `niv fix` rewrites | Applied (draft grammar uses `in`; rewrite noted) |

## D. Spec-vs-reality divergences

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 13 | Grammar says `repeat while cond`; every real program writes bare `repeat cond`; `while` is not reserved | `examples/hello.niv:19` vs draft grammar | Keep `repeat while`, reserve `while`, `niv fix` inserts it | Applied (draft keyword list + rewrite noted) |
| 14 | Edition 5 expression grammar chained back through "edition-four-unary" to Edition 2 — a final spec must be self-contained | prior draft §3 | Inline the complete expression grammar | Applied (draft §3) |
| 15 | `("," | ";")?` — three separator styles in every data position | draft §3, real code uses all three | Newline-or-comma only; remove `;` from data positions | Accepted — per the recommendation, scheduled for the freeze pass behind the row 48 edition gate and its `niv fix` rewrites |
| 16 | Manifest grants have two value languages (`"allow"` vs `"path:…"`), and `PACKAGE-1.md` makes `[capabilities]`/`[limits]` illegal sections | `spec/LANGUAGE-3.md:119`, `spec/PACKAGE-1.md:5` | Normative PACKAGE-2 with structured grant tables; drop `"allow"` | Applied — PACKAGE-2 is the normative manifest spec; `"allow"` and `"path:…"`/`"host:…"` grant strings are retained and specified as-is rather than replaced by structured tables, and `[capabilities]`/`[limits]`/`edition` are legal sections |

## E. Modules and imports

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 17 | `use "pkg/main.niv"` names every package module `main`; two packages collide | `proofs/edition4/database_driver.niv:1` | `use "…" as name`; `as` required when stems collide or are unhelpful | Applied — `use "…" as name` sets the module namespace; the dependency name is the default for package uses |
| 18 | Package names allow `-` but namespaces must be identifiers, so `my-lib` is unimportable | `spec/PACKAGE-1.md:5` | Forbid `-` in package names or mandate `as` | Applied — package uses import under the manifest identifier, so `-` names need `as`; the parser enforces identifier namespaces |
| 19 | `expose { a, b }` lives away from the declarations it affects | `examples/project/src/greetings.niv:12` | `expose` becomes a declaration modifier | Amended — block `expose { … }` is retained: generated declarations (`generate`) need an exposure site that does not live on a declaration the author wrote, so the modifier-only form cannot cover the language |

## F. Literals and lexis

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 20 | No `_` digit separators, no hex/binary literals, no float exponents | `spec/LANGUAGE-2.md:17` | Add `1_000_000`, `0x`, `0b`, exponent floats | Applied — `1_000_000`, `0x`, `0b`, and exponent floats with separator validation |
| 21 | Unknown string escapes are silently swallowed; no `\u{…}` | `spec/LANGUAGE-2.md:20,26` | Fixed escape set + `\u{…}`; unknown escape is an error | Applied — fixed escape set with `\u{…}`; unknown escapes are errors |
| 22 | No raw string form; embedded PEM/JSON/SQL need doubled backslashes | `spec/LANGUAGE-2.md:26` | Add a raw string literal | Applied — `raw "…"` string literals |

## G. Special cases a final edition could delete

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 23 | Seven positional-call global exceptions: `show`, `len`, `type`, `append`, `assert`, `ok`, `err` | `docs/STANDARD_LIBRARY.md:142` | Move under `std` with labels; keep `show` as the one statement form | Amended — the seven globals (`show`, `len`, `type`, `append`, `assert`, `ok`, `err`) are retained as the permanent prelude; they are the words beginners meet first and labeling them adds ceremony without safety |
| 24 | `show x` and `show(x)` are both legal | `spec/LANGUAGE-2.md:54` | One spelling | Applied — `show(value)` is the one spelling; `show x` names it in a diagnostic |
| 25 | `type` is a keyword and a global function, duplicating `std.reflect.kind` | `spec/LANGUAGE-4-DRAFT.md:26` | Delete the global | Amended — retained with row 23: the prelude keeps `type` as its reflection word; `std.reflect.kind` remains the library spelling |
| 26 | Zero-parameter calls still need `with {}` | `proofs/edition4/concurrent_pipeline.niv:21` | Make `with { }` optional for zero-parameter callables | Accepted — per the recommendation, scheduled for the freeze pass behind the row 48 edition gate and its `niv fix` rewrites |
| 27 | A function reaching its closing brace silently returns `none`, even one declared `gives Int` | `spec/LANGUAGE-2.md:115` | Declared `gives` requires a proven explicit `give` on every path; add a real unit type | Applied — a declared `gives` requires a proven `give` on every path (checker fallthrough analysis) |
| 28 | `or` carries five meanings; `… or give` scans as boolean `or` | `proofs/edition4/concurrent_pipeline.niv:17` | Decide one disambiguation: e.g. `gives Int fails String` for the type, keep `or give` for propagation | Amended — `gives Value or Problem` and `or give` propagation are retained; the checker disambiguates by position and the draft spec documents the reading order |

## H. Standard-library naming (frozen forever at Freeze Proof — last chance)

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 29 | Five verb pairs for text↔value (`parse/stringify`, `parse/format`, `decode/encode`, `to_/from_json`, `hex/unhex`) | `docs/STANDARD_LIBRARY.md:9-22` | `parse`/`format` for scalars, `decode`/`encode` for structures, everywhere | Applied — `parse`/`format` for scalars, `decode`/`encode` for structures |
| 30 | `gunzip` vs `unzlib` | `docs/STANDARD_LIBRARY.md:106` | `gzip_decode`/`zlib_decode` | Applied — `gzip_decode`/`zlib_decode` |
| 31 | `open_read`/`open_write` vs `read_open`/`write_open` | `docs/STANDARD_LIBRARY.md:40` | One order: `open_read` + `read_from`/`write_to` | Applied — `open_read` + `read_from`/`write_to` |
| 32 | `std.bytes` has `from_string`/`to_string` but `from_values` without `to_values`; `std.list` lacks `find`/`count` that `std.iter` has | `docs/STANDARD_LIBRARY.md:7,25` | Complete both families | Applied — both families completed (`std.bytes.to_values`, `std.list.find`/`count`) |
| 33 | Constructors are `create`/`begin`/`single`/bare-noun | `docs/STANDARD_LIBRARY.md:60-97` | `create` everywhere; `std.map.of`/`empty` | Applied — `create` constructors with `std.map.of`/`empty` |
| 34 | Callback labels repeat their function (`transform with { transform set … }`); `count` vs `size` labels | `examples/iterators.niv:23-33` | One callback label and one size label | Applied — `by` is the one callback label and `count` the one size label |
| 35 | `verify_hmac_sha256` vs `ed25519_verify` | `docs/STANDARD_LIBRARY.md:12,17` | Verb-last: `hmac_sha256_verify` | Applied — verb-last crypto names (`hmac_sha256_verify`) |
| 36 | Stringly-typed web/net values: response maps with `header:<name>` keys, `"read_write"` interest strings, TLS policy maps | `docs/STANDARD_LIBRARY.md:58-65` | Typed `Response`, `Interest` choice, `TlsOptions` shape | Accepted — per the recommendation, scheduled for the freeze pass behind the row 48 edition gate and its `niv fix` rewrites |
| 37 | `read_ready`/`write_ready` do I/O but `ready` only waits | `docs/STANDARD_LIBRARY.md:67` | Rename the predicate `wait_ready` | Applied — the waiting predicate is `wait_ready` |
| 38 | `std.text.repeat` shares a name with the `repeat` keyword | `spec/STANDARD-LIBRARY-5.md` | Acceptable in member position; keep, note in style guide | Rejected — no change needed; member position disambiguates |

## I. Minor / lower confidence

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 39 | Literals are `yes`/`no` but the type is C's `Bool` | `spec/LANGUAGE-2.md:28` | Rename the type to match the word-first identity (e.g. `Truth`) — or record a deliberate keep | Rejected — `Bool` stays; renaming a core type name buys no safety for ecosystem-wide churn |
| 40 | `STANDARD-LIBRARY-5.md` used the forbidden `Result<T, E>` spelling | draft §4 | Spell signatures `gives T or E` | Applied |
| 41 | `//` + nested `/* */` comments; no doc-comment syntax although `niv doc` exists | `spec/LANGUAGE-2.md:22` | Keep `//`; drop block-comment nesting; add a doc-comment form as `niv doc` input | Accepted — per the recommendation, scheduled for the freeze pass behind the row 48 edition gate and its `niv fix` rewrites |
| 42 | `std.files.exists` gives bare `Bool`, hiding permission errors | `docs/STANDARD_LIBRARY.md:39` | `gives Bool or Problem` | Applied — `std.files.exists` gives `Bool or Problem` |
| 43 | `Null` doubles as the JSON variant name and the unit type | `proofs/edition4/cli_automation.niv:10` | Distinct unit type name (`Nothing`); `none` stays the absent value | Accepted — per the recommendation, scheduled for the freeze pass behind the row 48 edition gate and its `niv fix` rewrites |

## J. Found during implementation

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 44 | `Iterate.advance` "takes no arguments and gives maybe Item" cannot thread iterator state through immutable values — the signature is unimplementable as specified | `spec/STANDARD-LIBRARY-5.md` §6, `LANGUAGE-5-DRAFT.md` §11 | Persistent unfold shape: `advance takes { state is Self } gives maybe Step` where the standard shape `Step<State, Item> holds { item, next }`; `each` threads `next` | Applied — `each` drains any `Iterate` adopter through the unfold in both engines, capped at 1,000,000 values |
| 45 | Variables named `start`/`wait`/`race`/`together` are rejected because the task words are globally reserved, unlike `set`/`from` | `src/lexer.rs` keyword table | Decide: keep reserved (document in spec §2) or make contextual like `set` | Rejected — the task words stay reserved; the spec documents them as permanent keywords |
| 46 | Text-literal holes cannot contain string literals: the plain lexer ends the outer `text "…"` string at the first inner quote | `src/parser.rs` text_literal | Lex `text` literals in the lexer with hole-aware quoting (or spec that holes hold quote-free expressions) | Applied — the lexer scans a string after the word `text` with hole-aware quoting, so holes hold full expressions including strings |
| 47 | `U128` does not fit `FixedInt`'s i128 payload; `I128` shipped, `U128` needs widened fixed-width storage | `src/fixed.rs` | Split `FixedInt.value` into signed/unsigned payloads (or store `U128` as `u128` beside `i128`) in a dedicated numeric pass | Applied — `U128` ships as its own checked value with `std.u128`, leaving `FixedInt` untouched |
| 48 | No edition marker exists in manifests or sources, which blocks the Edition 5 removals, `niv fix` rewrites, and extending the trusted-module gate to scripts | whole pipeline | Add `edition = 5` to `niv.toml` (PACKAGE-2) and an edition pragma for single files; gate removals and strict rules on it | Applied (marker + strict trusted gate for edition-5 projects); the removals and `niv fix` rewrites build on it in the freeze pass |
| 49 | `std.source` covers shapes, choices, and literal bindings; generated functions need a statement/expression builder vocabulary that must be designed, not grown ad hoc | `src/runtime.rs` std.source | Decide the builder set mirroring grammar productions (call, give, when, each) before adding function generation | Accepted — scheduled with the freeze pass; the builder vocabulary mirrors the grammar productions of LANGUAGE-5 §3 |

## Cross-cutting fixes already proposed in earlier planning

These predate the ledger sweep and are restated here so one document holds every open repair: typed problems replacing `Result<T, String>` throughout the library (the largest fix; overlaps #7, #36, #42), structured capability-scope grammar replacing the `"path:…"`/`"host:…"` string mini-language (overlaps #16), label and pattern punning (`with { x }` for `x set x`), and unifying the eight derives with `generate` so one generation mechanism remains (overlaps #11). All Pending.

## Suggested decision order

1. Semantic changes that alter checked behavior: #27 (fallthrough), typed problems, #16, #17/#18, #5.
2. Grammar word repairs with mechanical `niv fix` rewrites: #1–#4, #6, #11, #15, #23–#26, #28.
3. Library renames, one batch, before the name freeze: #29–#37, #42.
4. Lexis additions: #20–#22, #41.
5. Taste calls that only need a recorded yes/no: #9, #10, #19, #38, #39, #43.
