# Nivren Bytecode 8 (design draft)

Bytecode 8 is the Edition 5 development format. It extends Bytecode 7 with structured loop control:

- `Repeat { condition, body }` executes a `repeat while` loop. Both parts are nested chunks. Each pass runs the condition chunk, requires a boolean, and runs the body chunk while the condition holds. The instruction consumes nothing and pushes the final body value (or `none` when the body never ran), so its stack effect is `+1`. Replacing the Bytecode 7 inline-jump lowering with nested chunks lets loop exits unwind scopes safely instead of jumping across `EnterScope`/`ExitScope` pairs.
- `LoopExit { skip }` implements `stop` (`skip = false`) and `skip` (`skip = true`). It terminates its chunk like `Return`, has stack effect zero, and is valid **only** inside a loop body chunk: the direct body of `Repeat` or `Iterate`. The verifier rejects it anywhere else, including `Repeat` condition chunks, function bodies, `using` bodies, module bodies, and `choose` arms. The `stop` signal ends the nearest loop with its last completed pass value; the `skip` signal begins the loop's next pass.

The encoder tags `Repeat` as 31 (condition chunk, then body chunk) and `LoopExit` as 32 (one flag byte). The verifier treats `LoopExit` as a terminal instruction in control-flow joins, exactly like `Return`, and validates both nested `Repeat` chunks with the loop-body context flag set only for the body.

The `each` lowering (`Iterate`) is unchanged apart from consuming the two loop-exit signals. Runtime engines — the tree interpreter, the bytecode VM, and native trace execution — must agree on loop-exit behavior; a signal that reaches a scope boundary is a runtime defect, because the checker rejects `stop`/`skip` that would cross a function, task, or `using` boundary before code generation.

Bytecode 7 and Bytecode 8 are deliberately not interchangeable. All instructions and safety rules not changed here remain as specified by `BYTECODE-7.md`.
