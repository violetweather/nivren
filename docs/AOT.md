# Native ahead-of-time objects

`niv build --aot <project>` emits native object files under `target/aot` for top-level functions whose parameters and result are explicitly `Int`, which declare no capabilities or type parameters, and whose checked body uses the supported integer operations. Ineligible functions are omitted; the command fails when no function qualifies.

Each export is named `nivren_<function>` and uses this stable C-callable shape:

```c
int64_t nivren_double(const int64_t *arguments, uint8_t *overflow);
```

Arguments are ordered exactly as declared. Checked overflow returns zero and writes `1` to `overflow`; success leaves it unchanged. Export names are restricted to ASCII letters, digits, and underscore. Object generation is deterministic for an identical compiler, target, and source.

```text
niv build --aot examples/aot_project
clang examples/aot_project/host.c examples/aot_project/target/aot/double.o -o target/aot-example
./target/aot-example
```

This is the first native AOT tier, not a claim that the entire language is lowered yet. General Nivren applications continue to use verified bytecode, tiered JIT execution, or standalone executables. Expanding AOT coverage must preserve checked arithmetic, capabilities, cleanup, resource budgets, and diagnostics rather than silently changing semantics.
