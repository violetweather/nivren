# Nivren Bytecode 2

`niv sourcemap <source|project|bundle> <output.json>` exports the stable `org.nivren.sourcemap.v1` schema. It records bytecode version, source identity, and every top-level or nested instruction path with its one-based source line/column and stable operation name. Debuggers, crash processors, coverage viewers, and deployment systems can consume this JSON without linking compiler internals.

Edition 4 Language Proof compiles checked source into portable stack bytecode version 6. The normative format is `spec/BYTECODE-6.md`; earlier versions remain documented as pre-freeze history. Version 6 retains canonical shape schemas, payload-choice metadata, and coherent protocol dispatch while preserving checked derive metadata for identical generated behavior in both engines.

A `.nivb` bundle begins with ASCII `NIVB`, followed by a bounded little-endian chunk. Every nested function, match arm, iterator, module, and `using` body is its own recursively verified chunk. The decoder rejects unsupported versions, unknown tags, invalid UTF-8, oversized counts, excessive nesting, truncation, and trailing data.

The verifier checks operands, jump targets, stack depth, lexical-scope depth, control-flow joins, nested chunks, and exit values before execution. Bytecode remains subject to runtime type checks, capability grants, instruction budgets, call-depth limits, deterministic cleanup, and I/O bounds.

Use:

```text
niv build .
niv check target/app.nivb
niv disasm target/app.nivb
niv run target/app.nivb
```

Bundles contain no host pointers or object layouts. VM, tree interpreter, bundle execution, and native tiers must agree on observable behavior.
