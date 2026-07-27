# Testing Nivren programs

`niv test [path]` discovers bytecode-verified `*_test.niv` files in stable path order. A test passes when it compiles, satisfies its project capability/resource policy, and finishes without a failed `assert` or runtime error.

## Value snapshots

`niv test --snapshots [path]` compares each test's final value with the adjacent `<name>_test.niv.snap` UTF-8 file. Missing or changed snapshots fail with the expected and actual values.

After reviewing an intentional change, run `niv test --accept-snapshots [path]`. Acceptance is explicit: ordinary test and build commands never rewrite snapshots. Snapshot files are written atomically and should be committed alongside their tests.

Snapshots are for stable values, not timing, task-race winners, secrets, host handles, or environment-dependent output. Use assertions, bounded loopback integration tests, property tests, and the Rust fuzz targets for those behaviors.

## Production matrix

Before 1.0, the release gate runs unit, language, integration, conformance, snapshot, property, fuzz-smoke, benchmark, registry, installer, platform, and deployment tests. Passing locally does not substitute for the clean-runner and independent-pilot evidence recorded in `docs/RELEASE_AUDIT.md`.
