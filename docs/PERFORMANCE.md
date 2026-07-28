# Nivren performance policy

Nivren uses tiered execution. Every bundle starts in the verified bytecode VM. Eligible bytecode functions use compact slot-based call frames for strings, records, arrays, results, and numeric values, avoiding a synchronized heap scope for every call. The compiler resolves lexical loads, stores, and definitions to instruction-local slot indices while preserving nested shadowing and closure behavior. Integer operations take a specialized checked path, and shape values share an indexed field layout instead of scanning field names. Functions with constructs that require ordinary captured environments stay on the general environment path without changing behavior.

Integer-only functions containing supported straight-line arithmetic become hot after 64 calls by default and are compiled to native code with Cranelift. The native tier is an additional optimization above the general slot-based VM, not a separate language mode.

The native tier preserves checked `Int` semantics. Addition, subtraction, multiplication, and negation emit explicit signed-overflow detection and return a normal source-located Nivren error. Finalized functions are published once and called without a per-execution JIT lock. Up to eight integer arguments use inline temporary storage; larger arities retain the allocation-backed fallback. Executable-memory ownership and the finalized function-pointer call are confined to `crates/nivren-jit`; the main compiler and runtime continue to forbid unsafe Rust.

Set `NIVREN_JIT_THRESHOLD` to a positive call count when tuning workloads. `Interpreter::set_jit_threshold` and `jit_stats` support embedding and tests.

## Published benchmark

`cargo bench --bench performance` runs a 200,000-iteration arithmetic workload through both the bytecode-only and tiered runtimes. It also compares recursive Fibonacci execution in the tree interpreter and bytecode VM so call-frame regressions remain visible, and reports a repeated eight-field shape workload to cover general slots and indexed properties. Every case reports a median over seven measured runs after warmup. Setting `NIVREN_BENCH_GATE=1` requires at least a 1.05x native-tier speedup and a 1.10x recursive bytecode speedup.

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

## General VM specialization result

The Phase 2 run expanded compact lexical slots to non-integer functions, resolved slot indices before execution, specialized checked integer dispatch, and indexed shape fields. Using eleven fresh processes on the same Apple M4 harness, startup measured 2.927 ms while recursive Fibonacci improved from the Phase 1 result of 702.500 ms to 658.734 ms and nested loops improved from 73.573 ms to 53.351 ms. The long tiered arithmetic case remained JIT-dominated at 666.458 ms.

The public harness now leads with representative short-lived workflows instead of treating hot-loop microbenchmarks as the entire product story:

| Workflow | Nivren | Node.js | Nivren lead | Nivren / Node peak memory |
| --- | ---: | ---: | ---: | ---: |
| Source-to-result startup | 2.927 ms | 23.835 ms | 8.14x | 9.0 / 42.8 MiB |
| One-shot source check | 2.777 ms | 21.644 ms | 7.79x | 8.8 / 41.1 MiB |
| Typed JSON file pipeline | 3.519 ms | 24.903 ms | 7.08x | 9.6 / 44.5 MiB |
| Text file pipeline | 3.500 ms | 24.862 ms | 7.10x | 9.5 / 44.1 MiB |

The checker row is explicitly unlike work: Nivren performs semantic, type, and capability checks while `node --check` checks JavaScript syntax. In the typed JSON pair, both implementations read, validate, canonicalize, and print the same document; Nivren additionally enforces the declared shape and `FileRead` grant. The compute-heavy rows remain in the report as current limits.

## Edition 4 Intent Proof gate

`NIVREN_INTENT_BENCH_GATE=1 cargo bench --bench intent_proof` compares direct and optimized intent-oriented forms over seven measured runs after warmup. It fails when latency/throughput or conservative runtime allocation work regresses by more than 10% for files, loopback HTTP, managed database transactions, or bounded channels. The benchmark validates identical results and uses the same runtime implementation on both sides; `perform` calls use Bytecode 7's fused `PerformCall` so inspection does not add a second dispatch.

The passing Apple M4 gate on July 28, 2026 was:

| Workload | Direct | Optimized intent | Time ratio | Allocation-work ratio |
| --- | ---: | ---: | ---: | ---: |
| Files | 1.403 ms | 1.212 ms | 0.864 | 1.000 |
| Loopback HTTP | 0.894 ms | 0.765 ms | 0.856 | 1.000 |
| Managed database transactions | 0.772 ms | 0.751 ms | 0.973 | 1.000 |
| Bounded channel concurrency | 0.222 ms | 0.220 ms | 0.989 | 1.000 |

An earlier gate correctly stopped when separate `Perform` instructions made the channel workload 23.8% slower. Fusing the visible boundary with the call corrected the design; the 10% requirement was not weakened. Absolute times vary by machine, so the enforced values are paired ratios.

## Managed environments

Closure environments use two generations. Frequent minor collections reclaim unreachable young scopes and promote survivors after two marks. Every eighth collection starts old-generation marking on a background worker while the bytecode mutator continues. Before sweeping, the runtime performs a final root remark and unions it with the concurrent snapshot, so values made reachable during marking cannot be reclaimed. `HeapStats` exposes minor/major counts and an in-progress concurrent-mark flag; GC stress tests exercise escaping closures and cyclic environments.
