# Changelog

## Unreleased

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
