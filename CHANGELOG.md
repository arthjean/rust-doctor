# Changelog

## Unreleased

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
