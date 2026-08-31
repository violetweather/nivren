# Nivren Language Specification, Edition 5 (design draft)

## 1. Status

This design draft defines the complete intended source surface for Edition 5, the final syntax edition of Nivren. It has not entered Language Proof. Nothing in this document weakens the passed Edition 4 gates; Edition 4 remains the executable candidate until the Edition 5 proof program begins.

Edition 5 exists to finish the language and to complete its identity. It completes pattern matching, compile-time generation, the numeric tower, user iteration, loop control, and text construction; it adds enforceable negative promises, checked samples, guaranteed effect recording and replay, a normative diagnostic contract, trusted systems modules, declared limits, and mobile plans; it removes every retained compatibility surface; and it closes every design question the Edition 3 and Edition 4 specifications left open. The open repair decisions inherited from earlier editions are tracked in `spec/EDITION-5-FIX-LEDGER.md`; every row there MUST be Applied or Rejected before Language Proof begins. After Edition 5 freezes, later editions MAY correct soundness defects and MAY assign reserved space, but MUST NOT introduce new syntax, new keywords, new operators, or new capability names. That finality clause is itself normative.

## 1.1 Implementation status (edition-5-draft branch)

Executable today in both engines, with tests: loop control (`stop`/`skip`), full `choose` patterns (literals, `any`, bindings, `or`, guards, `otherwise [as name]`, nested `carries`, shape patterns), patterns in `keep`/`each`, `when … carries` over `maybe` and choice values, `text "…"` literals with holes holding full expressions including strings, `raw "…"` literals, the complete numeric lexis (`_` separators, `0x`, `0b`, exponents) and strict escapes with `\u{…}`, `promise` (static and runtime enforcement in both engines, Bytecode 8 `Promise`), `sample`, `trusted` modules, `UInt`/`I128`/`U128` with `std.uint`/`std.u128`, the `Iterate` protocol as a persistent unfold drained by `each` in both engines, effect recording/replay (`niv record`/`niv replay`), plan mobility (`std.plans`), the `Gpu` capability with a visible-unavailability `std.gpu` stub, `niv explain --story`, grown `std.text` and `std.time`, function reflection, `Display`-deriving shapes plus date/times and all numeric forms in text holes, and `generate`/`expand` with the `std.source` builders (`shape`, `choice`, `binding`) — expansion runs before checking in every pipeline, inside an interpreter with an empty capability set and a frozen 1,000,000-instruction budget, splicing at most 1,024 declarations per expand. The freeze pass has landed: the `edition = "5"` manifest marker (ledger row 48) gates the strict rules, the legacy surfaces are removed with no fallbacks (Edition 5 is a breaking update — `=` bindings, `:` annotations, caseless `choose` arms, positional protocol members, `!`, `each … within`, and the retired library spellings all stop with diagnostics that name the Edition 5 form), the library renames are complete, and declared limits (`payload_bytes` under `[limits]`), the authority diff on install, and the diagnostics catalog (`NIV5001`–`NIV5020` in `docs/DIAGNOSTICS.md`, printed as `error[NIV…]`) are live. `spec/PACKAGE-2.md` is the normative manifest contract. Every ledger row is decided; the rows still marked Accepted (statement-level `std.source` builders and the remaining library-shape repairs) are scheduled additive work that does not move syntax. The decision record lives in `spec/EDITION-5-FIX-LEDGER.md`.

## 2. Identity invariants

Edition 5 source states what it keeps, changes, takes, gives, needs, promises, prepares, performs, generates, starts, waits for, and chooses. Bindings, labeled values, and patterns use words rather than assignment punctuation. Recoverable failure remains typed. Capabilities remain statically required and drawn from a closed vocabulary. Resource and task lifetime remain scoped. Familiar literal, arithmetic, comparison, indexing, and member expressions are retained.

The punctuation budget is permanent: Edition 5 adds the keywords `any`, `expand`, `generate`, `promise`, `sample`, `trusted`, and `while` (reserved so `repeat while` is unambiguous), adds the contextual keywords `text`, `never`, `only`, `shows`, `stop`, and `skip` (loop exits only where the word stands alone in statement position, so member names such as `std.iter.skip` keep their ordinary meaning), reuses `or`, `when`, `otherwise`, `holds`, `carries`, `set`, and `within` inside its new forms, and adds no new operators. Operator overloading is permanently rejected; user types expose named methods instead. An implementation MUST reject duplicate or unknown derives and duplicate labels. For a callable declared in the same source unit, labels MUST exactly match its parameter or field names in declaration order. A formatter MUST be idempotent.

`set` and `from` remain contextual keywords forever; they MUST stay valid as ordinary identifiers outside their clause positions. `text` is contextual in the same way: it is a keyword only immediately before a string literal. `never` and `only` are keywords only inside a `promise` clause, and `shows` only after a `sample` block. This closes the Edition 4 stop-and-correct item permanently.

## 3. Grammar

Edition 5 carries the entire Edition 4 grammar forward and extends it. The complete surface is:

```ebnf
binding        = "keep", binding-target, ["is", type], "set", expression, ";"? ;
mutable        = "change", identifier, ["is", type], "set", expression, ";"? ;
reassignment   = "change", identifier, "to", expression, ";"? ;
binding-target = identifier | shape-pattern ;
function       = "define", identifier, [generic-parameters],
                 ["takes", "{", {parameter}, "}"],
                 ["gives", type, ["or", type]],
                 ["needs", need, {",", need}],
                 "{", {declaration}, "}" ;
parameter      = identifier, "is", type, ("," | ";")? ;
need           = identifier, ["within", string] ;
nominal-type   = "type", identifier, "from", type, ";"? ;
shape          = "shape", identifier, [generic-parameters], "holds", "{",
                 {field}, "}", ["with", derive, {",", derive}] ;
field          = identifier, "is", type, ("," | ";")? ;
choice         = "choice", identifier, [generic-parameters], "holds", "{",
                 case, {case}, "}" ;
case           = "case", identifier, ["carries", type], ("," | ";")? ;
type           = "maybe", type | identifier | "[", type, "]" |
                 identifier, "<", type, {",", type}, ">" ;
preparation    = "prepare", identifier, "as", identifier, "with", labeled-values ;
labeled-call   = postfix, "with", labeled-values ;
labeled-values = "{", {identifier, "set", expression, ("," | ";")?}, "}" ;
selection      = "choose", expression, "{", {selection-arm}, [default-arm], "}" ;
selection-arm  = "case", pattern, {"or", pattern}, ["when", expression],
                 "=>", expression, ("," | ";")? ;
default-arm    = "otherwise", ["as", identifier], "=>", expression, ("," | ";")? ;
pattern        = "any" | literal | identifier | case-pattern | shape-pattern ;
case-pattern   = identifier, ["carries", pattern] ;
shape-pattern  = identifier, "holds", "{",
                 {identifier, "set", pattern, ("," | ";")?}, "}" ;
promise        = "promise", promise-clause, {",", promise-clause}, ";"? ;
promise-clause = "never", identifier |
                 identifier, "only", "within", string, {",", string} ;
sample         = "sample", string, "{", {declaration}, "}", ["shows", string] ;
trust-header   = "trusted", string, ";"? ;
generator      = "generate", identifier, [generic-parameters],
                 ["takes", "{", {parameter}, "}"], "{", {declaration}, "}" ;
expansion      = "expand", identifier, ["with", labeled-values], ";"? ;
conditional    = "when", expression, ["carries", pattern, {"or", pattern}],
                 statement, ["otherwise", statement] ;
iteration      = "each", binding-target, "in", expression, statement ;
repetition     = "repeat", "while", expression, statement ;
loop-exit      = ("stop" | "skip"), ";"? ;
text-literal   = "text", string ;
expression     = pipeline ;
pipeline       = fallible, {"through", fallible} ;
fallible       = coalesce, ["or", "give"] ;
coalesce       = logical-or, ["??", coalesce] ;
logical-or     = logical-and, {"or", logical-and} ;
logical-and    = equality, {"and", equality} ;
equality       = comparison, {("==" | "!="), comparison} ;
comparison     = term, {("<" | "<=" | ">" | ">="), term} ;
term           = factor, {("+" | "-"), factor} ;
factor         = unary, {("*" | "/" | "%"), unary} ;
unary          = ("perform" | "!" | "-"), unary | postfix ;
postfix        = primary, {call | labeled-call-suffix | index | member} ;
call           = "(", [expression, {",", expression}], ")" ;
labeled-call-suffix = "with", labeled-values ;
index          = "[", expression, "]" ;
member         = ".", identifier ;
primary        = integer | float | string | text-literal | "yes" | "no" |
                 "none" | identifier | array | selection |
                 "(", expression, ")" ;
array          = "[", [expression, {",", expression}], "]" ;
```

The expression grammar above is now self-contained; no production is defined by reference to a retired edition. The `!` and `??` operators and the positional `call` form remain only while fix-ledger rows A4 and G23 in `spec/EDITION-5-FIX-LEDGER.md` are undecided; their removal would be a grammar change recorded there, not a silent edit here.

Derives, labeled-call checking, scoped `needs`, preparation, `perform`, `through`, structured concurrency (`start`, `wait`, `together`, `race`), `using`, protocols, and adoptions retain their Edition 4 semantics except where a section below tightens them. `gives Value or Problem` remains the checked result type; `maybe Value` remains the standard optional type; Edition 5 source MUST NOT require `T?` or `Result<T, E>` spellings.

## 4. Pattern semantics

`choose` selects over a subject value with full structural patterns.

- A **case pattern** names a case of the subject's choice type. `carries` matches the payload with a nested pattern. An identifier in pattern position that resolves to a case of the subject's type is a case pattern; any other identifier is a **binding pattern** and binds the matched value immutably in the arm. Resolution is by name, never by casing convention, and a diagnostic MUST name the shadowed case when a binding would hide one.
- A **shape pattern** names a nominal shape and matches named fields with nested patterns. Omitted fields match anything; a listed field name MUST exist on the shape and MUST NOT repeat.
- A **literal pattern** (integer, float, string, boolean, `none`) matches by the type's standard equality. Float literal patterns MUST be rejected when the subject type is `Float`, because binary64 equality is not a safe selector.
- `any` matches every value and binds nothing.
- `or` joins alternative patterns in one arm. Every alternative MUST bind the same names at the same types.
- A `when` guard evaluates an arm-local boolean after the pattern matches and its bindings are in scope. Guard expressions MUST be pure: no `perform`, no capability use, no mutation of outer bindings.

Exhaustiveness remains mandatory and is checked structurally. Guarded arms contribute nothing to exhaustiveness. When the unguarded arms do not cover the subject, the `choose` MUST end with an `otherwise` arm; `otherwise as name` binds the unmatched value. A `choose` whose unguarded case arms are already exhaustive MUST NOT carry `otherwise`, and unreachable arms MUST be rejected with a diagnostic that shows the covering earlier arm. Arms match top to bottom; the checker MUST prove that ordering can only matter between guarded arms and `otherwise`.

The Edition 4 single-binding form `case Name carries x` is unchanged: it is a case pattern carrying a binding pattern.

### 4.1 Patterns in bindings and iteration

`keep` and `each` accept an **irrefutable** shape pattern as their binding target. Irrefutable means the pattern can never fail: shape patterns whose nested patterns are all binding patterns or `any`. Case patterns, literals, and guards are refutable and MUST be rejected in binding targets with a diagnostic that points to `choose` or `when … carries`.

```
keep Point holds { x set x, y set y } set corner
each Row holds { id set id } in rows { … }
```

All names introduced by a binding-target pattern are immutable, exactly as `keep` bindings are. `change` accepts only a plain identifier; mutation always names one binding.

### 4.2 Conditional matching with `when … carries`

`when subject carries pattern` tests one pattern without a full `choose`.

- If the subject is `maybe T`, the pattern matches the present value; `none` takes the `otherwise` branch.
- If the subject is a choice, the pattern MUST be a case pattern (with optional `or` alternatives), and a non-matching case takes the `otherwise` branch.
- Names bound by the pattern are in scope only in the matched statement, never in `otherwise` and never after the conditional.
- `or` alternatives follow the same same-names-same-types rule as selection arms.

This form is a conditional, not a selection: it is never required to be exhaustive. The checker MUST suggest `choose` when every case of a choice appears in a chain of `when … carries` conditionals.

## 5. Text literals

`text "…"` is a formatted text literal. Inside it, `{` and `}` delimit holes; each hole holds one expression. `{{` and `}}` spell literal braces inside a `text` literal only; plain string literals are unchanged and hole-free.

```
keep greeting set text "Hello {name}, you have {len(items)} tasks"
```

- A hole expression MUST be pure: no `perform`, no capability use, no mutation of outer bindings.
- A hole value MUST be a text value or adopt the `Display` protocol; its canonical `display` output is inserted. There is no format mini-language — width, precision, and padding use the named `std.text` functions, so formatting stays readable and greppable.
- Evaluation order is left to right, the result is an ordinary string, and construction allocates the result exactly once.
- The formatter owns hole spacing and MUST NOT reflow the literal's own text.

`text` before anything other than a string literal is an ordinary identifier.

## 5.1 Comments and doc comments

`//` starts a line comment. `/* … */` is a block comment ending at the first `*/`; block comments do not nest, and an unterminated block comment is an error. `///` starts a doc comment: consecutive `///` lines form one documentation block that documents the declaration immediately following it. Doc comments are part of the parse (so tooling sees them) but have no runtime effect; `niv doc` renders each block under its declaration's signature.

## 6. Loop control

`stop` ends the nearest enclosing `each` or `repeat` immediately. `skip` ends the current pass of the nearest enclosing loop and continues with the next pass. Both are statements, are rejected outside a loop body, and are rejected when the nearest loop boundary is crossed by a function, generator, `start` task, or `using` scope — a diagnostic names the boundary.

There are no loop labels, permanently. When a `stop` or `skip` would need to name an outer loop, the inner loop belongs in its own function with a typed result. `using` resources open in a loop body are released before `stop` or `skip` transfers control, preserving deterministic cleanup.

## 7. Promises

A `promise` declaration states what code will **not** do, and the compiler proves it. `needs` grants authority; `promise` renounces it. Together they make trust checkable in both directions.

- `promise never Capability` proves that the promising region and everything it reaches through the checked call graph — including dependencies, generated code, protocol dispatch, and code behind `start`, `through`, and `perform` — neither declares nor exercises the capability.
- `promise Capability only within "boundary", …` proves that every reachable use of the capability carries a scope contained in one of the listed boundaries. Containment follows the manifest grant rules: paths resist parent and symlink escapes, hosts match subdomain rules, and boundary strings obey the existing scope validation.
- A `promise` at module scope binds the whole module. A `promise` written as the first declaration of a function body binds that function. Multiple clauses compose with AND. Duplicate, contradictory (`never` plus `only within` for one capability), or unknown-capability clauses are rejected.
- Native and host boundaries count as their declared `needs` scopes; a dynamic library or host operation without a scope that satisfies the promise is a violation at compile time, not a runtime surprise.
- A violated promise fails the build. The diagnostic MUST show one complete offending call path from the promising region to the violating declaration, naming each function and the capability scope that broke the promise.
- Promises are enforced again at runtime, exactly as project grants are: a runtime effect that would violate an active promise is denied before it enters the authorized effect sequence. Source promises are never inferred from, or weakened by, manifest grants.
- Promises are public contract. They appear in `std.reflect.schema` output, in `niv explain`, in generated documentation, and in package authority locks. For published packages, adding or tightening a promise is a compatible change; removing or loosening one is a breaking change under the semantic compatibility rules.

## 8. Samples

A `sample` declaration is a checked, executable example: documentation that cannot lie.

```
sample "adding two points" {
    keep a set Point with { x set 1, y set 2 }
    keep b set Point with { x set 3, y set 4 }
    add with { left set a, right set b }
} shows "Point { x set 4, y set 6 }"
```

- The title string names the sample in documentation and test output; titles MUST be unique within a module and at most 120 bytes.
- A sample body is ordinary Edition 5 code with one restriction: it MUST NOT declare or exercise any capability. Samples are hermetic and deterministic; effectful demonstration belongs in example projects and recorded traces, not in samples.
- `niv test` discovers and executes every sample. A sample that fails to check, fails at runtime, or produces mismatched output fails the test run. `niv doc` renders the canonical formatted body — and the `shows` text when present — verbatim into generated documentation.
- When `shows` is present, the sample's final declaration MUST be an expression, and its value's canonical `display` output MUST equal the `shows` string exactly. Without `shows`, the sample passes by checking and running to completion.
- Samples are stripped from compiled applications, bundles, and packages' runtime output, but travel with packages as source so downstream documentation and `niv test` can re-verify them.

## 9. Compile-time generation

`generate` declares a generator: a compile-time callable that produces declarations. `expand` invokes one at module scope.

- A generator body is ordinary pure Edition 5 code. It runs during checking, before any runtime exists. It MUST NOT declare `needs`, call `perform`, observe time, randomness, or the environment, read files, or observe mutable runtime state. It receives only its labeled compile-time arguments, which MUST be self-contained literal data by the same rule that governs portable plans.
- A generator gives a `[source.Declaration]` value built with the typed `std.source` builder API from compiler facade v3. There is no quoting form, no token splicing, and no text pasting; generated code exists only as checked declaration values.
- `expand name with { ... }` checks the generator call, evaluates it, and inserts the produced declarations into the enclosing module as if written by hand. Generated declarations are hygienic: names they introduce MUST NOT capture or collide with surrounding bindings unless the generator was explicitly given the name as an argument. Collisions are rejected, never silently renamed.
- Generated declarations are ordinary source afterward. `niv expand` prints them, `niv doc` documents them, the formatter formats them, diagnostics point into them with their generator provenance, and the debugger steps through them. An implementation MUST be able to materialize every expansion as reviewable Edition 5 source.
- Expansion is deterministic and bounded: identical inputs give identical declarations, expansion depth and produced-declaration counts have fixed published limits, and `expand` MUST NOT trigger further `expand` beyond that depth.
- Generated declarations may include promises and samples; they are checked exactly as hand-written ones.

Generators replace every remaining reason to want macros. Nivren permanently has no syntax macros, no text preprocessors, and no unrestricted runtime code loading.

## 10. Numbers

Edition 5 completes the numeric tower and then closes it.

- Core numeric types are `Int` (checked signed 64-bit) and the new `UInt` (checked unsigned 64-bit), plus `Float` (binary64). `UInt` arithmetic is checked exactly as `Int` arithmetic is; wrap-around requires the explicit wrapping functions.
- The standard library's fixed-width family is completed with `I128` and `U128` alongside the existing 8/16/32/64-bit widths, and retains `BigInt` and `Decimal`.
- The sealed protocol `Number` now covers `Int`, `UInt`, `Float`, `Decimal`, `BigInt`, and every fixed-width numeric type. Generic numeric functions constrain on `Number` or on the sealed refinements `Integer` (all integer forms) and `Exact` (all non-float forms).
- Every conversion remains an explicit named function and every narrowing conversion remains typed-failure. There is no implicit widening, no implicit truncation, and no numeric literal defaulting across categories: an untyped integer literal is `Int` unless the expected type says otherwise at the literal itself.
- Operator overloading is permanently rejected. `+`, `-`, `*`, `/`, `%`, comparisons, and equality apply only to the types this section names. All other types use named methods; bounded text keeps `std.text.concat` alongside `text` literals.

## 11. User iteration

The sealed iterator surface opens exactly one seam: the `Iterate` protocol.

- A nominal type may adopt `Iterate<Item>` by providing the required member `advance`, which takes no arguments and gives `maybe Item`. `none` ends the sequence.
- `each x in value`, `through` stream stages, and the standard iterator adapters accept any adopter of `Iterate` in addition to arrays, maps, sets, strings, channels, and the standard sources.
- Iterators remain single-pass, non-transferable, and non-comparable. The published adapter-stage and materialization limits are unchanged and now frozen. There is no double-ended, reversible, or random-access iterator protocol, permanently.
- An `advance` implementation MUST be effect-free at the boundary: a source that performs effects (files, sockets, databases) is exposed through `using` resources and the standard bounded stream APIs, not through a bare `Iterate` adoption. The checker rejects `needs` on `advance`.

Map and set iteration order is insertion order, permanently. The Edition 3 carve-out for a future differently-ordered collection is closed: an ordered collection, if ever added, is a standard-library type, not a change to these semantics.

## 12. Reflection

`std.reflect` keeps exactly its three functions — `kind`, `fields`, `schema` — and completes them: `schema` now also describes generic declarations (with their parameters and constraints) and functions (parameters, labels, result, failure, declared capabilities with scopes, and active promises). Reflection permanently exposes no addresses, no layout, no lexical values, and no private state. No further reflection surface will be added in any edition.

## 13. Intent, recording, and replay

`prepare`, `perform`, `through`, plan portability, fused `PerformCall`, effect ordering, and `org.nivren.intent.v1` are unchanged from Edition 4 and are frozen by this edition. Pattern arms, guards, text-literal holes, and generator expansion introduce no new effect boundaries: all are pure, and generation completes before the intent graph exists. `stop` and `skip` are ordinary control flow inside the graph. `niv explain` output for a program that uses only frozen surface MUST remain byte-stable across compliant implementations at the same version of the intent schema.

### 13.1 Plain-language explanation

`niv explain --story` renders the same intent graph as deterministic plain-language sentences: what the program reads, writes, connects to, runs, and promises, in source effect order, with capability scopes spelled out. Story output is generated only from the validated graph, never from heuristics, and equal graphs MUST produce equal stories. The story is documentation surface, not a new analysis.

### 13.2 Effect trace and replay

Because every external effect crosses one visible boundary, Edition 5 guarantees record and replay as language behavior, not as a debugger trick.

- `niv record` executes a program normally while writing an `org.nivren.effects.v1` trace: one ordered entry per authorized effect, carrying the operation, capability and scope, a digest of the arguments, and the complete result. Scheduling decisions whose outcome a program can observe — `race` winners, `together` completion order, channel delivery order, and task cancellation observations — are recorded as trace entries too.
- `niv replay` re-executes the program against a trace with **no** capability grants: every `perform` boundary is satisfied from the recorded result instead of the outside world, and recorded scheduling decisions are reapplied. Given the same program, build, and trace, replay output MUST be byte-identical to the recorded run.
- Replay verifies as it goes: an effect whose operation, capability, scope, or argument digest differs from the next trace entry is a typed replay divergence error that names both sides. Replay MUST NOT skip, reorder, or synthesize entries.
- Traces contain real effect results and are sensitive artifacts; documentation MUST say so. `SecretKey` and other non-serializable values never enter a trace — an effect that would require one is recorded as unreplayable and `niv record` says so at record time, not at replay time.
- Trace entries are bounded by the same 16 MiB payload rules as the effects they record, and the format is versioned and frozen like the intent schema.

### 13.3 Plan mobility

A portable plan is already proven to be pure data. Edition 5 lets that data travel.

- `std.plans.encode` serializes a portable plan to the versioned `org.nivren.portable-plan.v1` bytes; `std.plans.decode` reconstructs one against an expected plan shape with strict validation, exactly as strict as `std.json.decode`. Only plans the compiler proved portable can be encoded; encoding anything else is a typed error, never a lossy fallback.
- A decoded plan carries **zero** authority. Performing it uses only the performing host's own grants, promises, and declared limits — never anything from the sender. A plan a receiving host is not authorized to perform is denied exactly as local code would be.
- A receiving program can inspect a decoded plan through reflection and `niv explain` before performing it, so accepting remote work is a policy decision over visible data.
- Transport is deliberately ordinary: plan bytes move over the existing channel, file, and network APIs under their normal capabilities. Mobility adds no new effect machinery and no hidden listener.
- Encoding is deterministic within a release: equal plans give equal bytes. Decode rejects unknown versions and mismatched shapes with typed errors.

Work requests become inspectable data, and the receiver's policy — not the sender's intent — decides what runs. This is the distributed consequence of the visible effect boundary.

## 14. Diagnostics

The intent-first diagnostic voice becomes contract. Every diagnostic the checker, runtime, or tooling emits MUST provide three parts, in order:

1. **Attempted** — what the program tried to do, in intent vocabulary, naming the construct.
2. **Found** — the relevant types, values, capabilities, or scopes actually present.
3. **Correction** — at least one concrete, applicable change, spelled as Edition 5 source whenever possible.

Diagnostics carry a stable identifier from a versioned public catalog (`org.nivren.diagnostic.v1`): identifiers are never reused or renumbered, new diagnostics append, and the catalog documents each identifier with a triggering example. Conformance vectors exercise the catalog: for every identifier, at least one vector MUST assert the three parts are present and the correction checks. Machine-readable diagnostic output is part of the frozen tooling surface alongside the intent schema.

## 15. Capabilities

The capability vocabulary is `FileRead`, `FileWrite`, `Environment`, `Time`, `Process`, `Network`, `Task`, `Channel`, `Log`, `Native`, `Random`, and the new `Gpu` for accelerator dispatch. This list is closed permanently. Scoped boundaries retain their Edition 4 rules; `Gpu` boundaries name adapters. New platform integrations MUST be expressed through these capabilities and packages, never through new capability names.

### 15.1 Authority diff

Dependency changes MUST NOT silently change authority. When `niv install`, `niv add`, or an update resolves a dependency version whose capabilities, scopes, or promises differ from the recorded authority lock, the command stops and presents the difference — capability by capability, scope by scope, promise by promise, per package — before anything is written. Acceptance is explicit and recorded in the authority lock; a non-interactive run fails instead of accepting. `niv authority report` reproduces the last accepted diff on demand.

### 15.2 Trusted modules

`trusted "reason"` as the first declaration marks a module as a systems escape hatch. The reason string is mandatory, at most 200 bytes, and states in plain language why raw power is needed; it appears in `niv authority report`, generated documentation, and the authority diff.

- Only trusted modules may call the escape-hatch families: `std.native`, `std.host`, and the unchecked systems APIs (raw memory views, SIMD, and OS threads) specified with the embedding ABI. Any such call from an untrusted module is rejected at compile time with a diagnostic naming this rule.
- `trusted` grants nothing. The module still declares and is checked for its `Native` and other capabilities with scopes, exactly like any module; project grants still gate it at runtime. Trusted is an additional static gate on top of capabilities, never a substitute for them.
- The exposed surface of a trusted module MUST be safe: exposed declarations cannot leak raw pointers, unchecked views, or foreign handles that outlive their scope, and its promises and samples are checked normally. Danger stays inside; the boundary presents an ordinary typed, capability-declared, promise-bearing API.
- `niv explain` and its story mode mark every effect that originates in a trusted module, so audits see exactly where the safe world ends.

This is the whole systems story: no unsafe blocks scattered through ordinary code, no undeclared escape. Power is quarantined per module, named, reasoned, and visible.

### 15.3 Declared limits

Every numeric bound the standard library publishes — payload bytes, materialized values, iterator stages, channel capacities, and the shared instruction and memory budgets — is a **default**, not a wall. The application manifest may declare different values for named limits.

- Limit names, their defaults, and their permitted ranges form a closed, published vocabulary frozen with this edition. Defaults never change after Freeze Proof, so unconfigured programs behave identically forever.
- Only the application's root manifest can declare limits. A dependency can never raise a limit for anyone, including itself; a package that needs more than the defaults documents that requirement and the application decides.
- Declared limits are visible policy: they appear in `niv explain`, story output, and `niv authority report`, and limit changes participate in the authority diff exactly as capability changes do.
- Bounded stays the identity: there is no "unlimited" value. Every declared limit is a concrete number the runtime enforces, and exceeding it remains the same typed error as today.

Nivren is therefore never the language that cannot — it is the language where big is stated, reviewed, and enforced.

## 16. Removals

Edition 5 deletes every retained compatibility surface. Each removal has an automatic rewrite in `niv fix`.

- Structural shape compatibility for unadorned Edition 3 shapes is removed. All shapes are nominal. Two shapes with identical fields are distinct types.
- The draft namespace aliases `std.fs`, `std.http`, `std.task`, and `std.channel` are removed in favor of `files`, `web`, `tasks`, and `channels`, exercising the removal clause of `spec/STANDARD-LIBRARY-3.md`.
- The `clock()` built-in and the Edition 2 `std.time.now()` compatibility clock are removed in favor of `std.time.now_zoned` for wall-clock instants and the new `std.time.monotonic()` for elapsed time, both under `Time`.
- The Edition 3 grammar is no longer advertised, checked, or formatted. Edition 3 and Edition 4 conformance vectors remain as historical black-box evidence against their retained specifications.
- No other builtin, keyword, or operator is removed, and none will be after the freeze.

## 17. Migration and evidence

- `niv fix` rewrites any checking Edition 4 project to Edition 5 source with no semantic change, using the formatter's canonical output. Round-tripping every Edition 4 proof program is a blocking gate. Mechanical rewrites include bare `repeat condition` to `repeat while condition` and iteration `within` to `in`; the remaining accepted rows of `spec/EDITION-5-FIX-LEDGER.md` add their own rewrites here as they are folded in.
- Edition 5 executes on Bytecode 8 (`spec/BYTECODE-8.md`, to be authored with the implementation): Bytecode 7 plus checked pattern-dispatch instructions, loop-exit branches, text-literal construction, and `UInt` arithmetic. Promises and samples are checked metadata and emit no new instructions; generators emit no bytecode because expansion completes before lowering; recording and replay reuse the existing `Perform`/`PerformCall` boundary.
- The proof program reuses the Edition 4 ledger shape with four gates: Language Proof, Intent Proof, Compiler Proof, and a new Freeze Proof. Freeze Proof passes only when the specification contains no draft or provisional markings, no compatibility aliases survive, `niv fix` round-trips are green, the conformance corpus covers every grammar production in section 3 and every diagnostic identifier in the catalog, record-then-replay is byte-identical for every effectful proof program, and a repository-wide check finds no open design question in `spec/` or `docs/`.
- The standard-library completion that accompanies this edition — the full `Number` family, `std.text` growth, generic and function `schema`, and `std.source` builders — is specified in `spec/STANDARD-LIBRARY-5.md`.

## 18. Compatibility

Edition 4 remains executable during the Edition 5 proof program so its regression suites protect the implementation foundation. Edition 5 does not promise source compatibility with Edition 4 beyond the `niv fix` rewriter, and the final Edition 5 release will not advertise the older grammar as canonical. After Freeze Proof passes, this specification drops the draft marking, becomes `spec/LANGUAGE-5.md`, and the finality clause in section 1 takes permanent effect.
