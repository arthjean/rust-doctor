[PRD]
# PRD: Field Audit Remediation

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-11-15 | Arthur Jean | Initial draft from the 0.10.0 field audit (`bunx -y rust-doctor@latest .` on an 18-crate, 185,626-line desktop application, Windows 11, score 36, core-v6, schema 24) |
| 1.1 | 2026-11-16 | Arthur Jean | EP-007 gathers every Linux gate and criterion of release 0.12.0 into one run per epic on a Fedora machine; Windows sessions certify what Windows observes |
| 1.2 | 2026-11-16 | Arthur Jean | No epic before EP-007 runs a Linux gate, in WSL or anywhere else; A2 on Linux moves to US-035 |
| 1.3 | 2026-11-16 | Arthur Jean | `SCORE_MODEL` becomes "core-v7" in US-032, on the corpus replay, since `lambda_freeze.rs` ties the model to the corpus record; a missing database keeps `authoritative` |
| 1.4 | 2026-11-16 | Arthur Jean | The Linux run of the corpus replay moves to US-038 in EP-007; US-032 keeps the adjudication, the recomputation, the model move and the release note, from US-038's artifacts |
| 1.5 | 2026-11-16 | Arthur Jean | US-031 leaves release 0.12.0: the audited workspace is public, but its history carries the agent marker on 13 of 2280 commits, under the half the agent population requires; its population is decided in a later release |

## Problem Statement

On 2026-11-15, `bunx -y rust-doctor@latest .` (0.10.0, score model core-v6, schema 24) scored a real workspace 36 "Critical": 18 crates, 185,626 production lines, a GPUI desktop terminal with an FFI layer, on Windows 11 with rustc 1.98.0. The run took 27 seconds and reported 1438 findings from 23 of the 142 rules. Five reviewers then opened 429 reported sites by hand: 74 are false, 307 are true but not worth acting on in that project, 48 are worth acting on. The arithmetic of the score was rebuilt to the unit, and two runs were identical. The mechanics hold. The judgment does not.

1. **On Windows the scan states things that are false.**
   - `path_dependency_outside_workspace` (P1) is wrong on 37 of 37 findings. The manifest is canonicalized (`src/scan_target.rs:107-109,168`), which yields the verbatim form `\\?\C:\...`. Cargo then returns `workspace_root` with that prefix and each dependency path without it, and the prefix test of `leaves_workspace` (`src/cargo_health.rs:701-706`) fails every time. Reproduced with the scanner's own `cargo metadata` call: 0 of 37 paths recognized as internal. About 20 other sites compare paths the same way.
   - The RustSec database is not found. Discovery reads `CARGO_HOME`, then `HOME` (`src/cargo_health/advisories.rs:56-59`), never `USERPROFILE`. Under PowerShell neither exists, so the report says "No local RustSec database" while a clone fetched the day before sits in `~/.cargo/advisory-dbs`. The printed remedy, "run cargo audit once", creates a database the same lookup would miss.
   - When the database is found, it reads as empty. The clone has CRLF line endings, and `parse` (`advisories.rs:277-290`) cuts the front matter at a newline followed by the closing fence, which leaves a lone carriage return that TOML rejects. Result: 0 advisories read, 1489 skipped, nothing printed. With the same clone converted to LF, 1489 advisories are read and 8 findings appear: one vulnerable crate and seven unmaintained ones.
   - In both cases `score.authoritative` stays `true` (`src/report/assembly.rs:271-278`).
   - Nothing guards any of this: the Windows leg of `ci.yml` stops at `cargo check` (`docs/ci-workflows.md`), and 103 tests fail on Windows at HEAD.
2. **Five detectors flag code that compiles and runs correctly.**
   - `orphan_module_file` is wrong on 29 of 29. The crate declares its modules through a local `macro_rules!` that expands to `mod $module;`. The detector abstains only when a macro call holds the `mod` keyword (`src/structure/manifest.rs:268-279`), and here the keyword sits in the definition. The help says "delete it". The same report holds 225 Clippy findings inside those "uncompiled" files. The native rules never read them, the crate is counted at 10 files and 1380 lines instead of 39 files, and `unused_dependency` calls a crate unused that those files use.
   - `blocking_call_in_async` (P2) is wrong on 6 of 6: every call sits inside `smol::unblock`, and `OFFLOADING_CALLS` knows three tokio-shaped names (`src/source_kernel/detectors/blocking.rs:108`).
   - `unreferenced_feature` is wrong on 1 of 1: the feature is read through `CARGO_FEATURE_<NAME>` in the build script, a string that appears nowhere in `src/`. Following the help breaks the native build.
   - `secret_in_build_step` (P1) is the report's top warning for a public analytics key that the application embeds with `option_env!`. The help cannot be followed, and half of it is about `cargo publish` and OIDC in jobs that never publish.
   - A file reached by a build script and by a `#[cfg(test)]` module is scored as production, because `unanimous` returns `None` on disagreement (`src/source_kernel.rs:209-211,381-385`). A file's `oversized_unit` size counts its test items: one file is reported at 2243 lines while nearly every function in it is test-gated.
3. **The score is governed by what the reader cannot act on.**
   - Six manifest and CI rules (107 findings) take the score from 84 to 36. The 17 code rules (1331 findings) take it from 100 to 84. Workspace-scope rules divide by nothing (`src/audit/density.rs:24-32`), so the same finding weighs as much in 2,000 lines as in 185,000.
   - Three lines of YAML, the same key in three workflows, move the score from 65 to 36.
   - Two causes explain 697 findings and the report names neither: 303 of 310 `large_types_passed_by_value` findings are one 292-byte struct, and the project forbids source comments, which accounts for 394 `undocumented_unsafe_blocks`.
   - 8 of the 9 `duplicate_major_versions` warnings cannot be fixed from the workspace, and they alone take Dependencies to 5.
   - 11 of the 23 rules that fired have no precision measurement. They are charged whole in the score and halved in the ranking (`src/audit/ranking.rs:15-26`). Both rules that were entirely false are among them.
   - `complex_function` is measured at 85% noise and still prints 127 warnings. It fires when either threshold is met (`src/structure/hotspots.rs:111-115`); 23 findings have a cognitive complexity under 10.
4. **The report cannot be read without the source.**
   - The rule list follows an unprinted key (`src/presentation.rs:288-296`), so `x1` sits above `x118`.
   - `(+M more)` means Info findings on some rows and test findings on others (`src/render/tally.rs:166-179`).
   - "Bugs: 425 warnings" is 394 missing comments, and the same category is "Reliability" twenty lines later.
   - "Security 5 (P1 ceiling: ...)" is printed although the ceiling is 50 and does not bind (`src/render/score.rs:86-90`).
   - The withheld rules run on from the fix list without a break, and "P1", "unmeasured", "noise" and "corpus" are never defined.
   - `w=1265` in the share link matches no printed total.
   - A redirected run keeps `Linting <member> (1/18)` as its last progress line, and the report ends on a skill install prompt that fails under `bunx`.
   - `--json`, `--verbose`, `--yes`, `--rule`, `--category` and `[PATH]` have no description (`src/main.rs:197-218`).
   - Rule texts misdirect: all 27 `rc_buffer` findings print "Replace with: [T]" under a caret covering the whole type, and duplicate-version labels name a workspace member as the carrier.

**Why now:** 0.10.0 is what `bunx rust-doctor@latest` serves, and this is the first audit on a large workspace the tool was not calibrated on. Windows is one of the five platforms the launcher ships, and there the P1 ceiling is set by a false finding while the vulnerability check silently does nothing. The agent hooks and `ci install` put the scan into loops where "delete this file" and "delete this feature" are instructions an agent follows. The reproductions are fresh, the 429 classified sites exist today, and the score changes must ship once, with the website, as the core-v6 changes did.

## Overview

The plan ships in three releases, each useful alone.

**Release 0.10.1 (EP-001, EP-002)** makes the scan correct on Windows and honest about advisories. One `path_form` module owns every comparison between a canonical path and a Cargo path, and a test forbids a bare `canonicalize()` elsewhere. Advisory discovery follows Cargo's own home rule, the parser accepts CRLF, and a database that is found but unreadable fails the dependency stage instead of reading as clean. The Windows CI leg runs the whole test suite against a ledger of known failures, so a new Windows failure turns the job red and a fixed one must leave the ledger. No score model or schema change.

**Release 0.11.0 (EP-003)** changes text only. The rule list is ordered by a key it prints, every count says what it counts, one vocabulary names the dimensions, the footer separates what to fix from what the corpus distrusts, and every option has a description that a test enforces.

**Release 0.12.0 (EP-004 to EP-007)** changes what is reported and how it is scored, measured by one corpus replay. EP-007 runs every Linux gate of the release on a Fedora machine, one run per epic.
- The module walk follows modules declared through a local `macro_rules!`, and the compiled file set from Cargo's dep-info vetoes any orphan finding on a file that was compiled.
- Offloading is recognized outside tokio, a feature read by a build script counts as read, and a value the build embeds is reported at Info with its own message.
- Score model core-v7 charges causes with diminishing returns: `1 + ln(n)` sites within a cause, and the same curve across the causes of a workspace-scope rule. λ and the dimension weights stay frozen.
- An unmeasured rule is charged at the same prior the ranking uses, a withheld rule publishes at Info, and a duplicate major warns only when a member requirement can remove it.
- Schema 25 adds three fields: `diagnostics[].cause`, `audit.score.unverified` and `scan.unwalked_compiled_files`.

Key decisions, with their evidence in the story that carries them:
- **No new dependency.** The path helper is std-only, as both earlier PRDs require.
- **A missing advisory database keeps the score authoritative but is published as unverified.** `docs/subsystems/execution.md` keeps a check nobody could run from failing a stage. A database that exists and cannot be read is a failed stage.
- **Windows becomes a gate through a ratchet, not through a port.** The ledger shrinks as tests are fixed and can never grow silently.
- **Findings stay one per site.** A cause is a published key, so ids, the baseline and the delta keep their identity. Only the charge and the display group.

## Goals

| Goal | Release 0.10.1 Target | Release 0.12.0 Target |
|------|---------------|----------------|
| Findings among the 74 confirmed false that still report on the audited workspace | 37 or fewer (the path rule cleared) | 0 |
| Advisories read from a CRLF checkout of the RustSec database | 1489 of 1489 | 1489 of 1489 |
| Windows test failures that can appear without failing CI | 0 | 0 |
| Score points the audited workspace loses to findings that are false or not actionable | 29 or more (unchanged) | 0 |
| Report lines a first-time reader misreads (10 listed in the audit) | 10 (unchanged) | 0, reached in 0.11.0 |

## Target Users

### Rust developer on Windows
- **Role:** Maintains a Cargo workspace on Windows and runs `bunx rust-doctor@latest .` from PowerShell.
- **Behaviors:** Reads the linear report, starts with the top warning, trusts the band.
- **Pain points:** A P1 finding on every internal dependency, a "no database" notice while one exists, a score capped by something that is not there.
- **Current workaround:** Runs from Git Bash, passes `--advisory-db`, or ignores the Dependencies dimension.
- **Success looks like:** The same findings, score and advisory check as on Linux for the same tree.

### Coding agent in a scan loop
- **Role:** Claude Code, Codex or another agent that reads `--json` after each turn through the hooks.
- **Behaviors:** Follows the help text of the highest-ranked rule, then rescans.
- **Pain points:** "Delete it" on a compiled file, "delete the feature" on one the build script reads, a help text it cannot apply, 303 findings for one decision.
- **Current workaround:** None. It either edits correct code or stalls on a finding it cannot clear.
- **Success looks like:** Every warning it is pointed at is true, and its help can be applied at that site.

### Maintainer of rust-doctor
- **Role:** Arthur, sole maintainer, developing on Windows with CI on Linux and macOS, and validating Linux on a Fedora machine.
- **Behaviors:** Calibrates on the pinned corpus, gates on CI, releases through the release skill.
- **Pain points:** 103 tests fail locally at HEAD, so a red run says nothing; Windows defects ship unseen; 120 rules have no precision rate.
- **Current workaround:** Diffs failing tests against a HEAD worktree and replays CI in WSL.
- **Success looks like:** A Windows regression fails CI, and the next field audit finds no rule that is wrong on every site.

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **cargo-deny** keeps databases under `$CARGO_HOME/advisory-dbs/<name>-<hash>`, and **cargo-audit** under `~/.cargo/advisory-db`. rust-doctor already reads both layouts; the gap is the home lookup.
- **rustsec** (the upstream parser) anchors the front matter on a newline and trims, so CRLF files parse. The failure is this tool's own parser.
- **cargo-machete** documents its false positives (uses from generated or macro-declared code) and offers a manifest escape hatch. rust-doctor's rule should abstain where it cannot see, which is what `CrateReferences::incomplete` exists for.
- **SonarQube** normalizes maintainability by size and grades security and reliability by the worst severity present. core-v6 already has both mechanisms (density and tier ceilings), but workspace-scope findings also pile up without bound.
- **Market gap:** no Rust tool states which checks did not run next to the score. A clean result from a check that never ran reads as "no vulnerabilities".

### Best Practices Applied
- Compare paths on Windows through one normalizer applied to both sides; `std::fs::canonicalize` returns verbatim paths (Rust std docs, `dunce` documentation).
- When `CARGO_HOME` is unset, Cargo's home is `$HOME/.cargo`, or `%USERPROFILE%\.cargo` on Windows (Cargo reference, environment variables).
- Cargo writes a dep-info `.d` file per artifact listing every source file rustc read (Cargo reference, build cache). The scan already parses them in `src/audit/source_inventory.rs`.
- Cargo exposes each active feature to build scripts as `CARGO_FEATURE_<NAME>`, uppercased with `-` as `_` (Cargo reference).
- Clippy has no lint for blocking calls in async code, so a native detector must know each runtime's offload helper.

*Full research sources available in project documentation.*

## Assumptions & Constraints

### Assumptions (to validate)
- **A1:** Stripping the verbatim prefix is safe whenever the plain form is under 260 characters and names no reserved device. Longer or reserved paths keep the verbatim form, and comparisons normalize both sides. US-001 pins both branches.
- **A2:** Cargo's dep-info lists a module declared through `macro_rules!` on every platform. Confirmed on Windows by the audit (38 files in dep-info against 10 walked). US-018's fixture confirms it on Windows, US-035 on Linux and CI on macOS.
- **A3:** Charging causes with `1 + ln(n)` and charging unmeasured rules at the 5000 bp prior keeps the corpus distribution gate (spread of at least 20, distinct bands, healthy median above the agent median). US-023 validates this before US-025 and US-026 ship.
- **A4:** An unmeasured P0 or P1 rule charged at half still reads as severe, because the tier ceilings apply at the full tier (`src/audit/ceilings.rs:55-79`). US-026 adds a test for it.
- **A5:** Most of the 103 Windows failures share the path-form defect; the local notes list `path-outside-workspace` rejections as the main class. US-008 measures it.
- **A6:** The audited workspace can be pinned as a public corpus subject. Not verified. US-031 is blocked until Arthur confirms it.
- **A7:** The website can ship core-v7 copy in the same window as the CLI release; it has one maintainer.

### Hard Constraints
- **Dependencies:** No new dependency. Existing ones stay pinned exactly, and `Cargo.lock` stays committed.
- **Toolchain:** Edition 2024, rustc 1.95 or later. Every gate runs under the CI pin, toolchain 1.99.1.
- **Trust boundary:** No network, no telemetry. `--json` reports stay workspace-relative. A home directory printed in the linear report is written as `~`, through `src/report/sanitize.rs`.
- **No panics in production code:** no `unwrap`, `expect`, `panic!` or `dbg!`.
- **No comments or shebangs,** fixtures included (`tests/comment_free.rs`). A comment a rule reads is data in `tests/fixtures/comment-inputs.toml`.
- **Size bounds:** every file under 1000 lines and every `impl` under 500, tests included. New code goes in submodules. Current sizes of the files this PRD touches:

  | File | Lines |
  |---|---|
  | `src/report/assembly.rs` | 837 |
  | `src/source_kernel/detectors.rs` | 793 |
  | `src/audit.rs` | 792 |
  | `src/cargo_health.rs` | 756 |
  | `src/report.rs` | 756 |
  | `src/presentation.rs` | 701 |
  | `src/structure/manifest.rs` | 696 |
  | `src/render.rs` | 692 |
  | `src/main.rs` | 680 |
  | `src/source_kernel.rs` | 635 |
  | `src/cargo_health/advisories.rs` | 512 |
- **Report shape:**
  - Releases 0.10.1 and 0.11.0 keep `SCHEMA_VERSION` 24 and `SCORE_MODEL` "core-v6". `ReportError` codes are free strings, so a new code is not a shape change.
  - Release 0.12.0 ships `SCHEMA_VERSION` 25, additive only, and `SCORE_MODEL` "core-v7". The frozen v7 archive keeps projecting through `project_current_wire_to_v7`.
  - A finding id hashes its message (`src/report/normalize.rs:548-580`), and so does the delta fingerprint. Every message rewording ships in 0.12.0 and is named in its release note.
- **Score model:** λ and the dimension weights do not change; `src/audit/tests/lambda_freeze.rs` stays green untouched.
- **Catalog:** stays at 142 rules. A default-level, tier or help change follows `docs/catalog-and-corpus.md`.
- **Documentation:** each edit keeps its file between 85% and 100% of its word ceiling. Current counts:

  | File | Words / ceiling |
  |---|---|
  | AGENTS.md | 1254 / 1289 |
  | `docs/ci-workflows.md` | 1222 / 1225 |
  | `docs/subsystems/execution.md` | 2141 / 2155 |
  | `docs/subsystems/audit.md` | 1201 / 1249 |
  | `docs/subsystems/reporting.md` | 3322 / 3353 |
  | `docs/subsystems/source-kernel.md` | 1132 / 1149 |
  | `docs/subsystems/structure.md` | 381 / 401 |
  | `docs/testing.md` | 1220 / 1281 |
  | `docs/catalog-and-corpus.md` | 1974 / 2024 |

  A new measurement record goes under `frozen` in `docs/doc-budgets.json`. Raising a ceiling is a deliberate edit justified in the commit.
- **Test conventions:** integration test crates start with `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`; tests that run cargo or the binary use `support::scan_target`; regression seeds live under `tests/fixtures/regressions/<rule-tail>--<slug>/` with a weakness from `WEAKNESSES`; no test reaches the network.
- **E2E ledger:** a defect reproducible on Linux gets an appended row in `tasks/e2e-catalog/` and `tests/e2e/known-bugs.json`, with `fixed_by` naming its story. Existing rows are never edited.
- **Package manager:** `bun` under `npm/rust-doctor/` and `tests/e2e/`, never `npm` or `pnpm`.
- **Windows development machine:** `cargo clippy` and about 103 tests fail there at HEAD. Until US-008 and US-009 land, a change is judged by diffing failures against a detached HEAD worktree, and the gates below are settled on Linux, macOS or CI.

## Quality Gates

These commands must pass for every user story:
- `cargo clippy --all-targets --no-deps --locked -- -D warnings` - lint, must be clean
- `cargo check --locked --all-targets` - every target compiles
- `cargo test --locked` - unit, integration and contract tests, size bounds included
- `bash scripts/verify-doc-budgets.sh` - word ceilings and floors on AGENTS.md and the documents it links
- `bash scripts/verify-doc-refs.sh` - every path the documents cite resolves
- `cargo build --release --locked` - release binary the e2e suite drives
- `cd tests/e2e && bun install --frozen-lockfile && bun test` - end-to-end suite against the release build
- `cd tests/e2e && bun run typecheck` - e2e TypeScript type check

Every command runs under toolchain 1.99.1 (`RUSTUP_TOOLCHAIN=1.99.1`), on Linux or macOS, never in WSL.

From EP-004 on, a story is certified on Windows by Clippy, the documentation gates, the Windows gate and its own acceptance tests. No epic before EP-007 runs a Linux gate, in WSL or anywhere else, and a Linux criterion of an earlier story is certified by the EP-007 story that validates its epic. Every gate that needs Linux (`cargo test --locked` on Unix, the release build, the e2e suite and its type check, the launcher tests and the packed smoke) runs once per epic on Arthur's Fedora machine, through EP-007. macOS stays with CI's `Tests (macos-latest)`.

Three stories add gates:
- **US-007 and later:** `bash scripts/windows-test-gate.sh` on a Windows host, which CI runs in the `windows` job.
- **US-038:** the corpus replay on Fedora, `RUST_DOCTOR_CORPUS_DIR=<clone cache> RUST_DOCTOR_CORPUS_ARTIFACTS=<artifacts> cargo test --locked --test corpus_precision`, whose artifacts US-032 consumes.
- **US-033:** `bun run build:verified` in the `rust-doctor-web` checkout.

## Epics & User Stories

The plan is phased because it holds 37 stories. Each phase stays under 22 stories and ships as one version.

| Release | Version | Epics | Stories | Priority |
|---|---|---|---|---|
| 1 | 0.10.1 | EP-001, EP-002 | 9 | P0 and P1, no model or schema change |
| 2 | 0.11.0 | EP-003 | 7 | P1, text only |
| 3 | 0.12.0 | EP-004 to EP-007 | 21 | P0 to P2, one corpus replay, `core-v7`, schema 25, Linux validation on Fedora |

### EP-001: The scan is right on Windows and honest about advisories

Release 0.10.1. A Windows run reports the same findings as a Linux run of the same tree, and the vulnerability check either runs or says, next to the score, that it did not.

**Definition of Done:**
- On a Windows host, a workspace whose members depend on each other by path reports 0 `path_dependency_outside_workspace` findings.
- With `CARGO_HOME` and `HOME` unset and a clone under `%USERPROFILE%\.cargo`, the scan consults it.
- A CRLF checkout of the database reads every advisory an LF checkout reads.
- A database that yields nothing readable produces a `ReportError` at stage `dependencies` and `authoritative: false`.
- No file under `src/` calls `canonicalize()` outside `src/path_form.rs`.

#### US-001: Compare paths in one form
**Description:** As a Rust developer on Windows, I want internal path dependencies recognized as inside my workspace, so that the P1 rule and the overall ceiling stop resting on a false finding.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A new `src/path_form.rs` exposes `canonical(path)`, `within(child, root)` and `relative_to(child, root)`, std-only.
- [ ] `canonical` returns the plain form of a verbatim path: `\\?\C:\x` becomes `C:\x` and `\\?\UNC\server\share\x` becomes `\\server\share\x`. On non-Windows targets it returns `fs::canonicalize`'s result unchanged.
- [ ] Given a verbatim path whose plain form is 260 characters or longer, or holds a reserved device name (`CON`, `NUL`, `COM1` and the rest of the Win32 list) as a component, when `canonical` runs, then it keeps the verbatim form.
- [ ] `within` and `relative_to` strip the verbatim prefix from both arguments before comparing, and compare drive letters without regard to case.
- [ ] `leaves_workspace` (`src/cargo_health.rs:701-706`) calls `within`, and both `canonicalize()` calls of `src/scan_target.rs` (lines 107-109 and 168) call `canonical`.
- [ ] Given a `Metadata` whose `workspace_root` is `\\?\C:\w` and a dependency path `C:\w\crates\a`, when the dependency pass runs, then no `path_dependency_outside_workspace` finding is produced. The test builds the `Metadata` from JSON and runs on every platform.
- [ ] Given a dependency path `C:\elsewhere\a` under the same root, when the pass runs, then the finding is still produced.
- [ ] Given a workspace on a Windows host whose members depend on each other by path (`tests/fixtures/cargo-health/path-members/`), when `rust-doctor . --yes --json` runs, then the report holds 0 findings of the rule. The test is gated `cfg(windows)` and runs in the job of US-007.

#### US-002: Route every path comparison through the helper
**Description:** As the maintainer, I want every comparison between a canonical path and a Cargo or git path to use the same helper, so that the defect of US-001 cannot survive at another site or return at a new one.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] These sites use `path_form`: `src/cargo_health.rs:101,145,216,338`, `src/cargo_health/resolution.rs:86`, `src/audit/source_inventory.rs:44,56,118,259-262`, `src/walk.rs:76,96,411,434,579-587`, `src/delta.rs:140-204`, `src/presentation/code_frame.rs:92-117`, `src/execution/lockfile.rs:54`, `src/hook.rs:171-202`, `src/git_hook.rs:99-102`, `src/ci.rs:218-221`, `src/baseline.rs:183,387-392`, `src/baseline/staged.rs:147`, `src/repo_hygiene.rs:463` and `src/workspace_path.rs:14-18`.
- [ ] A test under `tests/` reads every file of `src/` and fails, naming the file and line, on a `.canonicalize()` call or a `fs::canonicalize` path outside `src/path_form.rs`.
- [ ] Given a workspace root reached through a verbatim path, when the walk enumerates it, then no unit is rejected with `path-outside-workspace`. The test passes the verbatim form explicitly and runs on every platform through `within`.
- [ ] Given a symbolic link that points outside the workspace, when the walk meets it, then it is still refused: confinement is unchanged for paths that really leave the root.
- [ ] `docs/subsystems/execution.md` states the rule in one sentence, within its word ceiling.

#### US-003: Find the RustSec database where Cargo keeps its home
**Description:** As a Rust developer on Windows, I want the scan to find the advisory database `cargo deny` or `cargo audit` already fetched, so that the vulnerability check runs from PowerShell as it does from a Unix shell.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `AdvisoryDatabase::locate` resolves the Cargo home as: `CARGO_HOME` when set and not empty; else `HOME` joined with `.cargo`; else, on Windows, `USERPROFILE` joined with `.cargo`.
- [ ] The resolution is a pure function of three optional values, tested without touching the process environment.
- [ ] Given `HOME` unset and `USERPROFILE` naming a directory that holds `.cargo/advisory-dbs/<clone>`, when discovery runs on Windows, then that clone is returned with source `cargo-deny`.
- [ ] Given `HOME` and `USERPROFILE` both set to different directories, when discovery runs, then `HOME` wins, as it does for Cargo.
- [ ] Given no variable set, when discovery runs, then it returns no database and reads no directory.
- [ ] The missing-database line reads: "Known-vulnerability check skipped: no RustSec database under ~/.cargo. Run `cargo deny fetch` or `cargo audit` once, or pass --advisory-db <path>." The home is printed as `~`.

#### US-004: Read advisory files with CRLF line endings
**Description:** As a Rust developer whose git converts line endings, I want every advisory read whatever its line endings, so that the check does not silently pass on an empty set.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `parse` (`src/cargo_health/advisories.rs:277-290`) normalizes CRLF to LF before it looks for the fence, for Markdown and for `.toml` advisories.
- [ ] Given the same advisory written with LF, with CRLF, and with CRLF except a final LF, when `parse` runs on each, then the three results are equal.
- [ ] Given a fixture database of at least five advisories checked out with CRLF (`tests/fixtures/advisory-db/crlf/`, protected from conversion by a `.gitattributes` entry marking it `-text`), when the dependency pass runs against a lockfile that one advisory affects, then `vulnerable_dependency` reports it and `scan.advisory_db.skipped` is 0.
- [ ] Given a file with a `\r` inside a TOML string value, when `parse` runs, then the file is counted as skipped and the others are still read.
- [ ] A row is appended to `tasks/e2e-catalog/failures.md` and `tests/e2e/known-bugs.json` with `fixed_by` "US-004", and an e2e test scans with the CRLF fixture through `--advisory-db`.

#### US-005: Fail the dependency stage when a database yields nothing readable
**Description:** As a Rust developer, I want a database that was found but could not be read to be reported as a failure, so that I never take "0 advisories" for "no vulnerability".

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**
- [ ] A consulted database is unreadable when `advisories == 0 && skipped > 0`, or when `skipped * 20 > advisories + skipped` (more than 5% skipped).
- [ ] Given an unreadable database, when the report is assembled, then it carries one `ReportError` with code `advisory-db-unreadable` at stage `dependencies`, `complete: false`, `audit.score.authoritative: false`, and both advisory rules marked not evaluated.
- [ ] The linear report prints, in the warning style: "The RustSec database at <path> could not be read: N of M files were skipped. The known-vulnerability check did not run." The path is workspace-relative or home-relative.
- [ ] Given a readable database with `skipped > 0` under the threshold, when the report renders, then one muted line states the count, and the score stays authoritative.
- [ ] Given `--advisory-db` naming a directory with no `crates/` folder, when the scan starts, then the existing refusal is unchanged.
- [ ] The exit status follows the documented precedence: an incomplete scan exits 1.
- [ ] `docs/subsystems/execution.md` states the unreadable rule beside the missing rule, within its word ceiling.

#### US-006: State the skipped vulnerability check beside the score
**Description:** As a reader of the linear report, I want the dimensions that were scored without the advisory check to say so, so that Security 100 is not read as "no known vulnerability".

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Given a report where both advisory rules are not evaluated for `advisory-db-missing`, when the score block renders, then a line directly under the dimensions reads: "Security and Dependencies were scored without the known-vulnerability check."
- [ ] The line is produced by the code the linear report and the TUI share (`score_block`), and the Markdown report prints it too.
- [ ] Given a scan that consulted a database, when the block renders, then the line is absent.
- [ ] Given `--advisory-db /dev/null`, which turns the check off on request, when the block renders, then the line is present and names the flag as the reason.
- [ ] No JSON field is added in this release; `policy.rules[].not_evaluated` already carries the fact.

---

### EP-002: Windows is a gate

Release 0.10.1. A change that breaks a test on Windows fails CI, and the list of tests that still fail there can only shrink.

**Definition of Done:**
- The `windows` job of `ci.yml` runs `bash scripts/windows-test-gate.sh` and is green on `main`.
- A failure absent from the ledger fails the job, and so does a ledger entry that passes.
- The ledger holds fewer than 103 entries, each with a cause class.
- `docs/ci-workflows.md` describes the gate in place of the paragraph that explains its absence.

#### US-007: Run the test suite on Windows against a ledger of known failures
**Description:** As the maintainer, I want the Windows job to run every test and compare the failures with a committed list, so that a Windows regression fails CI without waiting for a full port.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] `tests/windows-known-failures.txt` holds one entry per line: a test binary name, a test path and a cause class, separated by single spaces. The classes are a closed set: `path-form`, `separator`, `unix-only`, `toolchain`, `wall-clock`.
- [ ] `scripts/windows-test-gate.sh` runs `cargo test --locked --no-fail-fast`, collects every `FAILED` test per binary from the output, and compares the set with the ledger.
- [ ] Given a failing test that the ledger does not list, when the script ends, then it exits 1 and prints the test under "new failure".
- [ ] Given a ledger entry whose test passed, when the script ends, then it exits 1 and prints the entry under "passes now, remove it".
- [ ] Given a ledger entry whose test no longer exists, when the script ends, then it exits 1 and prints the entry under "unknown test".
- [ ] Given a build error, when the script runs, then it exits with cargo's status and prints no comparison.
- [ ] The script sets `RUST_DOCTOR_BENCHMARK_ALLOWANCE` as the e2e job does, so a wall-clock bound does not flake on the runner.
- [ ] The `windows` job of `.github/workflows/ci.yml` keeps its `cargo check` step and adds one step, `shell: bash`, that runs the script. Its timeout stays 45 minutes, and no other job or workflow changes.
- [ ] `docs/ci-workflows.md` replaces the paragraph "The Windows leg stops at `cargo check`" with the ledger rule and stays within its ceiling of 1225 words. AGENTS.md still counts five workflows, and `tests/docs_contract.rs` stays green.
- [ ] A unit of the script's comparison logic is covered by a test that feeds it a recorded cargo output and a ledger, on every platform.

#### US-008: Seed the ledger and remove what the path fix repaired
**Description:** As the maintainer, I want the ledger to start from a measured run after US-002, so that it records only what is still broken and names why.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] The ledger is generated from one run on a Windows host at the commit that holds US-002, with an LF checkout (`core.autocrlf=false`), and every entry carries a class.
- [ ] No entry carries the class `path-form`: a test that still fails for that reason is fixed through `path_form` or split out with its reproduction in this story's notes.
- [ ] The number of entries is recorded in `docs/testing.md` with its date, and is lower than 103.
- [ ] Given the ledger and the commit it was generated from, when the gate runs twice on the Windows runner, then both runs are green.
- [ ] Given a test that fails only under a CRLF checkout, when it is found, then it is recorded with the class `separator` and the checkout setting that triggers it.

#### US-009: Make the Clippy gate clean on Windows
**Description:** As the maintainer developing on Windows, I want `cargo clippy --all-targets -- -D warnings` to pass there, so that the lint gate means the same on my machine as in CI.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Every item that is dead or unused on Windows because its only users are `cfg(unix)` carries the same `cfg`, or moves into the `cfg(unix)` module that uses it.
- [ ] No `#[allow(dead_code)]` or `#[allow(unused_imports)]` is added to reach the result.
- [ ] Given a Windows host, when `cargo clippy --all-targets --no-deps --locked -- -D warnings` runs, then it exits 0.
- [ ] Given a Linux or macOS host, when the same command runs, then it still exits 0: no item used there lost its definition.
- [ ] The `windows` job runs the Clippy command as a step before the test gate.

---

### EP-003: The report reads without the source

Release 0.11.0. Text only: no finding, score, id or JSON value changes. A first-time reader can say what each line of the linear report means.

**Definition of Done:**
- The ten lines the audit quotes as misleading are gone or reworded as specified below.
- Every option of every subcommand has a description, and a test fails on one without.
- The Markdown fixtures under `tests/fixtures/report-markdown/` and the linear oracles are regenerated in the commit that changes them.
- `tests/skill_contract.rs` stays green against the new `--help`.

#### US-010: Order the rule list by a key the report prints
**Description:** As a reader, I want the rules listed in an order I can see the reason for, so that a rule with one site above a rule with 118 no longer looks like noise.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Each rule header starts with its tier and severity, for example `P1 warning  Secret in build step  x3`.
- [ ] Rules are ordered by tier (P0 first), then severity, then the points the rule costs (its contribution to the score, highest first), then its id.
- [ ] Rules the corpus distrusts (the withheld set) are listed after every other rule of their tier, under one sub-heading: "Reported, but found wrong on most reviewed sites".
- [ ] The count after `x` is the number of findings that weigh on the score. A suffix names every other count by kind: `(+51 info)`, `(+68 in tests)`, or both, never `(+M more)`.
- [ ] Given a rule whose every finding is Info or in tests, when the list renders, then its header shows `x0` and the suffix, and it sorts after the rules that weigh.
- [ ] The TUI group headings use the same order and the same suffixes.

#### US-011: Name categories and dimensions with one vocabulary
**Description:** As a reader, I want the tallies and the score to use the same five names, so that "Bugs: 425" and "Reliability 60" are visibly the same thing.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The text tallies are labeled Security, Reliability, Maintainability, Performance and Dependencies, in the order of the dimensions line.
- [ ] `AuditCategoryName::Bugs` keeps its wire value "Bugs" in `--json` (schema 24 is unchanged); only the rendered label changes.
- [ ] The Reliability tally line states its largest rule when one rule holds more than half of it: "Reliability: 425 warnings (394 from Undocumented unsafe blocks)".
- [ ] Given a report with a finding in a category outside the five, when the tallies render, then it appears under "Other" as today.
- [ ] `docs/subsystems/reporting.md` states the mapping from JSON category to label in one sentence, within its ceiling.

#### US-012: Rewrite the score footer
**Description:** As a reader, I want the lines around the score to say what limits it, what to fix and what the tool itself distrusts, as three separate statements, so that I do not fix a rule the tool told me to leave.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A ceiling line is printed only when the ceiling binds, meaning the dimension or the score equals the ceiling value, and it states the value: "Security is capped at 50 by a P1 finding: <rule>".
- [ ] Given a dimension below its ceiling, when the footer renders, then the line reads "Security 5: driven by <rule> (3 findings)" and names no ceiling.
- [ ] The sentence "A P1 finding caps the score at 65/100, and the dimensions already hold it lower" is removed.
- [ ] The fix list and the distrusted rules are two paragraphs separated by a blank line. The second starts with "Left out of what to fix first:".
- [ ] One line defines the terms the footer uses, printed once: "P0 to P3 rank a rule's severity. Noise is the share of reviewed sites where the corpus (pinned public repositories) found the rule wrong; unmeasured means no site was reviewed."
- [ ] Given a workspace score lower than every member score, when the footer renders, then one line says why: "Findings in CI files and the lockfile weigh on the workspace score only."
- [ ] The summary line prints the two counts the share link carries: "1265 scored warnings, 141 info", so `w` and `i` can be read off the report.
- [ ] "Migration advisory: <rule> appears N times across M files." becomes "Spread across M files: <rule> (N findings). Plan it as one change, not N fixes."

#### US-013: Reword the gate, configuration, progress and closing lines
**Description:** As a reader, I want the lines around the report body to say one clear thing each, so that "Gate passed" beside "Critical" and a leftover progress line no longer mislead.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The gate line reads "Exit gate (--blocking error): passed, no error-level finding." and, when it fails, "Exit gate (--blocking error): failed, N error-level findings."
- [ ] The configuration line reads "Configuration: no rust-doctor.toml; defaults apply." when none is loaded.
- [ ] Given stderr that is not a terminal, when Clippy lints members, then no per-member line is written and one line is written at the end: "Linted N members."
- [ ] Given stderr that is a terminal, when the scan ends, then the progress line is cleared as today.
- [ ] The skill hint is written before the first progress line, not after the report, so the output ends on the report's own footer.
- [ ] Given the binary was started by the Node launcher, when the hint is printed, then it names a command that resolves without a global install: the launcher passes how it was invoked in an environment variable, and the hint prints `bunx rust-doctor@<version> skill install --agent <name>` or its `npx` form.
- [ ] Given the binary was started directly, when the hint is printed, then it keeps `rust-doctor skill install --agent <name>`.
- [ ] `npm/rust-doctor` tests cover the variable the launcher sets.

#### US-014: Describe every option in --help and fail the build on a missing one
**Description:** As a new user, I want every option explained in `--help`, so that I do not have to guess what `--yes` answers.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `[PATH]`, `--json`, `--verbose`, `--yes`, `--rule`, `--category` and `--blocking` carry a `help` attribute (`src/main.rs:197-218`). `--yes` says it skips the interactive report and is required for scripted runs.
- [ ] `--rule` and `--category` state the accepted levels and give one example each.
- [ ] `--advisory-db` states that discovery follows the Cargo home (`CARGO_HOME`, else the user's home).
- [ ] The top-level `about` gains two sentences: what the score is (out of 100, bands at 75 and 50) and what P0 to P3 mean.
- [ ] A test walks the clap command tree, subcommands included, and fails naming any argument whose help is absent or empty. Clap's own `help` and `version` arguments are skipped.
- [ ] Given a new option added without help, when the test runs, then it fails.
- [ ] `tests/skill_contract.rs` and `skills/rust-doctor/SKILL.md` agree with the new text.

#### US-015: Disclose the platform the scan judged
**Description:** As a reader, I want the report to say which platform's code was compiled, so that I know code behind another platform's `cfg` was not linted.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The scope sentence reads: "Clippy linted library and binary targets for <host triple>; code gated to other platforms and test code were not linted. Native rules also read tests."
- [ ] The host triple is the `host:` line of `rustc -vV`, read by extending the toolchain probe the scan already runs. It appears in the linear and Markdown text only; no JSON field is added.
- [ ] Given a probe that returned no host line, when the sentence renders, then it omits the triple and keeps the rest.
- [ ] The Markdown report prints the same sentence.

#### US-016: Print a suggestion only under the text it replaces
**Description:** As a reader, I want a "Replace with" line shown only when its caret covers what would be replaced, so that 27 `rc_buffer` findings stop proposing to replace a whole type with a fragment.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a Clippy suggestion whose span equals the finding's span, when the code frame renders, then "Replace with:" is printed as today.
- [ ] Given a suggestion whose span is a strict part of the finding's span, when the frame renders, then the caret covers the suggestion's span and the line reads "Replace `<original>` with: <replacement>".
- [ ] Given a suggestion whose span cannot be located in the file as read, when the frame renders, then the suggestion line is omitted and the catalog help is still printed.
- [ ] The JSON `suggestion` field is unchanged.
- [ ] A fixture with `Rc<Vec<T>>` covers the partial-span case.

---

### EP-004: Detectors stop flagging what compiles and runs

Release 0.12.0. The five detectors the audit found wrong on real code report none of those 74 sites, and each still reports its true positives.

**Definition of Done:**
- A fixture crate that declares modules through a local `macro_rules!` reports 0 `orphan_module_file`, its files are counted and read by the native rules, and a dependency used only there is not reported unused.
- A file Cargo compiled is never reported as an orphan.
- Each fixed rule has a regression seed under `tests/fixtures/regressions/` and an updated record in `tests/rule_evidence.json`.
- Each defect reproducible on Linux has an appended e2e ledger row naming its story.

#### US-017: Follow modules declared through a local macro
**Description:** As a Rust developer, I want modules my crate declares through its own `macro_rules!` to be read like any other module, so that the tool neither calls them orphans nor skips them.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The module walk recognizes a declaring macro: a `macro_rules!` item in a file of the package with a transcriber that holds the tokens `mod`, a metavariable, `;` in that order.
- [ ] For an invocation of that macro by its bare name, or by a `crate::` or `self::` path ending in it, in a file of the same package, each identifier among the arguments is tried as a module name through the walk's existing resolution (`<name>.rs`, then `<name>/mod.rs`).
- [ ] A module found this way carries the attributes the transcriber writes before `mod`: under `#[cfg(test)]` it is test-gated, under another `cfg` it is treated as an inline `#[cfg(...)] mod` is today.
- [ ] Given an argument that names no file, when the walk runs, then it is ignored and nothing is reported.
- [ ] Given a fixture crate with a declaring macro and three modules (`tests/fixtures/structure/macro-declared-modules/`), when the scan runs, then `orphan_module_file` reports 0, `audit.source_files` counts the three files, and a `complex_function` planted in one of them is reported.
- [ ] Given the same crate with a fourth file named by no invocation, when the scan runs, then that file alone is reported as an orphan.
- [ ] Given a dependency used only inside a macro-declared module, when the scan runs, then `unused_dependency` does not report it.
- [ ] Given a macro defined in another crate or by a procedural macro, when the walk runs, then it is not resolved; US-018 covers that case.
- [ ] A regression seed `tests/fixtures/regressions/orphan_module_file--macro-declared/` exists with its `seed.toml`.
- [ ] `docs/subsystems/structure.md` states the rule within its ceiling of 401 words.

#### US-018: Never call a compiled file an orphan
**Description:** As a Rust developer, I want a file Cargo compiled to be exempt from the orphan rule however it was declared, so that no declaration style the walk cannot follow yields "delete it".

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-017

**Acceptance Criteria:**
- [ ] `src/audit/source_inventory.rs` exposes the set of `.rs` files it reads from dep-info, not only their count.
- [ ] At report assembly, an `orphan_module_file` finding whose path is in that set is dropped.
- [ ] For a package where the set holds a file the walk did not reach, every `unused_dependency` finding of that package is dropped, since the rule could not see all of its code.
- [ ] `scan.unwalked_compiled_files` publishes the number of such files (schema 25), and the linear report prints one muted line when it is not zero: "N compiled files were not read by the native rules: they are declared in a way the module walk cannot follow."
- [ ] The reconciliation lives in a new submodule of `src/report/`, so `src/report/assembly.rs` (837 lines) does not grow.
- [ ] Given a scan where Clippy is disabled, failed, or the dep-info budget of 8 MiB was exceeded, when the report is assembled, then no finding is dropped and the count is absent.
- [ ] Given a fixture whose modules are declared by a macro from another workspace member, when the scan runs with Clippy, then 0 orphans are reported and the count is the number of those modules.
- [ ] The fixture test runs on Linux, macOS and, through US-007, Windows, which confirms assumption A2.

#### US-019: Recognize offloading outside tokio
**Description:** As a developer using smol or async-std, I want a blocking call I already moved to a blocking pool to stay unreported, so that a P2 rule stops flagging correct code.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `OFFLOADING_CALLS` (`src/source_kernel/detectors/blocking.rs:108`) gains `unblock`, the name `smol` and the `blocking` crate use.
- [ ] Given `smol::unblock(move || std::fs::write(path, data)).await` inside an `async` block, when the detector runs, then nothing is reported.
- [ ] Given a helper that reaches `std::fs` called inside `smol::unblock(|| helper())`, when the detector runs, then nothing is reported.
- [ ] Given `std::fs::write` called directly in an `async fn` of a crate that depends on `smol`, when the detector runs, then it is reported.
- [ ] Given a local function named `unblock` that runs its closure inline, when the detector runs, then the call is still treated as offloading; the limit is stated in the rule's `not_a_defect_when`.
- [ ] A regression seed `tests/fixtures/regressions/blocking_call_in_async--smol-unblock/` exists, and `tests/rule_evidence.json` gains the negative case.

#### US-020: Count a feature the build script reads as referenced
**Description:** As a crate author, I want a feature my build script reads to count as used, so that the tool does not tell me to delete a feature my build depends on.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A feature of a package is read when the text `CARGO_FEATURE_<NAME>` appears in a file its build script reaches, where `<NAME>` is the feature uppercased with `-` replaced by `_`.
- [ ] The files a build script reaches are the build target's root and the modules the walk resolves from it.
- [ ] Given `link = []` and a build script calling `env::var_os("CARGO_FEATURE_LINK")`, when the rule runs, then nothing is reported.
- [ ] Given a feature `foo-bar` and a build script reading `CARGO_FEATURE_FOO_BAR`, when the rule runs, then nothing is reported.
- [ ] Given `unused = []` that no cfg and no build script reads, when the rule runs, then it is reported as today.
- [ ] Given a package with no build script, when the rule runs, then its behavior is unchanged.
- [ ] A regression seed `tests/fixtures/regressions/unreferenced_feature--build-script-read/` exists.

#### US-021: Report a value the build embeds apart from a secret in a build step
**Description:** As a maintainer who compiles a public key into the binary on purpose, I want that case reported for what it is, so that the top warning of my report is not advice I cannot follow.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The source kernel collects the names passed as a string literal to `env!` and `option_env!` in production code, with the first site of each.
- [ ] At report assembly, a `secret_in_build_step` finding whose variable is in that set is published at severity Info, in the same reconciliation submodule as US-018.
- [ ] Its message reads: "`<command>` compiles with `<VARIABLE>` in scope, and `<macro>` at <path>:<line> embeds it in the binary: whatever it holds ships to every user."
- [ ] Its help reads: "Keep only a value meant to be public in a variable the build embeds. If this one is secret, read it at run time instead."
- [ ] Given a secret variable no `env!` or `option_env!` names, when the scan runs, then the finding keeps its tier, severity and message.
- [ ] The help of the unchanged case names OIDC and `cargo publish` only when the job declares `id-token: write` or the command is `publish`.
- [ ] Given a scope in which the source kernel did not run, when the report is assembled, then no finding is demoted.
- [ ] A fixture workflow and crate cover both cases, and `tests/rule_evidence.json` records them.

#### US-022: Classify a file by its production routes
**Description:** As a reader, I want code that only build scripts and tests reach to be marked as not scored, so that the example site printed for a rule is not test or build code.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `SourceUnit::context` (`src/source_kernel.rs:209-211`) returns production only when at least one route to the file is a production route.
- [ ] Given routes that are all non-production and disagree, when the context is computed, then it is the first of them in this order: build script, benchmark, example, tests.
- [ ] The test at `src/source_kernel/tests.rs:749-754` is replaced by one case per combination: production with tests, build with tests, tests alone.
- [ ] Given a file included by `build.rs` and by a `#[cfg(test)] mod` of the library, when a native rule reports in it, then the finding carries the build-script context and the label "build script, not scored".
- [ ] A file-level `oversized_unit` message states both sizes: "<path> is N lines long, M outside test code."
- [ ] Given a file whose lines outside test-gated items are under the file threshold, when it is reported, then the finding carries the tests context and is not scored.
- [ ] Given a file over the threshold outside test code, when it is reported, then it is scored as today.
- [ ] `no_unit_of_this_crate_s_own_source_is_a_hotspot` keeps counting whole files, tests included.

---

### EP-005: Score model core-v7 and report schema 25

Release 0.12.0. The score counts decisions, not sites, weighs a rule by what is known of its precision, and publishes what it could not verify.

**Definition of Done:**
- `SCHEMA_VERSION` is 25, additive. The core-v7 charges ship under the "core-v6" label until US-032 replays the corpus and moves `SCORE_MODEL`, since `lambda_freeze.rs` requires every corpus observation to carry the shipped model.
- On the audited workspace's recorded findings, 303 by-value findings of one type weigh as one cause, and one variable in three workflows weighs once.
- `src/audit/tests/lambda_freeze.rs` is unchanged and green.
- US-023's record is committed under `frozen` in `docs/doc-budgets.json`.

#### US-023: Validate assumption: cause-based charges and one precision prior keep the corpus distribution gate
**Description:** As the maintainer, I want the three model changes measured on the corpus record before any of them is written into the score, so that core-v7 does not ship a model the distribution gate rejects.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A test-only harness recomputes every corpus subject's score from the observations of `tests/corpus.json` under five variants: core-v6; cause weights alone; the workspace saturation alone; the unmeasured prior alone; and all three.
- [ ] For each variant the record states the per-subject scores, the spread, the bands of the two populations and the two medians.
- [ ] The record states the same figures for the audited workspace's finding counts as published in the audit (1438 findings, 23 rules).
- [ ] The assumption holds when the combined variant keeps a spread of at least 20, keeps the two populations in different bands, and keeps the healthy median above the agent median.
- [ ] If it does not hold, US-025 and US-026 are marked BLOCKED with the failing figure, and the decision returns to Arthur with the variant that does pass.
- [ ] The record is written as a new document under `docs/`, named `core-v7-calibration-2026-11.md`, and listed under `frozen`.

#### US-024: Publish the cause findings share
**Description:** As a reader, I want findings that come from one decision to carry the same cause, so that 303 findings for one type read as one thing to change.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-021

**Acceptance Criteria:**
- [ ] `Diagnostic` gains an optional `cause` string (schema 25). It is absent for a finding that shares its cause with no declared key.
- [ ] `clippy::large_types_passed_by_value` sets it to the last path segment of the type under the finding's span and the size from the message, for example `UiColors (292 byte)`.
- [ ] `rust_doctor::repo::secret_in_build_step` sets it to the variable name.
- [ ] A rule declares its cause through one function in the catalog entry; no rule other than these two declares one in this release.
- [ ] The finding's `id`, message and span are unchanged, so the baseline and the delta are unaffected.
- [ ] The linear report lists the causes of a rule with more than one finding per cause, largest first, at most three: "310 findings, 5 causes: UiColors (292 byte) x303, RowPalette (264 byte) x3, ...".
- [ ] The "Spread across M files" line names the largest cause when it holds more than half of the rule's findings.
- [ ] Given a span whose text cannot be read from the file, when the cause is computed, then it is absent and the finding counts as its own cause.
- [ ] `project_current_wire_to_v7` drops the field, and every v7 fixture still projects.

#### US-025: Charge causes with diminishing returns
**Description:** As a reader, I want the score to count how many decisions a rule asks of me, so that three lines of YAML and 303 copies of one parameter type stop outweighing everything else.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-023, US-024

**Acceptance Criteria:**
- [ ] `weight(n)` is `1 + ln(n)` for `n >= 1` and 0 for `n = 0`.
- [ ] For a per-kiloline rule, the charged sites are the sum of `weight(n)` over its causes, where a finding without a cause is a cause of one site. A rule with no declared cause is charged exactly as under core-v6.
- [ ] For a workspace-scope rule, each cause counts once, and the rule's core-v6 charge is multiplied by `weight(c) / c`, where `c` is its number of causes.
- [ ] Given one workspace-scope P2 finding, when the score is computed, then the dimension is what core-v6 gives: a lone direct duplicate still scores 72.
- [ ] Given the same P1 variable in three workflows, when Security is computed, then it equals the value for one finding.
- [ ] Given 303 findings of one cause and 7 single findings of a P3 per-kiloline rule, when the charge is computed, then the charged sites are `1 + ln(303) + 7`, within 0.001.
- [ ] `projected_after_top_three` and the ranking use the same charges.
- [ ] ~~`SCORE_MODEL` becomes "core-v7"; the audit fixture `audit-core-v6.json` and its references move to v7.~~ Moved to US-032 (PRD 1.3): the oracle keeps its name and label, and its cases follow the shipped charges.
- [ ] `src/audit/tests/lambda_freeze.rs` is not edited and passes.
- [ ] `docs/subsystems/audit.md` and `skills/rust-doctor/SKILL.md` state the two formulas within their limits.

#### US-026: Use one precision prior in the score and the ranking
**Description:** As a reader, I want a rule nobody measured to weigh as uncertain everywhere, so that an unmeasured rule that is wrong on every site does not cost full price.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-023

**Acceptance Criteria:**
- [ ] `charge` (`src/audit/density.rs:16-22`) keeps `10000 - 5000` basis points of an unmeasured rule, the smoothed rate `src/policy/noise.rs` already gives a rule with no reviewed site.
- [ ] The ranking (`src/audit/ranking.rs:15-26`) applies no further factor to an unmeasured rule, so score and ranking agree.
- [ ] Tier ceilings still apply at the full tier of an unmeasured rule.
- [ ] Given one unmeasured P1 workspace finding, when Security is computed, then it does not exceed the P1 ceiling of 50, and the overall score does not exceed 65.
- [ ] Given a measured rule, when it is charged, then its charge is unchanged from core-v6.
- [ ] The report's term line (US-012) says an unmeasured rule is charged at half.

#### US-027: Publish a distrusted rule at Info, and require both thresholds for complex_function
**Description:** As a reader, I want a rule the corpus finds wrong on most reviewed sites to stop printing warnings, so that 127 findings the tool itself distrusts do not sit among the ones I should fix.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A finding of a rule whose smoothed noise exceeds 5000 bp is published with `severity: "info"` and keeps its `base_severity`.
- [ ] The existing guard that fails when a P0 or P1 rule becomes withheld still passes and is not weakened.
- [ ] Given `--rule <id>=warn` for such a rule, when the scan runs, then its findings are published at the requested level.
- [ ] `complex` (`src/structure/hotspots.rs:111-115`) reports a function only when its cyclomatic complexity and its cognitive complexity both reach their thresholds.
- [ ] Given a function with cyclomatic complexity 35 and cognitive complexity 1, when the rule runs, then nothing is reported.
- [ ] Given a function over both thresholds, when the rule runs, then it is reported with both figures as today.
- [ ] The rule's `not_a_defect_when` no longer lists the flat `match` case, since the rule no longer reports it.
- [ ] US-032 re-adjudicates the rule's corpus sites under the new condition.

#### US-028: Warn on a duplicate major only when a member can remove it
**Description:** As a reader, I want a duplicated crate version reported as a warning only when changing one of my manifests removes it, so that eight findings I cannot fix stop taking Dependencies to 5.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A duplicate is a warning when at least one resolved version of the crate is required only by workspace members. Every other duplicate is published at Info and weighs nothing.
- [ ] Given a crate where a member requires 0.5 and a third-party crate requires 0.6, with no other requirer of 0.5, when the rule runs, then it warns and names the member manifest.
- [ ] Given a crate where a member and a third-party crate both require the older version, when the rule runs, then it is Info.
- [ ] A "through" label never names a workspace member: the carrier is the first non-member package on a shortest path from a member to the version (`src/cargo_health/resolution.rs:322-344`).
- [ ] A "required by" label names up to three members in name order, then "+N more".
- [ ] Versions are listed in semver order: "0.6.4, 0.9.5, 0.10.1".
- [ ] Given two carriers at the same distance, when the label is built, then the one whose name sorts first is used, so the output is deterministic.
- [ ] The message changes are listed in the release note, since they change finding ids.

#### US-029: Publish what the score could not verify
**Description:** As a consumer of `--json`, I want the score to list the checks it was computed without, so that an agent or a dashboard can tell a clean result from an unchecked one.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-006

**Acceptance Criteria:**
- [ ] `audit.score.unverified` is an array of strings (schema 25). It holds `"advisories"` when both advisory rules are not evaluated, and is empty otherwise.
- [ ] `authoritative` keeps its definition: true exactly when `reasons` is empty. A missing database does not add a reason.
- [ ] Given an unreadable database (US-005), when the report is assembled, then `authoritative` is false and `unverified` holds `"advisories"`.
- [ ] The line of US-006 is driven by this field.
- [ ] `docs/subsystems/audit.md` states the difference between `reasons` and `unverified` in two sentences, and AGENTS.md states schema 25.
- [ ] `project_current_wire_to_v7` drops the field, and every v7 fixture still projects.

---

### EP-006: Rule texts, re-baseline and public claims

Release 0.12.0. Help texts can be applied where they are printed, the corpus measures the new detectors and model once, and the website states core-v7 when it ships.

**Definition of Done:**
- The help of the six rules below fits every audited site it was printed on.
- `tests/corpus.json` is replayed under core-v7, and `score_distribution`, `precision`, `gate` and `CORPUS_NOISE` agree with the replay.
- rust-doctor.com builds green with `CURRENT_MODEL = "core-v7"`.

#### US-030: Correct the rule texts that misdirect
**Description:** As a reader or an agent, I want each help text to be something I can do at the site it is attached to, so that following the tool never breaks correct code.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-019, US-020, US-021

**Acceptance Criteria:**
- [ ] `clippy::undocumented_unsafe_blocks`: the help offers two forms, a `// SAFETY:` comment or `#[expect(clippy::undocumented_unsafe_blocks, reason = "...")]`, and the title says "unsafe block or impl".
- [ ] `structure::orphan_module_file`: the help reads "Declare the file with a mod declaration. Remove it only if nothing should compile it."
- [ ] `structure::unreferenced_feature`: the message says "which no cfg and no build script reads".
- [ ] `source::blocking_call_in_async`: the help names the offload helper generically ("your runtime's blocking pool: `spawn_blocking`, `unblock`") and no single runtime.
- [ ] `cargo::permissive_lint_table`: the message states how many findings the entry hid in this scan, from the count `src/execution/manifest_allows.rs` already drops: "hides N findings in this package".
- [ ] A manifest finding on a `workspace = true` dependency points at the member's own line for that key (`src/cargo_health/keys.rs:82-86`), and its id is unchanged because dependency findings pass no span to the id.
- [ ] Given a `permissive_lint_table` entry that hid nothing, when it is reported, then the message says so.
- [ ] `rules list --json`, `tests/policy_cli.rs` and `tests/skill_contract.rs` agree with the new texts.
- [ ] The corpus `catalog` block is regenerated in US-032, not by hand.

#### US-031: Pin the audited workspace as a corpus subject
**Description:** As the maintainer, I want the 429 hand-classified sites of the field audit recorded in the corpus, so that the eleven rules that fired there without a precision rate get one.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The story starts only once Arthur confirms the repository is public and may be pinned at a revision; until then it is BLOCKED and US-032 proceeds without it.
- [ ] The subject is added at one pinned revision through the procedure of `docs/catalog-and-corpus.md`, with its clone in the cache outside this repository.
- [ ] Each audited site is entered through `.claude/skills/corpus-adjudicate`, by two blind passes, never typed into the measured blocks by hand.
- [ ] Given a site whose two passes disagree, when it is recorded, then it is settled by a third pass and the disagreement is kept in the record.
- [ ] The subject is scanned on Linux in CI, so its Windows-only findings are not part of the record; the story states how many audited sites that excludes.
- [ ] `corpus.yml` keeps its job structure; only the subject count and the clone cache key change.

#### US-032: Replay the corpus once under core-v7 and re-baseline
**Description:** As the maintainer, I want one corpus replay to measure every detector and score change together, so that the published calibration matches the shipped binary.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-038, which runs the replay on Fedora after US-017 to US-030

**Acceptance Criteria:**
- [ ] The record is built from US-038's artifacts, at the tree US-038 names; no part of the replay runs on Windows.
- [ ] New or changed sites of `orphan_module_file`, `complex_function`, `oversized_unit`, `duplicate_major_versions`, `blocking_call_in_async`, `unreferenced_feature` and `secret_in_build_step` are adjudicated by two blind passes.
- [ ] `position_proof` is regenerated, and `precision`, `agent_population.precision`, `gate` and `score_distribution` are recomputed with the existing helpers.
- [ ] `CORPUS_NOISE` (`src/policy/noise.rs`) matches the record.
- [ ] Every new candidate of US-038's `delta_pairing` output is classified in `tests/fixtures/delta-pairing/verdicts.json`.
- [ ] `SCORE_MODEL` becomes "core-v7" in the replay commit, and the audit fixture `audit-core-v6.json` and its references move to v7 (moved from US-025, PRD 1.3).
- [ ] `SCHEMA_VERSION` is 25, and every file that states the model or the schema agrees: `tests/audit_contract.rs`, `tests/score_credibility_kernel.rs`, `src/audit/tests.rs`, `tests/docs_contract.rs`, `tests/e2e/*.ts`, `npm/rust-doctor/scripts/smoke-packed.mjs`, AGENTS.md, `docs/subsystems/audit.md` and `skills/rust-doctor/SKILL.md`.
- [ ] Given a subject whose score moves by more than 10 points, when the replay completes, then the release note explains the move by the story that caused it.
- [ ] Given a replay that fails the distribution gate, when it completes, then the release does not proceed and the failing subjects are listed in this story's notes.
- [ ] `.github/releases/v0.12.0.md` lists every reworded message, since each changes finding ids and the baseline.

#### US-033: Bring rust-doctor.com and the release skill in line with core-v7
**Description:** As the maintainer, I want the website and the release procedure to state the shipped model, so that no page publishes a retired formula.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-032

**Acceptance Criteria:**
- [ ] In the `rust-doctor-web` checkout, `CURRENT_MODEL` (`src/app/share/share-contract.ts`), `terminal-data.ts`, `src/lib/data.ts` and, under its `content/docs/` folder, `scoring.mdx` state core-v7, with the cause rule and the unmeasured prior.
- [ ] In the same folder, `report.mdx` documents `cause`, `unverified` and `unwalked_compiled_files`.
- [ ] `public/catalog/rules.json` and `public/catalog/corpus.json` are regenerated by `catalog:sync` and `corpus:sync`.
- [ ] Given a share link carrying `m=core-v6`, when the site renders it, then the card says the number comes from a model Rust Doctor has retired.
- [ ] `.claude/skills/release/references/site.md` names core-v7 in its model step.
- [ ] Given the site, when `bun run build:verified` runs, then it passes.

### EP-007: Linux validation on Fedora

Release 0.12.0. Every gate and criterion that needs Linux runs on Arthur's Fedora machine, once per epic, so a Windows session certifies what Windows observes and no Linux verdict depends on WSL.

**Definition of Done:**
- EP-004 and EP-005 each have one recorded Fedora run on the tree they were certified at, with every Quality Gate green.
- The 0.12.0 release candidate, which carries EP-006, passes every Linux job of `ci.yml` on Fedora, then in CI.
- A failure that Fedora and CI disagree on is recorded as an environment difference, never as a pass.
- The core-v7 corpus replay runs once on Fedora, and its artifacts are what US-032 builds the record from.

#### US-034: Replay CI's Linux jobs on Fedora
**Description:** As the maintainer, I want one documented way to run CI's Linux jobs on my Fedora machine against a given tree, so that a Linux verdict there means what a CI run means.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The recipe lives in `docs/testing.md` within its word ceiling, or in a new document linked from AGENTS.md with a ceiling in `docs/doc-budgets.json`.
- [ ] It installs toolchain 1.99.1 with clippy and the `x86_64-pc-windows-msvc` target, which the `Tests` job adds, toolchain 1.95 for the MSRV check, and `stable` with clippy, which the baseline fixture's `rust-toolchain.toml` pins.
- [ ] It runs with `CARGO_TARGET_DIR` unset and no `target-dir` in any `config.toml` Cargo reads from the checkout or from `CARGO_HOME`, so each fixture builds into its own `target/`.
- [ ] It installs tmux, strace and jq under `/usr/bin`, bun at the version `tests/e2e/package.json` names for the e2e suite and bun 1.3.14 for the launcher jobs, and allows unprivileged user namespaces so `E2E_REQUIRE_NETNS=1` holds.
- [ ] It sets CI's environment: `RUST_DOCTOR_BENCHMARK_ALLOWANCE=3` for every test run, and `E2E_REQUIRE_TMUX=1`, `E2E_REQUIRE_NETNS=1` and `RUST_DOCTOR_E2E_BIN` for the e2e suite, which runs through `bash ci-run-files.sh` on the files `bun ci-shards.ts` plans.
- [ ] Every test run passes `--locked` after one `cargo fetch --locked`.
- [ ] Given a tree under review, when it is replayed, then the checkout's `git write-tree` prints the reviewed tree id, and the record names it.
- [ ] Given a failure on the reviewed tree, when it is judged, then the same test runs alone on the tree's parent commit in the same environment, and a failure both share is recorded as pre-existing, not as a pass.
- [ ] Given a test that passes on Fedora and fails in CI, or the reverse, when it is recorded, then it is named an environment difference: Fedora ships GNU coreutils, while CI's ubuntu-26.04 image ships uutils.

#### US-035: Validate EP-004 on Linux
**Description:** As the maintainer, I want EP-004's tree run through every Linux gate, so that its detector changes are proven where CI and most users run them.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-034, US-017, US-018, US-019, US-020, US-021, US-022

**Acceptance Criteria:**
- [ ] On the tree EP-004 was certified at, or the commit that lands it, every Quality Gate passes on Fedora.
- [ ] `cargo test --locked` passes `scan_failures`, `score_credibility_packs` and `source_kernel_cli`, whose ledger-listed tests no platform observed for EP-004 on 2026-11-16.
- [ ] `tests/detector_false_positives.rs` passes, which confirms assumption A2 on Linux: dep-info lists the files another member's macro declares.
- [ ] The e2e suite passes `known-bugs.test.ts`, `cli-surface.test.ts` and `output-streams.test.ts`, the files EP-004 edited.
- [ ] Given a failure of a wall-clock budget (`source_kernel::tests::benchmark`, `structure::benchmark`, `presentation_nfr`), when it is judged, then it is rerun alone on the tree and on its parent, as US-034 states, before it counts.
- [ ] The run is recorded in the `notes.linux` of US-017 to US-022 with the tree id, the toolchain and the pass counts.
- [ ] Given a Linux failure EP-004 caused, when it is found, then the story it traces to returns to IN_REVIEW and its fix is certified through `/review-epic`.

#### US-036: Validate EP-005 on Linux
**Description:** As the maintainer, I want core-v7 run through every Linux gate, so that the score a Linux user sees is the one Windows certified.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-034, US-023, US-024, US-025, US-026, US-027, US-028, US-029

**Acceptance Criteria:**
- [ ] On the tree EP-005 was certified at, every Quality Gate passes on Fedora.
- [ ] `tests/audit_contract.rs`, `tests/score_credibility_kernel.rs` and `src/audit/tests.rs` pass on Fedora with the exact scores they assert on Windows, which settles the floating-point question that Technical Considerations raises for `1 + ln(n)`.
- [ ] `src/audit/tests/lambda_freeze.rs` passes untouched.
- [ ] US-023's harness reproduces its committed record byte for byte on Fedora.
- [ ] The run is recorded in the `notes.linux` of US-023 to US-029.

#### US-037: Validate the 0.12.0 release candidate on Linux
**Description:** As the maintainer, I want the release candidate to pass every Linux job before the tag, so that 0.12.0 ships nothing only Windows has seen.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-035, US-036, US-030, US-032, US-033, US-038

**Acceptance Criteria:**
- [ ] On the release candidate commit, every Quality Gate passes on Fedora, together with the Node launcher tests (`cd npm/rust-doctor && bun test tests`), the packed smoke (`bun run smoke:packed` against the release binary) and `cargo check --locked --all-targets` under toolchain 1.95.
- [ ] The corpus test `corpus_precision` passes on Fedora from the clone cache against US-032's record.
- [ ] The CI run of the candidate's push is green on ubuntu and macOS before the tag; the Fedora run does not replace it.
- [ ] Given a candidate that fails any of these gates, when the run completes, then the release does not proceed and the failing gate is listed in this story's notes.

#### US-038: Produce the core-v7 corpus artifacts on Fedora
**Description:** As the maintainer, I want the corpus replay run on a native Linux host, so that the record US-032 builds is a Linux measurement that US-037 can replay on the same machine.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-034, US-017, US-018, US-019, US-020, US-021, US-022, US-024, US-025, US-026, US-027, US-028, US-029, US-030

**Acceptance Criteria:**
- [ ] The replay runs on a committed tree that carries every story it is blocked by, under toolchain 1.99.1 and US-034's environment, with the clone cache outside this repository at the pinned commits of `tests/corpus.json`.
- [ ] `RUST_DOCTOR_CORPUS_DIR=<clone cache> RUST_DOCTOR_CORPUS_ARTIFACTS=<artifacts> cargo test --locked --test corpus_precision` runs to completion, and its artifacts and `position-proof.json` are handed to US-032 unedited.
- [ ] `cargo test --release --test delta_pairing -- --ignored --nocapture` runs on the same tree, and its output is handed to US-032.
- [ ] The run is recorded in this story's notes with the tree id, the toolchain, the subject count and the artifact digests.
- [ ] Given a test of the replay that fails on the old record, when it is judged, then it is listed for US-032 as a measured change, never as a pass or a defect.
- [ ] Given a subject whose clone is missing or at another commit, when the run starts, then the run stops and names it.

---

## Functional Requirements

- FR-01: A path Cargo places inside the workspace root must be judged inside it on every platform, whatever form either path takes.
- FR-02: No file under `src/` other than `src/path_form.rs` may call `canonicalize`.
- FR-03: Advisory discovery must resolve the Cargo home as Cargo does: `CARGO_HOME`, else `HOME`, else `USERPROFILE` on Windows.
- FR-04: Advisory parsing must give the same result for LF and CRLF files.
- FR-05: A database that yields no advisory, or skips more than 5% of its files, must produce a `ReportError` at stage `dependencies` and a non-authoritative score.
- FR-06: A report scored without the advisory check must say so beside the score, and from schema 25 in `audit.score.unverified`.
- FR-07: The Windows CI job must fail on a failing test absent from the ledger and on a ledger entry that passes.
- FR-08: The rule list must be ordered by a key printed on each row, and every count must name what it counts.
- FR-09: Every command-line option must carry a description.
- FR-10: A module declared through a `macro_rules!` of the same package must be walked, counted and read by every native rule.
- FR-11: The system must NOT report a file as an orphan when Cargo's dep-info lists it.
- FR-12: A blocking call inside a closure passed to `unblock`, `spawn_blocking`, `spawn` or `block_in_place` must not be reported.
- FR-13: A feature a build script reads through `CARGO_FEATURE_<NAME>` must count as referenced.
- FR-14: A variable the workspace embeds with `env!` or `option_env!` must be reported at Info, with a message naming the embedding site.
- FR-15: A file is production only when at least one route to it is a production route.
- FR-16: Findings sharing a declared cause must be charged `1 + ln(n)` sites, and a workspace-scope rule `1 + ln(c)` causes.
- FR-17: An unmeasured rule must be charged at the 5000 bp prior in the score and in the ranking alike.
- FR-18: A rule above 5000 bp of smoothed noise must publish at Info unless the request sets its level.
- FR-19: A duplicate major must warn only when a version is required by workspace members alone.
- FR-20: The system must NOT change a finding's id, message or span when it adds a cause.

## Non-Functional Requirements

- **Performance:**
  - The warm scan of the audited workspace stays at or under 28.4 s. The baseline is 25.8 s on 2026-11-15, and the budget allows 10% for the macro resolution and the reconciliation.
  - The warm self-scan stays at or under 840 ms.
  - The reconciliation reads at most 8 MiB of dep-info, the existing budget.
  - The Windows job finishes within its 45-minute timeout.
- **Security:**
  - 0 network connections, unchanged.
  - 0 absolute paths in `--json`; a home directory in the linear report is printed as `~` in 100% of the new lines.
  - 0 scans report the advisory check as run when fewer than 95% of the database's files were read.
- **Reliability:**
  - 0 `orphan_module_file` findings on a file listed in dep-info, across the fixtures of US-017 and US-018.
  - 0 of the 74 audited false findings reproduce on their fixtures.
  - Two scans of one tree stay identical: same findings, ids, order and score.
- **Compatibility:**
  - Schema 25 adds 3 fields and removes 0.
  - The v7 archive projects for 100% of its fixtures.
  - Adding a cause changes 0 finding ids.
  - Releases 0.10.1 and 0.11.0 change 0 JSON values on a report that held no Windows or advisory defect.
- **Maintainability:**
  - Every file stays under 1000 lines and every impl under 500.
  - Every touched document stays within 85% to 100% of its word ceiling.
  - The Windows ledger never grows: 0 entries added after US-008 without a story.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | UNC workspace (boundary) | Workspace on `\\server\share` | `\\?\UNC\server\share\x` and `\\server\share\x` compare equal | None |
| 2 | Long path (boundary) | Plain form of 260 characters or more | Verbatim form kept; comparisons still normalize both sides | None |
| 3 | Reserved device name (boundary) | A component named `NUL` or `COM1` | Verbatim form kept | None |
| 4 | Path that really leaves the workspace | `path = "../../elsewhere"` | Finding reported as today | Existing message |
| 5 | No home variable (empty state) | `CARGO_HOME`, `HOME` and `USERPROFILE` unset | No database, no directory read, check disclosed as skipped | "Known-vulnerability check skipped: no RustSec database under ~/.cargo. ..." |
| 6 | Two homes | Git Bash sets `HOME`, Windows sets `USERPROFILE` | `HOME` wins, as for Cargo | None |
| 7 | Unreadable database (error state) | CRLF before the fix, a truncated clone, a schema the parser rejects | Stage error, non-authoritative score | "The RustSec database at <path> could not be read: N of M files were skipped. ..." |
| 8 | Partly unreadable database | Skipped files under 5% | Check runs; count disclosed | "N advisory files could not be read." |
| 9 | Advisory check turned off | `--advisory-db /dev/null` | Disclosed beside the score, naming the flag | "... scored without the known-vulnerability check (--advisory-db /dev/null)." |
| 10 | New Windows failure | A test fails that the ledger does not list | Job fails | "new failure: <binary> <test>" |
| 11 | Stale ledger entry | A listed test passes or was renamed | Job fails | "passes now, remove it: ..." or "unknown test: ..." |
| 12 | Macro argument naming no file | `native_modules!(a, missing)` | `missing` ignored | None |
| 13 | Macro from another crate or a proc macro | Declaring macro not in the package | Not resolved; compiled set vetoes the orphan finding | "N compiled files were not read by the native rules: ..." |
| 14 | No compiled set (external dependency) | Clippy disabled, failed, or dep-info over 8 MiB | No veto; syntactic resolution only | None |
| 15 | Feature with a dash (boundary) | `foo-bar` read as `CARGO_FEATURE_FOO_BAR` | Counted as read | None |
| 16 | Variable embedded and also secret | `option_env!("TOKEN")` with a real secret | Info finding naming the embedding site | "... embeds it in the binary: whatever it holds ships to every user." |
| 17 | One site (boundary) | A cause with a single finding | `weight(1)` is 1; charge equals core-v6 | None |
| 18 | Unreadable span | Cause text cannot be read from the file | No cause; the finding is its own cause | None |
| 19 | Withheld P0 or P1 | Re-adjudication pushes a P0 or P1 rule over 5000 bp | Guard test fails in CI | Test names the rule |
| 20 | Interrupted run | Ctrl-C during the scan | Existing interrupt path, exit 130 | None |
| 21 | Non-terminal stderr | Output redirected to a file | One closing progress line, no per-member lines | "Linted N members." |

Network degradation, permission changes and concurrent modifications do not apply. The tool opens no connection, has no sessions, and a scan is one process over a read-only view.

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | core-v7 moves every corpus subject and the distribution gate fails | Med | High | US-023 measures the variants on the record first; US-025 and US-026 are blocked on it; one replay, run by US-038 and recorded by US-032 |
| 2 | Halving unmeasured rules raises every score and hides a severe unmeasured rule | Med | Med | Tier ceilings apply at the full tier (US-026 test); US-031 measures eleven of those rules |
| 3 | The macro resolution follows a module that is not one, and reads a file twice or under the wrong `cfg` | Med | Med | Only identifiers that resolve to an existing file are followed; attributes come from the transcriber; fixture with a non-module argument |
| 4 | Native findings depend on whether Clippy ran, through the dep-info veto | Med | Low | The veto only removes orphan and unused-dependency findings; `scan.unwalked_compiled_files` publishes when it applied |
| 5 | The Windows job is slow or flaky and gets ignored | Med | Med | 45-minute timeout kept; benchmark allowance set; a `wall-clock` class in the ledger; one job, one check |
| 6 | Reworded messages change ids, and baseline users see old findings as fixed and new ones as introduced | High | Med | All rewordings ship in 0.12.0 with the model bump; the release note lists each; adding a cause changes no id |
| 7 | A truly secret value read by `option_env!` is demoted to Info and missed | Low | High | The Info message says the value ships to every user and names the site; the P1 case is unchanged when nothing embeds the variable |
| 8 | Files near the 1000-line bound cannot absorb new code (`src/report/assembly.rs` 837, `src/source_kernel/detectors.rs` 793, `src/audit.rs` 792) | High | Low | Stories that add code there name a new submodule; the hotspot gate catches a miss |
| 9 | Documents have 3 to 61 words of room | High | Low | Each story replaces text instead of adding; a ceiling is raised only with a stated reason |
| 10 | Website and CLI ship at different times | Med | Med | US-033 is blocked only by US-032; the retired-model card covers old links |
| 11 | The audited workspace cannot be pinned as a corpus subject | Med | Low | US-031 is P2 and blocked on Arthur's answer; US-032 does not depend on it |

## Non-Goals

- **Linting test targets with Clippy (`--all-targets`).** Decided in the previous PRD: it would compile dev-dependencies offline and depend on each repository's `allow-*-in-tests` settings. The asymmetry is disclosed (US-015).
- **Linting for targets other than the host.** It would need each target's toolchain and sysroot. The limit is disclosed (US-015); revisit if a user asks for `--target`.
- **New detections the audit suggested.** A signing key imported in a job that compiled third-party code, a remote script piped into a shell when the command is built as data, `ureq` and smol coverage for the HTTP and `block_on` rules. Each is a rule change that needs evidence and a corpus measurement; they go through `.claude/skills/rule-candidate`.
- **Reviewing the audited workspace's own security.** The audit listed twelve trust-boundary problems there. They belong to that project, and no pattern rule can express ten of them.
- **A full port of the test suite to Windows.** The ledger makes Windows a gate now; entries are fixed as their area is touched.
- **Changing λ or the dimension weights.** The lambda freeze stays. core-v7 changes what is counted, not how density maps to a dimension.
- **Merging findings that share a cause into one finding.** It would change ids, the baseline and the delta. The cause is a key on each finding.
- **Cause extractors beyond two rules.** `rc_buffer`, `oversized_unit` nesting and others wait for evidence from a second workspace.
- **Renaming the JSON category value "Bugs".** A wire value change breaks the Action and the site; only the rendered label changes.
- **Fetching the advisory database.** It breaks the no-network boundary. The report names the two commands that fetch it.
- **A `dunce` or `home` dependency.** Both earlier PRDs forbid a new crate, and the needed behavior is about forty lines of std code.

## Files NOT to Modify

- `.github/workflows/release.yml`, `corpus.yml`, `dogfood.yml`, `install.yml`, `action.yml`, `.github/releases/` and version fields: releasing is a separate step through `.claude/skills/release`. Exceptions: US-007 and US-009 edit the `windows` job of `ci.yml` only, US-031 edits the subject count of `corpus.yml`, and US-032 writes `.github/releases/v0.12.0.md`.
- `src/source_kernel/detectors/secrets/`: permission-blocked for agents; no story needs it.
- `src/audit/tests/lambda_freeze.rs` and the λ values of `src/audit/density.rs:98-111`: frozen by decision.
- `Cargo.lock` beyond what `cargo check` refreshes: no dependency is added or bumped.
- `tests/fixtures/*/v7-*.json`: frozen archives, projected, never edited.
- The measured blocks of `tests/corpus.json` (`adjudication`, `position_proof`): regenerated by the replay, never typed by hand.
- Existing rows of `tasks/e2e-catalog/*.md` and `tests/e2e/known-bugs.json`: append-only.
- The documents listed under `frozen` in `docs/doc-budgets.json`: measurement records, not edited afterwards. US-023 adds one.
- `tasks/prd-security-audit-remediation.md`, `tasks/prd-self-scan-audit-remediation.md` and their status files: completed PRDs.

## Technical Considerations

Questions for engineering input, not mandates:

- **Path form:** Should `canonical` return the plain form, or should every consumer keep verbatim paths and only comparisons normalize? Recommended: plain form at the source plus normalizing comparisons, since Cargo and git print plain paths and reports must match them. Engineering to confirm no consumer relies on the verbatim form for long paths beyond the two cases of A1.
- **Reconciliation stage:** Where should the dep-info veto and the embedded-variable demotion live? Recommended: one `src/report/reconcile.rs` called from assembly, since both need facts from two passes that run in parallel. Trade-off: findings then depend on pass results joined late. Alternative: run the repository pass after the source kernel, which costs the parallel start.
- **Macro resolution:** Is matching `mod $x;` in the transcriber enough, or should `mod $x { ... }` and nested macro calls be followed? Recommended: the declaration form only; the compiled set covers the rest. Engineering to confirm against the corpus in US-032.
- **Data model:** Should `cause` be a string or an object with a kind and a label? Recommended: a string, to keep schema 25 minimal. Revisit when a third rule declares one.
- **Model shape:** Is `1 + ln(n)` the right curve, or should a cause be capped at a fixed number of site-equivalents? Recommended: the logarithm, since it needs no constant. US-023 records both curves so the choice is measured. Engineering to confirm the floating-point result is stable across platforms to the rounding the score applies.
- **Unmeasured prior:** Should the 5000 bp prior apply to every unmeasured rule, or should P0 and P1 rules keep full price? Recommended: every rule, with the ceilings guarding severity. Trade-off: a P1 workspace finding moves from 37 to 61 before the ceiling brings it to 50.
- **Windows ledger:** Should the ledger key on test names alone, or on names plus a failure signature? Recommended: names alone; a signature would churn with every message change.
- **Migration:** Schema 25 is additive and the v7 archive keeps projecting. Rollback: a 0.12.x patch can restore core-v6 charges behind the same schema, and the share link's `m=` key keeps old cards labeled.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Confirmed false findings on the audited workspace | 74 (37 path, 29 orphan, 6 blocking, 1 feature, 1 unused dependency) | 0 | Release 0.12.0 (37 or fewer at 0.10.1) | Rescan the audited revision and recheck the 74 sites |
| Advisories read from a CRLF database of 1489 files | 0 | 1489 | Release 0.10.1 | `scan.advisory_db` with `--advisory-db` on the CRLF clone |
| Known-vulnerable or unmaintained crates reported on the audited lockfile | 0 of 8 | 8 of 8 | Release 0.10.1 | `--json` with the local database |
| Windows test failures that reach `main` unseen | 103 at HEAD, none gated | 0 unlisted; ledger under 103 | Release 0.10.1 | `scripts/windows-test-gate.sh` in CI |
| Report lines the audit quotes as misleading | 10 | 0 | Release 0.11.0 | Linear output of the audited workspace against the audit's list |
| Options without a description | 6 | 0 | Release 0.11.0 | The help-coverage test of US-014 |
| Score points the audited workspace loses to false or non-actionable findings | at least 29 (65 to 36 on one embedded key) | 0 | Release 0.12.0 | `audit.score` with and without those findings |
| Site-equivalents charged for 303 findings of one type | 303 | 6.7 | Release 0.12.0 | Unit test of US-025 and the rescan |
| Rules that fired on the audit with no precision rate | 11 of 23 | 0 of 23 if US-031 ships, else unchanged | Release 0.12.0 | `policy.rules[].corpus_reviewed_sites` |
| Warm scan time on the audited workspace | 25.8 s | at most 28.4 s | Each release | `--timings` on a warm target |

## Open Questions

- **Authoritative with no database.** This PRD keeps `authoritative: true` when no database exists and adds `unverified`. Should a missing database instead drop `authoritative`, as the audit first recommended? Arthur decides before US-029; US-006 and the website copy depend on it. Decided 2026-11-16: `authoritative` stays true and the missing check is published in `unverified`.
- **Corpus subject.** Is the audited repository public, and may it be pinned at a revision in `tests/corpus.json`? Arthur answers before US-031 starts; eleven precision rates depend on it. Answered 2026-11-16: public, to be pinned at the last revision committed before the audit was written. It fails the agent population's selection criterion, so US-031 is cancelled for 0.12.0 and carried to a later release (PRD 1.5).
- **Naming the audited workspace.** This document does not name it, because `tasks/` is tracked in a public repository. Arthur says whether it may be named, here and in the release note.
- **Release cadence.** Should 0.10.1 and 0.11.0 ship separately, or as one 0.11.0? Arthur decides before tagging; EP-003 does not need EP-001.
- **Windows runner cost.** A full test run on `windows-latest` on every push adds about 20 to 30 runner minutes. Should it run on pull requests only? Arthur decides during US-007.
- **`complex_function` condition.** Requiring both thresholds drops 23 of the 127 audited findings. Should the rule key on cognitive complexity alone instead? Arthur decides on US-032's adjudication.
- **Website window.** Can rust-doctor.com deploy core-v7 copy within 24 hours of the 0.12.0 tag? Arthur confirms during US-033.
[/PRD]
