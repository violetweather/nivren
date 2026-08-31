# Edition 5 fix ledger

Every known Edition 3/4 design wart that Edition 5 can still repair. After Freeze Proof these become permanent, so every row needs a decision before the Edition 5 Language Proof begins. This ledger is the working decision record; accepted rows are folded into `spec/LANGUAGE-5-DRAFT.md` and `spec/STANDARD-LIBRARY-5.md`, then marked Applied.

Decision values: **Pending** (no decision yet), **Applied** (folded into the Edition 5 drafts), **Rejected** (deliberately kept as-is, with the reason recorded in place of the recommendation).

## A. Punctuation that survived the "words, not punctuation" invariant

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 1 | `using name = resource` binds with `=` | `docs/LANGUAGE.md:179` | `using name set resource` | Pending |
| 2 | `adopt … for … { name = value }` maps with `=` | `spec/LANGUAGE-3.md:32` | `{ name set value }` | Pending |
| 3 | Generic constraints use `:` (`<Value: Named>`) | `spec/LANGUAGE-3.md:26` | `<Value is Named>` | Pending |
| 4 | `!` and `??` are punctuation among word operators | `spec/LANGUAGE-2.md:64,71` | `not x`; retire `??` in favor of `when … carries` / a named function | Pending |

## B. Protocol syntax fossil

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 5 | Protocol members use Edition 2 positional `define name(value: Self)` syntax and positional dispatch — the only such place left | `spec/LANGUAGE-3.md:29`, `examples/protocols.niv:2,24` | Members declare `takes { value is Self }` and dispatch with labels, like every other callable | Pending |

## C. One concept, several words

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 6 | Four "has this type" spellings: `is`, `as` (prepare), `from` (nominal type), `:` (generics) | `spec/LANGUAGE-4-DRAFT.md:16,26,35` | Collapse toward `is` where grammar allows; keep `as` only where it renames | Pending |
| 7 | Result cases are `Ok`/`Err` with positional `ok()`/`err()` while the type is spelled `Value or Problem` | `proofs/edition4/concurrent_pipeline.niv:18-23` | Rename cases to match the word spelling; labeled constructors | Pending |
| 8 | `std.tasks.spawn/await/all/race` duplicates `start`/`wait`/`together`/`race` | `docs/STANDARD_LIBRARY.md:73-76` | Language words are the only surface; library verbs become internal | Pending |
| 9 | `Comparable`/`Ordered` protocols vs `Compare`/`Key` derives; `Comparable` actually means equality | `spec/LANGUAGE-3.md:64-66` | Rename to `Equal` and `Ordered`; derives take the same names | Pending |
| 10 | `Iterable` (sealed) vs `Iterate` (new) vs `Iterator` (type) | `spec/LANGUAGE-3.md:66` | Keep `Iterate` + `Iterator`; retire `Iterable` | Pending |
| 11 | `with` means derive list, labeled call, and preparation | `spec/LANGUAGE-4-DRAFT.md:27,35,37` | Derive lists become `derives Json, Compare` | Pending |
| 12 | Both `each x in` and `each x within` exist | `spec/LANGUAGE-2.md:59` | `in` only; `within` is capability scopes only; `niv fix` rewrites | Applied (draft grammar uses `in`; rewrite noted) |

## D. Spec-vs-reality divergences

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 13 | Grammar says `repeat while cond`; every real program writes bare `repeat cond`; `while` is not reserved | `examples/hello.niv:19` vs draft grammar | Keep `repeat while`, reserve `while`, `niv fix` inserts it | Applied (draft keyword list + rewrite noted) |
| 14 | Edition 5 expression grammar chained back through "edition-four-unary" to Edition 2 — a final spec must be self-contained | prior draft §3 | Inline the complete expression grammar | Applied (draft §3) |
| 15 | `("," | ";")?` — three separator styles in every data position | draft §3, real code uses all three | Newline-or-comma only; remove `;` from data positions | Pending |
| 16 | Manifest grants have two value languages (`"allow"` vs `"path:…"`), and `PACKAGE-1.md` makes `[capabilities]`/`[limits]` illegal sections | `spec/LANGUAGE-3.md:119`, `spec/PACKAGE-1.md:5` | Normative PACKAGE-2 with structured grant tables; drop `"allow"` | Pending |

## E. Modules and imports

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 17 | `use "pkg/main.niv"` names every package module `main`; two packages collide | `proofs/edition4/database_driver.niv:1` | `use "…" as name`; `as` required when stems collide or are unhelpful | Pending |
| 18 | Package names allow `-` but namespaces must be identifiers, so `my-lib` is unimportable | `spec/PACKAGE-1.md:5` | Forbid `-` in package names or mandate `as` | Pending |
| 19 | `expose { a, b }` lives away from the declarations it affects | `examples/project/src/greetings.niv:12` | `expose` becomes a declaration modifier | Pending |

## F. Literals and lexis

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 20 | No `_` digit separators, no hex/binary literals, no float exponents | `spec/LANGUAGE-2.md:17` | Add `1_000_000`, `0x`, `0b`, exponent floats | Pending |
| 21 | Unknown string escapes are silently swallowed; no `\u{…}` | `spec/LANGUAGE-2.md:20,26` | Fixed escape set + `\u{…}`; unknown escape is an error | Pending |
| 22 | No raw string form; embedded PEM/JSON/SQL need doubled backslashes | `spec/LANGUAGE-2.md:26` | Add a raw string literal | Pending |

## G. Special cases a final edition could delete

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 23 | Seven positional-call global exceptions: `show`, `len`, `type`, `append`, `assert`, `ok`, `err` | `docs/STANDARD_LIBRARY.md:142` | Move under `std` with labels; keep `show` as the one statement form | Pending |
| 24 | `show x` and `show(x)` are both legal | `spec/LANGUAGE-2.md:54` | One spelling | Pending |
| 25 | `type` is a keyword and a global function, duplicating `std.reflect.kind` | `spec/LANGUAGE-4-DRAFT.md:26` | Delete the global | Pending |
| 26 | Zero-parameter calls still need `with {}` | `proofs/edition4/concurrent_pipeline.niv:21` | Make `with { }` optional for zero-parameter callables | Pending |
| 27 | A function reaching its closing brace silently returns `none`, even one declared `gives Int` | `spec/LANGUAGE-2.md:115` | Declared `gives` requires a proven explicit `give` on every path; add a real unit type | Pending |
| 28 | `or` carries five meanings; `… or give` scans as boolean `or` | `proofs/edition4/concurrent_pipeline.niv:17` | Decide one disambiguation: e.g. `gives Int fails String` for the type, keep `or give` for propagation | Pending |

## H. Standard-library naming (frozen forever at Freeze Proof — last chance)

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 29 | Five verb pairs for text↔value (`parse/stringify`, `parse/format`, `decode/encode`, `to_/from_json`, `hex/unhex`) | `docs/STANDARD_LIBRARY.md:9-22` | `parse`/`format` for scalars, `decode`/`encode` for structures, everywhere | Pending |
| 30 | `gunzip` vs `unzlib` | `docs/STANDARD_LIBRARY.md:106` | `gzip_decode`/`zlib_decode` | Pending |
| 31 | `open_read`/`open_write` vs `read_open`/`write_open` | `docs/STANDARD_LIBRARY.md:40` | One order: `open_read` + `read_from`/`write_to` | Pending |
| 32 | `std.bytes` has `from_string`/`to_string` but `from_values` without `to_values`; `std.list` lacks `find`/`count` that `std.iter` has | `docs/STANDARD_LIBRARY.md:7,25` | Complete both families | Pending |
| 33 | Constructors are `create`/`begin`/`single`/bare-noun | `docs/STANDARD_LIBRARY.md:60-97` | `create` everywhere; `std.map.of`/`empty` | Pending |
| 34 | Callback labels repeat their function (`transform with { transform set … }`); `count` vs `size` labels | `examples/iterators.niv:23-33` | One callback label and one size label | Pending |
| 35 | `verify_hmac_sha256` vs `ed25519_verify` | `docs/STANDARD_LIBRARY.md:12,17` | Verb-last: `hmac_sha256_verify` | Pending |
| 36 | Stringly-typed web/net values: response maps with `header:<name>` keys, `"read_write"` interest strings, TLS policy maps | `docs/STANDARD_LIBRARY.md:58-65` | Typed `Response`, `Interest` choice, `TlsOptions` shape | Pending |
| 37 | `read_ready`/`write_ready` do I/O but `ready` only waits | `docs/STANDARD_LIBRARY.md:67` | Rename the predicate `wait_ready` | Pending |
| 38 | `std.text.repeat` shares a name with the `repeat` keyword | `spec/STANDARD-LIBRARY-5.md` | Acceptable in member position; keep, note in style guide | Pending |

## I. Minor / lower confidence

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 39 | Literals are `yes`/`no` but the type is C's `Bool` | `spec/LANGUAGE-2.md:28` | Rename the type to match the word-first identity (e.g. `Truth`) — or record a deliberate keep | Pending |
| 40 | `STANDARD-LIBRARY-5.md` used the forbidden `Result<T, E>` spelling | draft §4 | Spell signatures `gives T or E` | Applied |
| 41 | `//` + nested `/* */` comments; no doc-comment syntax although `niv doc` exists | `spec/LANGUAGE-2.md:22` | Keep `//`; drop block-comment nesting; add a doc-comment form as `niv doc` input | Pending |
| 42 | `std.files.exists` gives bare `Bool`, hiding permission errors | `docs/STANDARD_LIBRARY.md:39` | `gives Bool or Problem` | Pending |
| 43 | `Null` doubles as the JSON variant name and the unit type | `proofs/edition4/cli_automation.niv:10` | Distinct unit type name (`Nothing`); `none` stays the absent value | Pending |

## J. Found during implementation

| # | Wart | Evidence | Recommendation | Decision |
| --- | --- | --- | --- | --- |
| 44 | `Iterate.advance` "takes no arguments and gives maybe Item" cannot thread iterator state through immutable values — the signature is unimplementable as specified | `spec/STANDARD-LIBRARY-5.md` §6, `LANGUAGE-5-DRAFT.md` §11 | Persistent unfold shape: `advance takes { state is Self } gives maybe Step` where the standard shape `Step<State, Item> holds { item, next }`; `each` threads `next` | Pending |
| 45 | Variables named `start`/`wait`/`race`/`together` are rejected because the task words are globally reserved, unlike `set`/`from` | `src/lexer.rs` keyword table | Decide: keep reserved (document in spec §2) or make contextual like `set` | Pending |
| 46 | Text-literal holes cannot contain string literals: the plain lexer ends the outer `text "…"` string at the first inner quote | `src/parser.rs` text_literal | Lex `text` literals in the lexer with hole-aware quoting (or spec that holes hold quote-free expressions) | Pending |
| 47 | `U128` does not fit `FixedInt`'s i128 payload; `I128` shipped, `U128` needs widened fixed-width storage | `src/fixed.rs` | Split `FixedInt.value` into signed/unsigned payloads (or store `U128` as `u128` beside `i128`) in a dedicated numeric pass | Pending |
| 48 | No edition marker exists in manifests or sources, which blocks the Edition 5 removals, `niv fix` rewrites, and extending the trusted-module gate to scripts | whole pipeline | Add `edition = 5` to `niv.toml` (PACKAGE-2) and an edition pragma for single files; gate removals and strict rules on it | Pending |
| 49 | `std.source` covers shapes, choices, and literal bindings; generated functions need a statement/expression builder vocabulary that must be designed, not grown ad hoc | `src/runtime.rs` std.source | Decide the builder set mirroring grammar productions (call, give, when, each) before adding function generation | Pending |

## Cross-cutting fixes already proposed in earlier planning

These predate the ledger sweep and are restated here so one document holds every open repair: typed problems replacing `Result<T, String>` throughout the library (the largest fix; overlaps #7, #36, #42), structured capability-scope grammar replacing the `"path:…"`/`"host:…"` string mini-language (overlaps #16), label and pattern punning (`with { x }` for `x set x`), and unifying the eight derives with `generate` so one generation mechanism remains (overlaps #11). All Pending.

## Suggested decision order

1. Semantic changes that alter checked behavior: #27 (fallthrough), typed problems, #16, #17/#18, #5.
2. Grammar word repairs with mechanical `niv fix` rewrites: #1–#4, #6, #11, #15, #23–#26, #28.
3. Library renames, one batch, before the name freeze: #29–#37, #42.
4. Lexis additions: #20–#22, #41.
5. Taste calls that only need a recorded yes/no: #9, #10, #19, #38, #39, #43.
