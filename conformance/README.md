# Nivren Edition 2 conformance suite

`edition2-baseline.json` is the frozen, implementation-independent black-box baseline. Each case supplies UTF-8 source, command arguments, expected process status, and optional exact stdout / required stderr substring. `{source}` is replaced with the path to a temporary `.niv` file.

The release policy pins the baseline's SHA-256. Do not edit or remove baseline vectors during Edition 2; add new non-breaking coverage in a separate corpus when needed. A deliberate defect correction requires the documented compatibility process and a new policy review.

A candidate implementation passes this layer only when its command-line executable satisfies every vector without linking to or importing the reference implementation. The repository's `tests/conformance.rs` runner invokes the built `niv` process. Other implementations may reuse the JSON from any language.

Passing these vectors is necessary, not sufficient: implementations must also satisfy the normative specification, hostile bytecode/package vectors, standard-library contract tests, and platform gates.
