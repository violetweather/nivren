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
