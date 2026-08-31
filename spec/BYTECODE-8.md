# Nivren Bytecode 8 (design draft)

Bytecode 8 is the Edition 5 development format. It extends Bytecode 7 with structured loop control:

- `Repeat { condition, body }` executes a `repeat while` loop. Both parts are nested chunks. Each pass runs the condition chunk, requires a boolean, and runs the body chunk while the condition holds. The instruction consumes nothing and pushes the final body value (or `none` when the body never ran), so its stack effect is `+1`. Replacing the Bytecode 7 inline-jump lowering with nested chunks lets loop exits unwind scopes safely instead of jumping across `EnterScope`/`ExitScope` pairs.
- `LoopExit { skip }` implements `stop` (`skip = false`) and `skip` (`skip = true`). It terminates its chunk like `Return`, has stack effect zero, and is valid **only** inside a loop body chunk: the direct body of `Repeat` or `Iterate`. The verifier rejects it anywhere else, including `Repeat` condition chunks, function bodies, `using` bodies, module bodies, and `choose` arms. The `stop` signal ends the nearest loop with its last completed pass value; the `skip` signal begins the loop's next pass.

- `IfCarries { binding, then, else? }` executes `when subject carries binding`. It consumes the subject value; a present value binds the name immutably and runs the then chunk, while `none` runs the else chunk (or pushes `none` when absent). Stack effect is zero. Both chunks are **transparent** to loop-exit signals: the verifier passes the enclosing loop-body context through them, and a `stop`/`skip` inside a branch ends or advances the enclosing loop.

- `Match` arms carry full Edition 5 patterns: one or more `or`-joined pattern alternatives (any, literal, name, forced binding, `carries` case tests with nested payload patterns, and `holds` shape patterns with named field patterns), an optional pure guard chunk evaluated with the arm's bindings, and the body chunk. Arms select top to bottom; a failed guard moves selection to the next arm. The arm encoding is: pattern count, patterns (tags 0–5: any, literal, name, binding, carries, shape — each with its span, then its payload), one guard-presence byte with an optional guard chunk, the arm span, and the body chunk.

- `MakeText(n)` implements the `text "…"` literal. It pops the top `n` piece values, renders each to its canonical text form (text as-is, whole numbers and booleans canonically, finite floats canonically; anything else is a typed error), joins them in order, and pushes one string. The joined result is bounded at 16 MiB. Stack effect is `1 - n`, identical in shape to `MakeArray(n)`.

The encoder tags `Repeat` as 31 (condition chunk, then body chunk), `LoopExit` as 32 (one flag byte), `IfCarries` as 33 (binding string, then chunk, one presence byte, optional else chunk), and `MakeText` as 34 (one count). The verifier treats `LoopExit` as a terminal instruction in control-flow joins, exactly like `Return`, and validates both nested `Repeat` chunks with the loop-body context flag set only for the body.

The `each` lowering (`Iterate`) is unchanged apart from consuming the two loop-exit signals. Runtime engines — the tree interpreter, the bytecode VM, and native trace execution — must agree on loop-exit behavior; a signal that reaches a scope boundary is a runtime defect, because the checker rejects `stop`/`skip` that would cross a function, task, or `using` boundary before code generation.

Bytecode 7 and Bytecode 8 are deliberately not interchangeable. All instructions and safety rules not changed here remain as specified by `BYTECODE-7.md`.
