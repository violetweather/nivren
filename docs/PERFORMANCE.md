# Nivren performance policy

Nivren uses tiered execution. Every bundle starts in the verified bytecode VM. Integer-only functions containing supported straight-line arithmetic become hot after 64 calls by default and are compiled to native code with Cranelift. Eligible integer bytecode functions use compact slot-based call frames before they tier, avoiding a synchronized heap scope for every call. Unsupported functions stay in the general VM without changing behavior.

The native tier preserves checked `Int` semantics. Addition, subtraction, multiplication, and negation emit explicit signed-overflow detection and return a normal source-located Nivren error. Finalized functions are published once and called without a per-execution JIT lock. Up to eight integer arguments use inline temporary storage; larger arities retain the allocation-backed fallback. Executable-memory ownership and the finalized function-pointer call are confined to `crates/nivren-jit`; the main compiler and runtime continue to forbid unsafe Rust.

Set `NIVREN_JIT_THRESHOLD` to a positive call count when tuning workloads. `Interpreter::set_jit_threshold` and `jit_stats` support embedding and tests.

## Published benchmark

`cargo bench --bench performance` runs a 200,000-iteration arithmetic workload through both the bytecode-only and tiered runtimes. It also compares recursive Fibonacci execution in the tree interpreter and bytecode VM so call-frame regressions remain visible. Both cases report medians and relative speedups over seven measured runs after warmup. Setting `NIVREN_BENCH_GATE=1` requires at least a 1.05x native-tier speedup and a 1.10x recursive bytecode speedup.

The CI release gate runs on the pinned Rust 1.88 toolchain. The initial Apple Silicon result was:

```text
nivren_benchmark_vm_ms 129.642
nivren_benchmark_tiered_ms 62.187
nivren_benchmark_jit_speedup 2.085
```

These numbers are a historical baseline, not a promise for every machine. CI enforces the relative speedup because it is less sensitive to shared-runner hardware than an absolute time limit.

## Call-path optimization result

On July 27, 2026, the public Apple M4 benchmark harness was rerun with eleven measured fresh processes after the call-path changes. Against the published `0.10.0-beta.6` baseline, the two targeted workloads changed as follows:

| Workload | Published beta | Optimized runtime | Improvement |
| --- | ---: | ---: | ---: |
| Two-million-call integer kernel | 788.816 ms | 682.038 ms | 13.5% |
| Recursive Fibonacci(30) | 1570.085 ms | 702.500 ms | 55.3% |
| Top-level nested integer loops | 118.211 ms | 73.573 ms | 37.8% |

Source-to-result startup remained below 3 ms. The raw comparison used the same paired programs, alternating runtime order, warmups, output checks, and median statistic as the public site benchmark.

## Managed environments

Closure environments use two generations. Frequent minor collections reclaim unreachable young scopes and promote survivors after two marks. Every eighth collection starts old-generation marking on a background worker while the bytecode mutator continues. Before sweeping, the runtime performs a final root remark and unions it with the concurrent snapshot, so values made reachable during marking cannot be reclaimed. `HeapStats` exposes minor/major counts and an in-progress concurrent-mark flag; GC stress tests exercise escaping closures and cyclic environments.
