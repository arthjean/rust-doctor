# Report V1 migration contract

Report V1 is the only structured machine-output contract from rust-doctor 0.2 onward. `--json`, its `--json-compact` and `--json-out` modifiers, SARIF, and MCP projections are built from the same immutable report value.

`report_constructed`, `outcome`, `completeness`, `summary.score_authoritative`, and `gate_result` are independent. A schema-valid failure report has `report_constructed = true` even when scanning failed. `requested_root` preserves user input while `resolved_root` identifies the discovered Cargo root.

## Compatibility

V1 may add optional fields and enum variants. Consumers must ignore unknown fields and handle unknown enum values conservatively. Removing a field, changing a field's meaning, narrowing an accepted value, or changing identity inputs requires a new `schema_version`.

The legacy top-level `score`, `score_label`, `dimension_scores`, `source_file_count`, `elapsed`, `skipped_passes`, and severity counts remain through the v0.2 migration release. Their canonical replacement is `summary` plus `completeness`. A scope with no eligible Rust files has `outcome = "nothing_to_scan"` and `score = null`.

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
