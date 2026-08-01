# Changelog

## Unreleased

### Report schema v6

- Every resolved inspection now includes a top-level `scope` object.
- Full inspections emit `{"mode":"full","execution_scope":"workspace","comparison_base":null,"files":null}`.
- Files inspections emit `mode: "files"`, `execution_scope: "workspace"`, the resolved merge-base OID and the sorted changed paths.
- Failures before scope resolution emit `scope: null`.
- Consumers restricted to schema v5 must reject schema v6 or add the `scope` field to their model. All former fields retain their v5 meaning and shape.
