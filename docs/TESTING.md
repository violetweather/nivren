# Testing Nivren programs

`niv test [path]` discovers bytecode-verified `*_test.niv` files in stable path order. A test passes when it compiles, satisfies its project capability/resource policy, and finishes without a failed `assert` or runtime error.

## Value snapshots

`niv test --snapshots [path]` compares each test's final value with the adjacent `<name>_test.niv.snap` UTF-8 file. Missing or changed snapshots fail with the expected and actual values.

After reviewing an intentional change, run `niv test --accept-snapshots [path]`. Acceptance is explicit: ordinary test and build commands never rewrite snapshots. Snapshot files are written atomically and should be committed alongside their tests.

Snapshots are for stable values, not timing, task-race winners, secrets, host handles, or environment-dependent output. Use assertions, bounded loopback integration tests, property tests, and the Rust fuzz targets for those behaviors.

## Test profiles and time

`niv test --property [path]`, `niv test --compat [path]`, and `niv test --fuzz-smoke [path]` give those suites stable first-class CI names while retaining the ordinary bounded Nivren test runner. With no path they select `tests/property`, `tests/compat`, and `tests/fuzz`. Fuzz-smoke is for deterministic checked regression cases discovered by a fuzzer; long-running byte mutation remains in the repository fuzz workflow.

`niv test --time <unix-seconds> [path]` fixes `clock` and `std.time.now_zoned` for the complete test process. The value must be finite and nonnegative, Time authority is still required, and the scoped override is restored before the command exits. Use it for expiry, retry, token, and schedule tests; do not use sleeps or wall-clock tolerances to simulate time.

## Production matrix

Before 1.0, the release gate runs unit, language, integration, conformance, snapshot, property, fuzz-smoke, benchmark, registry, installer, platform, and deployment tests. Passing locally does not substitute for the clean-runner and independent-pilot evidence recorded in `docs/RELEASE_AUDIT.md`.
