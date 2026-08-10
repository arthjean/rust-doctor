# Rust Doctor

Your agent writes bad Rust, this catches it.

Rust Doctor scans a local Cargo workspace and reports what is wrong with it: panics waiting to happen, blocking calls held across `await`, dependencies that only resolve on your machine, secrets-adjacent patterns like disabled TLS verification. It ends with a score out of 100 and the three rules worth fixing first.

Everything runs locally. No network, no upload, no telemetry.

## Install

Not published yet. Build it from source:

```bash
git clone https://github.com/arthjean/rust-doctor
cd rust-doctor
cargo build --release
```

The binary lands in `target/release/rust-doctor`. Requires Rust 1.95 or later.

## Use

```bash
rust-doctor                 # scan the current workspace
rust-doctor path/to/project # scan somewhere else
rust-doctor --json          # machine-readable report
rust-doctor --verbose       # every finding, not just the top one
```

A scan looks like this:

```
Scanning Rust files...
Scope: full codebase
Scanned 27 files in 5.0s
Top warning: Indexing slicing
Rule ID: clippy::indexing_slicing
indexing may panic
Help: Use get or get_mut and handle the absent element instead of indexing,
which panics out of bounds.
src/audit.rs:526:27
────────────────────────────────────────────────
All 64 occurrences across 64 findings
Bugs: 0 errors, 58 warnings, 0 info, 0 unknown (occurrences)
Dependencies: 0 errors, 2 warnings, 0 info, 0 unknown (occurrences)
Maintainability: 0 errors, 4 warnings, 0 info, 0 unknown (occurrences)
Gate passed: blocking error, 0 blocking diagnostic(s)
  ┌─────┐  95 / 100 Great
  │ ◠ ◠ │  ████████████████████████████████████████████████░░
  │  ▽  │  Rust Doctor
  └─────┘
Fix the top 3 rules to reach a projected 96/100: clippy::indexing_slicing,
clippy::string_slice, clippy::print_stderr
```

## Trust boundary

**Inspect trusted local repositories only.** Rust Doctor runs `cargo clippy`, and Cargo executes `build.rs` files and procedural macros from the scanned workspace. Scanning a repository you do not trust runs its code on your machine. This is a property of Cargo, not of Rust Doctor, and no scanner that type-checks Rust can avoid it.

The native detectors carry no such risk: they parse source text and never build anything.

## Scan scope

By default the whole workspace is scanned. Two narrower modes exist:

```bash
rust-doctor --scope files --base HEAD     # only files changed since a ref
rust-doctor --scope baseline --base main  # only findings your change introduced
```

`--scope baseline` materializes the comparison ref in a temporary worktree, scans both sides, and reports the delta. That is the mode for CI: it ignores the existing backlog and fails only on what the change adds.

## Rules

62 rules today: 37 selected Clippy lints, 16 native detectors and 9 structural rules.

The Clippy lints are curated, not the whole `restriction` group. They cover panic paths (`unwrap_used`, `indexing_slicing`, `panic_in_result_fn`), async and concurrency hazards (`await_holding_lock`, `arc_with_non_send_sync`, `rc_mutex`), and allocation waste (`redundant_allocation`, `unnecessary_to_owned`, `useless_vec`).

The native detectors find what Clippy does not:

| Rule | Catches |
|---|---|
| `rust_doctor::source::disabled_tls_verification` | certificate validation switched off |
| `rust_doctor::source::dynamic_shell_command` | a shell command built from runtime data |
| `rust_doctor::cargo::unpinned_git_dependency` | a git dependency with no pinned revision |
| `rust_doctor::cargo::unbounded_registry_dependency` | a version requirement with no upper bound |
| `rust_doctor::cargo::duplicate_major_versions` | one crate resolved at incompatible majors |
| `rust_doctor::cargo::missing_lockfile` | a binary shipped without `Cargo.lock` |
| `rust_doctor::cargo::path_dependency_outside_workspace` | a path that only resolves on your machine |
| `rust_doctor::cargo::permissive_lint_table` | a `[lints]` entry switching a catalogued rule off from the manifest |
| `rust_doctor::cargo::unused_dependency` | a declared dependency no source of its package references |
| `rust_doctor::cargo::test_only_dependency` | a `[dependencies]` entry referenced only from test, bench or example code |
| `rust_doctor::cargo::unchecked_release_overflow` | a release profile compiling a binary without overflow checks |
| `rust_doctor::cargo::release_debug_symbols` | a release profile shipping full debug info unstripped |
| `rust_doctor::cargo::permissive_rustflags` | a workspace rustflag drawn from the closed checks-disabling list |
| `rust_doctor::repo::tracked_secret_file` | a git-tracked file whose name marks it as secret-bearing |
| `rust_doctor::repo::hardcoded_credential` | a tracked source literal matching a closed list of credential prefixes |
| `rust_doctor::repo::unignored_build_output` | a Cargo target directory git does not ignore |
| `rust_doctor::structure::complex_function` | a function past its cyclomatic or cognitive threshold |
| `rust_doctor::structure::crate_level_allow` | an `#![allow]` covering a whole file or module |
| `rust_doctor::structure::duplicate_function_body` | functions with identical bodies under other names |
| `rust_doctor::structure::near_duplicate_function_body` | functions alike above a similarity threshold |
| `rust_doctor::structure::orphan_module_file` | a file no `mod` declaration reaches, which Cargo never compiles |
| `rust_doctor::structure::oversized_unit` | a file, function, impl block or module grown too large |
| `rust_doctor::structure::stacked_allow_attribute` | several suppressions accumulated on one item |
| `rust_doctor::structure::unreasoned_allow_attribute` | a lint switched off with no `reason` given |
| `rust_doctor::structure::unreferenced_feature` | a feature nothing reads, or a `cfg` naming one nothing declares |

The structural rules read the same syntax tree, but ask what the codebase does as a whole rather than what a call site does. A duplication is reported as one family naming every site, never one finding per site, and its identity is the shape rather than the position, so inserting lines above it leaves a baseline comparison unmoved.

Native detectors parse the syntax tree with `ra_ap_syntax` and resolve call provenance through the manifest's dependency aliases, so a renamed import or a fully qualified path is recognized the same way. They never match a written path against a string.

## Configure

Override any rule or category from the command line:

```bash
rust-doctor --rule clippy::unwrap_used=off
rust-doctor --category performance=error
rust-doctor --blocking warning     # none | error | warning
```

Or persist the same decisions in a `rust-doctor.toml` next to your workspace manifest. Command-line overrides win over the file.

The gate exits non-zero when a diagnostic at or above the blocking level survives, which is what makes it usable in CI.

## Score

Five dimensions (security, reliability, maintainability, performance, dependencies) fold into one number out of 100, labelled `Great`, `Needs work`, or `Critical`. Two rules are zero-tolerance: one occurrence caps its dimension at 20 and the overall score at 40.

Findings from test targets, benches, examples, and build scripts are reported but do not weigh. A `println!` in `build.rs` is the channel Cargo imposes, not a defect of what you ship.

Precision is measured, not asserted: `tests/corpus.json` pins ten public Rust repositories by commit, and `cargo test --test corpus_precision` replays the rules against them. The measurement says how much noise a rule produces on healthy code. It does not claim recall, which needs adversarial fixtures and is a separate question.

Suppression is scored. A blanket `#![allow(clippy::all)]`, several suppressions stacked on one item, a `[lints]` table switching a catalogued rule off, a `.cargo/config.toml` capping lints for every build: each is a finding of its own, so silencing the scanner costs points instead of earning them. That is the one move a clean build cannot distinguish from a fix, and it is the reason the rest of the score means anything.

The structural rules are measured the same way, on the families that actually weigh on the score: `duplicate_function_body` adjudicates at 40% false positives over 20 reviewed clone families, against 100% for the two rules at the head of the Clippy catalog. The six other measured structural rules sit between 60% and 100% over 5 sites each, because healthy code documents its exceptions where no detector reads: an `#[allow]` justified in a trailing comment, a feature the manifest marks deprecated, a crate-level allow that exists only to support older compilers. The ten repositories never triggered `orphan_module_file`, and triggered `stacked_allow_attribute` once, in a test. Those rates are published, not hidden, and none of them removes a rule.

Against a second corpus of eight agent-authored repositories, selected by commit-trailer evidence alone with at least half of all commits carrying an agent attribution marker, agent-written Rust measures 5.4 structural findings per thousand lines against healthy Rust's 3.5, a ratio of 1.51. That second population is also the only one where a declared dependency nothing references turned up at all. Adjudicated per-rule precision on it is not yet measured. The full record, confidence intervals included, sits in `docs/structural-precision-2026-08.md` and `docs/suppression-precision-2026-08.md`.

## JSON output

`--json` emits a versioned report (`schema_version`) with every diagnostic, its span, its rule, its category, its severity before and after policy, the gate verdict, and the score. Paths are workspace-relative. No absolute path, no environment, no user data leaves the report.

## License

MIT OR Apache-2.0 (declared in `npm/rust-doctor/package.json`; the license files are not in the repository yet).
