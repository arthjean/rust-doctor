[PRD]
# PRD: E2E Hardening: Close What the Real Binary Revealed, Then Prove the Whole Surface End to End

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-10-20 | Arthur Jean | Initial draft, from five end-to-end explorations of the 0.7.0 binary (`b5389f7`) recorded in `tasks/e2e-catalog/` (946 catalogued scenarios, 19 self-scan invariants, 18 corpus subjects) |

## Problem Statement

rust-doctor is tested by 60 Rust integration test files, 36 of which spawn the built binary, but always on fixtures (`tests/fixtures/`), always with absolute paths, and, for the interactive report, at one width on one fixture (`tests/local_cli_experience.rs:189-800`). On 2026-10-20 five explorations drove the release binary on throwaway copies of real repositories: this repository, git histories built for the purpose, the 18 pinned corpus subjects, the npm launcher and the GitHub Action. They catalogued 946 scenarios (`tasks/e2e-catalog/README.md`) and confirmed defects no existing test could see. Five problems carry the weight.

1. **The scan republishes a secret it promises never to republish.** The code frame masks a detected credential (`src/presentation/code_frame.rs`), but a Clippy suggestion that replaces the literal carries it verbatim: in `suggestion.replacement` of the JSON (`src/report/normalize.rs:434`), on the linear `Replace with:` line (`src/render.rs:459-460`) and in the interactive viewer (`src/tui/screens/viewer.rs:226`). Reproduced with a Slack bot token passed through `.to_string()`; the help of `rust_doctor::repo::hardcoded_credential` says "the matched value is never republished by the report" (`src/policy/catalog/native.rs:174`).

2. **Every commit-time rule is blind at commit time, and the baseline lies about it.** `--staged` and the base side of `--scope baseline` scan a snapshot materialized under `$TMPDIR` (`src/baseline.rs:162-250`, `src/baseline/staged.rs:158`). It has no `.git`, so the repository pass, which enumerates through `git ls-files` (`src/repo_hygiene.rs:1-10`, wired at `src/execution.rs:559-567`), finds nothing there. Consequences, all reproduced:
   - The installed pre-commit hook (`--staged --scope lines`) lets a staged access key and a force-added `.env` through with exit 0.
   - A baseline comparison reports every repository finding as introduced and never as fixed. A branch with no change at all fails `--blocking warning`. On this repository, a no-op branch "introduces" the credential example at `tasks/prd-react-doctor-gap-closure.md:697`, so the dogfood Action posts a comment on a pull request that changed nothing.
   - `--staged` also reads `rust-doctor.toml` from the working tree, so an unstaged configuration decides a commit that does not record it.

3. **The scan does not leave the workspace as it found it, and some of its answers are false.**
   - On a workspace without `Cargo.lock`, `cargo clippy` (`src/execution/clippy.rs:49-53`, no `--locked`) writes one, so the next scan reads it: a binary crate scores 94 with `missing_lockfile`, then 99; corpus subjects move from 100 to 85. Eight repositories of the corpus clone cache already carry such a lockfile.
   - Cargo may download missing dependencies, while `README.md:12` says "no network".
   - `--advisory-db` on a mistyped path silently disables the advisory check.
   - A relative PATH breaks `hook install git`, `ci install` and `setup`, because git receives both `current_dir` and `-C` of the same relative path (`src/git.rs:103-120`, `:242-246`).
   - A package Cargo refused before building, and an interrupted scan, both publish score 100 "Great".
   - SIGHUP is not handled, a terminal Ctrl-C through the npm launcher delivers SIGINT twice and takes the hard-exit path with an empty stdout (`src/execution/interrupt.rs:12`, `npm/rust-doctor/lib/launcher.js:119-127`), and a killed run's snapshot is never swept.

4. **The hooks and the documents promise more than the product does.** The pre-commit hook passes no `--blocking`, every catalogued rule defaults to `warn` and blocking defaults to `error`, so the hook blocks nothing; the Stop hook forces `--blocking warning` (`src/hook/agent.rs:51`). Three files, one of them the workflow template `ci install` writes into users' repositories (`src/ci.rs:449-452`), say baseline uses a "temporary worktree" when the code snapshots into `$TMPDIR`. `.github/workflows/ci.yml:33` counts 37 of 62 Clippy rules; the catalog holds 104 of 142.

5. **Nothing runs the real binary end to end the way a user does.** No test scans this repository through the CLI and checks the report's contract, no test drives the interactive report through a real terminal beyond one fixture at 100 columns, no test replays the Action's bash steps, and the corpus is never scanned outside its env-gated precision tests. The 13 interactive-report defects the exploration found (`tasks/e2e-catalog/tui.md` section 4) shipped because nothing could see them.

**Why now:** The gap-closure PRD (`tasks/prd-react-doctor-gap-closure.md`) is DONE and unreleased since `v0.7.0`. Problems 1 and 2 break the two promises the next release headlines: the redaction of detected credentials (its US-001) and a commit and agent loop that never misfires (its EP-002). Shipping them as they are would publish a secret leak and a hook that cannot see a secret. The exploration that found them is the end-to-end suite this PRD asks for, run once by hand: making it permanent costs less than the next audit.

## Overview

The PRD ships in three releases, in the order the problems were ranked. Release 1 fixes what is broken, each fix landing with a Rust regression test in the test file that owns its module. Release 2 builds an end-to-end suite in bun and TypeScript under `tests/e2e/`, modeled on the suite of pf (`/home/arthur/dev/pf/tests/e2e`), and covers every scan argument, scope, git state and subcommand. Release 3 covers failure, the terminal and the interactive report, and real codebases: this repository, the npm launcher, the Action and the pinned corpus. Every defect Release 1 does not fix is written into the suite as a `test.failing` row tagged with its ledger identifier, so a later fix flips it green.

Key decisions, each justified in the decision log behind this PRD and summarized here:

- **A suggestion that would carry a credential is withheld, not masked.** A masked machine-applicable fix would rewrite source with `<redacted>` in it. Credentials in message, help and note text are masked once, at normalization, so every renderer inherits it.
- **The repository pass gets a git view of every snapshot.** The snapshot already owns an index of the base commit (`src/baseline.rs:219`); the repository pass runs git with that index, the real git directory and the snapshot as work tree. A smoke test on 2026-10-20 confirmed `git ls-files` lists the base tree that way.
- **The five product decisions taken on 2026-10-20 are applied as decided.** The README's network sentence is narrowed to what holds (rust-doctor opens no connection; Cargo may fetch missing dependencies; `CARGO_NET_OFFLINE=true` forbids it) rather than forcing `--offline`, which would break the first scan of a fresh clone. The scan removes a `Cargo.lock` it created. The pre-commit hook passes `--blocking warning`, like the Stop hook. A missing `--advisory-db` path exits 2 and `/dev/null` stays the documented off switch. The credential example of the gap-closure PRD is rewritten and the rule's help names database URLs.
- **The suite transposes pf, with two departures.** It keeps pf's isolated spawn, tmux driver, weighted shards and retry-once-with-annotation. It adds Bun's built-in pseudo-terminal (`Bun.spawn({ terminal })`, verified on bun 1.4.2) for tests that only need terminal streams, instead of a Python helper, and it drops pf's PGSO corpus, ACP, MCP and fake model gateway, which have no counterpart here.
- **The suite asserts contracts, never this repository's defects.** On the self-scan it asserts the 19 invariants of `tasks/e2e-catalog/real-codebases.md` section 2 and never the score or a count, as `docs/testing.md:5-17` requires.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Catalogued scenarios automated in `tests/e2e/` (0 of 946 today) | Release 2 files: 639 scenarios | All 946 plus the 19 self-scan invariants and the launcher and Action matrices |
| Confirmed high-severity defects open (5 today: `failures/BUG-01`, `scopes/BUG-A`, `scopes/BUG-D`, `subcommands/F1`, `subcommands/F2`) | 0 | 0 |
| Defects known and untracked by any test (every one of the ledger today) | 0: each is a fix or a `test.failing` row | 0 |
| End-to-end wall time on a pull request, release build plus slowest shard | at most 10 minutes | at most 10 minutes with twice the scenarios |
| Self-scan of this repository: P1 findings | 0 | 0 |

## Target Users

### Maintainer
- **Role:** Arthur, sole maintainer, who releases rust-doctor and certifies epics with `/review-epic`.
- **Behaviors:** Ships through PRDs and agents; relies on `cargo test`, the dogfood workflow and `smoke:packed` to catch regressions.
- **Pain points:** Defects that only show on a real repository, a real git history or a real terminal surface after release, through manual exploration. A green suite said nothing about problems 1 to 3.
- **Current workaround:** Ad hoc exploratory sessions like the one of 2026-10-20, whose scripts and outputs are lost with the session.
- **Success looks like:** A defect of that class fails a named e2e test on the pull request that introduced it.

### Coding agent in the loop
- **Role:** Claude Code, Codex or Cursor running rust-doctor through the skill, the Stop hook or the pre-commit hook, and agents implementing this repository's epics.
- **Behaviors:** Runs `--yes --json`, reads exit codes and `delta`, follows `rules explain` and `why`.
- **Pain points:** A pre-commit hook that blocks nothing and cannot see a staged secret; a baseline that blames a no-op change; relative paths that fail where absolute ones work.
- **Current workaround:** None; the agent trusts the exit code.
- **Success looks like:** Every exit code, delta and message the agent reads is true, and stays true release after release.

### Developer adopting the tool
- **Role:** A developer, often JavaScript-first, who runs `npx rust-doctor`, installs the hooks with `setup` and adds the Action.
- **Behaviors:** Runs it in a terminal, presses Ctrl-C, scans fresh clones, uses relative paths.
- **Pain points:** A secret printed in a CI log, a lockfile appearing in the worktree, a score of 100 on a scan that never compiled, a terminal left without a cursor.
- **Current workaround:** None visible; the defect is discovered by accident.
- **Success looks like:** The tool behaves as its README says, verified on every release.

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **Cargo:** tests itself with `cargo-test-support` and snapbox: hermetic per-test `CARGO_HOME` and target directories, normalized paths, wildcard assertions. Praised for hermeticity, criticized for its size. rust-doctor's `tests/support/mod.rs` already applies the same isolation to its Rust tests.
- **trycmd:** runs the real binary from case files with regenerable goldens; its documentation sends interactive programs elsewhere. Golden files fit `--json` with a schema version, not a TUI or a self-scan.
- **Clippy:** `ui_test` goldens are precise, but every wording change re-blesses hundreds of files. This is why the suite asserts invariants and stable markers rather than full goldens.
- **pf** (`/home/arthur/dev/pf/tests/e2e`, 73 test files): a private bun package beside the compiled binary, a fabricated HOME and scrubbed environment per test (`tests/evals/eval-helpers.ts:155-192`, `tests/e2e/e2e-helpers.ts:40-69`), a tmux driver with `waitForPane` polling every 25 ms (`tests/e2e/tmux-helpers.ts:796-845`), one bun process and one tmux server per file with one retry and a warning on pass-on-retry (`tests/e2e/ci-run-files.sh`), and weighted shards that refuse a missing or stale weight (`tests/e2e/ci-shards.ts`). This is the model the owner asked for.
- **Market gap:** none of the Rust linters above drives its terminal UI through a real terminal in CI, or replays its GitHub Action's shell logic locally.

### Best Practices Applied
- Hermetic runs: private HOME, TMPDIR and target directory, an explicit environment allowlist, git identity and configuration pinned (pf; Cargo's testsuite).
- Invariants over goldens wherever output is volatile; goldens only for versioned JSON (Clippy's re-bless cost; `docs/testing.md:5-17`).
- A flake stays visible: a retry that passes emits a CI warning (pf `ci-run-files.sh`). Bun's own `--retry` counts a retried pass as a plain pass (`bun test --help` on 1.4.2), so it is not used for this.
- Redact at one serialization boundary, not per renderer: suggestions and SARIF fixes are where linters republish secrets they mask elsewhere (web research, gitleaks `--redact` documentation).
- `test.failing` passes only while its body throws, which turns a known defect into a test that flips when fixed (bun documentation, page `test/writing-tests.mdx` of its docs tree; verified on 1.4.2).
- `NO_COLOR` counts only when present and not empty (no-color.org, fetched 2026-10-20).
- Ubuntu 24.04 restricts unprivileged user namespaces, so `unshare -rn` fails on hosted runners until `kernel.apparmor_restrict_unprivileged_userns=0` is set ([runner-images #10443](https://github.com/actions/runner-images/issues/10443)).
- `--locked` refuses a missing lockfile and `--offline` resolves differently; `--frozen` is both; `--lockfile-path` is unstable (Cargo documentation). Removing the lockfile the scan created is the only stable way to leave the tree as found without changing what is linted.

*The five catalogs and their defect ledger are in `tasks/e2e-catalog/`.*

## Assumptions & Constraints

### Assumptions (to validate)
- **HIGH:** The repository pass run with the snapshot's index, the real git directory and the snapshot work tree returns the same findings as a scan of a real checkout of that commit. Evidence: `git ls-files` under that configuration listed the base tree on 2026-10-20; `git check-ignore` under the same view is unverified. US-002 carries an oracle criterion comparing the two.
- **HIGH:** Bun 1.4.2's pseudo-terminal (`Bun.spawn({ terminal })`) is stable enough for terminal-profile tests. Evidence: a local probe saw `tput cols/lines` of 97x31, both streams as terminals and the child's exit code. The version that introduced it is undocumented, so the suite pins bun 1.4.2 and keeps tmux for every rendering test.
- **MEDIUM:** Duplicate signal deliveries arrive within 500 ms of each other. Evidence: a terminal Ctrl-C reaches the binary from the terminal and from the launcher's forward in the same scheduling window; a person pressing Ctrl-C twice to force an exit takes longer.
- **MEDIUM:** Three shards keep the end-to-end critical path under 10 minutes. Evidence: local measurements (cold self-scan 5.8 s, Action replay 11.7 s, mini-repository scans 0.08 to 0.25 s) scaled by 3 to 4 for a 4-vCPU runner (`tasks/e2e-catalog/real-codebases.md` section 5).
- **LOW:** Users accept a pre-commit hook that blocks on warnings in the lines a commit adds, given `--no-verify` and that pre-existing findings never block.

### Hard Constraints
- No network in any test, no HTTP client, no telemetry (`AGENTS.md`, Trust boundary). Tests set `CARGO_NET_OFFLINE=true`.
- Every scan in a test runs in a copy of its workspace with its own `CARGO_TARGET_DIR`; the live repository is never scanned in place by the e2e suite, and never written.
- Production Rust carries no `unwrap`, `expect`, `panic!` or `dbg!`; every module stays under 1000 lines per file and 500 per `impl` block. Files near the bound that this PRD touches: `src/render.rs` 969, `src/report.rs` 948, `src/execution.rs` 912, `src/handoff.rs` 839, `src/baseline.rs` 725. A fix that grows them extracts first.
- `SCHEMA_VERSION` (`src/report.rs:44`, 23) moves only if a story changes the report shape. No story in this PRD adds a field or an enum value: they change values (`score: null`, a withheld `suggestion`) that are already nullable.
- `--json` stays workspace-relative, with no absolute path, environment value or user data.
- Credentials used by tests are built at runtime; no credential-shaped literal is committed, including in this PRD and the catalogs.
- `bun` under `npm/rust-doctor/` and `tests/e2e/`, never `npm` or `pnpm` (AGENTS.md).
- Releases ship in order: Release 2 starts when Release 1 is certified.

## Quality Gates

These commands must pass for every user story:
- `cargo build --release` - the crate compiles
- `cargo test` - the full Rust suite, including the size bounds, the no-unwrap scan and the docs contract
- `cargo clippy --all-targets --no-deps -- -D warnings` - clean, no exceptions
- `cd npm/rust-doctor && bun test tests` - the launcher still resolves and runs the binary
- `bash scripts/verify-doc-budgets.sh` - every document within its word ceiling, every new document registered
- `bash scripts/verify-doc-refs.sh` - every path the documents cite resolves
- `cargo run --release -- . --yes --verbose` - the tool scans its own repository without error

From US-015 on, also:
- `cd tests/e2e && bun install --frozen-lockfile && bun run typecheck` - the suite type-checks
- `cd tests/e2e && bun test known-bugs.test.ts ci-shards.test.ts <every file the story adds or changes>` - the ledger stays exact and the story's files pass

For stories that change terminal output (US-001, US-008, US-010, US-014):
- Render the changed surface in a real terminal at widths 40, 80 and 140 and compare it with the test the story adds.

## Epics & User Stories

### EP-001: Nothing the scan sees leaks, and every commit-time rule sees the commit

**Release:** 1. No surface republishes a detected credential or an absolute path, and the staged scan and both sides of a baseline run every pass on the tree they judge.

**Definition of Done:**
- A credential that a Clippy suggestion would carry appears in no JSON field, no linear line, no TUI frame, no handoff prompt and no markdown report.
- A staged access key fails nothing silently: `--staged --scope lines` reports it.
- A baseline comparison of a branch with no change reports zero introduced and zero fixed, on a fixture with repository findings and on this repository.
- No report on the failure fixtures contains an absolute path.

#### US-001: Withhold suggestions that carry a detected credential, and mask credentials in diagnostic text
**Description:** As a developer whose repository holds a leaked key, I want no part of the report to reprint that key, including Clippy's suggested fix, so that a terminal, an agent transcript or a CI log never spreads it.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None
**Ledger:** `failures/BUG-01`

**Acceptance Criteria:**
- [ ] `select_suggestion` (`src/report/normalize.rs:434`) returns no suggestion when `repo_hygiene::credential_ranges` matches its replacement text or the source text of the span it replaces
- [ ] Diagnostic message, help and every note pass through credential masking (the first four characters followed by `<redacted>`, as the code frame does) once, during normalization, before any renderer reads them
- [ ] Given a git fixture where a Slack bot token built at test time is passed through `.to_string()` and triggers `clippy::unnecessary_to_owned`, when scanned with `--json`, then that diagnostic's `suggestion` is null and no string in the document contains the token's body
- [ ] Given the same fixture, when scanned with `--verbose`, then no `Replace with:` line is printed for that diagnostic and neither stdout nor stderr contains the token's body
- [ ] Given the same fixture, when the viewer's detail lines (`src/tui/screens/viewer.rs:226`), the handoff prompt (`src/handoff.rs`) and `report markdown` output are built, then none contains the token's body
- [ ] Given a suggestion replacing a string literal that matches no credential pattern, when scanned, then the suggestion is published unchanged (no over-redaction)
- [ ] The regression tests live in `tests/repo_hygiene.rs` and `src/render/tests.rs`

#### US-002: Give the repository pass a git view of the baseline base side
**Description:** As a pull request author, I want the baseline to compare repository findings on both sides, so that a file I did not touch is never reported as introduced and a secret I removed is reported as fixed.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None
**Ledger:** `scopes/BUG-A`, consequence `real/B1a`

**Acceptance Criteria:**
- [ ] The base side's repository pass runs git with `GIT_INDEX_FILE` set to the snapshot's index (`src/baseline.rs:219`), `--git-dir` set to the real repository's git directory and `--work-tree` set to the snapshot tree, and reads file contents from the snapshot tree
- [ ] The snapshot's index stays on disk until the base side's scan has finished, and is removed with the snapshot
- [ ] `ignored_lockfile` and `unignored_build_output` answer from the same view, so all five `rust_doctor::repo::*` rules pair across the two sides
- [ ] The base-side cache key (`target/rust-doctor/cache`) changes version, so no base side cached before this change, which holds zero repository findings, is reused
- [ ] Given a repository whose base commit tracks a credential-shaped literal and a branch with no change, when `--scope baseline --base <base sha> --blocking warning --json` runs, then `delta.summary.introduced` is 0, the repository finding is pre-existing, and the exit code is 0
- [ ] Given the branch deletes that file, when compared, then `delta.summary.fixed` is 1 and the fixed finding is the repository finding
- [ ] Given the branch adds a second credential file, when compared, then exactly one repository finding is introduced
- [ ] Given the fixture's base commit, when its base side's repository findings are compared with a full scan of a real checkout of that commit, then the codes, paths and lines are identical
- [ ] Given the snapshot index cannot be read by git, when the base side runs, then the scan fails with the existing `baseline-scan-incomplete` error and exit 2, rather than reporting every head-side repository finding as introduced
- [ ] `tests/precision_controls.rs:250-260` asserts `introduced == 0` beside its existing `fixed` and `pre_existing` assertions, and the new cases live in `tests/baseline_kernel.rs`

#### US-003: Run the repository pass on the staged index, and read the staged configuration
**Description:** As a developer committing through the pre-commit hook, I want the staged scan to judge exactly what the commit records, secrets and configuration included, so that the hook cannot pass a staged secret or obey a file the commit does not contain.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002
**Ledger:** `scopes/BUG-D`, `subcommands/F3`, `subcommands/F10`

**Acceptance Criteria:**
- [ ] Under `--staged`, the repository pass enumerates the staged entries through the same git view as US-002, with the staged index and the staged snapshot as work tree
- [ ] Given a staged access key built at test time in `src/leak.rs` and a force-added `.env` holding a token, when `--staged --scope lines --json` runs, then `rust_doctor::repo::hardcoded_credential` and `rust_doctor::repo::tracked_secret_file` are both reported
- [ ] Given `rust-doctor.toml` staged with `blocking = "warning"` and changed to `blocking = "none"` in the working tree, when a staged scan runs, then the gate applies `warning`
- [ ] Given an untracked `rust-doctor.toml`, when a staged scan runs, then it is not read
- [ ] Code frames of a staged scan are drawn from the staged snapshot, not the working tree
- [ ] Given a conflicted index, when a staged scan runs, then it fails with the existing `index-conflicted` error, unchanged
- [ ] The regression tests live in `tests/repo_hygiene.rs` and `tests/git_change_scope_product_proof.rs`

#### US-004: Scrub absolute paths that Cargo and rustup print
**Description:** As a developer whose report is posted to a pull request, I want no absolute path in it, so that the report discloses neither my machine's layout nor a CI runner's.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None
**Ledger:** `failures/BUG-02`

**Acceptance Criteria:**
- [ ] `sanitize_text` (`src/report/sanitize.rs:32`) rewrites paths under the resolved `RUSTUP_HOME` and `CARGO_HOME` to `$RUSTUP_HOME/...` and `$CARGO_HOME/...`, and any other absolute path outside the workspace root to `<external>/<file name>`
- [ ] Given a workspace whose manifest lists `members = ["../gone"]`, when scanned with `--json` and without, then neither output contains an absolute path, and the message still names `gone/Cargo.toml`
- [ ] Given `RUSTUP_HOME` set outside HOME and a toolchain error, when scanned, then the error text contains no absolute path
- [ ] Given text holding a relative path or a URL, when sanitized, then it is unchanged
- [ ] A test in `tests/scan_failures.rs` walks every string of the JSON and fails on any substring equal to the test's temporary root, HOME, `RUSTUP_HOME` or `CARGO_HOME`

---

### EP-002: The scan leaves the workspace as it found it, and says what happened

**Release:** 1. A scan mutates nothing it scans, refuses inputs it cannot honor, publishes no score it did not compute, and ends cleanly whatever signal ends it.

**Definition of Done:**
- Two consecutive scans of a lockfile-less binary crate produce identical reports and leave the tree byte-identical.
- `--advisory-db` on a missing path, and any subcommand given a missing PATH, exit 2 and write nothing.
- `hook install git crates/x` behaves exactly like its absolute-path form.
- No report with an unlinted package or an interrupted scan carries a numeric score for what was not linted.
- A terminal Ctrl-C through the npm launcher yields a parseable `interrupted` report, and no snapshot outlives its run.

#### US-005: Remove the Cargo.lock a scan created
**Description:** As a developer scanning a library or a fresh clone without a lockfile, I want the scan to leave my tree as it found it, so that a second scan agrees with the first and no untracked file appears.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None
**Ledger:** `failures/BUG-03`, `real/B3`, `real/B3b`

**Acceptance Criteria:**
- [ ] Before Clippy runs, execution records whether `Cargo.lock` exists at the workspace root and, when it does, keeps its bytes
- [ ] After Clippy ends, whatever its outcome (success, failure, deadline, interrupt), a lockfile that did not exist before is removed, and an existing lockfile Cargo rewrote gets its original bytes back
- [ ] Given a binary crate without a lockfile, when scanned twice, then both reports are identical once `timings` is removed, both carry `rust_doctor::cargo::missing_lockfile`, and the tree fingerprint after each scan equals the one before
- [ ] Given a workspace with a committed lockfile Cargo does not need to change, when scanned, then the lockfile is byte-identical and its modification time unchanged
- [ ] Given the removal fails (read-only directory), when the scan ends, then stderr carries one line naming the file and the report is otherwise unchanged
- [ ] `docs/subsystems/execution.md` states the rule and the accepted race: a lockfile another process creates during the scan is removed with it
- [ ] The regression tests live in `tests/scan_failures.rs`

#### US-006: Refuse a missing `--advisory-db` path, and keep `/dev/null` as the off switch
**Description:** As a CI maintainer pinning a RustSec database, I want a wrong path to fail loudly, so that I never believe advisories are checked when they are not.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None
**Ledger:** `failures/BUG-07`

**Acceptance Criteria:**
- [ ] Given `--advisory-db <path>` naming nothing on disk, when run, then the process exits 2 before any scan, stderr holds one line naming the flag and the path as given, and stdout is empty even with `--json`
- [ ] Given `--advisory-db <path>` naming an unreadable file or directory, when run, then the process exits 2 the same way
- [ ] Given `--advisory-db /dev/null`, when scanned, then the behavior is today's: the advisory rules are marked `not_evaluated` with `advisory-db-missing` and the exit code follows the gate
- [ ] The `--help` text of `--advisory-db` says that `/dev/null` disables the check
- [ ] The regression tests live in `tests/advisory_database.rs`

#### US-007: Resolve every PATH argument before git sees it, and refuse one that does not exist
**Description:** As an agent installing hooks in a monorepo, I want `crates/x` to work exactly like its absolute form, and a typo to fail without creating directories, so that setup is predictable.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None
**Ledger:** `subcommands/F1`, `subcommands/F5`

**Acceptance Criteria:**
- [ ] Every subcommand taking a PATH (the scan, `hook install git|claude|cursor`, `ci install`, `setup`, `skill install`, `rules explain`, `why`) resolves it against the current directory once, at the CLI boundary, before discovery or any git call
- [ ] `git_command` (`src/git.rs:242`) never combines `current_dir` with `-C` of the same relative path
- [ ] Given a repository with a member at `crates/x`, when `hook install git crates/x`, `ci install crates/x --dry-run` and `setup crates/x --dry-run` run from the repository root, then each succeeds with the output its absolute-path form gives
- [ ] Given a PATH that does not exist, when any of those subcommands runs, then it exits 2 with one stderr line naming the path as given, and the tree fingerprint of the current directory is unchanged
- [ ] Given a PATH containing spaces and non-ASCII characters, when installers run, then they succeed
- [ ] The regression tests live in `tests/setup_cli.rs`, `tests/agent_hooks.rs` and `src/git/tests.rs`

#### US-008: Make `rules explain`, `why` and the scope line tell the truth about their inputs
**Description:** As an agent explaining a finding, I want `rules explain` and `why` to read the configuration and report a scan reads, and a failed scope to say which scope failed, so that the explanation matches the verdict.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-007
**Ledger:** `subcommands/F4`, `subcommands/F6`, `scopes/BUG-B`

**Acceptance Criteria:**
- [ ] `rules explain RULE PATH` resolves PATH to its Cargo workspace root through the scan's discovery and reads that root's `rust-doctor.toml`; a member-level file, which scans ignore, is not read
- [ ] Given a workspace whose root configuration sets a rule to `error` and PATH naming a member, when explained, then the effective level is `error (config-rule)`, as a scan applies it
- [ ] `why --report FILE` refuses a document without `schema_version` or with a different one, with the message and exit code 2 that `report markdown` gives (`src/report_command.rs:50-56`)
- [ ] Given `--scope files --base HEAD~1`, when run without `--json`, then the scope line reads `Scope: files (not resolved)` above the failure, never `Scope: full codebase`
- [ ] The JSON of a failed scope is unchanged (`scope: null`)
- [ ] The regression tests live in `tests/explain_cli.rs`, `tests/ci_and_report_markdown.rs` and `tests/git_change_scope_product_proof.rs`

#### US-009: Publish no score for what was never linted or was interrupted
**Description:** As a consumer reading `.audit.score.value`, I want a scan that linted nothing, or was stopped, to publish no score, so that 100 always means a clean result.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None
**Ledger:** `failures/BUG-05`, `failures/BUG-06`

**Acceptance Criteria:**
- [ ] When Cargo exits before emitting `build-finished` (`src/execution/clippy.rs:226-234`), every selected package is unlinted: `unlinted: true`, `score: null`, and the report carries `packages-unlinted`
- [ ] Given a crate with `rust-version` above the active toolchain, when scanned, then its package score is null and the report names it unlinted
- [ ] Given a scan interrupted by SIGINT, when its report is read, then `audit.score` is null, like every other failed report
- [ ] Given every package unlinted, when the linear report is printed, then the score block shows no number and says the scan could not score
- [ ] Given one broken member among linted ones, when scanned, then today's behavior holds: `tests/scan_failures.rs` `a_broken_member_leaves_the_others_linted_and_is_named` stays green
- [ ] `SCHEMA_VERSION` is unchanged and the frozen v7 projection (`tests/support/mod.rs:200-267`) still holds
- [ ] The regression tests live in `tests/scan_failures.rs` and `src/report/tests/normalization.rs`

#### US-010: End cleanly on SIGHUP and on duplicate signals, sweep dead snapshots, restore the terminal
**Description:** As a developer who closes a terminal tab or presses Ctrl-C through `npx`, I want the scan to stop its children, clean up and leave a usable terminal, so that nothing keeps compiling in the background and no temporary tree accumulates.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None
**Ledger:** `failures/BUG-04`, `real/B2`, `scopes/K4`, `tui/BUG-06`

**Acceptance Criteria:**
- [ ] SIGHUP is handled like SIGTERM (`src/execution/interrupt.rs`): child process groups are killed, snapshots removed, the report published when stdout is open, and the exit status is 130, the one status the exit-status help (`src/main.rs:49-55`) gives a scan a signal stopped; that help names SIGHUP beside SIGINT and SIGTERM
- [ ] A second signal arriving within 500 ms of the first is the same delivery: the scan ends with the `interrupted` report and exit 130; a second signal after 500 ms keeps today's immediate exit
- [ ] Given the npm launcher and a SIGINT sent to the whole process group during Clippy, when the scan ends, then stdout holds a parseable report with error `interrupted`, the exit status is 130, and no `rust-doctor-baseline-*` directory remains in `TMPDIR`, in full and in baseline scope
- [ ] At the start of a baseline or staged scan, every `rust-doctor-baseline-<pid>-<n>` directory under the temporary root whose pid is not alive and whose owner is the current user is removed; a live pid's directory is never touched
- [ ] Given a sweep that cannot remove a directory, when the scan starts, then it continues without error
- [ ] When the interactive report owns the terminal, an interrupt restores the terminal mode and shows the cursor before the process exits
- [ ] `docs/subsystems/execution.md` documents the three signals, the 500 ms window and the sweep
- [ ] The regression tests live in `tests/scan_failures.rs`, `tests/local_cli_experience.rs` (PTY) and `npm/rust-doctor/tests`

---

### EP-003: Hooks that enforce, and promises that hold

**Release:** 1. The pre-commit hook blocks what the Stop hook blocks, and every promise the README, the skill, the help and the generated workflow make is true of the binary.

**Definition of Done:**
- A staged `todo!()` fails `git commit` through the installed hook, and `--no-verify` passes it.
- The self-scan of this repository reports no repository finding and no P1 finding.
- The README's network sentence, the `--advisory-db` and `--base` help, and the `ci install` template match the code.

#### US-011: Make the pre-commit hook block on warnings, and let a failed scan through
**Description:** As a developer whose agent has no Stop hook (Codex), I want the installed pre-commit hook to refuse a commit that adds a warning, so that the hook enforces what it reports.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003, US-007
**Ledger:** `subcommands/F2`

**Acceptance Criteria:**
- [ ] The hook body runs `rust-doctor <path> --yes --staged --scope lines --blocking warning` and the `hook install git` help (`src/hook.rs:50-52`) says so
- [ ] Given a staged change adding a catalogued warning, when `git commit` runs through the hook, then the commit is refused and the finding is printed
- [ ] Given the same change, when `git commit --no-verify` runs, then it succeeds
- [ ] Given a staged change that adds no finding in a file with pre-existing findings, when committed, then the commit succeeds
- [ ] Given the scan itself fails (exit 2), when the hook runs, then the commit goes through and stderr says the commit was not judged, as when `rust-doctor` is missing from `PATH`
- [ ] `hook install git --update` rewrites a hook an older release wrote to the new body, and leaves a foreign hook untouched
- [ ] `skills/rust-doctor/SKILL.md` and `README.md` describe the blocking, and `tests/skill_contract.rs` passes
- [ ] The regression tests live in `tests/agent_hooks.rs` and `tests/setup_cli.rs`

#### US-012: Rewrite the gap-closure PRD's credential example, and name database URLs in the rule's help
**Description:** As the maintainer, I want this repository's own scan to carry no repository finding and the rule's help to describe what the rule judges, so that the self-scan gate can require zero P1 findings.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002
**Ledger:** `real/D2`, `real/D3`

**Acceptance Criteria:**
- [ ] `tasks/prd-react-doctor-gap-closure.md:697` holds no credential-shaped literal and points to `tests/fixtures/regressions/hardcoded_credential--deployed-database-password/` for the example
- [ ] The help of `rust_doctor::repo::hardcoded_credential` (`src/policy/catalog/native.rs:174`) names database URL passwords among the forms it judges, and `rules list --json` and `rules explain` publish it
- [ ] `tests/corpus.json` is regenerated as `docs/catalog-and-corpus.md` prescribes, and the README rule count is unchanged
- [ ] Given this repository after the change, when scanned, then no `rust_doctor::repo::*` finding is reported
- [ ] Given the fixture, when scanned, then the finding is still reported
- [ ] Given a baseline from the commit before the rewrite, when compared, then the credential finding is reported as fixed

#### US-013: Narrow the network promise to what holds, document `CARGO_NET_OFFLINE`, and keep git filters off the network
**Description:** As a security-minded user, I want the documents to say exactly when a scan can reach the network and how to forbid it, so that the promise survives an audit.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None
**Ledger:** `failures/BUG-09`, `real/B3a`

**Acceptance Criteria:**
- [ ] `README.md:12` says that rust-doctor opens no connection itself, that Cargo downloads missing dependencies as `cargo build` does, and that `CARGO_NET_OFFLINE=true` forbids it; `skills/rust-doctor/SKILL.md` and the scan's long help say the same
- [ ] Snapshot materialization for baseline and staged scans runs `git checkout-index` with `GIT_LFS_SKIP_SMUDGE=1` and `-c filter.lfs.process= -c filter.lfs.smudge= -c filter.lfs.required=false`
- [ ] Given a repository with an LFS-tracked file and a fake `git-lfs` on `PATH` that records every invocation, when a baseline and a staged scan run, then the fake is never invoked
- [ ] Given `CARGO_NET_OFFLINE=true` and an uncached dependency, when scanned, then the scan is incomplete with Cargo's offline error quoted in a bounded report error, and does not hang
- [ ] The release checklist (`docs/publishing.md`) names the website copy of the promise as a page to align
- [ ] The regression tests live in `tests/baseline_kernel.rs`

#### US-014: Correct drifted documents, and honor an empty `NO_COLOR`
**Description:** As a reader of the workflows, the template and the help, I want each to describe the code as it is, so that no one follows a sentence the binary contradicts.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None
**Ledger:** `real/D1`, `real/D4`, `real/D5`, `real/D6`, `subcommands/F20`, `tui/BUG-11`

**Acceptance Criteria:**
- [ ] "temporary worktree" becomes "temporary snapshot" in `.github/workflows/dogfood.yml:8`, `.github/workflows/ci.yml:101` and the template in `src/ci.rs:449-452`, and the `ci install` output test is updated
- [ ] `.github/workflows/ci.yml:33` states no rule count, and `.github/workflows/corpus.yml` describes which subjects compile and no run length it cannot recompute
- [ ] `tests/fixtures/performance/baseline.json` is re-measured under toolchain 1.97.1 with `cargo test --release --test performance -- --ignored --nocapture` (`docs/subsystems/execution.md:112-113`)
- [ ] `skills/rust-doctor/SKILL.md` says the agent-hook installers merge into existing settings files rather than "never write over a file it did not create"
- [ ] The `--base` help names the accepted selector forms and gives `--base "$(git rev-parse HEAD~1)"` as the way to name a relative commit
- [ ] Given `NO_COLOR=` (empty), when the terminal profile resolves (`src/terminal_profile.rs:121`), then color stays on; given `NO_COLOR=0`, then color is off
- [ ] `bash scripts/verify-doc-refs.sh` and `tests/docs_contract.rs` pass with every changed document

---

### EP-004: A hermetic end-to-end harness that runs the real binary

**Release:** 2. A bun and TypeScript suite under `tests/e2e/` that spawns the release binary in isolated copies, drives its terminal through tmux and a pseudo-terminal, builds git histories on demand, tracks every known defect, and runs in CI on every push and pull request.

**Definition of Done:**
- `cd tests/e2e && bun test` runs locally after `cargo build --release`, with no network and no write outside its scratch root.
- CI builds the release binary once and runs the suite in weighted shards; a pass on retry produces a visible warning.
- Every ledger entry of `tasks/e2e-catalog/README.md` is either fixed or referenced by a `test.failing` row, and a test enforces it.

#### US-015: Scaffold `tests/e2e` with hermetic spawn helpers, proven on the CLI surface
**Description:** As the maintainer, I want one helper that spawns the real binary in a sealed environment and returns its exit, streams and JSON, so that every later test file is a few lines of scenario.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `tests/e2e/` is a private bun package: `package.json` with `"packageManager": "bun@1.4.2"`, exact dev dependency versions, a `typecheck` script, a committed `bun.lock`, a strict `tsconfig.json`, and a `bunfig.toml` setting the default test timeout
- [ ] `e2e-helpers.ts` resolves the binary from `RUST_DOCTOR_E2E_BIN` or `target/release/rust-doctor`, and throws "run cargo build --release first" when it is absent
- [ ] `runDoctor(args, options)` spawns with an environment built from an allowlist (`PATH`, `HOME`, `RUSTUP_HOME`, `CARGO_HOME`, `LC_ALL=C`, a pinned `TERM`, `GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`, `CARGO_NET_OFFLINE=true`), a private HOME and TMPDIR per test, a `CARGO_TARGET_DIR` per workspace, and a watchdog; it returns exit code, signal, stdout, stderr, parsed JSON and duration
- [ ] Every entry of `NON_INTERACTIVE_MARKERS` and `CI` is absent from the environment unless a test sets it; the list is parsed from `src/terminal_profile.rs` at load time, and the load fails if the parse finds none
- [ ] `normalizeReport` removes `timings`, `toolchain` and `scan.advisory_db`, and scans default to `--advisory-db /dev/null` unless a test opts out
- [ ] `treeFingerprint(dir)` hashes every file, `.git/index` bytes and modification time included, without spawning git
- [ ] `copyWorkspace(fixture)` copies a `tests/fixtures/` workspace without `target/` or an untracked `Cargo.lock`
- [ ] `known-bugs.json` lists every open ledger entry of `tasks/e2e-catalog/README.md`, and `known-bugs.test.ts` fails when a `test.failing` title carries an identifier the ledger lacks, or an open entry is referenced by no `test.failing` row
- [ ] `cli-surface.test.ts` automates CL-01 to CL-13 of `tasks/e2e-catalog/subcommands.md` section 1
- [ ] `Cargo.toml` `exclude` lists `tests/e2e/`, and `cargo package --list` does not list it
- [ ] Given the binary is absent, when any test file loads, then it fails with the build instruction
- [ ] Given a spawned process outlives its watchdog, when the test ends, then the process is killed and the test fails naming it

#### US-016: Build the git scenario builder
**Description:** As a test author, I want to declare a git history in a few chained calls with stable commit hashes, so that every scope and git-state scenario is reproducible.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] `git-scenario.ts` implements the builder of `tasks/e2e-catalog/scopes.md` "Git scenario builder the suite needs": templates and fixtures, commits with a fixed identity and date, branch, stage, unstaged, untracked, symlink, submodule, clone (depth, single branch), set-head, worktree, detach, orphan and criss-cross helpers
- [ ] Git runs with hooks disabled and `protocol.file.allow=always`, from the same environment allowlist as `runDoctor`
- [ ] `credential(kind)` builds an access key, a token or a database URL at runtime; no credential-shaped literal is committed under `tests/e2e/`
- [ ] `racyIndex()` produces a racily clean index, and `waitForSnapshot()` resolves once a `rust-doctor-baseline-*` directory exists in the test's TMPDIR
- [ ] `git-scenario.test.ts` proves the base commit of the `mini` template has the same hash on two runs
- [ ] Given a helper called out of order (staging before any commit), when built, then it throws naming the step

#### US-017: Port the tmux driver, add a pseudo-terminal helper and a fake tool PATH
**Description:** As a test author, I want to drive the interactive report like a person and intercept every external program it launches, so that terminal tests are deterministic and never reach a real agent, clipboard or browser.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] `tmux-helpers.ts` ports pf's `TmuxSession` (`/home/arthur/dev/pf/tests/e2e/tmux-helpers.ts`): one session per test on a short relative socket, explicit width and height, `escape-time 0`, `window-size manual`, `remain-on-exit on`, `status off`; `sendKeys`, `sendLiteral`, `capturePane` (plain, with escapes, with history), `waitForPane` polling every 25 ms with the last pane in its error, `waitForStable`, `resize`, a `pipe-pane` byte log, and an idempotent `kill`
- [ ] `pty.ts` wraps `Bun.spawn({ terminal })` with columns, rows, input, collected output and the child's exit code from `proc.exited`
- [ ] `fakes/` holds recording stand-ins for `claude`, `codex`, `cursor-agent`, `wl-copy`, `xclip`, `xsel`, `pbcopy`, `xdg-open`, `open`, `gh` and `git-lfs`, each writing its argv and stdin to a file the test reads, and PATH profiles (`none`, `clipboard`, `agents`, `all`) that exclude the machine's real tools
- [ ] `tmux-helpers.test.ts` and `pty.test.ts` prove the helpers on a shell before any rust-doctor test uses them
- [ ] Given tmux is absent and `E2E_REQUIRE_TMUX` is unset, when a TUI file loads, then it skips with one visible reason; given `E2E_REQUIRE_TMUX=1`, then it fails

#### US-018: Run the suite in CI: one release build, weighted shards, one visible retry
**Description:** As the maintainer, I want every push and pull request to run the suite against the binary under review in under 10 minutes, so that an e2e regression blocks the change that caused it.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-015, US-017

**Acceptance Criteria:**
- [ ] `.github/workflows/ci.yml` gains a `build-release` job (`cargo build --release --locked`, toolchain 1.97.1) uploading the binary as an artifact, and an `e2e` job of three shards that downloads it
- [ ] Each shard installs tmux, toolchain 1.97.1 with clippy, and bun at the version `tests/e2e/package.json` pins, then runs `bun install --frozen-lockfile`
- [ ] `ci-shards.ts` ports pf's planner (greedy, longest first, on `ci-shard-weights.json`) and fails on a test file with no weight, a weight with no file, or a duplicate; `ci-shards.test.ts` covers the planner
- [ ] `ci-run-files.sh` runs each file in its own bun process with its own tmux server, retries a failed file once after resetting that server, and prints `::warning title=E2E passed only on retry::` for a file that passed only on retry
- [ ] A gate job named `e2e` requires every shard
- [ ] Each shard enables unprivileged user namespaces (`sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0`) before the files that need a network namespace
- [ ] Scan target directories are cached per file, keyed on `Cargo.lock`, the toolchain pin and the file name
- [ ] Given a new test file without a weight, when CI runs, then the planner fails naming the file
- [ ] Given a file that fails twice, when CI runs, then its shard fails
- [ ] `docs/ci-workflows.md` describes the two new jobs

#### US-019: Document the suite and its known-defect workflow
**Description:** As a contributor or an agent, I want one document saying how to run the suite, what it may assert and how a defect moves from `test.failing` to fixed, so that the suite keeps its discipline.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] the suite document (`e2e.md` under `docs/`) covers: running locally (`cargo build --release`, then `bun install` and `bun test <file>` in `tests/e2e/`), the hermetic rules, the self-scan rule (assert contracts, never this repository's score or counts), and the defect workflow (a defect found end to end lands first as a Rust regression test; the fixing commit turns its `test.failing` row into a test and closes its ledger entry)
- [ ] the suite document is registered in `docs/doc-budgets.json` and states no number a test cannot recompute
- [ ] `AGENTS.md` gains a Commands row for the suite and a Testing line linking the suite document, within its word ceiling
- [ ] `docs/testing.md` links the suite document from its Working in `tests/` section
- [ ] Given a document that cites a path that does not exist, when `scripts/verify-doc-refs.sh` runs, then it fails

---

### EP-005: End-to-end coverage of every scan argument, scope and subcommand

**Release:** 2. Every scenario of `tasks/e2e-catalog/failures.md` sections EX, AR, OW and OC, of `scopes.md` outside section R, and of `subcommands.md` runs on every pull request.

**Definition of Done:**
- 626 catalogued scenarios run in CI (`exit-codes`, `output-streams`, seven `scope-*` files, eleven subcommand files), beside the 13 of `cli-surface.test.ts` from EP-004.
- Every row of `tasks/e2e-catalog/scopes.md` outside section R is automated exactly once; section R belongs to US-032.
- Every scenario Release 1 fixed is a plain test; every other known defect is a `test.failing` row tagged with its ledger identifier.

For every story of this epic: each catalogued scenario named becomes a test; a row the catalog marks as a defect is a plain test when Release 1 fixed it and a `test.failing` row tagged with its ledger identifier otherwise; every scenario asserts the tree fingerprint unchanged unless it is about a write.

#### US-020: Exit codes, argument validation and output contracts
**Description:** As an agent reading exit codes and JSON, I want every code and stream contract proven on the real binary, so that the precedence 2, 1, warning code, 0 never drifts.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] `exit-codes.test.ts` automates EX-01 to EX-16 and AR-01 to AR-18 of `tasks/e2e-catalog/failures.md`, the precedence matrix as one table-driven test
- [ ] `output-streams.test.ts` automates OW-01 to OW-08 and OC-01 to OC-15: one JSON document ending in one newline, empty stderr on success, `--timings`, `--verbose`, human score equal to JSON score
- [ ] Given stdout is `/dev/full`, when scanned, then the process exits 2
- [ ] Given stdout closed early by `head -c 10`, when scanned, then the process ends without a panic message

#### US-021: Scope arguments, base resolution and working-tree states
**Description:** As a developer scoping a scan to a change, I want every scope, base selector and working-tree state proven against real git histories, so that the scope I ask for is the scope I get.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-016

**Acceptance Criteria:**
- [ ] `scope-arguments.test.ts` automates sections A and A2 of `tasks/e2e-catalog/scopes.md`, and spawns no cargo
- [ ] `scope-base-resolution.test.ts` automates section C with H02, H03 and L01
- [ ] `scope-worktree-states.test.ts` automates section D with F03, F05, H05 and H07
- [ ] `scopes/BUG-C` (index stat refresh) and `scopes/BUG-G` (symlinked module) are `test.failing` rows tagged with their identifiers
- [ ] Given `--base HEAD~1`, when run, then the exit code is 2, the error is `scope/invalid-base`, and the human scope line is the one US-008 defines

#### US-022: Staged scans, baseline delta, workspace placement and repository integrity
**Description:** As a pull request author, I want the staged scan and the baseline proven on every git shape the catalog lists, so that the hook and the Action judge only what changed.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-016

**Acceptance Criteria:**
- [ ] `scope-staged.test.ts` automates section S with D11b, F01, F02, I01, H01 and H04 of `tasks/e2e-catalog/scopes.md`
- [ ] `scope-baseline-delta.test.ts` automates section E with L02, L03, H08 and F03d, including every `scopes/BUG-A` expectation as a plain test
- [ ] `scope-workspace-placement.test.ts` automates section G with H06
- [ ] `scope-repo-integrity.test.ts` automates the N and K rows and E11: fingerprint, TMPDIR sweep, signals, concurrent runs
- [ ] Every other F, H, I and L row of `tasks/e2e-catalog/scopes.md` that US-021 does not name is automated in the file of the section it exercises
- [ ] `scopes/BUG-E` (submodules) and `scopes/BUG-F` (`..` symlinks) are `test.failing` rows tagged with their identifiers
- [ ] Given a no-op branch of a repository holding a credential file, when compared with `--scope baseline --blocking warning`, then zero findings are introduced and the exit code is 0

#### US-023: `rules`, `report markdown` and `why`
**Description:** As an agent explaining findings, I want every explanation command proven against real reports and configurations, so that what it says matches what a scan does.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] `rules.test.ts` automates RL-01 to RL-33 of `tasks/e2e-catalog/subcommands.md` section 2
- [ ] `report-markdown.test.ts` automates RM-01 to RM-22, from one saved scan and static JSON documents
- [ ] `why.test.ts` automates WY-01 to WY-35
- [ ] `subcommands/F9`, `F12`, `F13`, `F14` and `F19` are `test.failing` rows tagged with their identifiers
- [ ] Given `why --report` on a document without `schema_version`, when run, then the exit code is 2

#### US-024: Git and agent hooks, installed and run
**Description:** As a developer relying on hooks, I want installation, update and execution of every hook proven through real commits and realistic agent payloads, so that the loop never misfires.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-016, US-017

**Acceptance Criteria:**
- [ ] `hook-git.test.ts` automates HG-01 to HG-30 of `tasks/e2e-catalog/subcommands.md` section 5
- [ ] `hook-agent-install.test.ts` automates HA-01 to HA-32, with HOME pointed at the test's scratch home
- [ ] `hook-run.test.ts` automates HR-01 to HR-27, feeding Claude Code and Cursor payloads on stdin
- [ ] `subcommands/F7`, `F15`, `F16` and `F17` are `test.failing` rows tagged with their identifiers
- [ ] Given a staged warning committed through the installed hook, when `git commit` runs, then it is refused (US-011)

#### US-025: `skill install`, `setup` and `ci install`
**Description:** As a developer adopting the tool, I want each installer proven idempotent, dry-run safe and honest about what it writes, so that setup never surprises.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-016, US-017

**Acceptance Criteria:**
- [ ] `skill-install.test.ts` automates SK-01 to SK-28 of `tasks/e2e-catalog/subcommands.md` section 8
- [ ] `setup.test.ts` automates SU-01 to SU-19, with agent detection driven by the fake PATH profiles
- [ ] `ci-install.test.ts` automates CI-01 to CI-34: the generated workflow parses with `Bun.YAML.parse`, passes `bash -n` on every `run:` block and pins toolchain 1.97.1
- [ ] Every `--dry-run` scenario asserts the tree fingerprint unchanged
- [ ] `subcommands/F8`, `F11` and `F18` are `test.failing` rows tagged with their identifiers

#### US-026: `rust-doctor.toml` and chained workflows
**Description:** As a team configuring rust-doctor, I want every configuration key and every chain of commands proven end to end, so that configuration and workflows compose as documented.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-016

**Acceptance Criteria:**
- [ ] `config.test.ts` automates CF-01 to CF-44 of `tasks/e2e-catalog/subcommands.md` section 11, table-driven
- [ ] `workflows.test.ts` automates WF-01 to WF-20: `setup` then a commit through the hook, `hook install claude` then `hook run claude`, `ci install` then the workflow's steps, a saved scan then `report markdown` and `why --report`
- [ ] Given an oversized (over 65,536 bytes), invalid or non-UTF-8 `rust-doctor.toml`, when scanned, then the exit code is 2 and the error names the file

---

### EP-006: End-to-end coverage of failure, the terminal and real codebases

**Release:** 3. The scan's failure modes, its terminal behavior, its interactive report, this repository, the launcher, the Action and the pinned corpus are proven on every pull request, or weekly for the corpus.

**Definition of Done:**
- Every scenario of `tasks/e2e-catalog/failures.md` (remaining sections), `tui.md` and `real-codebases.md` runs in CI.
- The self-scan's 19 invariants hold on every pull request, and none asserts the score or a count.
- A weekly job scans the 18 corpus subjects offline and compares them with their record.

#### US-027: Signals, deadlines, toolchains, workspace shapes and advisory databases
**Description:** As a user on an unusual machine or workspace, I want every failure mode to end with a truthful report and the documented exit code, so that failure is never mistaken for success.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-016, US-018

**Acceptance Criteria:**
- [ ] `signals-and-deadlines.test.ts` automates SG-01 to SG-14 of `tasks/e2e-catalog/failures.md`, synchronizing on a sleeping build script's pid file and asserting with `kill -0 -<pgid>` that no process group survives
- [ ] `toolchain-preflight.test.ts` automates TC-01 to TC-15 with PATH directories of symlinks into installed toolchains and a scratch `RUSTUP_HOME`
- [ ] `workspace-shapes.test.ts` automates WS-01 to WS-38, skipping permission cases under root with a stated reason
- [ ] `advisory-database.test.ts` automates AD-01 to AD-10
- [ ] `failures/BUG-08`, `BUG-10`, `BUG-11` and `BUG-12` are `test.failing` rows tagged with their identifiers
- [ ] Given SIGHUP during Clippy, when the scan ends, then the exit status is 130 and no snapshot remains (US-010)

#### US-028: Terminal profile, privacy, offline proof and poisoned repositories
**Description:** As a security-minded user, I want the privacy, no-network and trust promises proven by attempts to break them, so that each holds by test, not by assertion.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-017, US-018

**Acceptance Criteria:**
- [ ] `terminal-profile.test.ts` automates EN-01 to EN-22 of `tasks/e2e-catalog/failures.md` through `pty.ts`, the 26 markers as one table-driven test
- [ ] `privacy-redaction.test.ts` automates PV-01 to PV-12: one planted probe of credentials, environment values and a username-bearing HOME, searched for in JSON, linear, verbose, markdown and handoff output
- [ ] `offline-and-side-effects.test.ts` automates NW-01 to NW-05 and FS-01 to FS-06: a scan inside a network namespace equals a normal scan, and nothing is written outside the workspace, its target directory and the scan's own temporary root
- [ ] `poisoned-repository.test.ts` automates PG-01 to PG-06, each preceded by a control run proving the poisoned configuration fires under plain git
- [ ] Given a network namespace cannot be created locally, when the files run, then the namespace scenarios skip with one stated reason; in CI they run

#### US-029: Interactive report: gating, startup, keybindings and exit codes
**Description:** As a developer in a terminal, I want the report to open exactly when it should, land on the right screen and leave with the scan's exit code, so that the interactive path is as reliable as the linear one.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-017

**Acceptance Criteria:**
- [ ] `tui-gating.test.ts` automates GATE-01 to GATE-18 of `tasks/e2e-catalog/tui.md` section 3.1
- [ ] `tui-startup.test.ts` automates LAND-01 to LAND-18, each workspace's git state set explicitly
- [ ] `tui-keybindings.test.ts` automates KEY-01 to KEY-13, sending a lone Escape and waiting for the next frame before any other key
- [ ] `tui-exit-codes.test.ts` automates EXIT-01 to EXIT-12
- [ ] `tui/BUG-04`, `tui/BUG-05`, `tui/BUG-07` and `tui/BUG-12` are `test.failing` rows tagged with their identifiers
- [ ] Given `CLAUDECODE=1` on a terminal, when scanned, then the linear report prints and no menu opens

#### US-030: Interactive report: issue viewer, layout, resize and terminal hygiene
**Description:** As a developer reviewing issues, I want the viewer proven at every size and through resizes, with clean escape sequences, so that the report never corrupts the terminal.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-029

**Acceptance Criteria:**
- [ ] `tui-issues-viewer.test.ts` automates VIEW-01 to VIEW-15 of `tasks/e2e-catalog/tui.md`, on a fixture and on a clone of this repository
- [ ] `tui-layout.test.ts` automates LAY-01 to LAY-15, sizes from 1x1 to 300x80
- [ ] `tui-resize.test.ts` automates RSZ-01 to RSZ-08
- [ ] `tui-terminal-hygiene.test.ts` automates HYG-01 to HYG-08 on the `pipe-pane` byte log, never on a dead pane's cursor flag
- [ ] `tui/BUG-01`, `BUG-02`, `BUG-08` and `BUG-09` are `test.failing` rows tagged with their identifiers
- [ ] Given an external SIGTERM while the report waits, when the process exits, then the byte log ends with the cursor shown (US-010)

#### US-031: Interactive report: clipboard, CI workflow screens, handoff, interrupts and performance
**Description:** As a developer acting from the report, I want copy, workflow install and agent handoff proven against recording fakes, so that every action does exactly what its label says.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-029

**Acceptance Criteria:**
- [ ] `tui-clipboard.test.ts` automates CLIP-01 to CLIP-13 of `tasks/e2e-catalog/tui.md`, reading the payload from the fake clipboard's file
- [ ] `tui-ci-workflow.test.ts` automates CI-01 to CI-14 of section 3.8, asserting written files with `git status --porcelain --untracked-files=all` on the copy
- [ ] `tui-handoff.test.ts` automates HO-01 to HO-17 with the `agents` PATH profile
- [ ] `tui-interrupt-recovery.test.ts` automates SIG-01 to SIG-09
- [ ] `tui-performance.test.ts` automates PERF-01 to PERF-06, run serially with 50 percent headroom over the catalog's timings
- [ ] `tui/BUG-03`, `BUG-10` and `BUG-13` are `test.failing` rows tagged with their identifiers
- [ ] Given the `none` PATH profile and Enter on a copy, when the report handles it, then the payload is printed and the exit code is the scan's

#### US-032: This repository as a real codebase: the nineteen invariants
**Description:** As the maintainer, I want every pull request to scan this repository through the CLI and check its contract, so that a regression on a real 43,000-line workspace is caught where fixtures cannot reach.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-016, US-018

**Acceptance Criteria:**
- [ ] The subject is a `git clone --local --no-hardlinks` of HEAD in the test's scratch root, never the working tree
- [ ] `self-scan.test.ts`, `self-scan-parity.test.ts`, `self-scan-isolation.test.ts`, `scopes-real.test.ts`, `deadline-and-interrupt.test.ts` and `rules-catalog.test.ts` assert invariants 1 to 19 of `tasks/e2e-catalog/real-codebases.md` section 2
- [ ] Invariant 1 requires zero P1 findings and `applied_ceiling: null` (US-012)
- [ ] Invariant 13 requires `introduced == 0 && fixed == 0` for a baseline of HEAD against itself (US-002)
- [ ] `scopes-real.test.ts` also automates section R of `tasks/e2e-catalog/scopes.md`
- [ ] No test asserts the score value, the label, a count, or that a specific rule fires on this repository
- [ ] Given a seeded `unwrap()` committed on a scratch branch of the clone, when compared with `--scope baseline`, then exactly that finding is introduced

#### US-033: The npm launcher and the GitHub Action, end to end
**Description:** As a user installing through `npx` or the Action, I want the paths I actually take proven offline on every pull request, so that packaging and CI glue never break silently.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-017, US-018

**Acceptance Criteria:**
- [ ] `launcher-packed.test.ts` packs the launcher around the release artifact through `packLocal({ binaryPath })`, installs it offline, and proves stdout, stderr and exit code identical to the direct binary for exit codes 0, 1, 2, a warning code and 130, `--help`, `rules list --json` and `rules explain`
- [ ] `launcher-failures.test.ts` proves each resolution failure of `tasks/e2e-catalog/real-codebases.md` section 3 item 3 prints one line under 1,024 bytes and exits 1
- [ ] `launcher-signals.test.ts` proves single, group (US-010), double and SIGTERM delivery, and `launcher-tty.test.ts` opens the report through the launcher under tmux, resizes it and quits with `q`
- [ ] `action-replay.test.ts` runs `action.yml`'s bash steps with fake `GITHUB_*` files, a recording `gh` stub and a local bare origin, over the case matrix of section 4 (events times exit codes, shallow and full checkouts, injection attempts in `args`, unreadable report, comment API failure)
- [ ] `action-static.test.ts` checks that every `uses:` is pinned to a 40-character SHA, no `run:` contains `${{`, and `inputs.version.default` equals the crate version
- [ ] The `packed` job of `.github/workflows/ci.yml` consumes the `build-release` artifact instead of building its own binary
- [ ] Given a no-op pull request and `comment-on-clean: false`, when the Action is replayed, then no comment is posted (US-002)

#### US-034: Real external codebases on a schedule
**Description:** As the maintainer, I want the pinned corpus scanned end to end every week, offline, and compared with its record, so that a regression on real-world code surfaces within a week.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-018

**Acceptance Criteria:**
- [ ] `.github/workflows/e2e-corpus.yml` runs weekly, on manual dispatch and on pull requests touching the corpus paths `corpus.yml` already filters, restoring the corpus clone cache read-only
- [ ] `corpus-replay.test.ts` materializes each of the 18 pinned subjects into a scratch copy with `read-tree` and `checkout-index`, never scanning the cache in place, with `CARGO_NET_OFFLINE=true` after a `cargo fetch` in the copy
- [ ] Each subject's cold scan matches its record in `tests/corpus.json` (status, score, distinct count), a rerun is identical once `timings` is removed, and the copy's fingerprint is unchanged (US-005)
- [ ] `real-subject.test.ts` scans hexyl and ripgrep through the installed launcher and the direct binary and finds them identical
- [ ] Given the corpus cache is absent, when the job runs, then it fails naming the cache key rather than cloning from the network
- [ ] `AGENTS.md` and `docs/ci-workflows.md` count five workflows

---

## Functional Requirements

- FR-01: The report must not contain a credential the repository pass matches in any field, line, frame, prompt or rendered document, suggestions included.
- FR-02: The repository pass must run on the tree a scan judges: the working tree for a full scan, the staged index under `--staged`, both commits under `--scope baseline`.
- FR-03: A `--staged` scan must read `rust-doctor.toml` from the index.
- FR-04: The report must contain no absolute path and no environment value.
- FR-05: A scan must leave the scanned tree byte-identical, except for files a subcommand is asked to write.
- FR-06: The system must exit 2 on a PATH or `--advisory-db` path that does not exist, before writing anything.
- FR-07: The system must not publish a numeric score for a package it did not lint, or for an interrupted scan.
- FR-08: On SIGINT, SIGTERM or SIGHUP the system must kill its children's process groups, remove its snapshots, restore the terminal and exit 130.
- FR-09: The installed pre-commit hook must refuse a commit that adds a finding at `warning` level or above in the lines it adds, and must let a commit through when the scan cannot run.
- FR-10: The system must NOT reach the network itself; Cargo's network access must be documented and forbiddable with `CARGO_NET_OFFLINE=true`, and snapshot materialization must not invoke git-lfs.
- FR-11: The e2e suite must run the release binary in copies, with a private HOME, TMPDIR and target directory, and no network.
- FR-12: The e2e suite must NOT assert this repository's score, label, counts or specific findings.
- FR-13: Every known defect must be either fixed or referenced by a `test.failing` row whose identifier is in `tests/e2e/known-bugs.json`.
- FR-14: CI must report a test file that passes only on retry with a visible warning.

## Non-Functional Requirements

- **Performance:** End-to-end critical path on a pull request (release build plus slowest shard) at most 10 minutes on `ubuntu-24.04` hosted runners; each shard at most 4 minutes of tests; any single test file at most 120 seconds; `tmux` waits default to 3 seconds and 15 seconds for text that follows a scan.
- **Performance:** A warm scan of this repository stays under 1 second on the development machine (0.42 s measured on 2026-10-20); the lockfile bookkeeping of US-005 adds under 5 ms.
- **Security:** Zero credential bodies in any output surface across the 12 privacy scenarios; zero absolute paths in any JSON produced by the suite; zero socket system calls from the rust-doctor process (verified with `strace -f -e trace=network` where available).
- **Reliability:** 100 percent of e2e scans leave the scanned tree's fingerprint unchanged unless the scenario writes; zero `rust-doctor-baseline-*` directories left after exits 0, 1, 2 and 130.
- **Reliability:** At most 2 pass-on-retry warnings per 100 CI runs of the e2e job over Month 1; a file that crosses that rate is fixed or quarantined within one release.
- **Scalability:** The suite holds at least 1,000 scenarios in at most 60 files with 3 shards inside the 10-minute budget; a fourth shard is a one-line matrix change.
- **Accessibility:** The interactive report is proven readable with `NO_COLOR=1` and `TERM=dumb` at 40, 80 and 140 columns, and a key press repaints in under 50 ms (10.5 ms measured).

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Empty change | Baseline of a branch with no diff | Zero introduced, zero fixed, exit 0 | "Delta: +0 introduced; =N pre-existing; -0 fixed" |
| 2 | Credential in a fix | A Clippy suggestion replacing a credential literal | Suggestion withheld, finding kept | None (no "Replace with:" line) |
| 3 | Lockless workspace | First scan of a library without `Cargo.lock` | Lockfile removed after the scan; `missing_lockfile` reported on every scan | Finding help as today |
| 4 | Read-only workspace | Lockless and read-only | Removal impossible; scan otherwise unchanged | One stderr line naming `Cargo.lock` |
| 5 | Mistyped path | `--advisory-db` or PATH that does not exist | Exit 2 before any work, nothing written | "<flag or argument>: <path> does not exist" |
| 6 | Duplicate signal | Terminal Ctrl-C through the launcher | One `interrupted` report, exit 130 | Report error `interrupted` |
| 7 | Terminal closed | SIGHUP during Clippy | Children killed, snapshot removed, exit 130 | None (terminal gone) |
| 8 | Killed run | SIGKILL during a baseline | Next baseline or staged run sweeps the dead snapshot | None |
| 9 | Hook without a scan | Toolchain missing when committing | Commit goes through | "rust-doctor could not scan (...): this commit was not judged." |
| 10 | Offline with missing dependency | `CARGO_NET_OFFLINE=true`, uncached crate | Scan incomplete, bounded error | Cargo's offline message, quoted |
| 11 | Unreadable snapshot index | Index removed during a baseline | Existing `baseline-scan-incomplete` error, exit 2 | The existing baseline error text |
| 12 | tmux missing locally | Running TUI files without tmux | Skip with one reason; CI fails instead | "tmux is not installed: TUI files skipped" |
| 13 | Namespaces blocked | `unshare -rn` refused | Local skip with reason; CI enables them first | "network namespaces unavailable: skipped" |
| 14 | Boundary sizes | TUI at 1x1 and 300x80 | No crash, no frame wider than the window | None |
| 15 | Corpus cache absent | Weekly job without the cache | Job fails naming the cache key; no clone | "corpus cache <key> not found" |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | The git view of a snapshot answers `check-ignore` differently from a real checkout | Med | High | US-002's oracle criterion compares base-side findings with a scan of a real checkout of the base commit |
| 2 | TUI tests flake on CI timing | High | Med | Poll, never sleep; lone Escape then wait for a frame; per-file tmux server; retry once with a warning; quarantine above 2 per 100 runs |
| 3 | The suite outgrows the 10-minute budget | Med | Med | Weights refreshed from CI timings; cached scan targets per file; a fourth shard |
| 4 | A hook that blocks on warnings annoys people who commit by hand | Med | Med | Only lines the commit adds are judged; `--no-verify` documented in the hook body; a failed scan never blocks |
| 5 | The 500 ms duplicate window swallows a deliberate second Ctrl-C | Low | Low | A second press after 500 ms keeps the immediate exit; the first press already ends the scan within 0.1 s |
| 6 | Restoring a lockfile Cargo rewrote hides a stale lockfile from the user | Low | Low | `missing_lockfile` and Cargo's own next build surface it; the scan's job is to judge, not to update |
| 7 | Bun's pseudo-terminal changes behavior across versions | Med | Low | Pin bun 1.4.2 in `tests/e2e/package.json` and CI; rendering tests use tmux |
| 8 | A catalog row is wrong (it was written by an exploration, not a specification) | Med | Med | A row that disagrees with the documented contract is corrected in `tasks/e2e-catalog/` in the story's commit, with the reason |
| 9 | Hosted runners stop allowing the AppArmor sysctl | Low | Med | Namespace scenarios skip with a stated reason and the `strace` proof stays |

## Non-Goals

- Fixing the defects the suite records as `test.failing` (the interactive report's 11 remaining defects, `scopes/BUG-C`, `BUG-E`, `BUG-F`, `BUG-G`, the low-severity subcommand findings, `failures/BUG-08` and the cosmetic errors): each is tracked by a failing row and belongs to a follow-up PRD.
- macOS and Windows end-to-end runs: the Rust PTY tests already run on macOS, Windows is check-only in CI, and tmux is Linux-first. Revisit when a platform-specific defect appears.
- Forcing `--offline` or `--locked` on Cargo: rejected because the first scan of a fresh clone would fail.
- Sandboxing build scripts or procedural macros: the trust boundary stays "trusted local paths only".
- Replacing or porting the Rust integration tests: they stay the regression home of each fix; the e2e suite adds the real-binary layer.
- Golden snapshots of this repository's report: forbidden by `docs/testing.md:5-17`.
- Running the Action through `act` or on GitHub-hosted reruns: the replay of its bash steps covers its logic offline.
- Updating rust-doctor.com: the release checklist names the page; the site lives in its own repository.

## Files NOT to Modify

- `src/policy/catalog.rs` and `src/policy/catalog/`: rule set, levels and tiers stay; only the help text of `rust_doctor::repo::hardcoded_credential` changes (US-012).
- `tests/corpus.json`: regenerated only by the documented command after US-012's help change, never edited by hand.
- `tests/rule_evidence.json`: no rule is added or removed.
- The frozen v7 archive and `project_current_wire_to_v7` (`tests/support/mod.rs:200-267`).
- `tests/fixtures/**` existing oracles: the e2e suite copies fixtures, never edits them.
- Version fields (`Cargo.toml` `version`, `npm/rust-doctor/package.json`, `bun.lock`, `action.yml`): they move together only at release.
- `tests/local_cli_experience.rs` PTY tests: they stay; US-010 adds to them.
- `/home/arthur/dev/pf`: read-only reference for the port.

## Technical Considerations

- **Architecture:** Where does the git view of a snapshot live? Recommended: one constructor in `src/git.rs` returning a `GitCall` context with `GIT_INDEX_FILE`, `--git-dir` and `--work-tree`, passed to `repo_hygiene::inspect` beside the workspace root. Engineering to confirm that `check-ignore` under that view reads the snapshot's `.gitignore` files.
- **Architecture:** Where does signal bookkeeping live? `src/execution/interrupt.rs` today holds the handler and the watcher; the 500 ms window and SIGHUP fit there, the terminal restore needs a hook the TUI registers. Engineering to confirm the restore runs before `_exit`.
- **Data Model:** No report field changes. Should a withheld suggestion leave a trace (a note saying a fix was withheld)? Recommended: no, since the finding's help already says the value is never republished; a trace would need a schema bump.
- **API Design:** Should `PATH` resolution return the path as given for messages and the canonical path for work? Recommended: both, carried together from the CLI boundary.
- **Dependencies:** The suite's dev dependencies are `typescript` and `@types/bun`, pinned exactly; no test framework beyond `bun test`. Should the suite share helpers with `npm/rust-doctor/tests`? Recommended: no, the launcher tests stay self-contained.
- **Migration:** Existing pre-commit hooks keep their old body until `hook install git --update` or `setup` runs again. Should `setup` print that an older hook was found? Recommended: yes, one line naming `--update`.
- **CI:** Is one release build shared by `e2e`, `packed` and the corpus job worth the artifact plumbing? Recommended: yes, it removes one release build per run (`tasks/e2e-catalog/real-codebases.md` section 5).

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Catalogued scenarios automated | 0 of 946 | 639 (Release 2), 946 plus invariants and matrices (Release 3) | Month-1 / Month-6 | `bun test` test count per file against the catalog sections |
| Open high-severity defects | 5 | 0 | Month-1 | `tests/e2e/known-bugs.json` entries with severity high and status open |
| Known defects with neither fix nor test | All ledger entries | 0 | Month-1 | `known-bugs.test.ts` |
| P1 findings on this repository's scan | 1 (`hardcoded_credential`, score capped at 65) | 0 | Month-1 | `self-scan.test.ts` invariant 1 |
| E2E critical path on a pull request | N/A (new) | at most 10 min | Month-1 / Month-6 | CI job durations |
| Pass-on-retry warnings | N/A (new) | at most 2 per 100 runs | Month-1 | CI annotations on the `e2e` job |
| Defects found after release that a catalog section covers | N/A (new) | 0 | Month-6 | Issues labeled as regressions, matched to catalog sections |

## Open Questions

- Should the pre-commit hook's blocking level follow `rust-doctor.toml` when the file sets `blocking`, instead of the forced `warning`? Owner: Arthur, before US-011; it decides whether the hook passes the flag or a new default applies under `--staged`.
- Should `failures/BUG-08` (no git on PATH fails every scan, even outside a repository) be fixed in Release 1 rather than recorded? Owner: Arthur, before Release 1 is cut; it affects slim CI images.
- Is a weekly corpus job enough, or should the two lockfile-committing subjects (hexyl, ripgrep) run on every pull request through a restored cache? Owner: Arthur, at US-034; it trades about 30 seconds per run for daily real-world coverage.
[/PRD]
