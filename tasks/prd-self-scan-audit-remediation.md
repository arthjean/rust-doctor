[PRD]
# PRD: Self-Scan Audit Remediation

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-10-28 | Arthur Jean | Initial draft from the 0.9.0 self-scan audit (`bunx -y rust-doctor@latest .`, score 89, core-v5, schema 23) |

## Problem Statement

On 2026-10-28, `bunx -y rust-doctor@latest .` (0.9.0, score model core-v5, schema 23) scored this repository 89 "Great". The run reported 184 diagnostics, 249 occurrences. A cold rescan with an empty target directory reproduced every diagnostic and the score exactly. The arithmetic is correct: (4·100 + 3·75 + 2·100 + 2·100 + 2·64 + 6) / 13 = 89 (`src/audit.rs:969-976`). The audit then checked every finding, rebuilt the score by hand, and reproduced each defect below on a throwaway crate.

1. **A scan dies on ordinary text.** `database_passwords` adds 1 to the byte index of a character found by a char predicate (`src/repo_hygiene/credentials.rs:356-359`). When a multibyte character sits before `://`, the slice panics. Three paths were reproduced:
   - a tracked README holding `“https://…` ends the run with exit 101 and no report;
   - a Rust comment holding `“postgres://…` beside a finding crashes `--verbose` through the code frame (`src/presentation/code_frame.rs:414`);
   - a `build.rs` printing the same text crashes the Clippy thread through `src/cargo_stderr.rs:78`, and the report comes back `status: failed` with 0 diagnostics.

   Nothing contains a panic. `src/` holds no `catch_unwind`, and joins re-raise with `resume_unwind` (`src/execution.rs:506`). That contradicts AGENTS.md: "a pass that fails degrades to a complete report carrying a `ReportError` at its stage". Two neighbors were found by reading the code:
   - `BoundedChild::drop` does not kill its child (`src/execution/process.rs:122-133`);
   - `split_at_width` loops forever at width 1 on a double-width character (`src/tui/text.rs:347-394`).
2. **The score moves on things the reader cannot act on.** The 89 is "100 minus two mechanisms", each worth about 5 points.
   - **A withheld rule still caps.** One `clippy::exit` site caps reliability at 75. The corpus judges that rule wrong on 86% of its sites, and the report withholds it as noise. The P2 ceiling ignores the noise discount (`src/audit.rs:761-767`), so that one correct signal-watcher site (`src/execution/interrupt.rs:297`) costs 5 points.
   - **Transitive duplicate majors cost the rest.** winnow and rustc-hash arrive only through `ra_ap_syntax` and `toml`. They take dependencies to 64, and they become the top "fix this rule" advice. The code charges every duplicate alike (`src/cargo_health/resolution.rs:256-266`), against the comment that says such a duplicate publishes at Info (`src/audit/density.rs:40-42`).
   - **The report contradicts itself.** It claims the withheld rules leave "nothing worth repairing" (`src/render.rs:863-882`), yet removing `exit` alone gives 94.
3. **The counts mix what is scored with what is not.**
   - The Maintainability tally reads 138 warnings, and 137 of them are unscored test findings. The share URL carries `w=192`, and 140 of those are unscored.
   - The production-line denominator (46,574) includes about 6,000 lines of inline `#[cfg(test)] mod tests`, although `src/audit.rs:51` says test code is excluded.
   - The text output never prints the five dimensions or the ceiling that binds.
   - Clippy lints lib and bin targets only, while the native passes read `tests/` too. The report never says so.
4. **Detectors flag what no reviewer would act on.**
   - Of 50 duplicate-function families, 28 hold only `#[test]` functions. 8 are wrong:
     - macro arguments are erased while the message says "names and literals";
     - "at least X% alike" is the weakest single-linkage edge;
     - nested functions are reported twice.
   - A Cargo.toml `[lints.clippy] print_stdout = "allow"` does not hide the rule, because the scan's `-W` wins. `permissive_lint_table` then charges the allow too, as a P2.
   - `crate_level_allow` flags `#![allow(unsafe_code)]`, toolchain-compatibility pairs, and the shared `tests/support/mod.rs` dead-code allow.
   - `major_of` reads only the first number, so `0.59` and `0.61` count as one major (`src/cargo_health/resolution.rs:432`). Cargo treats them as incompatible.
5. **Documents misstate their own gates.**
   - AGENTS.md says `tests/score_credibility_packs.rs` fails on any production `unwrap`/`expect`/`panic!`/`dbg!`. That test filters to the concurrency pack (lines 416-440).
   - AGENTS.md also says fourteen `the_X_holds_the_size_bound` tests enforce the file and impl bounds. They omit `src/repo_hygiene.rs`, `src/presentation.rs`, `src/main.rs`, `src/ci.rs` and `src/lib.rs`, and check no impl.
   - Comments and SKILL.md state stale constants.

**Why now:** 0.9.0 is the version `bunx rust-doctor@latest` serves. The agent hooks, the Action and `ci install` put the scan into loops where a crash means a red build and a misattributed score means an agent "fixing" intentional test code. The reproductions are fresh, so each can become a regression test before the code drifts. The website restates the model in six places, so the score-model changes must ship once, together.

## Overview

The plan ships in three releases, each independently useful.

**Release 0.9.1 (EP-001)** makes a scan survive its input:
- the credential scanner finds the URL scheme on bytes;
- every pass runs under `catch_unwind` and degrades into a `ReportError` at its stage;
- a Clippy failure keeps the native findings;
- a dropped child is killed;
- the TUI wrap always advances.

The crate turns on Clippy's `string_slice` for its own code, and a non-ASCII table test pins every text scanner to char boundaries. Nothing in the score or the report shape moves.

**Release 0.10.0 (EP-002)** changes only what the report says. It states the criterion that withholds a rule, prints the five dimensions, discloses the scan's scope and the rules it did not evaluate, and counts only weighing findings in the text tallies. It also reads the freshest RustSec clone, fixes the Clippy phase timings, corrects the drifted documents, and clears the two self-scan sites that are correct by design.

**Release 0.11.0 (EP-003 to EP-005)** carries every change that moves a score or the report shape. It ships as score model `core-v6` and `SCHEMA_VERSION` 24, behind a single corpus replay.
- **Score model:**
  - a withheld rule caps nothing;
  - the ranking discounts a measured rule once;
  - a duplicate no member requires publishes at Info;
  - versions are compared by Cargo compatibility class;
  - inline test-gated items leave the denominator;
  - the JSON publishes the binding ceilings and the production tallies.
- **Detectors:**
  - drop test-case-only duplicate families and nested repeats;
  - fingerprint macro arguments and publish the lowest pairwise similarity;
  - honor a manifest `[lints]` allow;
  - narrow `crate_level_allow`;
  - count a unit from its keyword.
- **Catalog:** `indexing_slicing` and `string_slice`, adjudicated wrong on all 40 corpus sites each, become opt-in after a validation spike. rust-doctor.com and the release skill follow the model in the same window.

Key decisions:
- Containment wraps each pass rather than the whole run, and the error message never carries the panic payload.
- A manifest allow is honored by filtering per package after Clippy, not by dropping the `-W`. A `-W` applies workspace-wide and cannot express one member's allow.
- A transitive duplicate keeps `base_severity: warning`, so fingerprints, baselines and position proofs keep their ids.
- `clippy::exit` keeps tier P2, because the ceiling change already stops it from capping.
- Clippy keeps linting default targets only. The asymmetry is disclosed instead.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Inputs that abort a scan (exit 101 or `status: failed` from a panic) | 0 of the 3 reproduced paths (release 0.9.1) | 0 across the non-ASCII table and every new text scanner |
| Self-scan score points explained by a withheld rule or an unactionable transitive duplicate | 0 of the current 11 (release 0.11.0) | 0 |
| Self-scan duplicate families that are wrong or test-case-only | at most 5 (30 of 50 today) | at most 5, with no wrong family |
| Text tallies that count unscored findings as warnings | 0 categories (release 0.10.0) | 0 |
| Documents whose stated gate or constant disagrees with the code | 0 of the 9 listed in US-013 | 0, every one enforced by an existing doc gate or test |

## Target Users

### Rust developer running a scan
- **Role:** A developer, often a vibecoder, running `bunx -y rust-doctor@latest .` on a workspace they own.
- **Behaviors:** Reads the linear report and the score, follows "Fix this rule to reach N", and shares the link.
- **Pain points:**
  - a README with typographic quotes before a URL crashes the run;
  - the top advice names a transitive duplicate nothing in the workspace can change;
  - a correct `process::exit` in a signal path costs 5 points;
  - Maintainability shows 138 warnings, 137 of them in tests.
- **Current workaround:** None for the crash. The others need reading `--json` and the source to learn what is scored.
- **Success looks like:** The scan never aborts. Every point lost traces to a finding the developer can fix, and the text says what was scored.

### Coding agent consuming `--json` or the hooks
- **Role:** An agent reading the report through `rust-doctor hook` or `--json` and acting on the findings.
- **Behaviors:** Iterates over `diagnostics`, `summary` and `audit.categories`, and repairs what it reads.
- **Pain points:**
  - totals mix scored and unscored findings;
  - no field names the ceiling that binds;
  - "same 43-node body" claims send it to factor tests that differ inside their macros.
- **Current workaround:** Filter on `context` by hand, which the documents never ask for.
- **Success looks like:**
  - `audit.score.ceilings[]` names what binds;
  - `audit.categories[].production` separates what weighs;
  - every duplicate claim holds pairwise.

### CI maintainer
- **Role:** A maintainer running the Action, `ci install`, or the git hook on every pull request.
- **Behaviors:** Gates on `--blocking error` and reads the sticky comment.
- **Pain points:**
  - a panic turns the job red with exit 101 and no report;
  - a `build.rs` that prints non-ASCII text discards every native finding;
  - a Cargo.toml allow the project chose is overridden and charged.
- **Current workaround:** Re-run, or pin an older version.
- **Success looks like:**
  - a pass failure yields a complete, non-authoritative report and an error naming its stage;
  - a manifest allow is honored and charged once.

### Maintainer of rust-doctor and rust-doctor.com
- **Role:** Arthur, sole maintainer of the crate and the website.
- **Behaviors:** Releases through `.claude/skills/release` and keeps the site in step.
- **Pain points:**
  - AGENTS.md names tests that do not enforce what it claims;
  - the site restates the score model in six places that no gate compares to the binary;
  - "100 means nothing detected" becomes false once withheld rules stop capping.
- **Current workaround:** Grep the site for retired constants after each release.
- **Success looks like:** Every claim in AGENTS.md and the linked documents names the gate that actually holds it, and the site states core-v6 the day it ships.

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **cargo-deny:**
  - `bans.multiple-versions` defaults to `warn` and ignores dev-only edges (`multiple-versions-include-dev = false`).
  - It offers `skip` for upstream migrations and `skip-tree` ("a very big hammer") for transitive trees.
  - `highlight = simplest-path` points at the duplicate "often the best candidate for removal or upgrade" ([bans cfg](https://embarkstudios.github.io/cargo-deny/checks/bans/cfg.html)).
  - rust-doctor charges a transitive duplicate at full weight and gives no actionable path. US-017 aligns it.
- **SonarQube:**
  - The maintainability rating is debt over 30 min × LOC, graded A to E, on a configurable grid with a separate "new code" rating ([metric definitions](https://docs.sonarsource.com/sonarqube-cloud/managing-your-projects/metric-definitions)).
  - Duplication needs at least 100 successive tokens over 10 lines ([10.4 metric definitions](https://docs.sonarsource.com/sonarqube-server/10.4/user-guide/metric-definitions)).
  - rust-doctor's floors are 30 nodes and 3 statements (`src/structure/duplication.rs:61,73`), and it treats macro bodies as opaque.
- **Code Climate / Qlty:** Duplication uses a per-language AST "mass" threshold and separates "identical" from "similar" ([duplication concept](https://docs.codeclimate.com/docs/duplication-concept), [Qlty](https://docs.qlty.sh/duplication)).
- **react-doctor** (`/home/arthur/dev/react-doctor`, HEAD `84ad0c8d0`, read in this audit):
  - The score is computed server-side (`packages/core/src/calculate-score.ts:9-21`, `constants.ts:164`), which rust-doctor's no-network rule forbids.
  - Its surface model is local: the `score` and `ciFailure` surfaces drop test and story file contexts (`packages/core/src/filter-for-surface.ts:58-66`), and 336 `test-noise` rules are dropped outside production (`build-diagnostic-pipeline.ts:174-180`).
  - App-only rules stay silent on library files (`:260-268`).
  - The projection re-scores without the top rules and stays silent only when nothing moves (`packages/react-doctor/src/cli/utils/compute-score-projection.ts:19-40`).
  - Duplicate JSX families nested inside a larger family are suppressed (`react-cleanup/detect-duplicate-jsx-subtrees.ts:599-611`).
  - Retired rules stay registered and silent (react-doctor's `rule-retirements.md` under its `docs/` directory).
- **Market gap:** No Rust health tool publishes a score whose every lost point traces to a finding the workspace can repair, with test code, transitive dependencies and measured noise kept out of the charge, and that never aborts on its input.

### Best Practices Applied
- **Report only what the engineer will want to fix.** Developers distrust tools that produce false positives (Sadowski et al., "Lessons from Building Static Analysis Tools at Google", CACM 61(4), 2018, [Google Research](https://research.google/pubs/lessons-from-building-static-analysis-tools-at-google/)). This drives US-015, US-021 and US-027.
- **Cargo SemVer:** "only changes in the left-most non-zero component are considered incompatible" ([Cargo SemVer](https://doc.rust-lang.org/cargo/reference/semver.html)). This drives US-018.
- **`std::panic::catch_unwind`:**
  - it catches only unwinding panics, so it is inert under `panic = "abort"`;
  - it needs `UnwindSafe`, and `AssertUnwindSafe` is the documented escape;
  - the panic hook prints before the catch ([std docs](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html)).

  `thread::scope` re-panics when an auto-joined thread panicked, while `ScopedJoinHandle::join` returns `Err` (Context7 `/rust-lang/rust`). This drives US-002.
- **Cargo `[lints]`:**
  - entries become command-line flags, ordered by `priority`, for the current package only (Context7 `/rust-lang/cargo`, manifest reference);
  - the audit reproduced that the scan's trailing `-W` overrides a manifest `allow`;
  - source attributes still win over command-line levels.

  This drives US-023.

*Audit reports: the session scratchpad (`report.json`, `report-linear.txt`, probe crates). The recomputation is reproducible from the formulas above.*

## Assumptions & Constraints

### Assumptions (to validate)
- **A1:** The corpus noise rates are reliable enough that a withheld rule (smoothed noise above 5000 bp, `src/audit/ranking.rs:25`) may stop capping.
  - Today no withheld rule is P0 or P1; the score design review found this, and US-015 re-checks it.
  - `clippy::exit` rests on 5 adjudicated sites.
  - US-015 adds a guard test that fails if a P0 or P1 rule becomes withheld, which forces a human decision.
- **A2:** `catch_unwind` catches every panic a pass can raise.
  - The release profile sets no `panic = "abort"` (`Cargo.toml:55-56`). No pass crosses FFI that aborts.
  - US-002 adds a test that fails if a profile sets `panic = "abort"`.
- **A3:** Making `indexing_slicing` and `string_slice` opt-in loses no corpus true positive.
  - Both are adjudicated wrong on 40 of 40 corpus sites (smoothed noise 9762 bp).
  - The self-scan found 1 reachable `string_slice` site in 48, the US-001 crash.
  - US-026 validates this before US-027 ships.
- **A4:** A duplicate family made only of `#[test]` functions is never what a reviewer factors.
  - Both duplicate rules' own `not_a_defect_when` describe it.
  - The self-scan classified all 28 such families as intentional or wrong.
- **A5:** `under_test_attribute` (`src/source_kernel/detectors.rs:663`) classifies the same items as test code for the numerator and, after US-019, for the denominator.
- **A6:** The website can ship core-v6 copy in the same window as the CLI release; it has one maintainer.

### Hard Constraints
- **Dependencies:** No new dependency (no proptest, no cargo-fuzz). Existing ones stay pinned exactly, and `Cargo.lock` stays committed.
- **Toolchain:** Edition 2024, rustc 1.95 or later. Every test command runs under the CI pin, toolchain 1.99.1 (`.github/workflows/ci.yml:47`).
- **Trust boundary:** No network, no telemetry, and `--json` reports stay workspace-relative (AGENTS.md). A panic payload never reaches the report, since it can hold user text.
- **No panics in production code:** no `unwrap`, `expect`, `panic!` or `dbg!`. Enforced by `Cargo.toml` `[lints.clippy]` under CI's `-D warnings`.
- **Size bounds:** Every file stays under 1000 lines and every `impl` block under 500 (`no_unit_of_this_crate_s_own_source_is_a_hotspot`, `src/structure/tests.rs:456`). New code goes in submodules. Current sizes:

  | File | Lines |
  |---|---|
  | `src/audit.rs` | 989 |
  | `src/render.rs` | 977 |
  | `src/execution.rs` | 953 |
  | `src/cargo_health.rs` | 866 |
  | `src/source_kernel.rs` | 807 |
- **Report shape:**
  - Releases 0.9.1 and 0.10.0 keep `SCHEMA_VERSION` 23 and `SCORE_MODEL` "core-v5".
  - Release 0.11.0 ships `SCHEMA_VERSION` 24, additive only, and `SCORE_MODEL` "core-v6". The frozen v7 archive keeps projecting through `project_current_wire_to_v7` (`tests/support/mod.rs:200`).
  - `ReportError` codes are free strings (`src/report.rs:828-832`), so a new error code is not a shape change.
- **Catalog:** Stays at 142 rules. A default-level, tier or help-text change regenerates `tests/corpus.json` per `docs/catalog-and-corpus.md`. `ADMISSION_DEBT` is append-only (`tests/corpus_precision.rs:1316`).
- **Documentation:** Each edit keeps its file between 85% and 100% of its word ceiling (`scripts/verify-doc-budgets.sh:45-50`). Current counts:

  | File | Words / ceiling |
  |---|---|
  | AGENTS.md | 1182 / 1182 |
  | `docs/subsystems/audit.md` | 1104 / 1108 |
  | `docs/subsystems/reporting.md` | 3344 / 3353 |
  | `docs/testing.md` | 957 / 960 |
  | `docs/subsystems/policy.md` | 703 / 711 |

  `tests/docs_contract.rs` recomputes the stated numbers from the binary.
- **Worktree:** The worktree at `bad7c10` carries uncommitted changes to workflows, `tests/corpus.json`, oracles and `src/ci.rs`. They are settled (committed or reverted by Arthur) before EP-003 re-baselines any hash or replays the corpus.
- **Test conventions:**
  - Integration test crates start with `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`.
  - Tests that run cargo or the binary set `CARGO_TARGET_DIR` through `support::scan_target` (`tests/support/mod.rs:164`).
  - Regression seeds live under `tests/fixtures/regressions/<rule-tail>--<slug>/`, with a weakness from `WEAKNESSES` (`tests/regression_seeds.rs:32-41`).
- **Package manager:** `bun` under `npm/rust-doctor/` and `tests/e2e/`, never `npm` or `pnpm`.

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

Every command runs under toolchain 1.99.1 (`RUSTUP_TOOLCHAIN=1.99.1`).

Two stories add gates:
- **US-028:** the corpus replay, `RUST_DOCTOR_CORPUS_DIR=~/.cache/rust-doctor/corpus RUST_DOCTOR_CORPUS_ARTIFACTS=~/.cache/rust-doctor/corpus-artifacts cargo test --locked --test corpus_precision`.
- **US-029:** the website's `bun run build:verified` in `/home/arthur/dev/rust-doctor-web`.

## Epics & User Stories

The plan is phased because it holds 29 stories. Each phase stays at or under 20 stories and ships as one version.

| Release | Version | Epics | Stories | Priority |
|---|---|---|---|---|
| 1 | 0.9.1 | EP-001 | 6 | P0 |
| 2 | 0.10.0 | EP-002 | 8 | P1, render-only, no model or schema change |
| 3 | 0.11.0 | EP-003, EP-004, EP-005 | 15 | P0 to P2, one corpus replay, `core-v6`, schema 24 |

### EP-001: A scan survives its input

Release 0.9.1. No input text can abort a scan, and a pass that fails degrades into a complete, non-authoritative report naming its stage, as AGENTS.md already promises.

**Definition of Done:** Each of the three reproduced crash inputs completes:
- the tracked README with `“postgres://…`;
- the source comment with `--verbose`;
- the `build.rs` stderr.

A forced panic in each native pass yields a report with `complete: false` and one `ReportError` at that pass's stage. The non-ASCII table test covers every text scanner, and `clippy::string_slice` is on in the crate's own `[lints.clippy]`.

#### US-001: Locate the URL scheme on bytes in the credential scanner
**Description:** As a Rust developer, I want the credential scanner to find a URL scheme without slicing inside a multibyte character, so that a README or comment with typographic quotes before `://` no longer aborts my scan.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `database_passwords` (`src/repo_hygiene/credentials.rs:356-359`) finds the scheme start with a byte-level reverse search over `line.as_bytes()[..separator]` that accepts only ASCII alphanumerics, `+`, `.` and `-`. Every offset it slices at is a char boundary.
- [ ] Given a tracked `README.md` holding `Connect with “postgres://admin:Hunter2Secret@db.prod.example.com/app” today.`, when `rust-doctor . --yes --json` runs, then it exits with the gate's code, not 101, and reports `rust_doctor::repo::hardcoded_credential` at that line.
- [ ] Given `src/main.rs` with `// docs: “postgres://u:p@h.example.com/x”` above a `println!`, when `rust-doctor . --yes --verbose --package <name>` runs, then the code frame renders and the process does not panic.
- [ ] Given a `build.rs` that prints `“postgres://u:secretpw@db.example.com/x` to stderr and exits 1, when the scan runs, then the report is produced, the masking still hides `secretpw`, and no error carries the code `clippy-thread-failed`.
- [ ] Given `«https://example.com»`, `详见https://example.com` and `𝄞postgres://u:p@h.io/x`, when `credential_ranges` runs on each line, then it returns without panicking, and every returned range starts and ends on a char boundary.
- [ ] A fixture under `tests/fixtures/` holds the README case, and an integration test asserts the scan completes.

#### US-002: Contain a panicking pass as a ReportError at its stage
**Description:** As a CI maintainer, I want a panic inside one native pass to become an error at that pass's stage, so that the run still produces a complete report with every other pass's findings.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A helper in a new submodule of `src/execution/` runs a pass under `std::panic::catch_unwind(AssertUnwindSafe(..))`. It wraps every native pass:
  - cargo health (`src/execution.rs:586`) at stage `dependencies`;
  - repo (`:598`) at stage `repo`;
  - the walk (`:630`) at stage `source`;
  - the source kernel (`:687`) at stage `source`;
  - dependency truth (`:696`) at stage `dependencies`, truncating the candidates it added;
  - structure (`:708`) at stage `structure`.
- [ ] The two `resume_unwind` joins (`src/execution.rs:506` and the structure join) map a join `Err` to that pass's `ReportError` instead of re-raising.
- [ ] A contained panic produces exactly one `ReportError` with code `pass-panicked` and a fixed message naming the stage. The message never contains the panic payload.
- [ ] The default panic hook output does not reach stderr for a contained panic. The hook is installed once per process and restores the previous hook for panics outside a pass.
- [ ] Given a test-only fault injection that panics in each native pass in turn, when `inspect` runs, then:
  - the report has `complete: false` and `audit.score.authoritative: false`;
  - it carries the other passes' diagnostics;
  - it holds one error at the expected stage.
- [ ] Given no fault, when the self-scan runs, then the diagnostics and score are identical to the pre-change run (no behavior change on the happy path).
- [ ] A test fails if any `[profile.*]` in `Cargo.toml` sets `panic = "abort"`.
- [ ] `src/execution.rs` stays under 1000 lines.

#### US-003: Keep the native findings when the Clippy pass fails
**Description:** As a CI maintainer, I want a failing or panicking Clippy pass to leave the native findings in the report, so that a broken build script does not erase every other finding.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] The Clippy thread join (`src/execution.rs:519-527`) maps a panic to a `ReportError` at stage `execution` with code `pass-panicked`. The run continues to assembly instead of returning `ExecutionResult::failed` (`:543`).
- [ ] Given a workspace whose `build.rs` exits 1, when the scan runs, then the report:
  - carries the native diagnostics;
  - carries one `execution` error;
  - has `complete: false`, and its status is not `failed`.
- [ ] Given a forced panic in the Clippy stream reader, when the scan runs, then the `cargo clippy` child is no longer running after the report is written.
- [ ] The gate's exit code for an incomplete report is unchanged from today's documented behavior for a failed producer.

#### US-004: Kill the child on drop and turn an uncaught panic into exit 2
**Description:** As a CI maintainer, I want no orphaned `cargo clippy` and no raw abort, so that a failing run leaves the machine clean and exits with a documented code.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `BoundedChild::drop` (`src/execution/process.rs:122-133`) kills and reaps the child's process group when the child has not been waited, on every platform the crate builds for.
- [ ] Given a `BoundedChild` dropped before `wait`, when the test checks the pid, then the process no longer exists.
- [ ] `src/main.rs` runs the command under `catch_unwind`. An escaping panic prints one fixed line, `rust-doctor: internal error, please report it`, without the payload, and exits 2.
- [ ] Given a test-only panic injected after assembly (the renderer path), when the binary runs, then it exits 2 and stdout holds no partial JSON document.

#### US-005: Always advance the TUI wrap by at least one character
**Description:** As a Rust developer using the interactive report, I want a narrow detail pane to wrap a wide character instead of hanging, so that resizing the terminal never freezes the report.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `split_at_width` (`src/tui/text.rs:386-394`) returns at least one character as the head when `content` is non-empty, even when that character is wider than `width`.
- [ ] Given width 1 and the word `日本`, when `place` runs, then it terminates with one character per line, in under 10 ms.
- [ ] Given width 0 or an empty word, when `place` runs, then it terminates and pushes no empty line in a loop.

#### US-006: Pin char-boundary safety with a table test and the crate's own string_slice lint
**Description:** As the maintainer, I want every text scanner tested against non-ASCII text and Clippy to refuse new unchecked string slices, so that this class of crash cannot return unnoticed.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001, US-005

**Acceptance Criteria:**
- [ ] A unit test builds seed lines at runtime: one per credential pattern, a PEM header line, an `unwrap_or("…")` line, and a `run: cargo build # x` line. It inserts `é`, `“`, `𝄞`, `e\u{301}` and `日` at every char boundary of each.
- [ ] That test feeds every generated line to each of these:
  - `credential_ranges`, `first_credential`, `mask_credentials` and `redact_line`;
  - the Cargo stderr excerpt and `sanitize_text`;
  - `compiling_command`;
  - the terminal `wrap` and `truncate`;
  - `wrap_spans` at widths 1 to 4.

  It asserts none panics and every returned range lies on char boundaries.
- [ ] `Cargo.toml` `[lints.clippy]` sets `string_slice = "warn"`. Each production site is resolved:
  - rewritten with `.get(..)`, or
  - kept under `#[allow(clippy::string_slice, reason = "…")]` naming the invariant that makes it safe.
- [ ] Test code keeps compiling under `-D warnings`. Integration crates and test modules add `clippy::string_slice` to their test-only allow header.
- [ ] Given a new unchecked `&s[a..b]` in production code, when CI's Clippy gate runs, then it fails.

---

### EP-002: The report states what it measured

Release 0.10.0. Render-only and documentation changes, plus three self-contained fixes: the advisory clone choice, the phase timings, and two self-scan sites. `SCORE_MODEL` stays "core-v5" and `SCHEMA_VERSION` stays 23, so no score and no JSON field moves.

**Definition of Done:**
- The linear and Markdown reports print the five dimensions, the scan scope, and the not-evaluated rules.
- They state the withholding criterion truthfully.
- The text tallies separate unscored findings.
- The freshest RustSec clone is read, the Clippy phases are split where Cargo actually finishes the dependencies, and the 9 drifted statements are corrected.
- The self-scan reports neither `clippy::exit` nor `dynamic_shell_command`.

#### US-007: State the criterion that withholds a rule
**Description:** As a Rust developer, I want the report to say why a rule is left out of the advice, so that I am not told "nothing worth repairing is left" when repairing it would move my score.

**Priority:** P1
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `withheld_sentence` (`src/render.rs:863-882`) states the applied criterion in this shape: "… are left out of what to fix first: the corpus found each wrong on most of the sites it reviewed." It no longer claims that nothing worth repairing is left.
- [ ] Given withheld rules whose removal moves the score, when the report renders, then no sentence claims the repair is worth nothing.
- [ ] Given no withheld rule, when the report renders, then no withheld sentence appears.
- [ ] `src/render/tests.rs` covers the one-, two- and three-or-more-rule forms.

#### US-008: Print the five dimension scores
**Description:** As a Rust developer, I want to see each dimension's score next to the overall one, so that I can tell which dimension holds my score down.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `render_score` (`src/render.rs:782`) prints one line with the five values from `audit.score.dimensions`, in the order security, reliability, maintainability, performance, dependencies. Example: `Security 100 · Reliability 75 · Maintainability 100 · Performance 100 · Dependencies 64`.
- [ ] The Markdown report and the TUI header print the same values.
- [ ] Given a non-authoritative or absent score, when the report renders, then no dimension line appears and the existing reason line is unchanged.
- [ ] Given a workspace with several members, when the report renders, then the line shows the workspace score's dimensions, not a member's.
- [ ] `src/render.rs` stays under 1000 lines. The new code lives in a submodule if needed.

#### US-009: Disclose the scan's scope and the rules it did not evaluate
**Description:** As a coding agent or developer, I want the report to say what was scored and what Clippy read, so that I do not assume test code or every target was linted.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The line `Scanned {} files in {:.1}s` (`src/render.rs:194`) becomes `Scored {files} production files ({lines} lines) in {secs}s; Clippy linted library and binary targets, native rules also read tests`, with figures from `audit.source_files` and `audit.production_lines`.
- [ ] Rules published `not_evaluated: crate-version-unsupported` are listed by id in the default output, near `src/render.rs:618`. The `crate-absent` count is printed only under `--verbose`.
- [ ] Given a `--package` scan, when the report renders, then the scope line names the selected members.
- [ ] Given a run where the Clippy pass is off (no active Clippy rule), when the report renders, then the scope line says Clippy did not run.
- [ ] `skills/rust-doctor/SKILL.md` states the same scope in one sentence, and `tests/skill_contract.rs` stays green.

#### US-010: Count only weighing findings in the text tallies and group headings
**Description:** As a Rust developer, I want the category tallies and group headings to count only findings that weigh, with the rest shown apart, so that Maintainability does not read 138 warnings when one is scored.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The category line (`src/render.rs:561`) counts diagnostics with no `context` and no `unscored`. It appends `(+N outside production code)` when N > 0.
- [ ] A group heading (`src/render.rs:381-388`) counts only its own severity. Example: `Warning: Print stderr (1 occurrence, 31 info in binary targets)`. The singular reads "occurrence".
- [ ] Given the self-scan, when the report renders, then Maintainability shows 1 warning and 57 info with `(+137 outside production code)`, and Security shows no warning with `(+1 outside production code)`.
- [ ] `--json` `summary` and `audit.categories` are unchanged in this release (the JSON split is US-020).
- [ ] Given a delta scope, when the report renders, then the split applies to the introduced findings only.
- [ ] Given a non-authoritative report (a pass failed), when it renders, then the tallies still split and no category shows a negative or missing count.

#### US-011: Read the most recently fetched RustSec clone
**Description:** As a Rust developer, I want the advisory check to read the freshest local RustSec database, so that a stale cargo-audit clone does not hide advisories a same-day cargo-deny clone knows.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `discover` (`src/cargo_health/advisories.rs:103-120`) considers the cargo-audit clone and every cargo-deny clone that passes the existing RustSec remote check. It reads the one whose `.git/FETCH_HEAD` is newest, and cargo-audit's on a tie or when neither has `FETCH_HEAD`.
- [ ] Given `~/.cargo/advisory-db` fetched 88 days ago and a cargo-deny clone fetched today, when the scan runs, then the report names the cargo-deny source and its date.
- [ ] Given a newer clone whose remote is not RustSec, when the scan runs, then it is ignored, as today.
- [ ] Given `--advisory-db <path>`, when the scan runs, then the explicit path wins over discovery.
- [ ] `tests/advisory_database.rs` covers the four cases above.

#### US-012: Split the Clippy phases at the last dependency artifact
**Description:** As the maintainer reading `--timings`, I want `clippy_dependencies` to end when the last non-member artifact arrives, so that the workspace's own linting is not reported as dependency time.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `members.rs` (`src/execution/members.rs:40-66`) records the arrival of each non-member `compiler-artifact`. `clippy.rs` (`src/execution/clippy.rs:198,211`) ends `clippy_dependencies` at the last one.
- [ ] Given a workspace with no dependency, when `--timings` runs, then `clippy_dependencies` is 0 and `clippy_linting` holds the whole Clippy time.
- [ ] Given a warm cache, when `--timings` runs, then the two phases still sum to the Clippy wall time, within 5 ms.
- [ ] The doc on the phase (`src/timings.rs:29`) states the new boundary.

#### US-013: Correct the documents that misstate their gates and constants
**Description:** As the maintainer, I want every statement in AGENTS.md, the linked documents and the skill to name the gate that actually holds it, so that agents trust the map.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] AGENTS.md and `docs/testing.md:22-25` attribute the no-`unwrap`/`expect`/`panic!`/`dbg!` rule to `Cargo.toml` `[lints.clippy]` under CI's `-D warnings`, not to `tests/score_credibility_packs.rs`.
- [ ] AGENTS.md names `no_unit_of_this_crate_s_own_source_is_a_hotspot` as what enforces the file and impl bounds across `src/`. It describes the fourteen `the_X_holds_the_size_bound` tests as per-module file checks over hand-listed files. The strings `tests/docs_contract.rs` requires stay present.
- [ ] `src/structure/hotspots.rs:36-40` and `src/structure/duplication.rs:52-60,78-82` state their 2026-08-08 measurements in the past tense.
- [ ] `src/audit/density.rs:133-134` and `skills/rust-doctor/SKILL.md:41-42` state the λ values: maintainability 3, security and performance 4, dependencies 6, reliability 10. They no longer call security the tightest.
- [ ] `docs/subsystems/policy.md:31-33` no longer states in the present tense that `src/policy` holds five `indexing_slicing` findings.
- [ ] Given the edits, when `bash scripts/verify-doc-budgets.sh` runs, then every touched file stays between 85% and 100% of its ceiling, and AGENTS.md stays at or under 1182 words.
- [ ] Given a correction that does not fit its file's ceiling, when it is written, then the reasoning moves to the document the line links instead of raising the ceiling, and the doc references still resolve.

#### US-014: Clear the two self-scan sites that are correct by design
**Description:** As the maintainer, I want the self-scan to stop reporting a correct signal exit and a literal-only test shell command, so that the self-scan lists only what should change.

**Priority:** P2
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `src/execution/interrupt.rs:297` carries `#[allow(clippy::exit, reason = "…")]`, stating that the watcher thread has no caller and must honor Ctrl-C after replacing its default. It uses `allow`, not `expect`: the lint is allow-by-default, so an `expect` would fail CI's `-D warnings` with `unfulfilled_lint_expectations`.
- [ ] `tests/scan_failures.rs:352-354` passes the signals as positional arguments to a literal `sh -c` script instead of interpolating them with `format!`.
- [ ] Given the self-scan, when it runs, then neither `clippy::exit` nor `rust_doctor::source::dynamic_shell_command` is reported.
- [ ] Given the existing signal tests, when `cargo test` runs, then they pass unchanged.
- [ ] Given Ctrl-C while no scan is active, when the watcher handles it, then the terminal is restored and the process exits 130, as before.

---

### EP-003: Score model core-v6 and report schema 24

Release 0.11.0. Every point a workspace loses traces to a finding it can repair, and the JSON says what binds and what weighs.

**Definition of Done:**
- `SCORE_MODEL` is "core-v6" and `SCHEMA_VERSION` is 24.
- A withheld rule caps nothing, and a transitive-only duplicate publishes at Info.
- Versions compare by Cargo compatibility class, and the denominator excludes inline test-gated items.
- `audit.score.ceilings[]` and `audit.categories[].production` are published and rendered.
- The v7 archive projects.
- With EP-004 and EP-005, the corpus is replayed once (US-028).

#### US-015: A withheld rule caps no dimension
**Description:** As a Rust developer, I want a rule the corpus judges mostly wrong to stop capping its dimension, so that one correct `process::exit` cannot take 5 points.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `scoring_tier` (`src/audit.rs:761-767`) returns `None` for a rule where `is_withheld()` (`src/audit/ranking.rs:64-67`) is true. Its density charge is unchanged.
- [ ] Given a crate with one `std::process::exit` in a library function and nothing else, when it is scanned, then reliability is 100 and `worst_tier` is null.
- [ ] Given a crate with one site of an unmeasured P2 rule, when it is scanned, then reliability is still capped at 75.
- [ ] A test fails if any catalogued P0 or P1 rule has smoothed noise above the withhold threshold in `CORPUS_NOISE` (`src/policy/noise.rs:108`).
- [ ] The independent recomputation in `tests/support/corpus.rs:1216-1219` skips withheld rules when it folds the worst tier.
- [ ] The `worst_tier` field doc reads "the worst tier that caps".

#### US-016: Discount a measured rule once in the ranking
**Description:** As a Rust developer, I want the "fix these first" ranking to discount a measured rule once, so that a rule the corpus confirmed is not ranked below an unmeasured one.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] `expected_repair_value` (`src/audit/ranking.rs:46-50`) and the projection gain (`:120-123`) apply the kept-share factor only to an unmeasured rule (`noise: None`). A measured rule's contribution is already discounted by `charge` (`src/audit.rs:738-744`).
- [ ] Given two rules with equal raw relief, one measured at 3333 bp and one unmeasured, when the projection ranks them, then the measured one ranks first.
- [ ] Given a rule that holds the ceiling, when the projection runs, then its gain still reflects the ceiling it releases.
- [ ] `src/audit/tests/ranking.rs` covers both cases.

#### US-017: Publish a duplicate major no member requires at Info
**Description:** As a Rust developer, I want a duplicate major version that only my dependencies pull in to be shown without charge, so that the score and the advice point at what my manifests control.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A duplicate whose every version is `Requirement::Through` gets effective `severity: info` and keeps `base_severity: warning` (`src/cargo_health/resolution.rs:132-141`, a new field on `Candidate` in `src/cargo_health.rs:45-59`).
- [ ] A duplicate with at least one `Requirement::Direct`, or an unreached requirement, stays a warning.
- [ ] Given the self-scan, when it runs, then both duplicates are Info, the dependencies dimension is 100, and `projected_rule_ids` no longer names `duplicate_major_versions`.
- [ ] Given a crate that requires `hb = "1"` directly while a dependency requires `hb = "2"`, when it is scanned, then the finding is a warning naming the direct requirement.
- [ ] A finding's `id` is the same before and after this change, because the fingerprint hashes `base_severity` (`src/report/normalize.rs:629-641`).
- [ ] The docstring at `src/cargo_health/resolution.rs:256-262` no longer says every duplicate is charged alike.
- [ ] The help (`src/policy/catalog/native.rs:12`) reads: "Align the requirement your manifest controls, or bump the dependency the message names so it moves to the version you require."

#### US-018: Compare versions by Cargo compatibility class
**Description:** As a Rust developer, I want `0.59` and `0.61` treated as incompatible, as Cargo treats them, so that a duplicate 0.x crate compiled twice is reported.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-017

**Acceptance Criteria:**
- [ ] `major_of` (`src/cargo_health/resolution.rs:432`) is replaced by a compatibility key:
  - `M` for `M.x.y` with M > 0;
  - `0.m` for `0.m.y` with m > 0;
  - `0.0.p` for `0.0.p`.
- [ ] The message reads "incompatible versions", not "incompatible major versions".
- [ ] Given path dependencies `winsys` 0.59.0 and 0.61.2, both required directly, when the crate is scanned, then one warning is reported.
- [ ] Given `0.0.3` and `0.0.4`, when the crate is scanned, then they are reported as incompatible.
- [ ] Given `1.4.0` and `1.9.2`, when the crate is scanned, then nothing is reported.
- [ ] Given pre-release or build metadata (`1.0.0-rc.1+abc`), when the key is computed, then it ignores both.
- [ ] `src/cargo_health/resolution/tests.rs` covers every case above.

#### US-019: Leave inline test-gated items out of the production lines
**Description:** As a Rust developer, I want the density denominator to count only production code, so that an inline test module does not dilute or inflate my score.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `production_source` and `package_measurements` (`src/source_kernel.rs:438-482`) subtract the lines of each outermost item that `under_test_attribute` (`src/source_kernel/detectors.rs:663`) classifies as test code, through one shared helper.
- [ ] Given a 21-line lib whose last 12 lines are `#[cfg(test)] mod tests { … }`, when it is scanned, then `production_lines` is 9.
- [ ] Given a crate whose only source is a `#[cfg(test)]` module, when it is scanned, then `production_lines` is 0, no score is published, and the existing unscorable reason applies.
- [ ] Given two members, when the workspace is scanned, then each member's measurement excludes its own inline tests.
- [ ] `audit.source_files` is unchanged by this story.
- [ ] `src/source_kernel.rs` stays under 1000 lines.

#### US-020: Publish the binding ceilings and the production tallies (schema 24)
**Description:** As a coding agent, I want the JSON to name each binding ceiling and to split production counts from the rest, so that I can act on what weighs without re-deriving the model.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-008, US-010, US-015

**Acceptance Criteria:**
- [ ] `audit.score.ceilings` is an array of `{dimension, tier, ceiling, rule_ids}`, filled where the scoring tier is computed. It is empty when no ceiling binds.
- [ ] `audit.categories[].production` holds the scored counts, and the existing `audit.categories` and `summary` values are unchanged (FR-06 equality holds).
- [ ] `SCHEMA_VERSION` is 24 (`src/report.rs:45`). `project_current_wire_to_v7` gains the ledger lines for both fields, and the v7 archive fixtures still project.
- [ ] The renderer names the binding ceiling from `ceilings[]`. Example: `Reliability 75 (P2 ceiling: clippy::exit)`. It replaces `capping_rule_ids` (`src/render.rs:740-766`), which named rules without filtering out test-context findings.
- [ ] The share URL (`src/audit/share.rs:55-75`) carries production counts. Given the self-scan, when the link is built, then `w` equals the sum of the warnings in `audit.categories[].production` (52 on today's findings), not the 192 it reads today.
- [ ] Given a report with a P0 overall ceiling, when it renders, then both the overall and the dimension ceilings are named.
- [ ] `docs/subsystems/reporting.md` documents both fields within its word budget.

---

### EP-004: Detectors flag what a reviewer would act on

Release 0.11.0. Each structural or cargo-health finding is a claim a reviewer would accept or a decision the project made, never intentional test similarity or a hidden override.

**Definition of Done:**
- On the self-scan, no duplicate family is made only of `#[test]` functions, none repeats a nested family, and every published similarity is a pairwise lower bound.
- A manifest allow hides its rule and is charged once at P3.
- `crate_level_allow` stays quiet on the four reviewed false-positive shapes.
- `oversized_unit` counts from the item keyword.
- Each change ships with a regression seed.

#### US-021: Skip test-case-only duplicate families and nested items
**Description:** As a coding agent, I want duplicate findings to exclude families of `#[test]` functions and nested repeats, so that I do not refactor test cases that are meant to look alike.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `Member` (`src/structure/duplication.rs:148`) carries a `test_case` flag, true when an attribute path ends in `test` (`#[test]`, `#[tokio::test]`, `#[rstest]`). A family whose members are all test cases is not reported.
- [ ] `Canonical::node` (`src/structure/normalize.rs:154`) treats an item nested in a function body as one opaque node, as it treats `ATTR` (`:165`).
- [ ] Given two `#[test]` functions sharing a 40-node body, when they are scanned, then no duplicate finding is reported.
- [ ] Given a `#[test]` function and a helper sharing a body, when they are scanned, then the family is reported.
- [ ] Given two functions that each hold the same nested `fn visit`, when they are scanned, then one family is reported (the outer pair), not two.
- [ ] Given a nested fn identical to a top-level fn, when they are scanned, then the pair is still reported.
- [ ] Two seeds land under `tests/fixtures/regressions/`: `duplicate_function_body--test-cases` and `duplicate_function_body--nested-function`. `WEAKNESSES` (`tests/regression_seeds.rs:32-41`) gains the classes they need.

#### US-022: Fingerprint macro arguments and publish the lowest pairwise similarity
**Description:** As a coding agent, I want "same body" and "at least X% alike" to hold for every pair of the family, so that a duplicate claim is never false.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-021

**Acceptance Criteria:**
- [ ] At `src/structure/normalize.rs:183`, a macro call's token tree is walked: identifiers become placeholders, literals become their type tag, punctuation is kept, and one shingle is added per nested token tree. `FINGERPRINT_DOMAIN` (`:32`) becomes `rust-doctor-structure-normalized-v2`.
- [ ] Given two functions that differ only inside `assert!` arguments (`tests/corpus_agreement.rs:319` and `:585` shapes), when they are scanned, then no "same body" finding is reported.
- [ ] Each single-linkage group (`src/structure/duplication.rs:534-536`) is refined by complete linkage, with groups capped at 64 members: a member joins only when every cross pair scores at least 8000 bp. The published `similarity_basis_points` is the lowest pairwise score in the family.
- [ ] Given a chain A-B-C where A and C score 6600 bp, when they are scanned, then A and C are not in one family at 80%.
- [ ] Given a group larger than 64 members, when it is refined, then the refinement completes and the family is split deterministically (same input gives the same families).
- [ ] The messages say "once names, literals and macro arguments are normalized".
- [ ] The oracles that freeze structural ids are regenerated by their documented commands:
  - `kernel-contract/precision-matrix`;
  - `delta-pairing/verdicts.json`;
  - the v7 reports.

#### US-023: Honor a manifest [lints] allow and charge it once at P3
**Description:** As a CI maintainer, I want my Cargo.toml allow to hide the rule it names, and to be reported once as a choice, so that the scan respects my project's configuration without charging it twice.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `inspect_lint_tables` and `permissive_entries` (`src/cargo_health.rs:374-460`) return the effective allows as (package, rule) pairs, including root `[workspace.lints]` entries inherited through `lints.workspace = true`.
- [ ] The Clippy diagnostics whose `package` and `code` match an effective allow are dropped where `ExecutionResult` is assembled (`src/execution.rs:531`), unless the level came from the request (`--rule X=warn` or `rust-doctor.toml`).
- [ ] Given `[lints.clippy] print_stdout = "allow"` and a `println!` in a lib, when the crate is scanned, then `clippy::print_stdout` is absent and `permissive_lint_table` is reported once.
- [ ] Given a two-member workspace where only one member allows the lint, when it is scanned, then the other member's sites are still reported.
- [ ] Given `--rule clippy::print_stdout=warn`, when the crate is scanned, then the sites are reported.
- [ ] `permissive_lint_table` moves from P2 to P3 (`src/policy/catalog/native.rs:30-42`), so it caps no dimension.
- [ ] Its help names the three exits:
  - remove the entry;
  - switch the rule off in `rust-doctor.toml`;
  - add `# rust-doctor: allow(rust_doctor::cargo::permissive_lint_table) -- reason` above the key (already honored by `src/report/exclusion.rs:120-140`).
- [ ] `tests/fixtures/suppression/lint-table/src/lib.rs` gains a site of the allowed lint, and `tests/suppression_audit.rs` asserts it is absent.
- [ ] `why` (`src/explain.rs`) says a site is hidden by `[lints]`.

#### US-024: Narrow crate_level_allow to allows that hide catalogued or dead code
**Description:** As a Rust developer, I want `crate_level_allow` to fire only when a file-wide allow hides something rust-doctor or rustc would report, so that toolchain-compatibility pairs and shared test support do not count against me.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `crate_level` (`src/structure/suppression.rs:105`) reports a `#![allow(..)]` only when it names:
  - an active catalogued rule (the `plan.is_active` check `permissive_entries` uses);
  - a group containing one (`clippy::all`, `warnings`, `clippy::pedantic` and the like);
  - `dead_code`, `unused`, or an `unused_*` lint.
- [ ] An attribute that also names `unknown_lints` is not reported (the toolchain-compatibility pair).
- [ ] A file in non-production context is not reported.
- [ ] Given `#![allow(unsafe_code)]`, `#![allow(mismatched_lifetime_syntaxes, unknown_lints)]` and `tests/support/mod.rs` with `#![allow(dead_code)]`, when they are scanned, then none is reported.
- [ ] Given `#![allow(clippy::all, reason = "x")]` in `src/lib.rs`, when it is scanned, then it is reported.
- [ ] A production site and a test-context site are never merged into one family without a context, the shape observed with `src/main.rs:4` and `tests/support/mod.rs:1`.
- [ ] Three quiet seeds land: `crate_level_allow--toolchain-compat`, `--allow-by-default-lint` and `--test-support-dead-code`. The `catches` line in `tests/rule_evidence.json` is updated.

#### US-025: Count a unit's lines from its keyword
**Description:** As a Rust developer, I want `oversized_unit` to measure a function from its `fn` keyword, so that documenting a function never pushes it over the bound.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `lines_of` (`src/structure/hotspots.rs:196`) counts fn, impl and mod nodes from their first child that is not an attribute, a comment or whitespace. The published span and file line counts are unchanged.
- [ ] Given a function of 149 lines from `fn` with 6 doc and attribute lines above it, when it is scanned, then no finding is reported.
- [ ] Given a function of 150 lines from `fn`, when it is scanned, then it is reported as 150 lines.
- [ ] Given a file of exactly 1000 lines, when it is scanned, then the file finding is unchanged.

---

### EP-005: Re-baseline, catalog defaults and public claims

Release 0.11.0. The corpus measures the new model once, the two rules the corpus finds wrong on every reviewed site stop reporting by default, and the website states core-v6 when it ships.

**Definition of Done:**
- `tests/corpus.json` is replayed under core-v6.
- `score_distribution`, `precision`, `gate` and `CORPUS_NOISE` agree with the replay.
- `indexing_slicing` and `string_slice` default to Off.
- rust-doctor.com builds green with `CURRENT_MODEL = "core-v6"` and no sentence claiming 100 means nothing was detected.
- The release skill names the score-model step.

#### US-026: Validate assumption: making indexing_slicing and string_slice opt-in loses no corpus true positive
**Description:** As the maintainer, I want evidence that the two rules' corpus sites hold no confirmed defect, so that turning them off by default does not hide real panics.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Every adjudicated site of both rules in `tests/corpus.json` is listed with its verdict, and the count of true positives is stated: 0 expected from the published 40 of 40 wrong.
- [ ] The self-scan's one reachable site (US-001) is recorded as evidence that the rules catch real defects in code that slices untrusted text. That kind of code enables them through `Cargo.toml` or `rust-doctor.toml`.
- [ ] If any corpus true positive exists, US-027 is marked BLOCKED with the site's `path:line` and the decision returns to Arthur.
- [ ] The finding is written to `docs/catalog-and-corpus.md` within its word budget.

#### US-027: Make indexing_slicing and string_slice opt-in
**Description:** As a Rust developer, I want the two rules the corpus finds wrong on every reviewed site to stay silent unless I enable them, so that the default report lists defects I would fix.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-026

**Acceptance Criteria:**
- [ ] Both rules declare `default_level: RuleLevel::Off` in the catalog. They stay catalogued (142 rules) and enable through `--rule clippy::indexing_slicing=warn` or `rust-doctor.toml`.
- [ ] `docs/catalog-and-corpus.md` gains the procedure for changing a rule's default level or tier, covering every surface it moves:
  - the corpus `catalog` block;
  - the `5 + 2*N` Clippy argument assertions;
  - the policy-gate and rule-scaling hashes;
  - the site sync.
- [ ] Given a default scan of the self-scan workspace, when it runs, then neither rule is passed to Clippy and neither appears in the report.
- [ ] Given `--rule clippy::string_slice=warn`, when it runs, then the rule reports as before.
- [ ] `rules list --json` shows the new default level, and `tests/skill_contract.rs` stays green.

#### US-028: Replay the corpus once under core-v6 and re-baseline
**Description:** As the maintainer, I want one corpus replay to measure every score and detector change together, so that the published calibration matches the shipped binary.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-015, US-016, US-017, US-018, US-019, US-020, US-021, US-022, US-023, US-024, US-025, US-027

**Acceptance Criteria:**
- [ ] `SCORE_MODEL` is "core-v6" (`src/audit.rs:46`). The `audit-core-v5.json` fixture and its references (`src/audit/tests.rs:319`, `tests/score_credibility_kernel.rs`, `tests/audit_contract.rs:31`) move to v6.
- [ ] The corpus replay runs under toolchain 1.99.1 with the documented environment. Then:
  - the new or changed sites of the duplicate rules, `crate_level_allow`, `oversized_unit` and `duplicate_major_versions` are adjudicated by two blind passes;
  - `position_proof` is regenerated;
  - `precision`, `agent_population.precision`, `gate` and `score_distribution` are recomputed with `precision_of`, `gate` and `score_distribution` (`tests/support/corpus.rs:1413,1572,1641`);
  - `CORPUS_NOISE` (`src/policy/noise.rs:108`) matches the record.
- [ ] Given the replayed record, when `cargo test --test corpus_precision` runs offline from the clone cache, then it passes, and the score distribution gate keeps a positive separation.
- [ ] `docs/subsystems/audit.md`, `skills/rust-doctor/SKILL.md` and the AGENTS.md schema mention state core-v6 and schema 24:
  - "a withheld rule caps nothing";
  - a lone direct duplicate scores 75 under the P2 ceiling, and a transitive-only one weighs nothing.

  Each file stays within its word budget.
- [ ] Given a subject whose score moves by more than 10 points, when the replay completes, then the move is explained in the release note by the story that caused it.
- [ ] Given a replay that fails the distribution gate or leaves a separation of zero or less, when it completes, then the release does not proceed, and the failing subjects and the story that moved them are listed in this story's notes.

#### US-029: Bring rust-doctor.com and the release skill in line with core-v6
**Description:** As the maintainer, I want the website and the release procedure to state the shipped model, so that no page publishes a retired formula.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-028

**Acceptance Criteria:**
- [ ] In `/home/arthur/dev/rust-doctor-web`, the following state core-v6:
  - `CURRENT_MODEL` (`src/app/share/share-contract.ts`);
  - under `content/docs/`, `scoring.mdx`, `index.mdx`, `report.mdx` and `privacy.mdx`, whose share-link counts become production counts;
  - `src/lib/data.ts`.
- [ ] `src/lib/data.ts:36` no longer says "100 means nothing detected". It says what 100 means under core-v6: no charged finding remains, with Info and withheld findings still listed.
- [ ] `public/catalog/rules.json` and `public/catalog/corpus.json` are regenerated by `catalog:sync` and `corpus:sync`.
- [ ] Given a share link carrying `m=core-v5`, when the site renders it, then the card says the number is not comparable with the current model.
- [ ] `.claude/skills/release/SKILL.md` step 8 names the score-model check: grep the site for the previous model id, and update `CURRENT_MODEL`.
- [ ] Given the site, when `bun run build:verified` runs, then it passes.

---

## Functional Requirements

- FR-01: No input file, source line, manifest or Cargo stderr text may make the process exit 101.
- FR-02: When a native pass or the Clippy pass panics, the system must publish a report with `complete: false`, `audit.score.authoritative: false`, the other passes' findings, and one `ReportError` with code `pass-panicked` at that pass's stage.
- FR-03: The system must NOT write a panic payload to stdout, stderr or the report.
- FR-04: A dropped child process must be killed and reaped.
- FR-05: The linear, Markdown and TUI reports must print the five dimension scores whenever the score is authoritative.
- FR-06: The text tallies must count only findings without `context` or `unscored`, and show the rest as `(+N outside production code)`.
- FR-07: The report must state the scan scope, including that Clippy lints library and binary targets only.
- FR-08: A rule whose smoothed noise exceeds 5000 bp must not impose a dimension or overall ceiling.
- FR-09: A duplicate whose every version arrives through a dependency must publish at Info and weigh nothing.
- FR-10: Version compatibility must follow Cargo's left-most non-zero component rule.
- FR-11: Production lines must exclude items classified as test code by `under_test_attribute`.
- FR-12: The JSON report must publish `audit.score.ceilings[]` and `audit.categories[].production` in schema 24, keeping every schema 23 field and value.
- FR-13: Duplicate-function findings must NOT report a family made only of test cases, must NOT repeat a nested family, and must publish a similarity that holds for every pair.
- FR-14: A `[lints]` allow in a member's manifest must hide the matching Clippy findings of that member, unless the request sets the rule's level.
- FR-15: Advisory discovery must read the most recently fetched RustSec clone.

## Non-Functional Requirements

- **Performance:**
  - The warm self-scan total stays at or under 840 ms. The baseline is 765 ms on 2026-10-28, and the budget allows 10% for containment and the denominator pass.
  - Complete-linkage refinement processes a 64-member group in under 50 ms.
  - The TUI wrap of any 10,000-character line terminates in under 10 ms at any width from 1 to 400.
- **Security:**
  - A panic payload never reaches the report: 0 occurrences across the US-002 fault-injection matrix.
  - Credential masking keeps 100% of the existing masking tests green.
  - 0 network connections, unchanged.
- **Reliability:**
  - 0 aborts across the US-006 table: 5 inserted characters × every char boundary × every seed line × 10 scanners.
  - A forced panic in each of the 7 pass sites yields exactly 1 error and keeps 100% of the other passes' diagnostics.
- **Compatibility:**
  - Schema 24 adds 2 fields and changes 0 existing values.
  - The v7 archive projects for 100% of its fixtures.
  - Finding ids of `duplicate_major_versions` are unchanged across US-017.
- **Maintainability:**
  - Every file stays under 1000 lines and every impl under 500.
  - Every touched document stays within 85% to 100% of its word ceiling.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Multibyte character before `://` | `“postgres://…` in a tracked file, a source comment, or build-script stderr | Scan completes; credential masked | None |
| 2 | Pass panic | Any native pass panics | Complete report, `complete: false`, one error at the stage | `rust-doctor: the {stage} pass failed; the score is not authoritative.` |
| 3 | Clippy pass fails | `build.rs` exits 1 or the stream reader panics | Native findings kept; error at `execution`; child killed | Existing Clippy failure line |
| 4 | Panic after assembly | Renderer panics | Exit 2, no partial JSON | `rust-doctor: internal error, please report it` |
| 5 | TUI at width 1 (boundary) | Terminal narrower than the detail indent | One character per line, no hang | None |
| 6 | Only test code (boundary) | Crate whose sole source is a `#[cfg(test)]` module | `production_lines` 0, no score | Existing unscorable reason |
| 7 | 0.0.x versions (boundary) | `0.0.3` and `0.0.4` both resolved | Reported as incompatible | `Crate "x" is resolved with incompatible versions …` |
| 8 | Mixed provenance | One duplicate version direct, one transitive | Warning kept | Message names the direct requirement |
| 9 | Inherited allow | Member with `lints.workspace = true`, root `[workspace.lints]` allow | Member's sites hidden; one `permissive_lint_table` at P3 | Help names the three exits |
| 10 | Explicit request over allow | `--rule clippy::print_stdout=warn` with a manifest allow | Sites reported | None |
| 11 | Both advisory clones stale or missing (external dependency) | No `FETCH_HEAD`, or no clone at all | cargo-audit's on a tie; missing database disclosed as today | Existing age or missing notice |
| 12 | Large duplicate group (boundary) | Over 64 near-identical functions | Refinement capped; deterministic split | None |
| 13 | Withheld P0 or P1 (rule change) | Corpus re-adjudication pushes a P0 or P1 rule over 5000 bp | Guard test fails in CI | Test names the rule |
| 14 | Interrupted run | Ctrl-C during a contained pass | Existing interrupt path: terminal restored, exit 130 | None |

Network degradation, permission changes and concurrent modifications do not apply. The tool opens no connection, has no sessions, and a scan is one process over a read-only view.

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | core-v6 moves every corpus subject's score, and the distribution gate fails | Med | High | One replay after all moving stories (US-028); recompute the derived blocks with the existing helpers; explain moves over 10 points |
| 2 | A truly harmful P2 rule with a small, unlucky sample becomes withheld and stops capping | Low | Med | Withholding needs smoothed noise over 5000 bp; the US-015 guard fails on any withheld P0 or P1; US-028 lists withheld rules with their sample sizes |
| 3 | After US-015 and US-017 this repository scores 100 with production warnings, and "100 means nothing detected" reads false | High | Med | US-027 removes the two noisiest rules from defaults; US-029 rewrites the site claim |
| 4 | `catch_unwind` misses a panic (abort profile, FFI, double panic) | Low | High | US-002 guard test on profiles; the `main.rs` backstop (US-004); passes call no FFI |
| 5 | Macro token-tree hashing raises node counts and floods new near-duplicates | Med | Med | Shingles per token tree keep `MINIMUM_NODES` meaningful; corpus re-measure in US-028 before release |
| 6 | Honoring manifest allows hides defects a team allowed carelessly | Med | Low | `permissive_lint_table` still reports each allow; the `--rule` override restores the sites |
| 7 | Files at the 1000-line bound (`src/audit.rs` 989, `src/render.rs` 977, `src/execution.rs` 953) cannot absorb new code | High | Low | Every story adding code there extracts a submodule; the hotspot gate catches a miss in CI |
| 8 | The dirty worktree skews re-baselined hashes | Med | Med | Hard constraint: settle it before EP-003 starts |
| 9 | Website and CLI ship at different times | Med | Med | US-029 is blocked only by US-028, and the release skill gains the model step |

## Non-Goals

- **Linting test targets with Clippy (`--all-targets`).** It would compile dev-dependencies offline, tag every message `test: true` so test findings could not be told apart, and depend on each repository's `allow-*-in-tests` settings (rust-clippy#13981). The asymmetry is disclosed instead (US-009).
- **Server-side or remote scoring as in react-doctor.** It breaks the no-network trust boundary.
- **A property-testing or fuzzing dependency** (proptest, cargo-fuzz). The table test in US-006 covers the class without one. Revisit if a second crash class appears.
- **Retrying Clippy per member after a workspace-wide failure.** This is react-doctor's split-and-retry. The failure is now contained and disclosed; revisit with a user report.
- **Changing λ or the dimension weights.** The audit found the density calibration consistent with the corpus (ceilings cost 0 to 4.6 points on healthy subjects, densities up to 27). The insensitivity seen here came from discounted rules and test-scoped findings.
- **Changing `clippy::exit`'s tier.** US-015 already stops a withheld rule from capping.
- **A rule-retirement framework.** US-027 uses the existing `Off` level, and a lifecycle field waits for a second retirement.
- **A meaning-preserving rewrite oracle for native detectors.** Valuable, but it needs its own harness; deferred to a later PRD.

## Files NOT to Modify

- `src/source_kernel/detectors/secrets/`: permission-blocked for agents; no story needs it.
- `Cargo.lock` beyond what `cargo check` refreshes: no dependency is added or bumped.
- `.github/workflows/*`, `action.yml`, `.github/releases/` and version fields: releasing is a separate step through `.claude/skills/release`, and `tests/docs_contract.rs` pins the workflows.
- `tests/fixtures/*/v7-*.json`: frozen archives, projected through `project_current_wire_to_v7`, never edited.
- The measured blocks of `tests/corpus.json` (`adjudication`, `position_proof`): regenerated by the replay, never typed by hand.
- Existing rows of `tasks/e2e-catalog/*.md` and `tests/e2e/known-bugs.json`: append-only.
- `tasks/prd-security-audit-remediation.md` and its status file: a completed PRD.

## Technical Considerations

Questions for engineering input, not mandates:

- **Architecture:** Where should the containment helper live so that `src/execution.rs` (953 lines) shrinks rather than grows? Recommended: `src/execution/containment.rs`, holding the helper, the hook guard and the stage mapping. Engineering to confirm that each pass's captured state is safe to discard after a panic (`AssertUnwindSafe`).
- **Panic hook:** Should the hook be swapped per pass or installed once? Recommended: once, with a thread-local flag set inside a contained pass so the previous hook still prints outside passes. Trade-off: a global hook affects library embedders of `inspect`. Engineering to confirm against `src/lib.rs`'s public surface.
- **Data model:** Should `ceilings[]` carry the uncapped dimension value, so readers see the distance to the cap? Recommended: no, to keep schema 24 minimal. Revisit if agents ask.
- **Duplicate refinement:** Is a 64-member cap with deterministic splitting enough, or should larger groups publish a single "at least" bound with an explicit "chain" flag? Recommended: cap and split, since no self-scan or corpus group exceeds 64 today. Engineering to confirm on the corpus.
- **Manifest allow filtering:** Should the filter run on Clippy diagnostics only, or also on native rules a `[lints]` key could name? Recommended: Clippy only, since native rules have no rustc lint name. Engineering to confirm no native rule id collides.
- **Migration:** Schema 24 is additive and the v7 archive keeps projecting. Rollback: a 0.11.x patch can revert the model to core-v5 behind the same schema. The share URL's `m=` key keeps old cards labeled.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Reproduced crash paths that abort | 3 (README, code frame, build.rs stderr) | 0 | Release 0.9.1 | US-001 fixtures and the US-006 table test |
| Self-scan points lost to a withheld rule or a transitive-only duplicate | 11 (89, and 100 without them) | 0 | Release 0.11.0 | `--json` self-scan: `audit.score.ceilings` and dimensions |
| Self-scan duplicate families that are wrong or test-case-only | 30 of 50 (28 test-case-only, 2 more wrong) | at most 5, 0 wrong | Release 0.11.0 | Re-run the audit's classification over the self-scan |
| Maintainability text tally warnings that are unscored | 137 of 138 | 0 | Release 0.10.0 | Linear self-scan output |
| Share link `w=` on the self-scan | 192 (140 unscored) | 50 or fewer | Release 0.11.0 | `audit.share_url` |
| Production-line denominator inflation from inline tests | about 6,000 of 46,574 lines (13%) | 0 | Release 0.11.0 | `audit.production_lines` against an independent count |
| Documents misstating a gate or constant | 9 statements | 0 | Release 0.10.0 | US-013 checklist, `tests/docs_contract.rs` |
| Warm self-scan total time | 765 ms | at most 840 ms | Each release | `--timings` on a warm target |

## Open Questions

- **Dirty worktree.** Should the uncommitted workflow, corpus and oracle changes on `main` be committed before EP-003? Arthur decides before US-015 starts; every re-baselined hash depends on it.
- **Release cadence.** Should releases 0.9.1 and 0.10.0 ship separately or as one 0.10.0? Arthur decides before tagging; nothing in the stories depends on it, since EP-002 does not need EP-001.
- **Opt-in rules.** If US-026 finds a corpus true positive for `indexing_slicing` or `string_slice`, should the rule stay on with its discount, or go opt-in anyway with a release-note warning? Arthur decides on US-026's evidence.
- **Website window.** Can rust-doctor.com deploy core-v6 copy within 24 hours of the 0.11.0 tag? Arthur confirms during US-029.
[/PRD]
