# The source kernel

`src/source_kernel.rs` is the types and the detector registry. `walk.rs` is the
workspace walk that fills them, `aliases.rs` the per-unit import map,
`detectors.rs` the two native detectors and `references.rs` the crate names the
dependency rules judge a manifest against. `src/source_text.rs` sits outside the
module on purpose: span arithmetic and the two syntax-text helpers are read by
`repo_hygiene` and `cargo_health`, which parse no Rust at all and have no
business depending on the walk for a span type.

Four rules hold it together, and each of them replaced something that had a
cost.

One walk, one enumeration. `enumerate` loads and parses each reachable file
once, and every source-reading producer works off that single `Enumeration`.
That is why `references::collect` and `references::mentioned` take the
enumeration rather than a path: nothing here reads the disk twice.

One answer to unanimity. A file several targets reach publishes only what all of
them agree on: its package, its target, its non-production context, the crate
aliases in scope, the identity a candidate two units emit is merged under. Six
places used to answer that question with six different pieces of code.
`unanimous` is the one answer, and disagreement abstains rather than
arbitrating, because a finding naming the wrong package is worse than a finding
naming none.

One typed outcome per load. `Loaded` says whether a file was skipped or whether
the global byte budget is gone, since only the second ends the walk. That
difference used to be recovered by searching the rendered error message for the
substring `total-bytes`, so renaming a display string moved the control flow.
`Limit` now names each bound once, and the budget charges what left the disk
rather than what survived decoding: a file that is not UTF-8 was still read, and
a walk that did not charge it could read the per-file limit repeatedly with the
total never moving.

One test-code policy, named. `SourceUnit::is_test_code` asks Cargo's target kind
first, which covers `tests/`, `benches/` and `examples/` wherever a manifest
configures them to sit, and falls back to the path convention only for a module
file no target names on its own. A detector that stays quiet in test code calls
`in_test_code`; `disabled_tls` does, because verification is routinely disabled
against a self-signed test server, and `dynamic_shell` deliberately does not,
because an interpolated shell command is a finding wherever it runs. The reason
is written above each rather than left to be inferred from an asymmetry.

`the_kernel_holds_the_size_bound_it_enumerates_for` keeps every file of the
module, and `source_text.rs` with it, under the 1000 lines `oversized_unit`
reports, tests included. That is why the walk and the tests have files of their
own: the kernel that feeds the rule has to pass it.

