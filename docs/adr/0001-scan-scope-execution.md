# ADR 0001: Scan scope execution and reporting

- Status: accepted
- Date: 2026-07-22
- Stories: US-006, US-007, US-008

## Context

Rust Doctor has two different scope questions. Execution scope describes the work an analyzer must perform to remain correct. Reporting scope describes which proven diagnostics are relevant to the user's requested files or lines. Cargo and rustc do not offer a sound file-only compilation mode, so a report filter cannot be presented as an execution speedup.

## Pass classification

| Pass | Class | Narrowest sound execution | Network behavior |
|---|---|---|---|
| Custom `syn` rules | File-local | Selected Rust files before read or parse | None |
| Clippy | Package-global | Full affected Cargo package, optionally selected with `-p` | Cargo may resolve dependencies unless offline |
| MSRV | Package-global | Owning package manifest plus local `rustc --version` | None |
| Coverage | Package-global | Existing coverage artifacts for the owning package | None |
| cargo-machete | Workspace-global | Full selected Cargo graph | None after Cargo metadata is available |
| cargo-geiger | Package-global | Full affected package and dependency graph | Cargo may resolve dependencies unless offline |
| cargo-deny | Workspace-global | Lockfile and manifests for the selected workspace | Advisory and index access unless offline |
| cargo-audit fallback | Workspace-global | Workspace lockfile | Advisory database access unless offline |
| cargo-semver-checks | Network-dependent | Full affected package against its release baseline | May resolve or fetch a baseline |

An analyzer that cannot be narrowed safely must execute at the scope above or emit a structured skipped state. Rust Doctor must never label post-execution filtering as an early-scan optimization.

## Decision

The report contract names both scopes. Aggregate `execution_scope` values are `full_packages`, `affected_packages`, `isolated_snapshot`, or a documented composition of those values. Per-check accounting records narrower file-local work and explicit skips.

- `reporting_scope`: `full`, `files`, `changed`, `lines`, `staged`, or `baseline`.

File-local rules receive the selected file set before any file read or parse. Clippy and other package-global adapters run for the owning package. Workspace-global checks run once for every affected workspace or emit an explicit skip. Root `Cargo.toml`, `Cargo.lock`, and workspace policy changes affect every selected member.

### Staged compiler scans

Staged scans materialize the Git index into a `tempfile::TempDir` after these checks:

1. Reject unresolved index stages.
2. Parse `git ls-files --stage -z` and validate every path as relative, component-safe, and inside the destination.
3. Reject staged symlinks because Cargo could follow a link outside the isolated snapshot.
4. Run `git checkout-index --all --prefix=<temp>/snapshot/` without consulting working-tree contents.

Cargo runs from the snapshot with `CARGO_TARGET_DIR` under the same temporary directory. The temp directory owns sources, temporary index data, and build output, so RAII removes all materialization on success, error, timeout, or cancellation. Subprocesses run in a dedicated process group and are terminated before the guard drops.

Baseline scans use the same strategy with a private `GIT_INDEX_FILE`: `git read-tree <merge-base>` populates the temporary index, then `checkout-index` materializes the base without checking out or modifying the user's worktree.

## Benchmark evidence

Reproduce the fixtures and measurements from the repository root:

```bash
cargo run --quiet --example benchmark_scan_scope -- benchmarks/scope/fixtures.json
```

The manifest pins five deterministic fixture topologies through `schema_version = 1` and `generator_revision = scope-fixtures-v1`. Each strategy uses a cold, isolated target directory. `report_filtered` deliberately executes full Clippy and filters only after execution.

Host: Linux 7.1.4 x86_64, rustc 1.95.0, cargo 1.95.0. One smoke sample on 2026-07-22:

| Fixture | Full | Package selected | Report filtered |
|---|---:|---:|---:|
| Single crate | 70 ms | 66 ms | 67 ms |
| Virtual workspace, 2 members | 64 ms | 65 ms | 64 ms |
| Proc macro | 67 ms | 67 ms | 67 ms |
| Build script | 104 ms | 108 ms | 104 ms |
| Workspace, 20 members | 116 ms | 69 ms | 117 ms |

The small fixtures are dominated by process startup. The 20-member fixture shows the only material execution reduction: selecting an affected package. Report filtering remains equivalent to full execution, as expected.

## Consequences

Rust Doctor can promise early work reduction only for custom file-local rules and package selection. Line, changed, staged, and baseline modes still expose the full package cost of compiler-aware evidence. Reports retain enough execution accounting to distinguish a complete narrow report from a partial or degraded one.
