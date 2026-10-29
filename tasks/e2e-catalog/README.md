# End-to-end scenario catalogs

Source material for `tasks/prd-e2e-hardening.md`. Five read-only explorations ran the
release binary built from `b5389f7` (0.7.0) against throwaway copies on 2026-10-20 and
recorded, for every scenario, its setup, command, expected observable, the Rust test
that already covers it (if any) and whether it was verified on the binary. The raw logs,
probe scripts and scratch repositories they cite were not kept: every row carries the
recipe needed to replay it.

Credential literals the explorations planted are replaced by placeholders such as
`xoxb-<planted body>`: a tracked literal would be reported by
`rust_doctor::repo::hardcoded_credential` on this repository's own scan. Tests build
their credentials at runtime.

| Catalog | Domain | Scenarios | Proposed e2e files |
|---|---|---|---|
| [tui.md](tui.md) | Interactive report under tmux | 166 | 13 `tui-*.test.ts` |
| [scopes.md](scopes.md) | Scopes crossed with git states | 258 | 8 `scope-*.test.ts` |
| [subcommands.md](subcommands.md) | Non-scan subcommands and `rust-doctor.toml` | 337 | 12 |
| [failures.md](failures.md) | Exit codes, signals, toolchains, workspace shapes, environment, privacy | 185 | 10 |
| [real-codebases.md](real-codebases.md) | This repository, the pinned corpus, the npm launcher, the Action, CI topology | 19 invariants, 18 subjects, launcher and Action matrices | 14 |

## Defect ledger

Every defect the explorations confirmed, under a stable identifier `<catalog>/<id>`. The
PRD fixes some in Release 1; the e2e suite writes every other one as a `test.failing`
row tagged with this identifier, and `tests/e2e/known-bugs.json` mirrors this table.

| Identifier | Severity | Defect | PRD story |
|---|---|---|---|
| failures/BUG-01 | high | A Clippy suggestion republishes a credential the code frame masks | US-001 |
| scopes/BUG-A | high | Baseline scope reports every repository-pass finding as introduced, never as fixed | US-002 |
| scopes/BUG-D | high | `--staged` scans run no repository pass, so the pre-commit hook passes staged secrets | US-003 |
| subcommands/F3 | medium | `--staged` reads `rust-doctor.toml` from the working tree, not the index | US-003 |
| subcommands/F10 | low | Staged scans draw code frames from the working tree | US-003 |
| failures/BUG-02 | medium | Absolute paths outside the workspace and HOME pass through Cargo and rustup text | US-004 |
| failures/BUG-03, real/B3 | medium | A scan writes `Cargo.lock` into a workspace that had none; two scans disagree | US-005 |
| failures/BUG-07 | medium | `--advisory-db` on a missing path silently disables the check | US-006 |
| subcommands/F1 | high | A relative PATH breaks every installer that asks git where the repository is | US-007 |
| subcommands/F5 | medium | No installer or explainer checks that PATH exists | US-007 |
| subcommands/F4 | medium | `rules explain RULE PATH` reads `PATH/rust-doctor.toml` literally | US-008 |
| subcommands/F6 | medium | `why --report` performs no schema check | US-008 |
| scopes/BUG-B | low | A scope that fails to resolve is printed as `Scope: full codebase` | US-008 |
| failures/BUG-05 | medium | A package Cargo refused before `build-finished` scores 100 "Great" | US-009 |
| failures/BUG-06 | medium | An interrupted report carries score 100 "Great" | US-009 |
| failures/BUG-04 | medium | SIGHUP is unhandled: children keep running, a snapshot is left behind | US-010 |
| real/B2 | medium | Ctrl-C through the npm launcher delivers SIGINT twice and loses the report | US-010 |
| scopes/K4 | low | A snapshot left by a killed run is never swept | US-010 |
| tui/BUG-06 | medium | An external signal while the report waits leaves the cursor hidden and the tty raw | US-010 |
| subcommands/F2 | high (product) | The pre-commit hook blocks nothing by default | US-011 |
| real/D2, real/D3 | medium | `hardcoded_credential` help omits database URLs; the PRD example caps the self-scan at 65 | US-012 |
| failures/BUG-09, real/B3a | medium | Cargo and git filters can reach the network although the README says "no network" | US-013 |
| real/D1, real/D4, real/D5, real/D6, subcommands/F20 | low | Drifted documents: "temporary worktree" (three places, one shipped in users' workflows), rule counts, corpus wording, stale performance baseline, installer sentence in SKILL.md | US-014 |
| tui/BUG-11 | low | An empty `NO_COLOR` disables color, against no-color.org | US-014 |
| tui/BUG-01 to tui/BUG-05, tui/BUG-07 to tui/BUG-10, tui/BUG-12, tui/BUG-13 | low to medium | Interactive report defects (header width, status bar counts, hidden copy feedback, keys during the count-up, Ctrl-C during the count-up, no scrolling, no repaint on resize, lost category header, impossible CI entry offered, escape parsing, no in-frame confirmation) | `test.failing` |
| scopes/BUG-C, scopes/BUG-E, scopes/BUG-F, scopes/BUG-G | low | Index stat refresh by porcelain `git diff`, submodules refused by baseline, `..` symlinks refused, edits through a symlinked module dropped | `test.failing` |
| scopes/BUG-L | low | A `--max-duration` deadline that cuts the base side of a baseline reports only `baseline-scan-incomplete`, never `deadline-exceeded` | `test.failing` |
| subcommands/F7 to F9, F11 to F19 | low | Installer output order and exit codes, escape bytes echoed, frames, overrides, `why` gaps, `--update` dropping lines, settings reformatting, oversized Stop payload, staging leftovers, rule suffixes | `test.failing` |
| failures/BUG-08, failures/BUG-10 to BUG-12 | low | Missing git fails every scan, cosmetic error text | `test.failing` |
| failures/BUG-16 | low | The path scrubber replaces a prefix with no separator boundary, so a sibling path whose name only starts with the workspace root or target directory is rewritten into it | self-scan EP-002 |
| real/B4 | low | The orphan-module walk ignores `.gitignore`, so a gitignored `.rs` file beside the module tree is reported as `orphan_module_file` | `test.failing` |
| real/B7 | low | A SIGTERM through the npm launcher ends with status 143, not the 130 the binary documents | `test.failing` |
| real/B8 | low | A comment list that is not JSON fails the Action's comment step (exit 5) instead of downgrading to a warning | `test.failing` |
| real/B9 | low | An empty report leaves the Action's `authoritative` output empty instead of `false` | `test.failing` |
| tui/BUG-14 | low to medium | An external SIGINT, SIGTERM or SIGHUP while the report waits for a key sometimes exits with the scan's code instead of 130: the report's quit path races the signal watcher | `test.failing` |
