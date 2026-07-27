# Nivren performance policy

Nivren uses tiered execution. Every bundle starts in the verified bytecode VM. Integer-only functions containing supported straight-line arithmetic become hot after 64 calls by default and are compiled to native code with Cranelift. Unsupported functions stay in the VM without changing behavior.

The native tier preserves checked `Int` semantics. Addition, subtraction, multiplication, and negation emit explicit signed-overflow detection and return a normal source-located Nivren error. Executable-memory ownership and the finalized function-pointer call are confined to `crates/nivren-jit`; the main compiler and runtime continue to forbid unsafe Rust.

Set `NIVREN_JIT_THRESHOLD` to a positive call count when tuning workloads. `Interpreter::set_jit_threshold` and `jit_stats` support embedding and tests.

## Published benchmark

`cargo bench --bench performance` runs a 200,000-iteration arithmetic workload through both the bytecode-only and tiered runtimes. It reports median VM time, tiered time, and speedup over seven measured runs after warmup. Setting `NIVREN_BENCH_GATE=1` requires at least a 1.05x speedup.

The CI release gate runs on the pinned Rust 1.88 toolchain. The initial Apple Silicon result was:

```text
nivren_benchmark_vm_ms 129.642
nivren_benchmark_tiered_ms 62.187
nivren_benchmark_jit_speedup 2.085
```

These numbers are a historical baseline, not a promise for every machine. CI enforces the relative speedup because it is less sensitive to shared-runner hardware than an absolute time limit.

## Managed environments

Closure environments use two generations. Frequent minor collections reclaim unreachable young scopes and promote survivors after two marks. Every eighth collection starts old-generation marking on a background worker while the bytecode mutator continues. Before sweeping, the runtime performs a final root remark and unions it with the concurrent snapshot, so values made reachable during marking cannot be reclaimed. `HeapStats` exposes minor/major counts and an in-progress concurrent-mark flag; GC stress tests exercise escaping closures and cyclic environments.
