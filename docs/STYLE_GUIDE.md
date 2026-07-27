# The Nivren style

Nivren code should read like a small, precise account of intent. The formatter owns whitespace; authors choose names, boundaries, effects, and failure paths.

## Use Nivren's vocabulary

- Bind stable facts with `keep`; use `change` only for state that truly changes.
- Start behavior with `define` and finish it with `give`.
- Prefer `when`/`otherwise`, `each`/`within`, `shape`, `choice`, and `choose` over clever Boolean or indexing tricks.
- Declare externally visible work with `needs`. Keep the list narrow enough that a reviewer can understand the function's authority at a glance.
- Use `or give` when the current function cannot add useful context. Use `choose` when recovery, translation, logging, or a fallback is meaningful.
- Use `through` for a readable left-to-right transformation. Stop the pipeline when named intermediate values make intent clearer.
- Own closeable values with `using`. Do not manually close a resource whose lifetime is lexical.
- Keep concurrency scoped: `start` work, then `wait`, `together`, or `race` before leaving its owner.

## Shape a program around authority

Put pure transformations near the center and effects at the edge:

```nivren
define normalize(name: String) gives String {
    give "Hello, " + name
}

define load_name(path: String) gives Result<String, String> needs FileRead {
    give std.files.read(path)
}

define main() gives Result<Null, String> needs FileRead, Log {
    keep name = load_name("name.txt") or give
    std.log.info(normalize(name))
    give ok(none)
}
```

The `needs` list is part of the public contract, not bookkeeping. Project grants should normally be scoped, such as `FileRead = "path:./data"` or `Network = "host:api.example.com"`.

## Keep failures typed and local

Return `Result<Success, Problem>` for expected failure. Error strings should say what was attempted and retain actionable context. Avoid sentinel values such as an empty string or `-1` when the operation can fail.

```nivren
define title(document: Map<String, String>) gives Result<String, String> {
    keep value = std.map.get(document, "title")
    when value == none {
        give err("document has no title")
    }
    give ok(value ?? "")
}
```

## Prefer stable namespaces

Everyday code should begin with `std.files`, `std.web`, `std.json`, `std.time`, `std.tasks`, `std.channels`, `std.locks`, `std.process`, and `std.log`. Use `DateTime` with an explicit IANA zone for application time. These plural intent namespaces are final for 1.x. Compatibility aliases remain behavior-identical for the full 1.x line, but tooling and new Edition 3 examples always prefer the canonical form.

## Punctuation budget

Use parentheses when they clarify precedence, not as ceremony. Do not compress multiple conceptual steps onto one line. Semicolons are optional and should not be added mechanically. A short block with meaningful words is more idiomatic than a dense expression with repeated operators.

## Public APIs

- Expose the smallest useful surface with `expose { ... }`; everything else remains private.
- Annotate public parameters and return values.
- Use generic constraints that communicate behavior (`Value: Number`) rather than implementation details.
- Keep packages capability-light. A data-format package should not unexpectedly need network or process authority.
- Run `niv fmt`, `niv check`, `niv test`, and `niv doc` before `niv ship`.

This guide is conformance-tested through the Edition 3 style corpus. New syntax earns a place only when it makes representative programs more readable without weakening diagnostics, tooling, or predictable execution.
