# Report V1 migration contract

Report V1 is the only structured machine-output contract from rust-doctor 0.2 onward. `--json`, its `--json-compact` and `--json-out` modifiers, SARIF, and MCP projections are built from the same immutable report value.

`report_constructed`, `outcome`, `completeness`, `summary.score_authoritative`, and `gate_result` are independent. A schema-valid failure report has `report_constructed = true` even when scanning failed. `requested_root` preserves user input while `resolved_root` identifies the discovered Cargo root.

## Compatibility

V1 may add optional fields and enum variants. Consumers must ignore unknown fields and handle unknown enum values conservatively. Removing a field, changing a field's meaning, narrowing an accepted value, or changing identity inputs requires a new `schema_version`.

The legacy top-level `score`, `score_label`, `dimension_scores`, `source_file_count`, `elapsed`, `skipped_passes`, and severity counts remain through the v0.2 migration release. Their canonical replacement is `summary` plus `completeness`. A scope with no eligible Rust files has `outcome = "nothing_to_scan"` and `score = null`.

Score Core 2.1 adds four optional fields to each `root_causes` entry:
`score_dimension`, `current_penalty`, `maximum_penalty`, and
`remediation_title`. Existing fields and types are unchanged. Advisory,
audit-only, and unscored groups omit score fields. Consumers can use the new
values to reconcile group contributions before dimension rounding.

## Scope and score authority

`execution_scope` names the analyzer work that ran, while `reporting_scope` names the findings retained for the request. A files or lines report can therefore execute complete affected packages without claiming file-level compiler execution.

Every check carries `required`, `status`, and an optional reason. `completeness.score_authoritative` and `summary.score_authoritative` are false when required work, planned files, or a baseline comparison is incomplete. Optional unavailable adapters can make a report partial without invalidating a score that still covers every required check.

Diagnostics declare `ownership` as a Cargo package ID, `workspace`, or `unowned`, independently from source location. They also expose `source_surface`. Test, bench, example, and generated findings remain visible on terminal, SARIF, MCP, and PR surfaces by default but do not affect score or CI failure unless an explicit rule, category, tag, or path policy includes those surfaces.

Baseline reports preserve `requested_base` separately from `resolved_base` and the legacy `base_commit`, expose head and base policy fingerprints, introduced and fixed counts, base total, conservative cross-file matches, and degradation state. Consumers must treat `baseline_degraded = true` as a files-scope fallback, never as a successful introduced-only comparison.

## Stable identities

Both identities are lowercase SHA-256 hex. Fields are separated by a NUL byte after whitespace normalization.

`site_id` input:

```text
rust-doctor-site-v1\0provider\0canonical_rule\0normalized_project_relative_path\0location_evidence\0normalized_message
```

`baseline_key` excludes the path:

```text
rust-doctor-baseline-v1\0provider\0canonical_rule\0normalized_message\0location_evidence
```

Paths use `/` separators. A compiler diagnostic without a local primary span uses a project-level location. Lines and columns are one-based; byte offsets are zero-based or null when the adapter did not provide enough evidence.

## Configuration migration

Use typed `[rules.<id>]`, `[categories.<category>]`, `[tags.<tag>]`, and `[[path_overrides]]` policy. `rules_config`, `ignore.rules`, and `ignore.enable` remain accepted through v0.2 and emit a deprecation warning. Precedence is the last matching path override, exact rule, category, tag, then catalog default. Visibility surfaces do not activate rules.

## File output

`--json --json-out <path>` serializes the same Report V1 data as stdout. It writes a temporary sibling and renames only after serialization and flush complete. An error leaves stdout empty and does not leave a partial destination file.

## Score Core V2 and diagnostic trust

Rust Doctor 0.3 replaces the unversioned legacy score with **Score Core V2**. Every report, cache, and evaluation baseline now carries `score_model_version`. Legacy values and V2 values come from different model series and **must not be compared**: a moving number between the two releases is a model migration, not a health change. Caches and baselines recorded under a different model version are invalidated on load rather than reinterpreted.

The model itself is a reviewed artifact, `evaluation/score-model-v2.json`, compiled into the binary and validated at load: it pins the priority penalties, the bounded-occurrence multiplier cap, the label thresholds, the P0 ceiling, and the five approved dimension weights. A candidate model that changes those weights is rejected.

Impact is decided by two catalog properties, never by presentation severity:

- **priority** (`p0`–`p3`) sets the base penalty; an unranked rule contributes nothing.
- **aggregation policy** (`root-cause`, `bounded-occurrence`, `unique-rule`, `audit-only`) bounds repetition. A hundred occurrences of one bounded rule cost at most twice the first occurrence.

A rule only moves the Core Score when it is **score-eligible** *and* owned by a **core analyzer** (rustc, Cargo, Clippy, or a calibrated `syn` rule). Optional external executables — cargo-audit, cargo-deny, cargo-geiger, cargo-shear, cargo-semver-checks, cargo-llvm-cov — keep their diagnostics and completeness receipts but never change the number, so the same repository scores identically on machines with different tools installed. A calibrated heuristic becomes score-eligible only through a passing record in `evaluation/calibration-v1.json`, measured on the labeled dataset in `evaluation/truth-dataset-v1.json`.

## Additive coverage fields

Three additive fields join Report V1. Every pre-existing required field keeps its name, type, and meaning.

- `score_model_version` (string): the Score Core identifier that produced every score in the report.
- `dimensions` (array, also present per project): planned, scheduled, completed, skipped, and failed analyzers per health dimension, plus `covered_scope`, `uncovered_scope`, machine-readable `reasons`, and `authority` (`authoritative`, `partial`, `unobserved`, `failed`).
- `workspace_health` (object, multi-package only): the headline package, the authoritative minimum, median, and maximum, and the full per-package score list.

A dimension with no completed core analyzer reports `score: null` and `authority: "unobserved"` instead of a synthetic 100, and it removes `summary.score_authoritative`. The legacy `dimension_scores` object keeps its `u32` fields for existing consumers; `dimensions[].score` is the field that can be null.

## Workspace headline

The headline score is the **lowest authoritative package score**. A package whose required analysis did not complete never lends its number to the workspace, and it is never hidden by healthy siblings: one non-authoritative package sets `summary.score_authoritative = false` while `workspace_health` keeps the full distribution.

## Rule abstention receipts

A rule that cannot decide no longer guesses. When Cargo metadata is unavailable, the source surface is unclassified, or the file belongs to a context the rule declares unsupported, the rule is excluded before AST traversal and the scan records an abstention receipt instead of a diagnostic.

Two additive fields carry this. Every pre-existing required field keeps its name, type, and meaning.

- `audit.abstentions` (array): one `{ rule, reason, count }` record per rule and reason. Reasons are stable machine-readable strings: `missing-context:<requirement>`, `unsupported-source-surface:<surface>`, `unsupported-crate-role:<role>`, and `unsupported-origin:<origin>`.
- `completeness.abstentions` (integer): the total rule-file decisions declined, so a consumer can see observability loss without reading every receipt.

An abstention degrades observability; it never fails a required check and never becomes a finding. Consumers that ignore both fields keep their previous behaviour.

## Advisory diagnostics and analyzer curation

Three more additive fields join Report V1. Every pre-existing required field keeps its name, type, and meaning.

- `diagnostics[].advisory` (boolean): the owning rule runs by default but contributes exactly zero to the score. Terminal output marks these `[advisory · no score impact]`; consumers must not present them as the reason a score dropped.
- `audit.lint_policy` (array): the effective Cargo `[lints.clippy]` policy resolved per package as `{ lint, declared_level, source, rust_doctor_severity }`. `source` is `package` or `workspace-inherited`, so a level reached through `[lints] workspace = true` is distinguishable from one declared locally. A lint the package explicitly sets to `allow` is not forced back on.
- `dimensions[].uncovered_scope` now also carries `feature:<name>` entries. Compiling analyzers build the feature profile Cargo resolves by default rather than `--all-features`, so every other declared feature is named as uncovered scope instead of being inferred healthy.

Clippy configuration is written to an isolated temporary directory and passed through `CLIPPY_CONF_DIR`. Rust Doctor creates zero files in the scanned repository, and a project that ships its own `clippy.toml` keeps it.

Optional external adapters — cargo-audit, cargo-deny, cargo-geiger, cargo-shear, cargo-semver-checks — record their tool version, parser contract version, and evidence source in the finding's help text. A missing tool, timeout, cancellation, non-zero exit, truncated document, or unparseable payload produces a skipped or failed receipt; none of them normalizes to an empty successful result. One RustSec advisory reported by several analyzers collapses to a single root cause that retains every analyzer's provenance.

## Promotion gates and trust exceptions

Every default or score-eligible rule is requalified on every listing against the gate its trust tier defines. A calibrated heuristic needs a passing record in `evaluation/calibration-v1.json`; a compiler-proven rule needs its analyzer's parser contract to still match its row in the conformance matrix; an advisory-backed rule needs advisory-database identity or declarative policy evidence from a conforming tool; an audit-only rule may never claim score eligibility. Rules that predate this program get no exception: the gate reads the same evidence for all of them, and an unavailable metric is a failure, never a pass.

`evaluation/gate-history-v1.json` records the per-rule outcome on each approved corpus revision. A rule failing on the last two revisions is reported as `demotion-proposed` with a machine-readable reason, so demotion is a visible decision rather than a silent drift.

One additive field joins Report V1. Every pre-existing required field keeps its name, type, and meaning.

- `audit.trust_exceptions` (array): approved threshold exceptions still inside their window, as `{ rule, owner, granted_at, expires_at, reason, evidence }`. An exception is rule-specific, owner-attributed, time-bounded, and documented; the shipped default is an empty array, because no rule currently ships on one.

`rules list --json` and `rules explain --json` gain a `gate` object per rule: `status` (`not-gated`, `passing`, `exempt`, `failing`, `demotion-proposed`), the `authority` that lets a passing rule ship, and a stable `reason_code` plus `detail` when it does not.

## Analyzer conformance matrix

Each analyzer able to carry authority has a row in the versioned conformance matrix: supported tool versions, parser contract version, evidence source, fixture provenance, and whether it may contribute authority at all. Adapter provenance in finding help text now ends with the conformance state, for example `cargo-audit 0.21.2 (parser audit-json-v1, structured-json, supported, authoritative)`. A tool version outside the approved matrix reports `best-effort` and `non-authoritative`; a version that cannot be read reports `unsupported`. cargo-geiger is `non-authoritative` on every version by contract, because unsafe exposure is an observation rather than a proven defect.

Cargo metadata is requested at an explicit format version and tolerates additive unknown fields without losing package or target identity.

## Canonical decision metadata

Every canonical diagnostic now answers "how urgent, how trustworthy, what evidence, and what can I safely do about it" without the consumer re-deriving anything. Ten additive fields join `diagnostics[]` and every pre-existing required field keeps its name, type, and meaning.

- `priority` (string, optional): product urgency, `p0`–`p3`. Absent means unranked. An unknown Clippy lint, dynamically discovered rule, or unmapped adapter finding never receives a fabricated priority.
- `trust_tier` (string): `compiler-proven`, `advisory-backed`, `calibrated-heuristic`, `audit-only`, or `unknown`.
- `score_eligible` (boolean) and `score_impact` (string): eligibility is the catalog contract; impact is what this occurrence actually did. `score_impact` is `scored`, `advisory`, `ineligible`, or `suppressed`, and only `scored` moved the number.
- `aggregation_policy` (string): how repeated findings from this rule aggregate.
- `root_cause_key` (string, optional): stable identity of the underlying defect — `advisory:RUSTSEC-YYYY-NNNN` or `rule:<canonical-id>`. Message text is never hashed into it, so wording changes do not split one root cause. Absent for unmapped rules.
- `evidence_summary` (string) and `limitations` (array): what the analyzer observed and where it can be wrong.
- `fix_recipe` (string, optional): identifier of a validated recipe. Absent means the remediation is guidance; output never implies machine applicability it does not have.
- `suppressed` (boolean): a suppressed record that appears in an export that permits it always carries `score_impact = "suppressed"` and contributes exactly zero.

`root_causes` (array) joins the top level: one entry per canonical root cause, ordered highest impact first, carrying `key`, `title`, `rule`, `category`, `priority`, `trust_tier`, `aggregation_policy`, `score_impact`, `occurrences`, `file_count`, and the `site_ids` of every occurrence. The group owns the priority and the bounded score impact; the occurrences stay individually inspectable in `diagnostics`.

## One ordering contract

Terminal, JSON, SARIF, MCP, CI annotations, plans, and handoffs order findings through one shared comparator: priority, then evidence authority, trust tier, root-cause impact, category, rule ID, package, path, and location, with stable fallbacks so a missing path or line can never make the order nondeterministic. Severity is only a tie-breaker; it describes presentation, not urgency. `tests/cross_surface_ordering.rs` compares the real binary's JSON, SARIF, and handoff output and fails release validation when a surface reorders on its own.

Above 50 findings spread across at least 10 files or 5 root-cause groups, the terminal and the handoff switch to migration grouping and lead with the highest-impact root causes. Below that threshold they use the normal priority list without empty migration sections. When a consumer truncates, the omitted count is reported by priority and by category, and the top three root-cause groups are never displaced.

SARIF results carry `properties.priority`, `properties.rootCauseKey`, and `properties.scoreImpact`; rules carry `properties.precision` (`very-high` through `low`, derived from the trust tier), `properties.priority`, `properties.trustTier`, and `properties.scoreEligible`. Two results sharing `partialFingerprints["rustDoctorRootCause/v1"]` are one defect.

A span that falls outside the scanned project — Clippy attributes package-level lints to the isolated configuration directory — is reported as a project-level location. It never leaks an absolute temporary path and never destabilizes `site_id` or `baseline_key`.

## Fix eligibility

`diagnostics[].fixes[]` gains three additive fields. Applying an edit is now an explicit decision with a recorded reason, not a consequence of a suggestion existing.

- `eligibility` (string): `machine-applicable` or `guidance-only`. The default is `guidance-only`, so an unproven fix is never applied.
- `precondition_hash` (string, optional): SHA-256 of the target file as analyzed. Applying against a different hash means the source moved since the scan and the fix is refused.
- `ineligible_reason` (string, optional): why the edit stays advice.

A fix is machine-applicable only when rustc marked it so, the span is byte-addressed inside a readable UTF-8 file under the project root, the evidence is not macro-generated, and the rule family is not a policy decision. Security, dependency, Cargo, unsafe, public-API, MSRV, and feature changes are never automatic. Overlapping edits in one file and a root cause whose edits span several files both fall back to guidance.

`--fix` applies the plan one root-cause group at a time. A group is written only after the patched file still parses as Rust; when a group fails, later groups are reported as not attempted rather than presented as validated. MCP remains read-only: suggested commands and dependency changes are rendered as explicit user actions and are never executed.

## Certification

The Decision Quality Hardening program supersedes the earlier trust-parity certification claim. The earlier PRD remains a historical architecture record. Its release-evidence target is the [`decision-quality-v1.json`](../evaluation/certifications/decision-quality-v1.json) manifest; until that file exists with `certified: true`, the newer program remains in progress.

`rust-doctor-eval certify` requires a candidate corpus and an approved corpus baseline. It records the release binary SHA-256, Rust toolchain, OS and architecture, score-model version, hashes for every checked dataset and policy artifact, corpus totals, qualified adapter versions, conformance hashes, the exact path-safe command, and a UTC timestamp. It also executes the release binary for the named self-scan case and records scored, advisory, audit, authority, and top-remediation summaries without repository identity or diagnostic messages.

The required release-evidence artifact binds quality gates, cross-surface behavior, packaged artifact smoke, the 200-member workspace and 100,000-diagnostic budgets, pinned-corpus runtime, and 20-trial SIGINT/SIGTERM behavior to the same release binary and source revision. The certification schema is [`certification-v2.schema.json`](../evaluation/schemas/certification-v2.schema.json).

Evidence that is missing is not evidence that passes: absent corpus arguments, fewer than 260 complete roots, an unavailable metric, a stale hash, a non-authoritative self-scan, an unsupported adapter, a failed release verdict, or a binary mismatch writes `certified: false`, returns exit `1`, and emits no success wording.
