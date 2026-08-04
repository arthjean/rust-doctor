# Changelog

## Unreleased

### Pinned evaluation corpus and precision gate

- Ten Rust repositories are pinned by full commit in `tasks/rust-doctor-score-credibility-corpus.json`, each with the reason it is in the corpus: a binary, a library, a ten-member workspace, a procedural macro crate and two asynchronous projects. No corpus code is committed. The harness reads a local cache named by `RUST_DOCTOR_CORPUS_DIR`, refuses to run while one revision is missing rather than measuring a partial corpus, and writes only under `RUST_DOCTOR_CORPUS_ARTIFACTS`: each revision is materialised through a temporary index, so the cache stays read-only.
- The Clippy leg of a corpus scan is a declared trusted boundary, since Cargo runs build scripts and procedural macros. The native detectors are not: replaying the corpus with every Clippy rule switched off proves no corpus build script runs at all.
- First measurement, on 1855 findings: 16 rules fired, all of them at a measured false-positive rate of 0 %, and 24 of the 40 rules active by default never fired. A corpus of healthy repositories measures precision, never recall: `dtolnay`, `BurntSushi` and `tokio-rs` do not commit the defects those 24 rules target, so silence there proves nothing either way. The catalog is unchanged, but the 24 are now named one by one in a frozen admission debt, and that is where the threshold becomes opposable: the suite fails the moment a rule joins the default catalog without measured precision, the moment a rule measured above 5 % stays active by default, and the moment a debt entry no longer matches an active rule. The debt only shrinks. Proving those 24 needs pinned adversarial fixtures, which is a separate slice.
- Two of the ten scans are published as incomplete, and five of the ten scores as non-authoritative. `--all-targets` compiles the benches of `bytes` and `ripgrep`, which need `#![feature]` on nightly, so Cargo reports a failed build after Clippy has already linted the crates themselves. Each observation carries its own status, exit code and authority rather than hiding behind the ten-repository count.
- Every finding carries a verdict and a justification. The trigger of each one is re-verified inside its reported span at the pinned revision on every corpus run, and the two contested sites are recorded as individually adjudicated exceptions.
- The corpus score distribution is published: ten healthy codebases land between 90 and 94, with no tier ceiling applied. The artifact states both halves of the answer plainly, `collapsed_into_one_band` and `collapsed_into_one_value`: every repository does fall in the single `Great` band, while the values keep a five-point spread. Separating bands is what the adversarial reference fixture, capped at 40, is there to prove.

### Wider catalog and live score dimensions

- The catalog grows from 12 to 40 contracted rules, 33 of them Clippy lints against the 8 exploited before. Rule identifiers, severities, help text and delta fingerprints of the existing rules are unchanged, so no recorded baseline is invalidated. Every scanned workspace will see new true positives.
- `CATEGORIES` accepts `performance` and `dependencies`, the two categories `audit::category_mapping` already routed to their score dimension. The five dimensions each carry at least three rules now, so none stays frozen at 100 and no weight of the scale is inert.
- Panic and placeholder pack: `clippy::exit`, `clippy::expect_used`, `clippy::indexing_slicing`, `clippy::panic`, `clippy::panic_in_result_fn`, `clippy::print_stderr`, `clippy::print_stdout`, `clippy::string_slice`, `clippy::unreachable` and `clippy::unwrap_used`. Whether one of them fires inside a Cargo test target is governed by the scanned workspace's own `clippy.toml`, which Clippy looks up by walking the parent directories: `allow-unwrap-in-tests` and `allow-expect-in-tests` silence the first two there, `allow-panic-in-tests` and `allow-print-in-tests` do the same for the other two, and each defaults to `false`. The pack fixture carries its own configuration and its oracle names the option behind every verdict.
- Performance pack, all in category `performance`: `clippy::format_collect`, `clippy::large_types_passed_by_value`, `clippy::manual_memcpy`, `clippy::rc_buffer`, `clippy::redundant_allocation`, `clippy::stable_sort_primitive`, `clippy::unnecessary_to_owned`, `clippy::useless_vec` and `clippy::vec_init_then_push`. No admitted lint reads optimized MIR, so no verdict depends on the compilation profile.
- Concurrency and asynchronous pack: `clippy::arc_with_non_send_sync`, `clippy::await_holding_lock`, `clippy::await_holding_refcell_ref`, `clippy::mut_mutex_lock`, `clippy::rc_mutex` and `clippy::unused_async`. A workspace without an asynchronous runtime stays silent, and the self-scan produces none of them.
- Local dependency health pack, offline and in category `dependencies`: `rust_doctor::cargo::duplicate_major_versions` names a crate resolved with incompatible major versions, `rust_doctor::cargo::missing_lockfile` names a package that ships a binary while no `Cargo.lock` sits next to the workspace manifest, and `rust_doctor::cargo::path_dependency_outside_workspace` names a path dependency that only resolves on the author's machine. The resolved graph is read from `Cargo.lock` before Clippy runs, since `cargo clippy` creates or rewrites that file. A lockfile without a resolved package section makes the pack abstain with a bounded `lockfile-resolution-absent` error under the new `dependencies` error stage, and the scan stays usable.
- Four Clippy lints on signatures, `large_types_passed_by_value`, `rc_buffer`, `redundant_allocation` and `rc_mutex`, only fire on non-exported items: Clippy refuses to propose a signature change on a public API.
- A candidate denied by default is refused: switching such a rule off would drop its `-W` flag and restore Clippy's refusal, turning the scan into a build failure. `clippy::async_yields_async` and `clippy::unused_io_amount` were rejected on that ground, `clippy::let_underscore_lock` because it was uplifted into rustc, `clippy::redundant_clone` because its verdict depends on the optimization level, and `clippy::unwrap_in_result` and `clippy::inefficient_to_string` because neither is observable on the normative toolchain. Every rejection is traced in `tasks/rust-doctor-score-credibility-kernel-evaluation.json`.

### Generic native source detection

- Native detectors no longer compare a written path to a string. Each source unit gets an alias map built from its `use` trees, and a detector matches a target crate and item segments against the resolved provenance of the call it inspects. The idiomatic imported form is now reported like the fully qualified one: `use std::process::Command;` followed by `Command::new("sh")`, a renamed import such as `use reqwest::Client as Http;`, and nested groups all resolve.
- Provenance that cannot be decided produces no diagnostic. A glob import, a locally declared item or generic parameter with the same name, a `crate`, `self` or `super` prefix, and a saturated alias map all make the detector abstain. Re-exports, trait methods and `Self` stay out of reach by design.
- A source unit whose alias map crosses the published binding limit reports a bounded `limit-exceeded` error naming `alias-bindings` and keeps the unit analyzable in indeterminate mode.
- Detectors are registered rather than hard-coded: the CST is traversed once per unit and each detector is solicited on the node kind it declares, so adding one changes no analysis signature. A policy that disables every native rule still loads no source unit at all.
- The targeted crate is resolved from the manifest, renames included, instead of a dedicated field on the traversal structure. A workspace that does not depend on the targeted crate emits nothing and reports no error.
- Report schema, rule identifiers, severities and delta fingerprints are unchanged. Workspaces that were writing the imported form will see new true positives on the two existing security rules.

### Report schema v9

- The score model becomes `core-v2`. Every catalog rule carries a `tier` among `P0`, `P1`, `P2` and `P3`, published in `policy.rules[]` and independent from `default_level` and from diagnostic severity. Rule IDs, `base_severity` and delta fingerprints are unchanged, so no recorded baseline is invalidated.
- The worst tier observed in a dimension caps that dimension, and the worst tier observed across all dimensions caps the overall score: `P0` caps at 20 and 40, `P1` at 50 and 65, `P2` at 75 on its dimension only, `P3` caps nothing. `audit.score` publishes `worst_tier` and the `applied_ceiling` actually used, so a capped score is explainable without recomputation.
- A rule penalty now grows with its occurrence count through bounded steps (1, 2-5, 6-20, 21 and more), so fifty occurrences cost more than one while no single rule can saturate its dimension.
- `summary` and `audit.categories[]` each publish both magnitudes under explicit names, `distinct` and `occurrences`, and they agree field by field. The five flat fields keep their historical name, type and meaning. A diagnostic that no catalog category covers now lands in an explicit `Other` bucket instead of disappearing from the tallies.
- A report whose counts diverge from its diagnostics, or whose score is inconsistent with its published dimensions and ceiling, fails to serialize instead of being published.
- Consumers restricted to schema v8 must reject schema v9 or explicitly remove `summary.distinct`, `summary.occurrences`, `policy.rules[].tier`, `audit.categories[].distinct`, `audit.categories[].occurrences`, `audit.score.worst_tier` and `audit.score.applied_ceiling`. No field was removed or retyped.

### Report schema v8

- Every report adds a deterministic top-level `audit` field containing analyzed Rust file count, ordered category tallies and the local `core-v1` score.
- Scores retain a numeric partial result but suppress projection and sharing when the scan or any diagnostic is not authoritative. Reports with no eligible Rust files expose `score: null`.
- Rule groups, migration-scale advisories and confined code-frame data are derived from the report without changing the diagnostic kernel.
- Consumers restricted to schema v7 must reject schema v8 or explicitly remove `audit` and normalize `schema_version`; every remaining serialized byte is preserved by compatibility tests.

### Report schema v7

- Every report adds a top-level `delta` field. Full, files, failed and incomplete inspections emit `delta: null`.
- Complete baseline inspections publish fingerprint version 1, side cardinalities, introduced IDs, current-to-baseline pre-existing ID pairs, complete fixed baseline diagnostics and deterministic delta counters.
- Baseline quality gates count only introduced current diagnostics. Full and files gates retain their schema v6 behavior.
- Baseline terminal output shows introduced and fixed details, suppresses pre-existing details and prints the stable delta summary.
- Consumers restricted to schema v6 must reject schema v7 or explicitly add the nullable `delta` field and the `baseline` scope mode. Existing schema v6 fixtures remain frozen.

### Report schema v6

- Every resolved inspection now includes a top-level `scope` object.
- Full inspections emit `{"mode":"full","execution_scope":"workspace","comparison_base":null,"files":null}`.
- Files inspections emit `mode: "files"`, `execution_scope: "workspace"`, the resolved merge-base OID and the sorted changed paths.
- Failures before scope resolution emit `scope: null`.
- Consumers restricted to schema v5 must reject schema v6 or add the `scope` field to their model. All former fields retain their v5 meaning and shape.
