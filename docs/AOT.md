# Native ahead-of-time objects

`niv build --aot <project>` emits a complete-program native control object, its verified bytecode payload, deterministic ABI metadata, and a C/C++ header under `target/aot`. Every verified Edition 4 bytecode construct is accepted by the complete-program trace. Native compilation failure stops the build; it never silently emits only a supported subset or redirects execution to the VM.

The generated `nivren_program` export drives native control flow and calls the stable trace callback once for each value operation. A non-negative callback result selects the next verified instruction; `-1` is normal completion, `-2` is a function return, and `-3` is a checked runtime failure. The paired `program.nivb` supplies typed constants and operations to the embedding runtime. `program.json` binds the object, payload, symbol, instruction count, and `nivren-trace-v1` ABI together.

Pure, explicitly typed integer functions are additionally emitted as optimized kernel exports named `nivren_<function>` with this stable C-callable shape:

```c
int64_t nivren_double(const int64_t *arguments, uint8_t *overflow);
```

Arguments are ordered exactly as declared. Checked overflow returns zero and writes `1` to `overflow`; success leaves it unchanged. Export names are restricted to ASCII letters, digits, and underscore. Object generation is deterministic for an identical compiler, target, and source.

```text
niv build --aot examples/aot_project
clang examples/aot_project/host.c examples/aot_project/target/aot/double.o -o target/aot-example
./target/aot-example
```

The complete-program object retains Nivren's checked runtime ABI for strings, collections, closures, capabilities, resources, cancellation, managed memory, and typed failures. The kernel exports are optional optimizations; absence of a kernel never means absence of complete-program coverage.

When the whole top-level program lowers to native integer code — every top-level function and the root chunk use only `Int` values, integer shapes, and planned calls — the build additionally emits `program_native.o` (`.obj` on Windows) with one export:

```c
int64_t nivren_program_native(const int64_t *slots, uint8_t *fault);
```

`slots` supplies the program's slot buffer (all zeros for a fresh run) and receives every slot's final value back on success. A nonzero `fault` byte reports checked failure: `1` overflow, `2` division by zero, `3` remainder by zero, `4` call-depth exhaustion. Planned functions are module-local and call one another directly in native code — the runtime callback is never used. The `aot native-program` line reports `1` when this object was emitted and `0` when the program is not integer-plannable; `0` never means loss of complete-program coverage.
