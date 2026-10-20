[PRD]
# PRD: react-doctor Gap Closure: a Loop That Never Misfires, a Delta That Survives Refactors, and the Rules Rust Services Need

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-10-13 | Arthur Jean | Initial draft, from the gap audit of 2026-10-13 (`tasks/react-doctor-gap-audit-2026-10.md`: 126 verified items, 1,412 citations re-read at their exact line) |
| 1.1 | 2026-10-14 | Arthur Jean | EP-001 review: US-001 masks every credential the repository pass matches on every frame line, a PEM body opened above the window included, instead of one range carried on the diagnostic; US-005 takes a manifest nested below another candidate for its member |

## Problem Statement

rust-doctor transposes react-doctor (`/home/arthur/dev/react-doctor`). The parity PRD (`tasks/prd-react-doctor-parity.md`) closed the 2026-09-25 audit against react-doctor `83466a8fa`: EP-001 to EP-005 are DONE and EP-006 is IN_REVIEW in the uncommitted worktree. On 2026-10-13 a second audit compared react-doctor `d1feed112` (114 commits later) with rust-doctor `9b8c0ab` plus that worktree. Twelve zones, one read-only auditor each, produced 159 findings. Every quote was re-read at its stated line and every absence grep replayed, and the findings merge into 126 items (`tasks/react-doctor-gap-audit-2026-10.md`). Eight problems carry most of the weight.

1. **A detected credential is printed wherever the report is.** `rust_doctor::repo::hardcoded_credential` keeps the value out of its message (`src/repo_hygiene.rs:325`), as FR-12 of the hygiene PRD requires (`tasks/prd-suppression-dependency-hygiene.md:454`). But the finding's span is the literal itself, and the linear report draws a code frame under it (`src/render.rs:391`) that prints the source line verbatim (`src/render.rs:463`). The interactive viewer draws the same frame (`src/tui/screens/viewer.rs:218`). The credential therefore appears with carets under it in the terminal, in agent transcripts and in every CI log of a job that runs the scan. The test that claims both surfaces reads only the serialized JSON (`tests/repo_hygiene.rs:80`, `:313-314`). react-doctor redacts at the point where a diagnostic is built (`packages/core/src/runners/oxlint/parse-output.ts:178`).

2. **`--scope lines`, the scope of both hooks, drops findings the change caused.** The pre-commit hook runs `--staged --scope lines` (`src/hook.rs:37`) and the Stop hook runs `--scope lines --base HEAD` (`src/hook/agent.rs:197`). Three classes of finding cannot reach either of them:
   - Every Cargo declaration finding is built without a span (`src/cargo_health.rs:129`, `src/cargo_health/resolution.rs:118`), and `(Some(_), None) => false` drops it (`src/git_scope.rs:424`). An agent that adds `serde = "*"` or an unpinned git dependency is never told.
   - A clone family is filtered on its first member in sorted order (`src/structure.rs:588-590`, `src/report/assembly.rs:492-502`). A new copy of existing code vanishes whenever the old copy sorts first.
   - Some findings sit on a line the diff never touched. The oracle puts `clippy::await_holding_lock` on the guard binding (`tests/fixtures/score-credibility/packs/concurrency/src/lib.rs:36`), not on the `.await` an edit adds below it.

3. **The agent and CI loop misfires in ordinary situations.**
   - `hook run cursor` reads only Claude's `stop_hook_active` (`src/hook/agent.rs:192`), so a turn the user aborted is rescanned and resubmitted, up to Cursor's default of 5 loops.
   - The git hook body is `exec rust-doctor ...` (`src/hook.rs:55`). In a committed `core.hooksPath` or `.cargo-husky/hooks` directory, every contributor without the binary gets exit 127 on every commit.
   - The Action runs a push at `--scope full` with the default gate (`action.yml:100`, `:107`) and exits with the scan's code on every event (`action.yml:187`). Legacy findings turn the default branch red, although the README promises the opposite (`README.md:52`).
   - `render_markdown` never reads `errors[]` (`src/render/markdown.rs:94-163`), so a scan that failed posts "No finding." and overwrites the previous sticky comment.
   - A depth-1 pull request checkout fails as `base-unavailable`. The fetch-depth remedy fires only for `merge-base-unavailable` (`src/git_scope.rs:640`).
   - The generated workflow runs rust-cache without `cache-on-failure` (`src/ci.rs:371`), whose default is false, so a failed gate discards the build its retry needs.
   - The score animation ignores `--yes` and the twelve agent and hook markers (`src/main.rs:775`, `:693`).

4. **The shipped native rules miss, or fire on, forms the audit measured.** Five of these land on EP-006, which is not yet certified.
   - A glob in scope, `std::io::prelude::*` included, turns every crate-rooted path Indeterminate (`src/source_kernel/aliases.rs:118-119`), so detectors abort.
   - A parameter resolves Indeterminate (`src/source_kernel/bindings.rs:133-134`), so `format!("... LIMIT {limit}")` with `limit: i64` reads as SQL injection.
   - A blocking call one same-module helper away from `async fn` is missed by design (`src/policy/catalog.rs:581`).
   - `QueryBuilder::push`, `diesel::dsl::sql` and `sea_orm::Statement::from_string` take raw SQL unseen (`src/source_kernel/detectors/sql.rs:59`, `:68`).
   - `.args(["-c", x])`, `cmd /C` and `powershell -Command` stay silent (`src/source_kernel/detectors.rs:212`).
   - A private-key header alone is a P1 finding (`src/repo_hygiene.rs:366`, test at `:657`).
   - `#[cfg_attr(unix, path = "sys/unix.rs")] mod sys;` reads as no path (`src/source_kernel/walk.rs:471`). The orphan rule then tells the user to delete a compiled file (`src/policy/catalog.rs:648`).
   - The variant gate stays green while every one of these gaps is open (`src/source_kernel/tests/variants.rs:28`). `tests/corpus.json` holds 147 false-positive verdicts on native rules and not one replayed seed.

5. **The first run fails where a JavaScript-first user starts.** Discovery only walks up (`src/scan_target.rs:89`) and fails with "no Cargo.toml found in this path or its ancestors" (`src/scan_target.rs:105`). `npx rust-doctor` at the root of a Tauri app (`src-tauri/Cargo.toml`) or of a repository with `backend/Cargo.toml` is a dead end.

6. **The scan is serial, the baseline rescans the base every time, and the delta reads refactors as new findings.**
   - `ExecutionContext::run` runs every pass in sequence (`src/execution.rs:440-530`).
   - A `--scope baseline` run materializes and lints the base on every invocation (`src/lib.rs:219`).
   - The deadline does not bound the version probes (`src/execution.rs:713`), `cargo metadata` (`src/scan_target.rs:160`) or git.
   - Ctrl-C leaves Cargo, rustc and build scripts running, and leaves snapshots behind (`src/execution/process.rs:87-92`).
   - Any `[target.*]` rustflags table makes the stripping return nothing (`src/execution/rustflags.rs:154`).
   - The delta fingerprint hashes the message and the normalized excerpt (`src/delta.rs:350`), so renaming a variable inside a flagged line reports one introduced and one fixed finding (`src/delta/tests.rs:283`).
   - Duplicates pair first-in, first-out (`src/delta.rs:519`), and an identical excerpt in any other file counts as a move (`src/delta.rs:437`).
   - The baseline gate blocks test and build-script findings the full gate ignores (`src/report/assembly.rs:702` against `:726`).

7. **Daily use loses information.**
   - Without `--verbose` the linear report prints one rule group (`src/render.rs:212-216`, `:339`), at a width read from `COLUMNS` and floored at 80 (`src/main.rs:559`, `:754`).
   - A failed clipboard or agent launch loses the handoff prompt (`src/main.rs:633`, `:584`).
   - There is no `rules explain` and no `why`.
   - The skill's version stamp is never read (`src/skill.rs:140`), so an agent follows stale instructions.
   - Setup takes five commands (`src/main.rs:206`).
   - The npm install path the Action uses is never exercised by a test (`npm/rust-doctor/scripts/smoke-packed.mjs:276`).

8. **The rule families where Rust services fail are still missing.**
   - Bodies read from a reqwest response whose status was never checked.
   - Secret environment variables that fall back to a literal.
   - The two tokio calls that panic.
   - Permissive CORS, auth cookies and disabled JWT validation.
   - CI steps that compile with secrets in scope.
   - An ignored `Cargo.lock` and a tracked `.cargo/credentials.toml`.
   - The catalog cannot gate a rule on a crate's version, and publishes no title and no false-positive check (`src/policy/catalog.rs:28-35`).

**Why now:** react-doctor moved 114 commits in 18 days, and most of them sharpen exactly the surfaces above:
- the contextual diff scope (`695e46eee`, #1965, 2026-10-12);
- count-based baseline deltas and saved baselines (`fcfb6ea06`, `8ed70df0f` #1860);
- a warning exit code (`6ff671033`, #1900);
- `comment-on-clean` (`041d1ab1c`, #1386);
- base-drift-free changed files (`418c6c82a`, #1447);
- a batch of false-positive and false-negative fixes, each pinned as a regression seed.

EP-006 is still uncommitted, so its five measured shortfalls cost least to fix now, before certification freezes them. CodeQL for Rust reached general availability on 2025-10-14 ([GitHub changelog](https://github.blog/changelog/2025-10-14-codeql-scanning-rust-and-c-c-without-builds-is-now-generally-available)), and 2.27.2 improved its async data flow on 2026-10-09 ([changelog](https://github.blog/changelog/2026-10-09-codeql-2-27-2-improves-c-go-rust-and-javascript-analysis)). rust-doctor's edge over it is a local, offline, sub-minute loop inside the agent's turn, and that edge holds only if the loop never leaks a secret, drops a finding or misfires.

## Overview

This PRD closes the 2026-10-13 audit in three releases, each of which ships something coherent. Release 1 fixes what is broken:
- the credential leak;
- the findings `--scope lines` drops;
- the agent and CI misfires;
- the measured gaps of the shipped native rules, EP-006's included.

Release 2 brings a warm scan down to the longer of Clippy and the native passes plus 10%, stops rescanning an unchanged base, makes the delta pair through Git renames and hunks, and gives daily use the surfaces react-doctor has: the full rule list, the live width, `rules explain`, `why`, a warning exit code, one-command setup. Release 3 adds the rule families above, the version gate they need, and the rule metadata an agent needs to judge a finding. Every audit item not built here gets a named destination in the Gap Audit Disposition table below.

| Release | Version | Epics | Stories | Theme |
|---------|---------|-------|---------|-------|
| 1 | 0.11.0 | EP-001, EP-002, EP-003 | US-001 to US-021 (21) | Nothing leaks, nothing slips through the loop, the shipped rules hold |
| 2 | 0.12.0 | EP-004, EP-005 | US-022 to US-037 (16) | Overlapped passes, a cached base side, a delta that survives refactors, daily use |
| 3 | 0.13.0 | EP-006 | US-038 to US-045 (8) | The rules Rust services need |

Key decisions, each argued in Research Findings and Technical Considerations:

- **Redact at the frame, for every surface at once.** The repo pass attaches a mask range to its credential diagnostics. `presentation::code_frame` replaces those columns before any renderer sees the line, so the linear report, the viewer and any later surface share one masked frame. This is react-doctor's model of redacting where the diagnostic is built, applied at the one place rust-doctor reads source text for display.
- **`--scope lines` keeps span-less findings by file.** Manifest findings are anchored at their key, and a finding that still has no span is in scope when its file changed. This is a bug fix, applied to the existing scope.
- **A contextual scope is opt-in.** `--scope contextual` keeps a finding when its rule declares an item or file reach and the owning item changed, as react-doctor's `diffScope` does. The hooks stay on `lines` until a release of measurement shows contextual does not trap agents on legacy findings.
- **The delta keeps evidence pairing and adds hunk correspondence, not a pure count.** react-doctor now compares counts per file and rule (`fcfb6ea06`). The SARIF guidance and its committee recommend matching on line-level diffs first and on context hashes second ([GitHub SARIF support](https://docs.github.com/en/code-security/reference/code-scanning/sarif-files/sarif-support), [SARIF committee](https://lists.oasis-open.org/archives/sarif/201805/msg00018.html)). A pure count hides a fix and a new finding of the same rule in the same file. US-028 measures the three policies on real history before US-029 builds the chosen one.
- **Overlap Clippy with the native passes** using `std::thread::scope`, keep the output byte-identical, and cache the base side of a baseline run keyed by its git tree. A current-side cache waits for the benchmark of US-022.
- **Signals use `libc` as a normal dependency.** It is already in `Cargo.lock` (as a dependency of `console` and as a dev-dependency), so no new crate enters the build. The alternative, keeping Cargo in the caller's process group, loses the whole-tree kill at the deadline.
- **`src/main.rs` stops growing.** It sits at 999 lines against the 1,000-line bound. US-013 extracts the terminal decisions into one tested `TerminalProfile` module, and every new subcommand lives in its own module.
- **Crate packs gain a version gate** (`requires`), as react-doctor's capability gates do. Release 3 rules that match a non-standard crate declare one.

It adds no network access, no telemetry, no new crate in `Cargo.lock` and no fix subcommand.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Surfaces that print a detected credential's value (2 today: linear frame, viewer) | 0, asserted by a test per surface | 0 |
| Cargo declaration and lockfile rules reported under `--scope lines` when their manifest or lockfile changed (0 of 7 today) | 7 of 7 | 7 of 7, plus `--scope contextual` shipped |
| Loop misfires from Problem 3 still reproducible (7 today) | 0 | 0 |
| Measured shortfall cases of the shipped native rules that neither fire nor carry a reasoned carve-out (every case of Problem 4 today) | 0 | 0 |
| Native false-positive verdicts in `tests/corpus.json` with a replayed seed or a written reason (0 of 147) | Every verdict on a rule Release 1 touches | 147 of 147 |
| Cargo processes spawned for the base side of a second baseline run on an unchanged base (all of them today) | n/a (Release 2) | 0 |
| Catalogued rules (133) | 133 | 133 plus the Release 3 rules (about 10), each with trigger evidence |

## Target Users

### Vibecoder shipping Rust with an agent
- **Role:** builds Rust mostly through Claude Code, Codex or Cursor, often inside a JavaScript-first repository (Tauri, a `backend/` crate next to a web app), without a senior reviewer.
- **Behaviors:** runs `npx rust-doctor` at the repository root, lets the agent fix the top findings, commits through the installed hook.
- **Pain points:**
  - The first run at a Tauri root fails with "no Cargo.toml found".
  - A credential the scan detected is printed into the agent transcript.
  - The default report shows one rule group of many.
  - A teammate without the binary cannot commit.
- **Current workaround:** `cd src-tauri` before scanning, `--verbose` and scrolling, `git commit --no-verify`.
- **Success looks like:** one command at the repository root scans the right workspace, sets up the loop, and never prints a secret.

### The coding agent itself
- **Role:** the process that edits code and runs the tool, under a pipe or a PTY, driven by the skill and the Stop hook.
- **Behaviors:** runs `rust-doctor . --json --scope lines`, reads the findings, fixes them, and is rescanned at the end of its turn.
- **Pain points:**
  - A dependency it added with `*` passes every gate.
  - An `.await` it added under a lock is not reported.
  - An aborted Cursor turn is resubmitted.
  - Its skill can be one release stale with nothing saying so.
  - A refactor inside a flagged line blocks the turn as an "introduced" finding.
- **Current workaround:** none. The agent trusts the gate.
- **Success looks like:** every finding its change caused reaches it, nothing it did not cause blocks it, and each finding says when it is not a defect.

### Engineer owning CI and review
- **Role:** decides what gates a merge, reviews agent-authored pull requests.
- **Behaviors:** uses the composite Action or the `ci install` workflow, reads the sticky comment.
- **Pain points:**
  - Pushes to main fail on legacy findings.
  - A scan that broke posts "No finding.".
  - A shallow checkout fails without a remedy.
  - Every failed gate recompiles from cold.
  - Actions are pinned to moving branches.
  - Every clean pull request gets a bot comment.
- **Current workaround:** hand-edited workflows, `fetch-depth: 0` learned from a failure, a separate rust-cache step.
- **Success looks like:** a push reports without failing, a pull request comment says what happened including why a scan failed, and clean pull requests stay quiet when asked.

## Research Findings

### Competitive Context
- **react-doctor** (`d1feed112`, the reference):
  - a contextual diff scope driven by a per-rule `diffScope` (`packages/oxlint-plugin-react-doctor/src/plugin/utils/rule.ts:23`);
  - count-based deltas with Git renames and line maps (`packages/core/src/compute-diagnostic-delta.ts:96`, `:108`);
  - saved-report baselines (`8ed70df0f`);
  - a whole-repository result cache (`packages/react-doctor/src/cli/utils/scan-result-cache.ts:456`);
  - parallel passes (`packages/core/src/run-inspect.ts:407`);
  - signal handling with exit 130 (`packages/react-doctor/src/cli/utils/constants.ts:7`);
  - an Action that never fails a push (`action.yml:721`), explains a failed scan (`scripts/render-github-action-comment.mjs:431`) and fetches its base (`action.yml:155`);
  - capability gates on framework packs (`packages/core/src/project-info/capabilities.ts:87`);
  - a regression-seed corpus replayed on every test run (`packages/fuzz/src/load-fuzz-corpus.ts:20`).

  **How we differ:** rust-doctor keeps a local, reproducible score with adjudicated per-rule precision, and takes react-doctor's loop, scope and integration mechanics without its telemetry, remote APIs or network lookups (`AGENTS.md:38`).
- **CodeQL for Rust:** build-free extraction, generally available since 2025-10-14. Alert identity comes from SARIF fingerprints plus GitHub's alert lifecycle, and 2.26.4 deliberately churned Rust alert locations ([changelog](https://github.blog/changelog/2026-09-03-codeql-2-26-4-improves-github-actions-security-detections)). **How we differ:** CodeQL is cloud-side and minutes long. rust-doctor runs inside the agent's turn, offline, and scores. Its new security rules overlap CodeQL's taint queries, so they must win on precision and locality, not on breadth.
- **cargo-deny and cargo-audit:** single-purpose supply-chain checks with no scope, no baseline and no score. **How we differ:** their advisory cache already feeds rust-doctor's advisory rule (parity US-031), joined to one score and one gate.
- **reviewdog with clippy-sarif:** filters annotations to added lines (`-filter-mode=added`) with no persisted baseline and no score. **How we differ:** the sticky comment carries the score and the introduced and fixed counts, and `--scope contextual` reaches findings outside the added lines.
- **Market gap:** no Rust tool combines a changed-code gate inside the agent's turn, precise enough not to trap the agent, with offline security rules for the service stack (tokio, reqwest, sqlx, axum, tower-http) and a calibrated score.

### Best Practices Applied
- **Never print a matched secret, because CI masking cannot be relied on.** GitHub masks secrets by substring after registration only, and transformed values escape it ([Actions hardening](https://docs.github.com/zh/github-ae@latest/actions/security-guides/security-hardening-for-github-actions)). US-001 masks at the frame, with the type prefix kept visible as react-doctor does (`packages/core/src/utils/redact-sensitive-text.ts:10`).
- **Match baselines on line-level diffs first and context hashes second.** A path change orphans a pure fingerprint ([GitHub SARIF support](https://docs.github.com/en/code-security/reference/code-scanning/sarif-files/sarif-support)), and the SARIF committee recommends file remapping plus line diffs before hashes ([OASIS, 2018-05](https://lists.oasis-open.org/archives/sarif/201805/msg00018.html)). US-028 and US-029 follow that order.
- **Pin third-party actions by full SHA, drop persisted checkout credentials, and save caches on failure explicitly.** The tj-actions compromise spared SHA-pinned repositories. `actions/checkout` persists the token unless `persist-credentials: false`. Swatinem/rust-cache v2 documents `cache-on-failure` with "default: false" (Context7, `/swatinem/rust-cache`).
- **Guard end-of-turn hooks against loops.** Cursor's stop hook passes `status` (completed, aborted, error) and `loop_count`, with a default `loop_limit` of 5 (cursor.com/docs/agent/hooks, fetched 2026-10-13). react-doctor continues at most once (`packages/react-doctor/src/cli/utils/constants.ts:135`).
- **Detect runtime panics only where the documentation says they happen.** tokio documents that `block_in_place` "Panics if called from a current_thread runtime" and runs the closure normally outside any runtime. `Handle::block_on` panics "if it's called from within an asynchronous context". `#[tokio::test]` "By default ... uses a single-threaded current-thread runtime" (Context7, tokio 1.49).
- **Version-gate rules on APIs that moved.** jsonwebtoken v10 removed `Validation.insecure_disable_signature_validation` in favor of `dangerous::insecure_decode()` (Context7, `/keats/jsonwebtoken`), so one rule needs two version-gated forms.
- **Every confirmed false positive becomes a replayed seed with a closed weakness vocabulary.** react-doctor's loader recognizes only `pass|fail`, so 19 seeds headed `verdict: safe` silently drop to mutation-only (`packages/fuzz/src/load-fuzz-corpus.ts:20`). US-021 refuses unknown directive values.
### Gap Audit Disposition

Every item of the 2026-10-13 audit (`tasks/react-doctor-gap-audit-2026-10.md`), by its number there, and where it goes. "Later" names the follow-on PRD that owns it. "To measure" items were not reproduced by the audit and need a measurement before any story.

| Audit items | Disposition |
|---|---|
| 1 mask detected credentials in terminal and viewer code frames | US-001 |
| 47 keep span-less manifest findings in a lines scope | US-002 |
| 49 keep a clone family in scope when any member changed | US-003 |
| 48 add a context-aware lines scope for remotely reported lints | US-004 |
| 50 discover nested Cargo workspaces when no manifest is found; 56 stop the manifest ancestor walk at the git root | US-005 |
| 89 honor Cursor's abort status and loop count in the stop hook | US-006 |
| 94 make the installed hook survive a missing binary and upgrades | US-007 |
| 90 stop the Action failing default-branch pushes on legacy findings | US-008 |
| 91 render the report's errors in the summary and comment; 95 add comment-on-clean so clean pull requests stay quiet | US-009 |
| 92 make the Action survive or explain a shallow pull request checkout | US-010 |
| 93 persist Cargo's build in the Action, even when the gate fails; 97 pin the generated workflow through one Action ref bots can bump; 99 drop persisted credentials from every checkout that runs code | US-011 |
| 96 write ci install's workflow at the git root, scanning the workspace | US-012 |
| 69 stop animating and rewriting for hooks, agents and --yes runs; 76 resolve one tested terminal profile instead of scattered gates; 77 widen the non-interactive markers to more agents and CI providers | US-013 |
| 108 add semantic variant cases with carve-out reasons to variant tables | US-014 |
| 2 stop prelude globs from silencing every crate-rooted path | US-015 |
| 3 carry declared parameter types to clear numeric SQL interpolation | US-016 |
| 5 follow same-module synchronous helpers called from async code | US-017 |
| 6 cover the raw-SQL escape hatches of sqlx, diesel and sea-orm; 7 catch argv-array and Windows shell forms of dynamic shell commands | US-018 |
| 8 require a key body and veto placeholders in credential matching | US-019 |
| 4 withhold orphan findings where the module walk lost declarations | US-020 |
| 107 pin each adjudicated native false positive as a replayed seed; 109 require line-exact source citations in every adjudication pass | US-021 |
| 43 time each phase and gate a whole-binary benchmark | US-022 |
| 38 overlap Clippy with the native passes and the probes | US-023 |
| 39 cache the baseline side's findings keyed by its tree | US-024 |
| 41 name members the deadline left unlinted and Cargo's lock wait; 44 bound probes, cargo metadata and git by the run deadline | US-025 |
| 42 kill Cargo's group and clean snapshots on SIGINT and SIGTERM | US-026 |
| 40 strip lint levels from every rustflags source Cargo reads | US-027 |
| 51 gate the delta on per-file, per-rule count increases | US-028 |
| 51 gate the delta on per-file, per-rule count increases; 54 pick the introduced duplicate by Git-mapped line distance; 58 exempt context-marked findings from the baseline gate | US-029 |
| 81 measure the live terminal width in the linear report; 82 list every scored rule in the default linear report | US-030 |
| 83 label test, bench, example and build-script sites in both reports; 86 say why a report is empty or unscored, in both reports | US-031 |
| 71 print the handoff prompt when launch or clipboard fails | US-032 |
| 72 add `rules explain <id>` with the effective level and its source; 73 add `why <path:line>` built on the report's suppressions | US-033 |
| 70 add an opt-in exit code for non-blocking warnings | US-034 |
| 74 tell an agent when its skill is missing or stale | US-035 |
| 100 detect installed agents and set everything up in one command | US-036 |
| 101 smoke-test the packed launcher through npm, the path users take | US-037 |
| 28 gate crate packs on each member's resolved dependency version | US-038 |
| 15 add tokio rules for current-thread block_in_place and nested block_on | US-039 |
| 13 report reqwest bodies read without an HTTP status check | US-040 |
| 14 report secret-named env vars that fall back to a literal | US-041 |
| 17 add config-call detectors for CORS, auth cookies, JWT and MAC compare | US-042 |
| 20 report a binary's Cargo.lock that git ignores; 24 treat tracked Cargo credentials files as secret files | US-043 |
| 18 report CI steps that compile the workspace with secrets in scope | US-044 |
| 30 publish a title and a false-positive check with every rule | US-045 |
| 9 unify test-item classification into one structural reader; 10 resolve crate-relative paths and re-exports across the module tree; 11 reparse expression-list macros so detectors see the calls inside; 12 add per-body reachability and dominance for binding and guard questions; 16 give the kernel a caller-input source before path traversal and SSRF; 36 write a detector authoring guide mapped to the kernel API; 37 let a detector declare several node kinds and required crates; 45 bound detector cost by counted work, not by wall clock | Later: source-kernel PRD (the resolution and control-flow work US-015 to US-017 prepare) |
| 29 add an opt-in tier that category overrides cannot switch on; 31 retire and rename rules as resolvable ids, not unknown ones; 32 mark advisories on dev-only crates as test context; 33 type stages and error codes as closed enums; 34 let closed detector lists take project-specific names; 35 record native rule candidates and rejections like the Clippy queue; 117 fail catalog admission of lints newer than the Clippy floor | Later: catalog-lifecycle PRD (one mechanism for tiers, retirement and per-rule options) |
| 19 flag library crates that enable tokio's full feature; 21 judge dev and build dependencies for unused entries; 22 report patch and source replacement that bypass crates.io; 23 detect cloned blocks inside different functions; 25 report unused pub items of internal workspace crates; 26 report regexes built from unescaped runtime text; 27 triage first the queued Clippy lints RD's families map to | Later: second rules PRD, each rule measured on the corpus first |
| 52 let a saved report replace the baseline compilation; 53 degrade a failed base side to a files-scoped report; 55 judge a staged scan under the index's own configuration; 57 accept an explicit changed-file list for the files scope; 59 anchor native directives to the next statement and stacked chains; 60 report malformed directives and hint near misses on findings; 61 add per-surface score, gate and comment exclusions; 62 discover configuration in Cargo metadata and above the workspace; 63 accept short rule ids and name the nearest catalogued id; 64 derive every path-based context from one classifier; 65 emit a rust-doctor.toml schema for editor completion; 66 publish the report schema and decode saved reports typed; 67 write one file per rule under --output-dir; 68 separate and document the library surface from binary support | Later: report, scope and configuration PRD (saved-report baselines need published fingerprints this PRD does not add) |
| 75 freeze the command-line surface in a ledger test; 78 add --color and honor FORCE_COLOR, CLICOLOR_FORCE and empty NO_COLOR; 79 edit rule levels from the CLI without rewriting user files; 80 add `--score` that prints only the number; 84 show findings arriving on the progress line during the scan; 85 repaint the interactive report when the terminal is resized; 87 detect OSC 8 support instead of tying links to color; 88 highlight Rust tokens and mark multi-line spans in code frames | Later: CLI and terminal PRD, on top of the TerminalProfile of US-013 |
| 98 anchor introduced findings as inline review comments on changed lines; 102 ship musl and Windows ARM binaries with libc-aware native packages; 103 ship a read-only audit-then-plan reference that writes executable plans; 104 hold references/expert-review.md to the catalog it supplements; 105 publish a dev dist-tag snapshot from every main push; 106 verify the crate tarball on every push | Later: distribution PRD |
| 110 publish a site-level corpus diff between base and head; 111 audit what agent commits introduce, through the Stop hook's scope; 112 calibrate agent judges against human labels written beforehand; 113 sample silent applicable units to mine false negatives; 114 fuzz native producers with seeded mutations and crash, slow oracles; 115 grow a native-only breadth corpus with a measured exclusion list; 116 make the Windows leg a green cargo test with reasoned ignores | Later: measurement PRD, on top of the seeds and citations of US-021 |
| 46 run the Clippy child below normal CPU priority; 118 split two files at the bound, reformat once, then gate rustfmt; 119 install rust-doctor's own Stop hook on this repository | Later: engineering-system PRD |
| 120 materialize snapshots without running checkout filters; 121 ship audit mode as a single --force-warn pass; 122 exclude findings in build-script output under the target directory; 123 read keys during the score reveal instead of echoing them; 124 add an HTTP pack for reqwest clients and axum routes; 125 emit a Cargo-JSON stream for rust-analyzer's check.overrideCommand; 126 add a Cursor plugin manifest over the existing skills directory | To measure before any story (not reproduced in the audit) |

*Full research sources: the gap audit of 2026-10-13 (`tasks/react-doctor-gap-audit-2026-10.md`, with its verification log); react-doctor at `d1feed112`; [GitHub SARIF support](https://docs.github.com/en/code-security/reference/code-scanning/sarif-files/sarif-support); [SARIF committee on result matching](https://lists.oasis-open.org/archives/sarif/201805/msg00018.html); [GitHub Actions hardening](https://docs.github.com/zh/github-ae@latest/actions/security-guides/security-hardening-for-github-actions); [CodeQL Rust GA](https://github.blog/changelog/2025-10-14-codeql-scanning-rust-and-c-c-without-builds-is-now-generally-available), [2.26.4](https://github.blog/changelog/2026-09-03-codeql-2-26-4-improves-github-actions-security-detections) and [2.27.2](https://github.blog/changelog/2026-10-09-codeql-2-27-2-improves-c-go-rust-and-javascript-analysis); [Cursor hooks](https://cursor.com/docs/agent/hooks) (fetched 2026-10-13); Context7 documentation for tokio 1.49, jsonwebtoken and Swatinem/rust-cache v2 (fetched 2026-10-13). Facts verified locally during the audit: the pinned clippy-driver 1.97.1 lint levels, `libc` in `Cargo.lock`, rustfmt 1.97.1 on `src/main.rs`, and the counts recomputed from `tests/corpus.json`.*

## Assumptions & Constraints

### Assumptions (to validate)
- **HIGH: hunk-correspondence pairing removes the refactor-inside-a-line false introductions without hiding a same-file swap.** react-doctor's count policy removes them by construction (`fcfb6ea06`), and the SARIF guidance favors line-level matching. Neither source measured a Rust history. US-028 measures all three policies before US-029 builds one.
- **HIGH: an item reach does not flood a lines-scoped hook with legacy findings in the touched function.** react-doctor ships contextual scope as opt-in (`packages/react-doctor/CHANGELOG.md:23`). US-004 keeps it opt-in and measures how many extra findings it returns on this repository's last 50 commits.
- **HIGH: the base side of a baseline run is a pure function of its git tree, the toolchain, the policy and the environment variables Cargo reads.** Build scripts can read inputs git does not track. The base side is a tree checked out into a fresh directory, which limits that risk to environment variables and files outside the workspace. US-024 keys on rust-cache's environment prefixes (`CARGO`, `RUST`, `CC`, `CFLAGS`, `CXX`) and stores only complete scans.
- **MEDIUM: overlapping Clippy with the native passes cuts a warm full scan's wall time.** The gain is max(Clippy, native) instead of their sum, and it is unmeasured. US-022's harness measures it before and after US-023.
- **MEDIUM: rust-cache's target pruning keeps the dependency artifacts under `target/rust-doctor/baseline`.** Unconfirmed in the audit (Z5-N08). US-011 measures two consecutive dogfood runs and adds the directory to `cache-directories` if it is pruned.
- **MEDIUM: npm honors a local optional-dependency override offline.** Unconfirmed (Z9-N09). US-037 is the test that proves or refutes it.
- **MEDIUM: appending `-W warnings` after the catalog flags undoes an inherited `-D warnings` without reimplementing Cargo's configuration discovery.** Unconfirmed on 1.97.1. US-027 observes it first and implements the full hierarchy only if it fails.
- **LOW: the Cursor stop-hook fields `status` and `loop_count` stay as documented on 2026-10-13.** A missing or malformed field keeps today's behavior, which is to scan.
- **LOW: the agent markers added in US-013 are the ones those agents set.** Most come from react-doctor's list (`packages/react-doctor/src/cli/utils/is-ci-environment.ts:68`), not from official documentation. A wrong marker only costs the animation and the interactive report, which `--yes` already gives up.

### Hard Constraints
- Single crate, edition 2024, `rust-version = "1.95"`, every workflow pinned to toolchain 1.97.1 (`docs/ci-workflows.md`). No new workspace member and no new binary.
- **No new crate in `Cargo.lock`.** `libc` moves from `[dev-dependencies]` to `[dependencies]` at its locked version for US-026 only. Globs, YAML reading (US-044) and every parser stay in-crate, as react-doctor's indentation walk does (`packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/build-pipeline-secret-boundary.ts:24`).
- Dependencies stay pinned exactly and `Cargo.lock` stays committed.
- **No network, no telemetry, no upload** (`AGENTS.md`). The base fetch of US-010 runs in the Action's shell step on the runner, never in the binary. No test touches the network.
- **`--json` stays workspace-relative.** No absolute path, home directory or environment value in any new field, discovery note or error message. Config files read outside the workspace (US-027) are named `$CARGO_HOME/config.toml` or by their workspace-relative ancestor depth.
- **A credential value never leaves the process.** No surface prints it: the linear report, the viewer, the handoff prompt, `report markdown` or `--json`.
- **Writes into the workspace happen only on an explicit command** (`skill install`, `hook install`, `ci install`, `setup`). Each prints what it writes and never overwrites a file rust-doctor did not create. `--update` rewrites only files carrying rust-doctor's marker. The side cache of US-024 writes only under Cargo's target directory.
- **`src/main.rs` does not grow.** It is at 999 of 1,000 lines. US-013 moves the terminal decisions out first, and `why`, `rules explain`, `setup` and every new flag's logic live in modules (`src/ci.rs`, `src/hook.rs` and `src/skill.rs` are the pattern).
- Every file stays under 1,000 lines and every `impl` under 500 (the fourteen `the_X_holds_the_size_bound` tests). The files these stories touch that are already above 900 lines: `src/report/tests.rs` 996, `src/audit/tests.rs` 944, `src/render.rs` 939, `src/source_kernel/tests.rs` 931, `src/audit.rs` 930, `src/cargo_health/tests.rs` 926, `src/report.rs` 915. A story that needs room splits a file in the same change.
- Production code carries no `unwrap`, `expect`, `panic!` or `dbg!` (`tests/score_credibility_packs.rs`).
- Every new rule has a trigger record in `tests/rule_evidence.json`, sits inside its category's `TIER_WINDOWS` band, and lands with its catalog entry, regenerated `tests/corpus.json` and README count in the same change. A rule matching a non-standard crate declares a `requires` gate once US-038 exists.
- **`SCHEMA_VERSION` moves once per release:** 20 to 21 in Release 1 (`ScopeMode::Contextual`, `ambiguous-workspace`), 21 to 22 in Release 2 (timings, delta counters, `interrupted`), 22 to 23 in Release 3 (`requires`, the two `NotEvaluated` variants). Every new optional field uses `skip_serializing_if`. The frozen v7 archive keeps projecting.
- Every test that runs `cargo` or the binary sets its own `CARGO_TARGET_DIR` through `support::scan_target`.
- `skills/rust-doctor/SKILL.md` is updated with every new flag and subcommand (`tests/skill_contract.rs`).
- No code copied from react-doctor. Published formats and documented behavior may be reimplemented.

## Quality Gates

These commands must pass for every user story:
- `cargo build --release` - the crate compiles
- `cargo test` - the full suite, including the catalog, admission, schema, variant-gate, seed and size-bound invariants
- `cargo clippy --all-targets --no-deps -- -D warnings` - clean, no exceptions
- `bash scripts/verify-doc-budgets.sh` - every document within its word ceiling, and every new document registered
- `bash scripts/verify-doc-refs.sh` - every path the documents cite resolves
- `cd npm/rust-doctor && bun test tests` - the launcher still resolves and runs the binary
- `cargo run --release -- . --yes --verbose` - the tool scans its own repository without error

For stories that change terminal output (US-001, US-013, US-030 to US-033):
- Render the changed surface in a real terminal at widths 40, 80 and 140, and compare against the golden or the PTY test the story adds.

## Epics & User Stories

### EP-001: Nothing leaks, nothing is lost

**Release:** 1 (0.11.0). A credential the scan detected never reaches a screen or a log, and every finding a change caused reaches the scope that judges the change, starting with the first run at a repository root.

**React Doctor reference:**
- Redaction where the diagnostic is built: `packages/core/src/runners/oxlint/parse-output.ts:178`.
- Per-rule diff scope and contextual mode: `695e46eee` (#1965), `packages/oxlint-plugin-react-doctor/src/plugin/utils/rule.ts:23`, `packages/core/src/utils/create-changed-line-matcher.ts:36`.
- Manifest entries anchored at their key: `packages/core/src/check-supply-chain.ts:364`.
- Clone family promoted to its changed occurrence: `packages/core/src/services/maintainability.ts:82`.
- Downward project discovery: `packages/core/src/resolve-diagnose-target.ts:20`.

**Definition of Done:**
- No surface prints a detected credential, asserted by a test per surface.
- A staged `foo = "*"` fails the installed pre-commit hook.
- A staged copy of committed code is reported at the copy.
- `--scope contextual` reports an `.await` added under an existing lock.
- `npx rust-doctor` at the root of a Tauri-shaped fixture scans `src-tauri/` and says so.

#### US-001: Redact detected credentials in every rendered frame
**Description:** As a developer whose repository holds a leaked key, I want the scan to show where the key is without ever printing it, so that running rust-doctor in a terminal, an agent transcript or a CI log cannot spread the secret further.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 1 (Z3-N17)
**React Doctor reference:** `packages/core/src/runners/oxlint/parse-output.ts:178`, `packages/core/src/runners/oxlint/parse-output.ts:179` (message and help redacted at build), `packages/core/src/utils/redact-sensitive-text.ts:10` (the type prefix stays visible, the body is masked).

**Acceptance Criteria:**
- [ ] The code frame masks every credential the repository pass matches (`repo_hygiene::credential_ranges`), on every line of the frame and not only the reported one, and masks whole the body of a PEM private key whose header sits in or above the window. Nothing about the mask is serialized.
- [ ] `presentation::code_frame` replaces the masked columns of the primary line with the value's first four characters followed by `<redacted>`, before any renderer reads the line
- [ ] Given the credential fixture rendered as the linear report with `--verbose`, when stdout is captured, then the full key is absent and `<redacted>` is present
- [ ] Given the same fixture opened in the TUI viewer, when the frame lines are built (`src/tui/screens/viewer.rs:218`), then the full key is absent
- [ ] Given the same fixture, when the handoff prompt and `report markdown` output are built, then neither contains the full key
- [ ] Given a credential shorter than eight characters, when masked, then no character of it is printed and the frame shows `<redacted>` alone
- [ ] Given a line with two matched credentials, when masked, then both are masked
- [ ] `tests/repo_hygiene.rs` asserts the terminal claim of FR-12 on rendered text, no longer on JSON alone

#### US-002: Keep span-less Cargo findings in a lines scope, anchored at their key
**Description:** As an agent working under the Stop hook, I want a dependency I add to be judged when I add it, so that `serde = "*"` or an unpinned git dependency cannot pass both gates silently.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 47 (Z6-N01, Z1b-N08, Z4-N02)
**React Doctor reference:** `packages/react-doctor/src/cli/utils/build-project-scan-plan.ts:58`, `packages/react-doctor/src/cli/utils/build-project-scan-plan.ts:67` (a changed manifest widens the narrowed scan), `packages/core/src/check-supply-chain.ts:364` (dependency findings anchored at the key), `695e46eee` (#1965, file-scoped rules kept when their file changed).

**Acceptance Criteria:**
- [ ] The five declaration rules (`unbounded_registry_dependency`, `unpinned_git_dependency`, `path_dependency_outside_workspace`, `unused_dependency`, `test_only_dependency`) publish the span of the dependency key in its member manifest, read with `toml::Spanned` as `src/cargo_health.rs:456` already does
- [ ] A dependency inherited with `workspace = true`, or a key that cannot be located, keeps `span: None`
- [ ] Diagnostic identity and the delta fingerprint are unchanged by the new span. `src/report/normalize.rs:115` keeps passing `None`, which a test asserts on a baseline run across the change.
- [ ] `ScopeReport::includes_span` answers, for a diagnostic with a path and no span, whether its file has any changed range, instead of `false` (`src/git_scope.rs:424`)
- [ ] Given a committed workspace and a staged `foo = "*"` in a member manifest, when `rust-doctor . --yes --staged --scope lines` runs, then `rust_doctor::cargo::unbounded_registry_dependency` is reported at the key's line
- [ ] Given a lines scope whose diff does not touch `Cargo.toml`, when scanned, then no Cargo declaration finding is reported
- [ ] Given a staged deletion of a `Cargo.lock` entry only, when scanned under `--scope lines`, then a span-less lockfile finding on `Cargo.lock` is in scope
- [ ] `src/git_scope/tests.rs:545` is flipped to the new answer, and an `agent_loop` test proves the Stop-hook command reports the added `*` requirement

#### US-003: Keep a clone family in scope when any member changed
**Description:** As an agent that copied a function, I want the copy I wrote reported at the copy, so that duplication I introduce is not hidden behind the original's sort order.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 49 (Z4-N01)
**React Doctor reference:** `packages/core/src/services/maintainability.ts:82`, `packages/core/src/services/maintainability.ts:90` (a family is kept when any occurrence meets a changed line, and that occurrence is promoted), `packages/core/tests/services/maintainability.test.ts:173`.

**Acceptance Criteria:**
- [ ] `project_diagnostics` (`src/report/assembly.rs:492`) keeps a diagnostic with non-empty `related` when its primary or any related location meets the scope
- [ ] The first member that meets the scope becomes the primary `path` and `span`, and the previous primary moves into `related`
- [ ] Given a function committed in `src/a.rs` and a staged copy in `src/b.rs`, when scanned with `--staged --scope lines`, then the duplicate finding is reported with `src/b.rs` as primary
- [ ] Given a family none of whose members meets a changed range, when scanned under a lines scope, then the family is not reported
- [ ] The family's identity does not change with the swap, which a baseline test asserts (`src/report/normalize.rs:217` keeps paths out of structural fingerprints)

#### US-004: Add `--scope contextual` for findings reported away from the line that caused them
**Description:** As a developer reviewing an agent's change, I want findings whose cause can sit elsewhere in the same function reported when that function changed, so that an `.await` added under an existing lock is caught although the diagnostic sits on the guard.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-002
**Audit items:** 48 (Z6-N03, Z2-N01)
**React Doctor reference:** `695e46eee` (#1965, `--scope contextual`), `packages/core/src/utils/create-changed-line-matcher.ts:36` (the rule's `diffScope` decides), `packages/core/src/utils/does-context-intersect-changes.ts:43` (deletion points count), `packages/oxlint-plugin-react-doctor/src/plugin/utils/rule.ts:23` (`line`, `function`, `file`).

**Acceptance Criteria:**
- [ ] `RuleDefinition` gains `diff_scope: DiffScope` with variants `Line` (default), `Item` and `File`, published in `CatalogEntry` and in `rules list --json`
- [ ] `Item` is declared on `clippy::await_holding_lock`, `clippy::await_holding_refcell_ref`, `clippy::unused_async` and the native `blocking_call_in_async`, `sql_built_from_format`, `dynamic_shell_command` and `disabled_tls_verification` (their cause can be a `let` or an `async` keyword above the reported line). `File` is declared on the span-less lockfile rules.
- [ ] `git_scope/lines.rs` records pure-deletion hunks as zero-width points on the new side, instead of dropping them (`src/git_scope/lines.rs:8`)
- [ ] `ScopeMode::Contextual` (`--scope contextual`, composable with `--staged`) keeps a finding when its span meets a changed range, or when its rule is `Item` and the innermost enclosing `fn` (nested `fn` and closure bodies excluded, parsed with `ra_ap_syntax`) holds a changed range or deletion point, or when its rule is `File` and its file changed
- [ ] Given the concurrency pack fixture with a committed lock and a staged `.await` added below the guard, when scanned with `--staged --scope contextual`, then `clippy::await_holding_lock` is reported; under `--scope lines` it is not
- [ ] Given a change to a line in a different function of the same file, when scanned with `--scope contextual`, then the finding is not reported
- [ ] Given a file that cannot be parsed, when an `Item` rule's finding is projected, then the finding falls back to line filtering and nothing fails
- [ ] Measured on this repository's last 50 commits touching `src/`: the extra findings contextual returns over lines, per commit, are recorded in `docs/contextual-scope-2026-10.md`, registered in `docs/doc-budgets.json` as a frozen record
- [ ] `scope.mode` publishes `contextual`, under `SCHEMA_VERSION` 21
- [ ] The hooks and the skill's rescan command keep `--scope lines`. The skill documents `--scope contextual` as the review option.

#### US-005: Find the workspace below the path when none is above it
**Description:** As a vibecoder running `npx rust-doctor` at the root of a Tauri app, I want the scan to find the one Cargo workspace inside my repository, or tell me which ones exist, so that my first run is not a dead end.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 50, 56 (Z4-N04, Z3-N15, Z4-N05)
**React Doctor reference:** `packages/core/src/project-info/discover-react-subprojects.ts:60` (bounded depth), `packages/core/src/resolve-diagnose-target.ts:16`, `packages/core/src/resolve-diagnose-target.ts:20` (one candidate is scanned), `packages/core/src/resolve-diagnose-target.ts:27` (several raise an error naming each), `packages/core/src/project-info/find-nearest-ancestor-package-json.ts:25` (the upward walk stops at a project boundary).

**Acceptance Criteria:**
- [ ] The upward manifest walk (`src/scan_target.rs:89`) stops after the first directory that contains `.git`, as a file or a directory, so worktrees and submodules count
- [ ] When the upward walk finds no manifest, discovery lists `Cargo.toml` files below the path: through `git ls-files` inside a repository, else a walk to depth 4 that skips hidden directories, `target` and `node_modules`. It keeps only manifests that are not nested below another one found, which it takes for their workspace.
- [ ] Given exactly one candidate, when scanned, then the scan runs on it, and one line naming its workspace-relative path is printed to stderr before Clippy starts
- [ ] Given several candidates, when scanned, then the run fails at stage `discovery` with code `ambiguous-workspace`, listing at most 10 sorted relative candidates and the hint to pass one, exit 2
- [ ] Given only manifests under `target/` or ignored by git, when scanned, then the existing `no-manifest` error is kept
- [ ] Given a path inside a git repository that sits below an unrelated outer `Cargo.toml`, when scanned, then the outer workspace is never selected
- [ ] No candidate outside the requested directory qualifies, and no absolute path appears in the error or in `--json`
- [ ] Tests in `tests/configuration_kernel.rs` cover a Tauri-shaped fixture, a two-workspace fixture and a fixture whose only manifests are under `target/`

---

### EP-002: An agent and CI loop that never misfires

**Release:** 1 (0.11.0). The installed hooks, the Action and the generated workflow behave correctly in the situations they meet every day: an aborted turn, a missing binary, a push to main, a failed scan, a shallow checkout, a failed gate, a nested workspace, a PTY.

**React Doctor reference:**
- Cursor continuation guard: `packages/react-doctor/src/cli/utils/install-agent-hooks.ts:262`.
- Hook binary probe: `packages/react-doctor/src/cli/utils/install-git-hook-file.ts:38`.
- Push never fails: `action.yml:721`.
- Failed-scan comment: `scripts/render-github-action-comment.mjs:431`.
- `comment-on-clean`: `041d1ab1c`, #1386.
- Base fetch and drift fix: `action.yml:155`, `418c6c82a`, #1447.
- Explicit cache save after a failed gate: `action.yml:105`.
- Workflow at the git root: `c6c9e99ff`.
- Animation gated on hooks and agents: `packages/react-doctor/src/cli/utils/onboarding-pacing.ts:46`.

**Definition of Done:**
- Every misfire listed in Problem 3 has a test that reproduces it and now passes.
- The dogfood workflow runs the Action on this repository with a shallow checkout and a push event.
- `src/main.rs` is shorter than before the epic.

#### US-006: Honor Cursor's abort status and loop count in the stop hook
**Description:** As a Cursor user who stops a turn, I want the end-of-turn hook to stay silent, so that a turn I aborted is not rescanned and resubmitted to the agent.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 89 (Z7-N02)
**React Doctor reference:** `packages/react-doctor/src/cli/utils/install-agent-hooks.ts:262` (scan only when `status` is `completed`), `packages/react-doctor/src/cli/utils/install-agent-hooks.ts:264` (continue only below the cap), `packages/react-doctor/src/cli/utils/constants.ts:135` (`AGENT_HOOK_MAX_CONTINUATIONS = 1`).

**Acceptance Criteria:**
- [ ] For `hook run cursor`, when the input carries a `status` other than `completed`, the hook exits 0 with empty stdout and starts no scan
- [ ] When the input carries a numeric `loop_count` of at least `MAX_FOLLOWUPS` (a new constant, 1), the hook exits 0 with empty stdout and starts no scan
- [ ] Given `{"status":"completed","loop_count":0}` and an introduced finding, when the hook runs, then it prints one `followup_message` as today
- [ ] Given missing, non-JSON or wrongly typed input, when the hook runs, then it scans as today
- [ ] `tests/agent_hooks.rs` covers `aborted`, `error`, `loop_count` 1 and malformed input, and asserts that no target directory is created when the scan is skipped
- [ ] The module comment of `src/hook/agent.rs` documents both guards beside the Claude `stop_hook_active` guard

#### US-007: Make the git hook survive a missing binary and refresh on reinstall
**Description:** As a teammate who never installed rust-doctor, I want to commit in a repository whose hook directory is shared, so that the tool another contributor chose does not block my work.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 94 (Z6-N06, Z7-N04, Z11-N05, Z7-N11)
**React Doctor reference:** `packages/react-doctor/src/cli/utils/install-git-hook-file.ts:38` (`command -v` probe), `packages/react-doctor/src/cli/utils/install-git-hook-file.ts:53` ("command not found; skipping staged scan."), `packages/react-doctor/src/cli/utils/install-git-hook-file.ts:73` (the managed block is replaced in place on reinstall).

**Acceptance Criteria:**
- [ ] The script `hook_script` writes runs `exec <command>` only when `command -v rust-doctor` succeeds. Otherwise it prints "rust-doctor is not on PATH: this commit was not judged." to stderr and exits 0.
- [ ] With the binary present, the gate's exit code is unchanged (fail closed)
- [ ] Given a scratch repository with the hook installed and `PATH=/usr/bin:/bin`, when `git commit` runs, then the commit succeeds and stderr names the skip, and `sh -n` passes on the script
- [ ] `hook install git|claude|cursor --update` rewrites a file or entry carrying rust-doctor's marker when its content differs from the current template, and prints the old and new command
- [ ] Without `--update`, an installed hook that differs from the current template is reported as outdated, naming `--update`, instead of "already installed"
- [ ] Given a hook file without rust-doctor's marker, when `--update` runs, then nothing is written and the command exits 2
- [ ] `lefthook.yaml`, `.lefthook.yml` and `.lefthook.yaml` are recognized beside `lefthook.yml`
- [ ] A test parses the hook command and the Stop-hook command with `Cli::try_parse_from`, so a renamed flag fails the build

#### US-008: Stop the Action failing default-branch pushes on legacy findings
**Description:** As the engineer owning CI, I want a push to the default branch to report its health without failing, so that legacy findings never turn main red while pull requests keep their gate.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 90 (Z9-N01)
**React Doctor reference:** `action.yml:721` (a push is a health snapshot that never fails on findings), `action.yml:725` (the exit code propagates only on `pull_request`).

**Acceptance Criteria:**
- [ ] The Action's exit step receives `EVENT_NAME` through `env` and turns exit 1 into 0 when the event is not `pull_request`
- [ ] Exit 2 (the scan itself failed) fails the step on every event
- [ ] The generated workflow applies the same rule to its scan step (`src/ci.rs:399`)
- [ ] Stubbed tests in `src/ci/tests.rs`: `EXIT_CODE=1` with `push` returns 0, with `pull_request` returns 1, and `EXIT_CODE=2` returns 2 on both
- [ ] The job summary of a push still renders the score and the findings
- [ ] The release note of 0.11.0 states the behavior change of the published Action

#### US-009: Say in the summary and the comment what happened, including why a scan failed
**Description:** As a reviewer, I want the sticky comment to explain a scan that did not complete, and to stay quiet on a clean pull request when I ask it to, so that "No finding." never stands for "nothing ran".

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 91, 95 (Z9-N02, Z9-N04)
**React Doctor reference:** `scripts/render-github-action-comment.mjs:431` (a failed report renders an error body), `scripts/render-github-action-comment.mjs:24` (the remedy names the workflow file), `041d1ab1c` (#1386, `comment-on-clean`), `action.yml:470` (no new comment when clean and opted out, but an existing one is still updated).

**Acceptance Criteria:**
- [ ] When `errors` is non-empty, `render_markdown` renders a section listing each error's `stage`, `code` and escaped `message`, bounded by the existing `MESSAGE_CHARACTERS`
- [ ] When `status` is `failed`, the findings line reads "No finding was collected." instead of "No finding."
- [ ] For `shallow-clone`, `base-unavailable` and `merge-base-unavailable`, the section appends the `actions/checkout` `fetch-depth: 0` remedy
- [ ] The Action computes `clean` (status complete, and no introduced finding in a baseline scope or no finding otherwise) and takes a `comment-on-clean` input, default `true`
- [ ] When `clean` is true, `comment-on-clean` is false and no prior comment exists, no comment is created. When a prior comment exists, it is still updated.
- [ ] Goldens cover a scope-failure report and the existing `tests/fixtures/report-markdown/baseline.json` errors, and a stubbed test covers the three comment cases
- [ ] The renderer stays a pure function of its input: no environment variable is read inside `src/render/markdown.rs`

#### US-010: Make the Action survive a shallow pull request checkout, or explain it
**Description:** As the engineer adding the Action to a workflow, I want a default depth-1 checkout to work or to fail with the fix, so that the first pull request after setup does not end in "Git base commit is unavailable."

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-009
**Audit items:** 92 (Z9-N03)
**React Doctor reference:** `action.yml:155` (best-effort fetch of the base before a pull request scan), `418c6c82a` (#1447, changed files from the event's base and head), `packages/react-doctor/src/cli/utils/install-github-workflow.ts:60` (the generated workflow sets `fetch-depth: 0` and says why).

**Acceptance Criteria:**
- [ ] In the Action's scan step, when the scope is not full and `git rev-parse --is-shallow-repository` prints `true`, the step runs `git fetch --no-tags --unshallow origin "+refs/heads/$PULL_REQUEST_BASE:refs/remotes/origin/$PULL_REQUEST_BASE" || true`, with every input passed through `env`
- [ ] The check at `src/git_scope.rs:640` also covers `base-unavailable`, so a missing base in a shallow clone reports `shallow-clone` with the remedy
- [ ] Given a stubbed shallow repository, when the step runs, then the fetch runs once and its failure never fails the step
- [ ] Given a full clone, when the step runs, then no fetch runs
- [ ] `README.md` and the Action's `base` input description state the `fetch-depth: 0` requirement
- [ ] A unit test sits beside `a_merge_base_missing_from_a_shallow_clone_says_so` for the base case

#### US-011: Keep the build when the gate fails, and harden every shipped workflow
**Description:** As the engineer paying for CI minutes, I want a failed gate to keep its Cargo cache and the shipped YAML to resist a compromised upstream action, so that the retry is warm and the token stays out of reach.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 93, 97, 99 (Z5-N08, Z9-N08, Z9-N07, Z11-N08)
**React Doctor reference:** `action.yml:105`, `action.yml:107` (an explicit save after the scan, because the post-job save is skipped when a blocking scan fails the job), `action.yml:65` (third-party actions pinned to a full SHA), `.github/workflows/ci.yml:55` (`persist-credentials: false`).

**Acceptance Criteria:**
- [ ] The Action gains a `cache` input (default `true`) that runs Swatinem/rust-cache v2 with `cache-on-failure: true` on the scanned workspace before the scan step
- [ ] The generated workflow's rust-cache step and `.github/workflows/dogfood.yml` set `cache-on-failure: true`
- [ ] Two consecutive dogfood runs show whether `target/rust-doctor/baseline` survives rust-cache's pruning. If it does not, it is added to `cache-directories`, and the measurement is recorded in the story's commit message.
- [ ] Every non-local `uses:` in `action.yml` and in the generated workflow is pinned to a 40-hex SHA followed by a `# <ref>` comment
- [ ] A test in `src/ci/tests.rs` fails on any `uses:` line, other than `./`, that does not match `@[0-9a-f]{40} # `
- [ ] Every `actions/checkout` step in this repository's workflows sets `persist-credentials: false`, and `tests/docs_contract.rs` asserts it for every workflow file
- [ ] Given a gate that fails the job, when the run ends, then the cache save still runs, which the dogfood workflow demonstrates once

#### US-012: Write `ci install`'s workflow at the git root, scanning the workspace
**Description:** As a developer whose Rust workspace sits in `src-tauri/`, I want `ci install` to write a workflow GitHub actually reads, so that CI is set up by the command that claims to set it up.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 96 (Z9-N05)
**React Doctor reference:** `c6c9e99ff` (nested workflows managed at the repository root), `packages/react-doctor/src/cli/utils/install-github-workflow.ts:150`, `packages/react-doctor/src/cli/utils/install-github-workflow.ts:156` (the git root is resolved and the workflow written there).

**Acceptance Criteria:**
- [ ] `ci install` resolves the git toplevel as `src/git_hook.rs:40` does and writes under `<toplevel>/.github/workflows/`
- [ ] When the Cargo workspace root differs from the toplevel, the workflow scans the relative workspace path and passes it to rust-cache's `workspaces` input
- [ ] The relative path is validated against `[A-Za-z0-9._/-]+` with no `..` segment before it is written into YAML, and refused otherwise
- [ ] Given a path outside any git repository, when `ci install` runs, then nothing is written and it exits 2
- [ ] Given any `.github/workflows/*.y{a,}ml` under the toplevel that already calls `arthjean/rust-doctor@` or carries the marker, when `ci install` runs, then it reports that file as the existing install and writes nothing
- [ ] Tests in `tests/ci_and_report_markdown.rs` cover the nested workspace and the no-repository case

#### US-013: Resolve one terminal profile, and stop animating for hooks, agents and `--yes`
**Description:** As an agent or a hook running rust-doctor under a terminal, I want plain, final output with no count-up and no line rewrites, so that a transcript or a hook log holds the result instead of frames.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 69, 76, 77 (Z8-N02, Z7-N15, Z8-N17)
**React Doctor reference:** `packages/react-doctor/src/cli/utils/onboarding-pacing.ts:46`, `packages/react-doctor/src/cli/utils/onboarding-pacing.ts:51` (no animation under a git hook or CI), `packages/react-doctor/src/cli/utils/is-ci-environment.ts:68` (the agent marker table), `packages/react-doctor/src/cli/utils/should-use-tui.ts:9`.

**Acceptance Criteria:**
- [ ] A new module (for example `src/terminal_profile.rs`) resolves once per run: interactive, color, animate, rewrite-progress and width, from the TTY state, `TERM`, `NO_COLOR`, `CI` by value, `--yes`, `--json` and the marker list. `src/main.rs` calls it instead of its scattered predicates and ends shorter than 999 lines.
- [ ] `animate` is true only when the run is interactive and colored, so `--yes`, any marker and `CI` set to anything but `false` or `0` disable the count-up
- [ ] Progress rewrites the line only when stderr is a terminal, `TERM` is not `dumb` and no marker is set. Otherwise it prints one line per phase.
- [ ] The marker list adds `CLAUDE_CODE`, `CODEX_CI`, `OPENCODE`, `GOOSE_TERMINAL`, `AMP_THREAD_ID`, `CLINE_ACTIVE`, `AUGMENT_AGENT`, `TRAE_AI_SHELL_ID`, `AGENT_SESSION_ID`, `AGENT_THREAD_ID`, `CODEBUILD_BUILD_ID`, `BITBUCKET_BUILD_NUMBER`, `TRAVIS` and `DRONE`, each with a comment naming its source. `SKILL.md` lists them.
- [ ] Table tests cover `GIT_DIR`, `CLAUDECODE`, `CI=false`, `CI=true`, `--yes`, `TERM=dumb` and a non-terminal stderr
- [ ] Given `rust-doctor . --yes` under a PTY, when it finishes, then stdout contains no cursor-up sequence, which `tests/local_cli_experience.rs` asserts
- [ ] Given a terminal whose size cannot be read, when the profile resolves, then the width falls back to `COLUMNS`, then to 80, and nothing fails

---

### EP-003: The shipped native rules hold their verdict

**Release:** 1 (0.11.0). Close the measured recall gaps and false-positive classes of the shipped native rules, five of them in the uncommitted EP-006, and make every future fix leave a replayed seed and every adjudication cite its evidence.

**React Doctor reference:**
- Positive proof instead of giving up on ambiguity: `943ca324c` (#1910).
- Same-file function resolution: `packages/oxlint-plugin-react-doctor/src/plugin/utils/resolve-exact-local-function.ts:516`, extended in `23d616017` (#1855).
- Raw-SQL escape hatches: `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/raw-sql-injection-risk.ts:33`.
- Package-graph completeness before orphan reports: `packages/core/src/project-analysis/report/files.ts:70`.
- Semantic variants with carve-out reasons: `scripts/fn-mining/fn-mining-case.ts:11`.
- Regression seeds replayed on every test run: `packages/fuzz/tests/fuzz-harness-smoke.test.ts:74`.
- Citations required in every judgment: `5915d6b0c` (#1886).

**Definition of Done:**
- Every shortfall case named in Problem 4 has a fixture that fires, or a carve-out with its reason in the variant table.
- The orphan rule reports nothing on the `cfg_attr(path)` and `cfg_if!` layouts.
- The seed directory replays inside `cargo test`.
- `tests/corpus.json` is regenerated for every rule whose verdicts moved, with the deltas recorded.

#### US-014: Variant gate: semantic cases with carve-out reasons, and the gap-exposing rewrites
**Description:** As the catalog maintainer, I want the variant gate to hold each detector to the real forms of its defect, not only to syntactic rewrites of one form, so that a gap is measured by `cargo test` instead of found by an audit.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 108 (Z10-N06, Z2-N15)
**React Doctor reference:** `scripts/fn-mining/README.md:4` (hand-written variants of the documented bad pattern), `scripts/fn-mining/fn-mining-case.ts:10`, `scripts/fn-mining/fn-mining-case.ts:11` (`shouldFire` and `carveOutReason`), `packages/fuzz/src/verdict-preserving-variants.ts:28` (a must-preserve tier and an advisory tier).

**Acceptance Criteria:**
- [ ] `Variants` (`src/source_kernel/detectors.rs:243`) gains `semantic: &'static [Semantic]`. Each `Semantic { description, source, fires, carve_out }` holds a whole unit.
- [ ] The gate requires every `fires: true` case to fire, every `fires: false` case to carry a non-empty `carve_out` and stay silent, and fails when a carved case fires (a stale carve-out)
- [ ] The rewrite list (`src/source_kernel/tests/variants.rs:28`) adds `fully-qualified-under-glob`, `inside-macro`, `crate-relative-import`, `through-helper` and a negative-only `numeric-parameter`
- [ ] Each table is seeded from its detector's source, including at least `reqwest::ClientBuilder::new()` for the TLS rule, `.args(["-c", x])` and `Command::new("/bin/sh")` for the shell rule, `String +` and `write!` into a `String` for the SQL rule, and an `async move` block inside `tokio::spawn` for the blocking rule
- [ ] Every case the current detectors miss is recorded as a carve-out naming the story that fixes it (US-015 to US-018), so each of those stories flips its entries
- [ ] An advisory tier reports, without failing, the forms a rule may legitimately not chase
- [ ] The gate still runs inside `cargo test` in under 5 seconds on this repository

#### US-015: Stop prelude globs from silencing every crate-rooted path
**Description:** As a developer whose async module starts with `use futures::prelude::*;`, I want `std::thread::sleep` in an async fn still reported, so that an idiomatic import does not switch the native rules off.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-014
**Audit items:** 2 (Z2-N02)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/semantic/scope-analysis.ts:859` (each reference walks its scope chain, so an unrelated import never removes provenance), `943ca324c` (#1910, demand positive proof rather than abstaining).

**Acceptance Criteria:**
- [ ] `AliasMap::provenance` distinguishes a glob-masked name (`GlobMasked`) from `Indeterminate` (`src/source_kernel/aliases.rs:118-119`)
- [ ] `Context::item_path` accepts a glob-masked head when it is a sysroot crate name or a key of the unit's crate aliases, and the masking glob's own path starts with an external crate, not `crate`, `self` or `super`
- [ ] Given `use futures::prelude::*;` and `std::thread::sleep(..)` in an async fn, when scanned, then `blocking_call_in_async` reports it
- [ ] Given `use diesel::prelude::*;` and `diesel::sql_query(format!(..))` with a runtime value, when scanned, then `sql_built_from_format` reports it
- [ ] Given `use crate::prelude::*;`, when a crate-rooted path is resolved, then the detector still abstains
- [ ] An explicit `use` of the same name still wins over the glob
- [ ] The `fully-qualified-under-glob` carve-outs of US-014 are removed, and the gate stays green

#### US-016: Clear numeric parameters from SQL and shell interpolation
**Description:** As a developer paginating with `LIMIT {limit}` where `limit: i64`, I want no injection finding, so that the security rules keep their precision on the most common integer interpolation.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-014
**Audit items:** 3 (Z2-N04)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/utils/get-symbol-type-annotation.ts:6` (a binding's declared type is read without inference), `packages/oxlint-plugin-react-doctor/src/plugin/semantic/scope-analysis.ts:55`.

**Acceptance Criteria:**
- [ ] `bindings.rs` resolves a plain-name parameter, or a `let x: T`, whose type is written to `Binding::Typed(ast::Type)` instead of `Indeterminate` (`src/source_kernel/bindings.rs:133-134`)
- [ ] `Context::injectable(&ast::Expr) -> bool` is true when the expression is not constant, and is neither a binding of a primitive type (`bool`, `char`, every integer and float primitive, `NonZero*`) nor a cast to one
- [ ] `dynamic_text` and the `format!` argument check ask `injectable` instead of `!constant` (`src/source_kernel/detectors.rs:190`)
- [ ] Given `fn page(limit: i64)` building `format!("SELECT * FROM t LIMIT {limit}")` into `sqlx::query`, when scanned, then nothing is reported; the same holds for `n as i64` and for `format!("sleep {secs}")` with `secs: u32` in the shell rule
- [ ] Given a `&str`, `String` or generic `T` parameter in the same position, when scanned, then the finding is reported as today
- [ ] Given a parameter typed through an alias (`type Id = i64;`) or a path type, when resolved, then it stays injectable, so no new silence exists without syntactic proof
- [ ] `sql_built_from_format` and `dynamic_shell_command` are re-measured on the corpus and the moved verdicts recorded
- [ ] The `numeric-parameter` carve-out of US-014 is removed

#### US-017: Follow same-module synchronous helpers called from async code
**Description:** As a developer of an async service, I want `fn load_config() { std::fs::read_to_string(..) }` reported when an async handler calls it, so that the blocking rule sees work hidden one call away, which is how services are commonly written.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-014
**Audit items:** 5 (Z2-N03)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/utils/resolve-exact-local-function.ts:516`, `packages/oxlint-plugin-react-doctor/src/plugin/utils/resolve-exact-local-function.ts:517` (a callee resolves to its same-file declaration), `23d616017` (#1855) and `79105a3b9` (#1182).

**Acceptance Criteria:**
- [ ] `Context::local_function(&ast::Path) -> Option<ast::Fn>` resolves a single-segment, unbound path to exactly one `fn` item of that name in the nearest enclosing block or module, plus `Self::f` and `self.f()` against the enclosing `impl`
- [ ] For a call in async code whose callee resolves to a non-async local fn, `blocking_call_in_async` searches that body, to depth 4 with a visited set, skipping nested fns and offloaded closures, for a direct blocking call
- [ ] The finding is reported at the helper call, and its message names the helper and the blocking call inside it
- [ ] Given two `fn` items of the same name in scope, or a recursive helper, when scanned, then the detector abstains, or terminates without a finding
- [ ] The fixture case `calls_a_synchronous_helper` (`tests/fixtures/source-kernel/native-families/service/src/blocking_negative.rs:12`) flips to positive, and the rule's help and the assertion at `tests/native_rule_families.rs:128` change together
- [ ] The rule is re-measured on the corpus, and the `through-helper` carve-out of US-014 is removed

#### US-018: Report the raw-SQL builder escape hatches and the argv and Windows shell forms
**Description:** As a developer, I want SQL handed to a builder as raw text and shell commands passed as an argv array reported, so that the two shipped injection rules cover the forms these APIs offer instead of one spelling each.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-014
**Audit items:** 6, 7 (Z1a-N04, Z1a-N05)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/raw-sql-injection-risk.ts:33` (the builders' raw entry points named explicitly), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/raw-sql-injection-risk.ts:22` (a pure literal stays silent), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/command-execution-input-risk.ts:14` (shell binaries including `cmd` and `powershell`), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/command-execution-input-risk.ts:32` (a discrete argv element of a non-shell program cannot inject).

**Acceptance Criteria:**
- [ ] `sql_built_from_format` covers `sqlx::QueryBuilder::push` when its receiver's chain root is `QueryBuilder::new(..)`, `diesel::dsl::sql`, and `sea_orm::Statement::from_string` with the SQL text at argument 1 (`src/source_kernel/detectors/sql.rs:59`, `:68`)
- [ ] `QueryBuilder::push_bind` with a runtime value stays silent, and the help names it as the fix
- [ ] `dynamic_shell_command` accepts `ksh`, `cmd`, `cmd.exe`, `powershell`, `powershell.exe` and `pwsh`, and an absolute path whose file name is one of them (`src/source_kernel/detectors.rs:212`)
- [ ] It accepts the flags `-c`, `/C`, `/c` and `-Command`, and the `.args([..])` form whose array holds the flag followed by a dynamic element
- [ ] `Command::new(<dynamic text>)` is reported as a tainted program
- [ ] Given a dynamic argument of a fixed non-shell program, such as `Command::new("git").arg(user)`, when scanned, then nothing is reported
- [ ] An `api-equivalent` rewrite (`.arg(a).arg(b)` to `.args([a, b])`) joins the variant gate, and the carve-outs of US-014 for these forms are removed
- [ ] Both rules are re-measured on the corpus

#### US-019: Require a key body and veto placeholders in credential matching
**Description:** As the author of a PEM-parsing crate, I want `const HEADER: &str = "-----BEGIN PRIVATE KEY-----";` left alone, so that a P1 security finding means a key was committed.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001
**Audit items:** 8 (Z1a-N03)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/key-lifecycle-risk.ts:20` (the PEM header must be followed by a base64 body), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/key-lifecycle-risk.ts:31` (placeholder words veto), `packages/oxlint-plugin-react-doctor/src/plugin/constants/security.ts:242` (one shared placeholder vocabulary).

**Acceptance Criteria:**
- [ ] `private_key_block` (`src/repo_hygiene.rs:366`) reports a header only when the following non-empty lines carry a base64 run of at least 38 characters
- [ ] A match is vetoed when the matched token, or the 40 bytes before it, contains `example`, `sample`, `dummy`, `fake` or `placeholder`, or when a `...` truncation follows it
- [ ] A database URL with an embedded password (`postgres`, `postgresql`, `mysql`, `mongodb`, `mongodb+srv`, `redis`) becomes a credential pattern, vetoed when the password is a placeholder (`pass`, `changeme`, `${VAR}`, `<...>`, `{{...}}`) or the host is dotless or `localhost`
- [ ] Given a bare header, AWS's documented `AKIAIOSFODNN7EXAMPLE`, or `postgres://user:pass@localhost/db`, when scanned, then nothing is reported; the tests at `src/repo_hygiene.rs:627` and `:657` move to the new verdicts
- [ ] Given a real-shaped key body under a header, or a `postgres` URL carrying a real-shaped password on a dotted host (the seed `tests/fixtures/regressions/hardcoded_credential--deployed-database-password/` splits it so this document holds none), when scanned, then the finding is reported and masked by US-001
- [ ] Each pattern's verdicts are measured on the corpus and recorded

#### US-020: Withhold orphan findings where the module walk lost declarations
**Description:** As a developer with platform-specific modules, I want no "delete this file" finding on a file Cargo compiles on another platform, so that an agent following the help never deletes working code.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 4 (Z4-N03)
**React Doctor reference:** `packages/core/src/project-analysis/utils/mark-complete-package-graphs.ts:225`, `packages/core/src/project-analysis/utils/mark-complete-package-graphs.ts:231` (parse and dynamic-loader uncertainty mark a package incomplete), `packages/core/src/project-analysis/report/files.ts:70` (orphan reports require a complete package graph), `packages/core/tests/project-analysis-unused-file-completeness.test.ts:423`.

**Acceptance Criteria:**
- [ ] The module walk attributes every lost `mod` request (unresolved, invalid `#[path]`, overlapping a parse error) to the declaring unit's packages, and exposes `Enumeration::module_tree_complete(package)`
- [ ] `structure/manifest.rs::orphans` skips a package whose module tree is incomplete, or one of whose reached units holds a macro call whose token tree contains the `mod` keyword
- [ ] Every `path` literal inside a `cfg_attr(..)` on a `mod` declaration counts as reached for the orphan pass (`src/source_kernel/walk.rs:471` reads only a top-level `path`)
- [ ] Given `#[cfg_attr(unix, path = "sys/unix.rs")] mod sys;` with `sys/unix.rs` and `sys/windows.rs`, or `cfg_if! { if #[cfg(unix)] { mod imp; } }`, when scanned, then `orphan_module_file` reports nothing in that package
- [ ] Given a truly undeclared file in a package whose module tree is complete, when scanned, then it is still reported
- [ ] Each abstention is counted, not reported as an error, and `orphan_module_file` is re-measured on the corpus

#### US-021: Pin every native false positive as a replayed seed, and require cited evidence in adjudication
**Description:** As the catalog maintainer, I want each confirmed false positive to become a minimal seed that `cargo test` replays, and each adjudication verdict to cite the lines that decided it, so that a fixed false positive cannot come back and a verdict nobody read the code for cannot enter the record.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None
**Audit items:** 107, 109 (Z10-N01, Z2-N09, Z10-N04)
**React Doctor reference:** `packages/fuzz/corpus/README.md:9` (every seed is a confirmed false positive), `packages/fuzz/src/load-fuzz-corpus.ts:20` (the verdict directive), `packages/oxlint-plugin-react-doctor/src/test-utils/run-rule.ts:137` (an in-memory rule harness), `packages/evals/src/utils/validate-sol-citations.ts:18` (a quote must equal the cited lines), `5915d6b0c` (#1886), `943ca324c` and `8fdead907` (fixes that each added a seed).

**Acceptance Criteria:**
- [ ] `tests/fixtures/regressions/<rule-tail>--<slug>/` holds hand-minimized seeds (never corpus code), each a single `.rs`, a single `Cargo.toml` or a minimal crate, with headers `// rule:`, `// weakness:`, `// source:` and `// verdict: quiet|fires`
- [ ] `weakness` takes a value from a closed list, and the loader refuses an unknown rule, a missing header, an unknown directive value or a parse error, naming the seed
- [ ] A test, `every_regression_seed_keeps_its_declared_verdict`, runs source-kernel seeds in process through a public-to-tests harness, `run_detectors(&CaseUnit)`, returning rule, message, span and context, and runs manifest and structure seeds through the binary with every Clippy rule off
- [ ] A seed for a false positive not yet fixed declares `fires` with `// known-false-positive: <reason>`, and the test fails when it stops firing, so the fix must flip the header
- [ ] Every rule touched by US-015 to US-020 lands with at least one `quiet` seed for the fixed shape, and one `fires` counterexample per necessary condition of the exemption
- [ ] Each adjudication pass in `tests/corpus.json` gains `evidence: [{path, start_line, end_line, quote}]`. Offline, every pass needs a citation on the site's path covering the site's line. In the cache-gated replay, every quote must equal those lines byte for byte, and a failing pass makes its pair non-agreeing.
- [ ] The 239 existing pairs are grandfathered by a `protocol_cutoff` date recorded in the corpus artifact, and the website snapshot schema bump is noted in the release checklist
- [ ] `.claude/skills/corpus-adjudicate` requires every `false_positive` verdict on a `rust_doctor::*` rule to name its seed or say why it cannot be minimized

---

### EP-004: Overlapped passes, a cached base side, and a delta that survives refactors

**Release:** 2 (0.12.0). Measure the scan per phase, overlap what is independent, stop rescanning an unchanged base, keep every limit and signal under control, and make the delta report what a change introduced, not what a refactor moved.

**React Doctor reference:**
- Parallel passes with byte-identical output: `packages/core/src/run-inspect.ts:407`.
- Result cache keyed on tree, toolchain and config: `packages/react-doctor/src/cli/utils/scan-result-cache.ts:456`.
- One deadline for every child: `packages/core/src/run-inspect.ts:475`.
- Signals and exit 130: `packages/react-doctor/src/cli/index.ts:27`.
- Benchmark harness with cohorts and a regression rule: `scripts/performance/run-performance.ts:273`.
- Delta by group with rename and line maps: `fcfb6ea06`, `8ed70df0f` (#1860), `3baee0f56`, `packages/core/src/compute-diagnostic-delta.ts:96`.

**Definition of Done:**
- `--timings` publishes per-phase milliseconds.
- A warm full scan of this repository takes at most max(Clippy phase, native phases) plus 10%, measured by the harness of US-022.
- A second baseline run on an unchanged base spawns no Cargo process for the base side.
- Ctrl-C leaves no child process and no snapshot behind.
- Renaming a variable inside a flagged line reports nothing introduced.

#### US-022: Time each phase and gate a whole-binary benchmark
**Description:** As the maintainer, I want per-phase timings and a reproducible benchmark of the release binary, so that every performance story in this epic is sized and verified by measurement instead of by estimate.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 43 (Z5-N05)
**React Doctor reference:** `scripts/performance/run-performance.ts:273` (warmups and samples per cohort), `scripts/performance/run-performance.ts:302` (diagnostics must not change between samples), `scripts/performance/constants.ts:46`, `scripts/performance/constants.ts:47` (a regression is at least 10 percent and 250 ms), `packages/core/src/schemas.ts:138` (`elapsedMilliseconds` in the report).

**Acceptance Criteria:**
- [ ] `ExecutionContext::run` records the milliseconds of: probes, metadata, lint-list, Clippy (split at the first workspace-member artifact into dependencies and linting), walk, source kernel, structure, cargo-health and repo
- [ ] `--timings` publishes them as an optional `timings` object in `--json`. Without the flag the field is absent, so default reports stay byte-deterministic. Under `--verbose` one line prints the phase totals.
- [ ] `tests/performance.rs`, marked `#[ignore]`, drives the release binary over the corpus clone cache in cold and warm cohorts, with 1 warmup and 5 samples, and writes median and median absolute deviation per phase to `target/performance/results.json`
- [ ] A run whose status is not complete, or whose diagnostics digest differs between samples or cohorts, fails the harness instead of being recorded
- [ ] A comparison against a committed baseline runs only when CPU model, core count and toolchain match, and fails on a regression of at least 10% and 250 ms
- [ ] The harness never runs as a required pull request gate. `docs/subsystems/execution.md` states how to run it.

#### US-023: Overlap Clippy with the native passes and the probes
**Description:** As a developer waiting on a scan, I want the native passes to run while Clippy compiles, so that a warm scan costs the longer of the two instead of their sum.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-022
**Audit items:** 38 (Z5-N01)
**React Doctor reference:** `packages/core/src/run-inspect.ts:407` (output stays byte-identical to the serial order), `packages/core/src/run-inspect.ts:639`, `packages/core/src/run-inspect.ts:641` (CPU passes forked to overlap the lint wave).

**Acceptance Criteria:**
- [ ] `source_kernel::enumerate`, narrowing and exclusion gathering move above the Clippy call, so the committed tree is measured before any `build.rs` can write into it, and `docs/subsystems/execution.md` states the reorder
- [ ] Clippy runs on a `std::thread::scope` thread while the source kernel, structure and dependency-truth passes run on the calling thread, and the results are joined before normalization
- [ ] The three version probes and the lint-list probe run concurrently in one `thread::scope`
- [ ] Given every oracle fixture, when scanned before and after the change, then `--json` is byte-identical (`src/report/normalize.rs:94` sorts diagnostics)
- [ ] Given a Clippy start failure, when scanned, then the early return and its stage error are unchanged
- [ ] The baseline's two sides stay serial (`src/execution/baseline.rs`)
- [ ] The harness of US-022 records the warm full-scan time of this repository before and after, in the commit message

#### US-024: Cache the baseline side's findings keyed by its tree
**Description:** As an agent rescanning with `--scope baseline` after each edit, I want the unchanged base judged once, so that every later run pays only for the current side.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-022
**Audit items:** 39 (Z5-N02)
**React Doctor reference:** `packages/react-doctor/src/cli/utils/scan-result-cache.ts:456`, `packages/react-doctor/src/cli/utils/scan-result-cache.ts:457` (keyed on HEAD and a worktree fingerprint), `packages/react-doctor/src/cli/utils/scan-result-cache-payload.ts:143` (only complete scans are stored), `packages/react-doctor/tests/scan-result-cache.test.ts:322`.

**Acceptance Criteria:**
- [ ] A new module (`src/execution/side_cache.rs`) keys the base side on a blake3 digest of length-prefixed fields: the binary version, `SCHEMA_VERSION`, `git rev-parse <base>^{tree}`, the workspace path relative to the repository root, the toolchain triple and `RUSTUP_TOOLCHAIN`, the lint-list digest, the compiled policy and structure settings, the effective rustflags, the `--package` set, the declared `.gitattributes` set, the advisory database identity, and the values of environment variables with the prefixes `CARGO`, `RUST`, `CC`, `CFLAGS` and `CXX`
- [ ] The payload is the base side's normalized diagnostics with their delta fingerprints, so a hit needs neither a snapshot nor a re-read
- [ ] Only complete base sides are stored, in `<target>/rust-doctor/cache/base-sides.json`, written through a temporary file and a rename, merged with the file on disk and pruned to 20 entries
- [ ] A decode failure, or stored member names and manifest paths that differ from the current metadata, is a miss and never an error
- [ ] `RUST_DOCTOR_NO_CACHE=1` disables the cache
- [ ] Given two baseline runs on an unchanged base, when the second runs, then its `--json` is byte-identical to the first and it spawns no Cargo process for the base side
- [ ] Given an edit to `rust-doctor.toml` between runs, when the second runs, then the cache misses
- [ ] Hit or miss is not published in `--json`. Under `--verbose` one line says which.

#### US-025: Bound every child by the run deadline and name what it left unlinted
**Description:** As a user of `--max-duration`, I want the limit to hold for the probes, `cargo metadata` and git, and the report to name the members it never linted, so that a timeout says what is missing instead of passing members off as clean.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 41, 44 (Z5-N07, Z5-N03)
**React Doctor reference:** `packages/core/src/runners/oxlint/spawn-batches.ts:208` (each child gets the smaller of its own timeout and the remaining budget), `packages/core/src/run-inspect.ts:475` (`capToDeadline`), `packages/core/src/runners/oxlint/spawn-batches.ts:235` (deadline-skipped files are recorded and never shown as clean).

**Acceptance Criteria:**
- [ ] The version probes take the nearer of their 10 s timeout and the run deadline, as `src/execution/lint_list.rs:40` already does
- [ ] `cargo metadata` (`src/scan_target.rs:160`), the baseline's metadata resolution and every bounded git call receive the run deadline
- [ ] Given a fake `cargo` that sleeps in the version probe and `--max-duration 2`, when scanned, then the run exits within 4 s with `deadline-exceeded`
- [ ] When the deadline ends Clippy, the members it left unlinted are named in the error (`src/execution/clippy.rs:181-182` no longer skips them)
- [ ] When Cargo's stderr shows `Blocking waiting for file lock`, the error appends "Cargo waited on the build directory lock held by another Cargo or rust-analyzer."
- [ ] A member listed as unlinted carries no per-member score that reads as clean
- [ ] The test `the_deadline_ends_the_scan_kills_the_build_script_and_keeps_what_was_found` asserts the sleeping member is named

#### US-026: Kill Cargo's process group and clean snapshots on SIGINT and SIGTERM
**Description:** As a developer who presses Ctrl-C, I want every process the scan started to stop and every snapshot to be removed, so that rust-analyzer is not left waiting on a lock held by an orphaned build.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 42 (Z5-N04, Z7-N12)
**React Doctor reference:** `packages/react-doctor/src/cli/index.ts:27`, `packages/react-doctor/src/cli/index.ts:28` (SIGINT and SIGTERM handlers), `packages/react-doctor/src/cli/utils/exit-gracefully.ts:19` (every registered scan is aborted), `packages/react-doctor/src/cli/utils/json-mode.ts:127` (a JSON cancellation report), `packages/react-doctor/src/cli/utils/constants.ts:7` (exit 130).

**Acceptance Criteria:**
- [ ] `libc` moves from dev-dependency to dependency at its locked version, with no other change to `Cargo.lock`, and `docs/subsystems/execution.md` records the reason
- [ ] On Unix, `execution/process.rs` registers each bounded child's process group, and removes it after `wait`
- [ ] SIGINT and SIGTERM handlers installed with `sigaction` only set an atomic flag and write one byte to a self-pipe. A watcher thread kills every registered group with the existing `kill_tree`.
- [ ] The interrupted pass ends as a deadline does, with stage error code `interrupted`. `--json` prints one valid failed report, the snapshot and temporary roots are removed by their normal `Drop`, and the process exits 130.
- [ ] A second signal exits at once
- [ ] Given the sleeping-build fixture run with `--max-duration 600 --json`, when SIGINT is sent after the build script's PID file appears, then the exit is 130, stdout parses with code `interrupted`, no process of the group remains, and the baseline temporary root is gone
- [ ] On Windows the behavior is unchanged and documented as such

#### US-027: Strip lint levels from every rustflags source Cargo reads
**Description:** As a developer whose `.cargo/config.toml` sets `[target.'cfg(all())'] rustflags = ["-D", "warnings"]`, I want catalogued warnings to stay warnings, so that the scan does not fail its build on the first one.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 40 (Z5-N06, Z4-N09)
**React Doctor reference:** `packages/core/src/utils/build-oxlint-child-env.ts:22`, `packages/core/src/utils/build-oxlint-child-env.ts:23` (the child environment is sanitized of every variable that changes how the tool runs), `packages/core/src/check-pnpm-hardening.ts:129` (governing configuration is read up to the monorepo root).

**Acceptance Criteria:**
- [ ] First, on 1.97.1, observe whether appending `-W warnings` after the catalog flags turns an inherited `-D warnings` back into warnings. If it does, ship that and stop here, recording the observation in `docs/subsystems/execution.md`.
- [ ] Otherwise, one `cargo_config` helper returns the ordered config files Cargo reads: the workspace root and its ancestors, `.cargo/config` then `.cargo/config.toml`, then `$CARGO_HOME/config.toml`, each bounded in size
- [ ] `src/execution/rustflags.rs`, `cargo_health::inspect_rustflags` and `repo_hygiene::build_directory` read that list
- [ ] Precedence follows the Cargo book: `CARGO_ENCODED_RUSTFLAGS`, `RUSTFLAGS`, `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` and the matching `[target.<triple>]` and `[target.'cfg(all())']` tables, then `CARGO_BUILD_RUSTFLAGS` or `[build] rustflags`
- [ ] A `[target.'cfg(..)']` table other than `cfg(all())` is left untouched, with a notice, instead of returning nothing (`src/execution/rustflags.rs:154`)
- [ ] Given `CARGO_BUILD_RUSTFLAGS=-Dwarnings`, or a `[target.'cfg(all())']` fixture, when scanned, then a catalogued warning stays a warning and the build succeeds
- [ ] Findings and notices about files outside the workspace name them as `$CARGO_HOME/config.toml` or by ancestor depth, never by absolute path, which a `--json` test asserts

#### US-028: Validate assumption: which delta pairing removes refactor noise without hiding new findings
**Description:** As the maintainer, I want the three candidate pairing policies measured on real history, so that the delta's semantics change on evidence and not by copying react-doctor's choice.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 51 (Z6-N02)
**React Doctor reference:** `fcfb6ea06` (counts compared within each file and rule), `8ed70df0f` (#1860), `packages/core/src/compute-diagnostic-delta.ts:96`, `packages/core/src/compute-diagnostic-delta.ts:108` (pairs `min(base, head)` per group).

**Acceptance Criteria:**
- [ ] A test-only harness replays, on the last 50 commit pairs of this repository touching `src/` and on the last 10 commit pairs of each of the 8 agent-population repositories in the corpus clone cache, three policies:
  - (a) today's evidence pairing;
  - (b) evidence pairing, then pairing of the leftovers of the same rule and Git-renamed path whose lines correspond through the `git diff --unified=0 --find-renames` hunk map;
  - (c) react-doctor's count per (renamed path, rule).
- [ ] For each policy it records introduced findings per pair, and classifies by reading the diff every finding that (a) introduces and (b) or (c) does not
- [ ] It records every pair where (c) reports zero introduced although a finding of a rule was fixed and another of the same rule was added in the same file (a hidden swap)
- [ ] The result is written to `delta-pairing-2026-10.md` under `docs/`, registered as a frozen record, with the decision rule applied: adopt (b) if it removes at least 80% of the false introductions (c) removes; else stop and put the choice to Arthur in Open Questions
- [ ] Given a clone cache that is absent, when the harness runs, then it is skipped with a message and does not fail `cargo test`

#### US-029: Pair the delta through Git renames and hunk correspondence
**Description:** As an agent whose turn is gated on introduced findings, I want a refactor inside a flagged line, a moved file, or a duplicate inserted above an old one judged by what the diff did, so that the gate blocks on what I wrote and points at it.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-028
**Audit items:** 51, 54, 58 (Z6-N02, Z6-N08, Z6-N09, Z3-N01)
**React Doctor reference:** `packages/core/src/compute-diagnostic-delta.ts:138` (base lines mapped through the hunk map), `3baee0f56` (re-rank after each consumption), `packages/core/src/compute-diagnostic-delta.ts:94` (paths mapped through Git renames before grouping), `packages/core/tests/compute-diagnostic-delta.test.ts:117`, `packages/react-doctor/src/cli/utils/resolve-scan-exit-code.ts:18` (one surface decides blocking in every mode).

**Acceptance Criteria:**
- [ ] One bounded `git diff --no-ext-diff --find-renames -z --name-status <base>` (with `--cached` when staged) maps base paths to current paths before any pass, so a renamed file pairs on the same-path passes. The move pass (`src/delta.rs:437`) keeps pairing across files on evidence, which Arthur chose on 2026-10-14 over pairing along renames only: it is what keeps a module extraction without a Git rename from reading as introductions (23 of them on the US-028 sample).
- [ ] One bounded `git diff --unified=0 --find-renames <base>` builds a line map. Among candidates of equal key, the current finding nearest the mapped base line is paired, replacing first-in, first-out (`src/delta.rs:519`).
- [ ] The policy US-028 measured and Arthur selected, (d), pairs leftovers: after evidence pairing, the unpaired findings of one (renamed path, rule) pair by count, each base finding taking the unpaired current finding nearest its mapped line, so the line map decides which finding is reported and never how many.
- [ ] Given a variable renamed inside `cfg.get("k").unwrap()`, when scanned with `--scope baseline`, then nothing is introduced or fixed
- [ ] Given a fix of `todo!()` in `a.rs` and an identical addition in `b.rs` with no rename, when scanned, then zero and zero with `cross_file_matches` 1, the accepted blind spot; given `git mv a.rs b.rs`, then zero and zero, with `cross_file_matches` 1
- [ ] Given an identical finding inserted above an old one, when scanned, then the inserted one is the one reported introduced
- [ ] Given a diff over 1 MiB, or a git call that fails or hits the deadline, when the maps are built, then pairing falls back to line distance with a notice and the delta still completes
- [ ] The baseline gate uses `DiagnosticContext::weighs`, as the full gate does (`src/report/assembly.rs:702` and `:726`). A `println!` added to `build.rs` is listed in `delta.introduced` and does not block under `--blocking warning`.
- [ ] `DeltaSummary` publishes the count of pairings made by count within a (renamed path, rule) group, under `SCHEMA_VERSION` 22, and `docs/subsystems/delta.md` records the change of decision and the accepted blind spot (a fix and an addition of one rule in one file offset each other)

---

### EP-005: A report a human and an agent can act on

**Release:** 2 (0.12.0). The default report shows every rule that weighs, at the width it is read in. Nothing built for the user is lost on a failed handoff. A rule and a site can be explained without a rerun, warnings can carry their own exit code, a stale skill says so, setup takes one command, and the npm path users take is tested.

**React Doctor reference:**
- Every rule group in the headless report: `packages/react-doctor/src/cli/utils/print-headless-report.ts:64`.
- Live measure width: `packages/react-doctor/src/cli/utils/resolve-measure-width.ts:18`.
- Prompt printed on a failed handoff: `packages/react-doctor/src/cli/utils/handoff-to-agent.ts:308`.
- `rules explain` and `why`: `packages/react-doctor/src/cli/index.ts:465`, `:322`.
- Warning exit code: `6ff671033` (#1900).
- Agent install hint: `packages/react-doctor/src/cli/utils/prompt-install-setup.ts:76`.
- `install` with agent detection: `packages/react-doctor/src/cli/utils/detect-agents.ts:12`.
- npm packed smoke: `scripts/smoke-packed-cli-install.ts:162`.

**Definition of Done:**
- A commit blocked by three rules shows three rows without `--verbose`.
- The report renders inside a 40-column pane without wrapping.
- `rust-doctor setup` on a machine with one agent installs its skill and hooks and nothing else.
- The packed launcher is exercised through npm in CI.

#### US-030: Measure the live terminal width and list every scored rule by default
**Description:** As a developer whose commit was blocked, I want every rule that weighs listed in the default report at my pane's real width, so that I see the whole set without `--verbose` and without broken lines.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-013
**Audit items:** 81, 82 (Z8-N01, Z8-N03)
**React Doctor reference:** `packages/react-doctor/src/cli/utils/resolve-measure-width.ts:18`, `packages/react-doctor/src/cli/utils/resolve-measure-width.ts:21` (the live column count, clamped), `packages/react-doctor/src/cli/utils/print-headless-report.ts:64` (every rule group is printed).

**Acceptance Criteria:**
- [ ] When stdout is a terminal, the width comes from the terminal's live size (`console::Term::stdout().size_checked()`), then `COLUMNS`, then 80, resolved by the profile of US-013
- [ ] `render::MIN_WIDTH` drops from 80 (`src/render.rs:28`) to 40, still at least `score_block::MIN_BLOCK_COLUMNS`, with the compile-time assertion kept
- [ ] Render and animation tests run at 40, 60, 80, 100 and 140 columns, and every rendered row fits its width
- [ ] Given a pane narrower than 40 columns, when rendered, then the width clamps at 40 and the animated block still rewinds exactly one block per frame
- [ ] Without `--verbose`, the first scored group keeps its frame, and every other scored group renders as one row: severity, title, `x<occurrences>`, rule id, first `path:line:col`
- [ ] Rows are capped at 20, followed by "and N more rules: --verbose", and unscored groups stay counted in the totals line only
- [ ] Given 25 scored groups, when rendered, then the output holds one frame, 19 rows and the overflow line
- [ ] `docs/subsystems/reporting.md` amends its "One width" and "One view" rules

#### US-031: Label non-production sites and say why a report is empty or unscored
**Description:** As a reader of a mixed report, I want each test, bench, example and build-script site labeled, and an empty or unscored report to say why, so that I know which findings weigh and never read "No issues found." for a scan that did not complete.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 83, 86 (Z8-N05, Z8-N13)
**React Doctor reference:** `packages/react-doctor/src/cli/utils/print-headless-report.ts:28` (each site carries its file context), `packages/react-doctor/src/cli/ink/components/diagnostic-detail.tsx:44`, `packages/react-doctor/src/cli/utils/build-empty-report-message.ts:15` (the empty message names its cause), `packages/react-doctor/src/cli/ink/components/report-landing.tsx:54`.

**Acceptance Criteria:**
- [ ] `DiagnosticContext::label()` returns `tests`, `bench`, `example` or `build script`
- [ ] The linear frame prints a labeled site as `src/x.rs:3:5 (tests, not scored)`, the TUI meta line appends ` · tests`, and the issue prompt's site list carries the same label
- [ ] Given a group mixing a `src/` and a `tests/` site, when rendered in both reports, then only the test site is labeled
- [ ] Given an incomplete scan with zero findings, when rendered, then the linear report prints a warning-styled "No issues detected, but <stages> did not complete." instead of "No issues found." (`src/render.rs:206`)
- [ ] Given source files with zero production lines, when rendered, then the no-score line says only tests, benches, examples or build scripts were found, in both reports
- [ ] The incomplete message is shared by both reports from `presentation`, not duplicated

#### US-032: Print the handoff prompt when the clipboard or the agent fails
**Description:** As a user who chose "copy prompt" over SSH, I want the prompt printed when the clipboard is unavailable, so that the work of building it is not lost and the scan's exit code still holds.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 71 (Z7-N03, Z8-N04)
**React Doctor reference:** `packages/react-doctor/src/cli/utils/handoff-to-agent.ts:308`, `packages/react-doctor/src/cli/utils/handoff-to-agent.ts:309` (a failed launch prints the prompt instead), `packages/react-doctor/src/cli/ink/run-scan-app.tsx:333` (a clipboard failure prints it between rulers).

**Acceptance Criteria:**
- [ ] On a clipboard error, the reason goes to stderr and the prompt, already sanitized and bounded by `PromptBuilder`, goes to stdout between two `---- Agent prompt ----` rulers
- [ ] On an agent that cannot be found or spawned, a warning names the executable, then the prompt prints the same way
- [ ] An agent that exits non-zero after a successful launch is not an error. The process keeps the scan's exit code.
- [ ] The clipboard messages no longer say "choose another destination" (`src/handoff.rs:108`)
- [ ] Given a PTY run with an empty `PATH`, when "copy" is chosen, then stdout holds "Validate with: rust-doctor" and the exit code equals the scan's, which `tests/local_cli_experience.rs` asserts
- [ ] The prompt never contains a credential value, which a test over the credential fixture asserts

#### US-033: Explain a rule and a site without a rerun: `rules explain` and `why`
**Description:** As a developer or an agent facing a finding, I want to ask what a rule does here and why a line is reported, so that I can decide to fix, configure or suppress without reading the source of rust-doctor.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None
**Audit items:** 72, 73 (Z7-N06, Z7-N05)
**React Doctor reference:** `packages/react-doctor/src/cli/index.ts:465` (`rules explain <rule>`), `packages/react-doctor/src/cli/commands/rules.ts:175` (the effective severity and its source), `packages/react-doctor/src/cli/index.ts:322` (`why <location>`), `packages/react-doctor/src/cli/utils/run-explain.ts:93` (the suppression diagnosis).

**Acceptance Criteria:**
- [ ] `rules explain <id> [PATH] [--json]` lives in its own module and resolves an exact id or an unambiguous suffix (`todo` resolves to `clippy::todo`). An unknown id exits 2 naming the closest catalogued id (`src/report/exclusion.rs:144`).
- [ ] It prints producer, category, tier, default level, help, rule URL and the corpus noise rate. With `PATH` it also prints the effective level, its `RuleLevelSource` and every `[[overrides]]` entry that changes it, reading `rust-doctor.toml` without `cargo metadata` or any process.
- [ ] `rules list` stays filesystem-free
- [ ] `why <path:line[:column]> [--report <FILE>] [--json]` lives in its own module and splits the location from the right. A non-positive line is a usage error.
- [ ] Without `--report`, `why` scans only the member owning the file (`--package`) and keeps the diagnostics whose primary span or related location covers the line. With `--report`, it reads the saved JSON and spawns nothing.
- [ ] For each hit it prints rule id, severity and level source, help, URL, code frame (masked by US-001), and the nearby suppressions with their status. With no directive, it prints the native or Clippy suppression to write.
- [ ] Given a line with no finding, when `why` runs, then it says whether the path is ignored, generated or vendored, or that nothing is reported there
- [ ] `SKILL.md` and `tests/skill_contract.rs` gain both subcommands, and `src/main.rs` does not grow

#### US-034: Add an opt-in exit code for passing runs that carry warnings
**Description:** As a CI owner on GitLab, I want a passing run with warnings to exit with a code I choose, so that the pipeline shows "passed with warnings" while errors still block.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** Blocked by US-013
**Audit items:** 70 (Z7-N01, Z3-N09)
**React Doctor reference:** `6ff671033` (#1900), `packages/react-doctor/src/cli/utils/resolve-scan-exit-code.ts:21` (the warning code applies when the gate passed), `packages/react-doctor/src/cli/utils/resolve-scan-exit-code.ts:15` (`--blocking none` still exits 0), react-doctor's `warning-exit-code.md:3` under `docs/`.

**Acceptance Criteria:**
- [ ] `--warning-exit-code <3..=125>` is parsed with a clap range, since 1 and 2 are taken and 126 and above belong to the shell and to signals
- [ ] When the exit code would be 0, the blocking level is not `none`, and at least one diagnostic would block under `--blocking warning` (with the same weighting, and only introduced findings in a baseline scope), the run exits with that code
- [ ] The precedence is 2, then 1, then the warning code, then 0
- [ ] Given a warning-only fixture, when scanned with the flag, then it exits with the code; given an error fixture, it exits 1; given `--blocking none`, it exits 0; given a baseline run with only pre-existing warnings, it exits 0
- [ ] The Action gains a `warning-exit-code` input, and the full exit-status table appears in `--help` and in `SKILL.md`
- [ ] No schema change: the code derives from fields the report already has

#### US-035: Tell an agent when its skill is missing or stale
**Description:** As an agent running a scan, I want to be told when my rust-doctor skill is absent or was written by another version, so that I never follow instructions for flags the binary no longer has.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 74 (Z7-N09)
**React Doctor reference:** `packages/react-doctor/src/cli/utils/prompt-install-setup.ts:70`, `packages/react-doctor/src/cli/utils/prompt-install-setup.ts:76` (a line addressed to the agent when the tool is not set up), `packages/react-doctor/src/cli/commands/inspect.ts:772`.

**Acceptance Criteria:**
- [ ] After a non-JSON report, when `CLAUDECODE`, `CODEX_SANDBOX` or `CURSOR_AGENT` is set, the scan reads that agent's `SKILL.md` under the workspace through the existing skill targets
- [ ] Absent: one stderr line names `rust-doctor skill install --agent <agent>`. Stamped with another version (`src/skill.rs:140`): one stderr line names `--update` and both versions.
- [ ] Nothing is printed under `--json`, and nothing is written
- [ ] The TUI handoff prompt carries the install line when the target agent's skill directory is absent
- [ ] Tests through the binary with `CLAUDECODE=1` cover absent, stale and current

#### US-036: `setup`: detect installed agents and install the loop in one command
**Description:** As a vibecoder, I want one command that installs the skill and the hooks for the agents I actually have, so that the loop is set up without five commands and without files for agents I never use.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-007
**Audit items:** 100 (Z7-N10)
**React Doctor reference:** `packages/react-doctor/src/cli/index.ts:338` (`install`, alias `setup`), `packages/react-doctor/src/cli/utils/detect-agents.ts:12` (agents detected from PATH binaries and config directories), `packages/react-doctor/src/cli/utils/install-react-doctor.ts:499` (refusal when none is found).

**Acceptance Criteria:**
- [ ] `skill install` gains `--agent detected`: the agents whose CLI `handoff::available_agents()` finds, unioned with those whose project directory exists (`.claude/`, `.agents/` or `.codex/`, `.cursor/`)
- [ ] `rust-doctor setup [PATH] [--dry-run]` lives in its own module and plans the detected skills, `hook install git`, and the Stop hook for a detected Claude Code or Cursor. It prints the whole plan, checks every target before writing anything, and reuses the existing installers and their refusal rules.
- [ ] `setup` never runs `ci install`, and it says so in its last line, naming the command
- [ ] Given no agent detected, when `setup` runs, then it exits 2 and lists what it probed
- [ ] Given a `PATH` holding only a fake `codex`, when `setup` runs, then only `.agents/skills` (or the Codex target the skill table names) and the git hook are written
- [ ] Given `--dry-run`, when `setup` runs, then nothing is written
- [ ] `README.md` shows `setup` as the second step, and `SKILL.md` and `tests/skill_contract.rs` gain it

#### US-037: Smoke-test the packed launcher through npm, the path users take
**Description:** As the maintainer, I want CI to install the packed tarballs with npm as `npx` and the Action do, so that npm's global layout and its optional-dependency selection are exercised before a user meets them.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 101 (Z9-N09)
**React Doctor reference:** `scripts/smoke-packed-cli-install.ts:162` (the packed tarballs are installed with npm), `.github/workflows/react-doctor.yml:53` (the dogfood runs the Action with a local package spec).

**Acceptance Criteria:**
- [ ] `npm/rust-doctor/scripts/smoke-packed.mjs` gains an npm leg: a scratch prefix with `npm_config_cache` inside it, then `npm install -g --prefix <scratch> --ignore-scripts --offline` of the native and wrapper tarballs, with the exact-version optional dependency resolved to the local tarball
- [ ] The leg runs `<scratch>/bin/rust-doctor --version` and a `--json` scan, with the bun leg's assertions: no registry access in stderr, the native binary resolved inside the prefix
- [ ] Given npm refuses the local override offline, when the leg runs, then it fails with npm's message, and the story records the finding and the workaround chosen
- [ ] The leg never reaches the network, and the packed CI job declares the npm it uses

**Finding (2026-10-14, npm 10.9.8):** npm applies `overrides` only from a root project, never to a global install, and `--offline` cannot resolve the wrapper's exact-version optional dependencies from a registry: it skips them as optional, silently, exit 0. **Workaround chosen:** the leg passes the native tarball and the wrapper tarball to one `npm install -g`, so the native lands as a sibling global package under `<prefix>/lib/node_modules/@rustdoctor/`, which the launcher's `createRequire` resolves, the same layout a registry install produces. The leg asserts that resolution stays inside the prefix.

---

### EP-006: The rules Rust services need

**Release:** 3 (0.13.0). The defects where Rust services break in production, which no Clippy lint covers, become catalogued rules with trigger evidence: an unchecked HTTP status, a secret that fails open, a tokio call that panics, permissive web configuration, CI that compiles with secrets in scope, and lockfile and credential hygiene. They come with a version gate and with the metadata an agent needs to judge each finding.

**React Doctor reference:**
- Capability and version gates on packs: `packages/core/src/project-info/capabilities.ts:87`, `packages/core/src/runners/oxlint/config.ts:203`.
- `no-fetch-response-used-without-status-check`: `4899bc491` (#1877).
- `secret-in-fallback`: `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/secret-in-fallback.ts:30`.
- Web hardening scans bounded by context: `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/cors-cookie-trust-risk.ts:19`.
- The build pipeline secret boundary: `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/build-pipeline-secret-boundary.ts:24`.
- A title and a false-positive check on every rule: `packages/oxlint-plugin-react-doctor/src/plugin/utils/rule.ts:30`, `packages/react-doctor/src/cli/utils/diagnostic-grouping.ts:96`.

**Definition of Done:**
- Every new rule is catalogued with a trigger record, a corpus measurement or an explicit `unproven`, a variant table, and the README count.
- Every native rule matching a non-standard crate declares a `requires` gate.
- `rules list --json` publishes a title and a not-a-defect check for all rules.

#### US-038: Gate crate rules on each member's resolved dependency version
**Description:** As the catalog maintainer, I want a rule to declare the crate versions it was validated against, so that a rule written for one API never fires, or misses silently, on another.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 28 (Z1b-N01)
**React Doctor reference:** `packages/core/src/project-info/capabilities.ts:87` (capabilities projected from manifest facts), `packages/core/src/project-info/capabilities.ts:236` (no version token past the last validated major), `packages/core/src/runners/oxlint/config.ts:203` (a framework rule without a gate is never enabled), `packages/oxlint-plugin-react-doctor/src/plugin/utils/mobx-rule-gates.test.ts:14`.

**Acceptance Criteria:**
- [ ] `RuleDefinition` gains `requires: &'static [CrateGate]`, with `CrateGate { krate, versions }` parsed once with `cargo_metadata::semver::VersionReq`. `validate_catalog` refuses an unparseable range. Amended 2026-10-19: a gate holds when at least one listed crate reaches the unit and every listed crate that reaches it is in range, since US-042's CORS rule needs tower-http or actix-cors, not both.
- [ ] The version is evaluated per source unit: the lowest, over the packages reaching the unit, of the `Cargo.lock` entry their direct dependency resolves to (`src/cargo_health/resolution.rs:345`). It falls back to the requirement's lower bound, and is unknown for `*` or an unparseable requirement.
- [ ] A gated rule's detectors skip a unit whose gate fails
- [ ] `CatalogEntry` publishes `requires`, and `NotEvaluated` gains `crate-absent` and `crate-version-unsupported`, neither of which drops `score.authoritative`, under `SCHEMA_VERSION` 23
- [ ] A catalog test fails when a native rule whose detector matches a non-sysroot crate declares no gate on that crate
- [ ] Given a workspace pinning a version outside a rule's range, when scanned, then the rule is published as `crate-version-unsupported` and reports nothing
- [ ] The website team is told the new `requires` field exists (rust-doctor.com/rules generates from `rules list --json`). Amended 2026-10-19: rust-doctor-web is Arthur's own repository, so the telling is step 8 of `.claude/skills/release`, which names the four new descriptor fields the rule pages must type and render.

#### US-039: tokio rules: `block_in_place` on a current-thread runtime, `block_on` inside async
**Description:** As a developer of a tokio service, I want the two tokio calls that panic at run time reported where tokio documents they panic, so that the panic is found in review instead of in a test run or in production.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-038
**Audit items:** 15 (Z1b-N02)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/rules/tanstack-query/query-stable-query-client.ts:43` (a rule gated on the stack it targets), `packages/core/src/project-info/capabilities.ts:121`, `packages/oxlint-plugin-react-doctor/src/plugin/rules/server/server-sequential-independent-await.ts:194` (react-doctor transposes runtime defects, not concurrency hints).

**Acceptance Criteria:**
- [ ] `rust_doctor::source::block_in_place_on_current_thread` (category `correctness`, tier `P1`, `requires` tokio `>=1, <2`) reports a `tokio::task::block_in_place` call, resolved through the alias map, in the body of a function attributed `#[tokio::test]` without `flavor = "multi_thread"`, or `#[tokio::main(flavor = "current_thread")]`
- [ ] A `block_in_place` outside such a function is not reported, because tokio runs the closure normally outside a runtime
- [ ] `blocking_call_in_async` stops exempting a `block_in_place` closure when the enclosing function is current-thread
- [ ] `rust_doctor::source::block_on_in_async` (category `reliability`, tier `P2`) reports `tokio::runtime::Runtime::block_on`, `tokio::runtime::Handle::block_on` or `futures::executor::block_on` inside an `async fn` or `async` block, using the async-context test of `blocking_call_in_async`
- [ ] Given `#[tokio::test(flavor = "multi_thread")]` around `block_in_place`, when scanned, then nothing is reported
- [ ] Both rules land with fixtures on the existing offline tokio stub (`tests/fixtures/source-kernel/native-families/service/Cargo.toml:13`), variant tables, evidence records citing the tokio documentation, and corpus measurements

#### US-040: Report reqwest bodies read without an HTTP status check
**Description:** As a developer calling an HTTP API with reqwest, I want a body read from a response whose status was never checked reported, so that a 4xx or 5xx error payload is not parsed as data.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-038
**Audit items:** 13 (Z1a-N01)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/rules/correctness/no-fetch-response-used-without-status-check.ts:35` (fetch resolves on 4xx and 5xx), `packages/oxlint-plugin-react-doctor/src/plugin/rules/correctness/no-fetch-response-used-without-status-check.ts:23` (the body consumers), `4899bc491` (#1877, guard polarity), `packages/oxlint-plugin-react-doctor/src/plugin/rules/correctness/fetch-optional-status-guard.test.ts:20`.

**Acceptance Criteria:**
- [ ] `rust_doctor::source::unchecked_http_status` (category `reliability`, tier `P2`, `requires` reqwest) reports `json`, `text`, `bytes`, `chunk` or `bytes_stream` on a chain whose root resolves to `reqwest::get`, `reqwest::Client::{new, builder}` or their blocking forms, when no link of the chain is `error_for_status` or `error_for_status_ref`
- [ ] When the response is bound with `let`, the rule abstains if the enclosing body reads `.status()`, `.is_success()` or `error_for_status` on that binding before the body read
- [ ] It abstains on an Indeterminate binding and on a response passed to another function
- [ ] Given `client.get(url).send().await?.error_for_status()?.json().await?`, when scanned, then nothing is reported
- [ ] Given `client.get(url).send().await?.json::<T>().await?`, when scanned, then it is reported with the help "Call .error_for_status()? before reading the body, or match on .status() and handle the error payload."
- [ ] The rule lands with fixtures that only parse (never compile reqwest), a variant table, an evidence record and a corpus measurement

#### US-041: Report secret-named environment variables that fall back to a literal
**Description:** As a developer, I want `env::var("JWT_SECRET").unwrap_or("dev-key")` reported, so that a committed secret that also makes the service fail open is caught at the call that creates both defects.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 14 (Z1a-N02)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/secret-in-fallback.ts:30` (the name gate and the fallback pattern), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/secret-in-fallback.ts:38` (fail closed), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/secret-in-fallback.regressions.test.ts:84`.

**Acceptance Criteria:**
- [ ] `rust_doctor::source::secret_env_fallback` (category `security`, tier `P1`) reports `std::env::var` or `var_os`, resolved through the alias map, whose literal argument passes the name gate, followed by `unwrap_or(<literal>)`, `unwrap_or_else` with a closure returning a literal, `unwrap_or_default()` or `.ok().unwrap_or(..)`
- [ ] The name gate accepts names containing `SECRET`, `TOKEN`, `PASSWORD`, `PRIVATE_KEY`, `API_KEY` or `SIGNING_KEY`, and excludes names ending in `_URL`, `_HEADER`, `_NAME`, `_ID` or `_ENDPOINT`
- [ ] The value gate requires a literal of at least 8 characters and vetoes placeholders (`changeme`, `your_`, `example`, `xxx`, a run of zeros)
- [ ] `unwrap_or_default()` is reported with its own message: an empty secret that fails open
- [ ] Given `env::var("DATABASE_URL").unwrap_or("postgres://localhost/dev".into())`, when scanned, then nothing is reported; inside test code nothing is reported
- [ ] The name and value vocabularies live in one module with unit tests, and the rule lands with its evidence record and corpus measurement

#### US-042: Web hardening rules: permissive CORS, insecure auth cookies, disabled JWT validation
**Description:** As a developer of an axum or actix service, I want the configuration calls that switch off a web security control reported, so that a permissive CORS layer, a readable session cookie or an unverified token does not ship.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** Blocked by US-038
**Audit items:** 17 (Z1a-N06)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/cors-cookie-trust-risk.ts:19` (credentialed CORS with a wildcard origin), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/insecure-session-cookie.ts:10` (bare `token` deliberately excluded for CSRF cookies), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/jwt-insecure-verification.ts:11`.

**Acceptance Criteria:**
- [ ] `rust_doctor::source::permissive_cors` (security, P1, `requires` tower-http or actix-cors) reports `CorsLayer::very_permissive()`, `allow_credentials(true)` in a chain that also allows `AllowOrigin::mirror_request()` or `Any`, and `actix_cors::Cors::permissive()`
- [ ] `rust_doctor::source::insecure_auth_cookie` (security, P1, `requires` cookie) reports `http_only(false)` on a cookie builder whose literal name matches `session`, `auth`, `jwt` or `access_token`, and never on a bare `token` or a CSRF cookie name
- [ ] `rust_doctor::source::disabled_jwt_validation` (security, P0, `requires` jsonwebtoken) reports `insecure_disable_signature_validation` for versions below 10 and `jsonwebtoken::dangerous::insecure_decode` for 10 and above
- [ ] Each rule reads booleans through `Context::value` like `disabled_tls_verification` (`src/source_kernel/detectors.rs:324`) and is silent in test code
- [ ] Given `CorsLayer::permissive()` without credentials, or `http_only(false)` on a cookie named `theme`, when scanned, then nothing is reported
- [ ] Every rule sits in the security P0 to P1 window of `TIER_WINDOWS` and lands with fixtures, variant tables, evidence records citing the crates' documentation, and corpus measurements

#### US-043: Report a binary's ignored `Cargo.lock` and a tracked Cargo credentials file
**Description:** As a developer, I want a lockfile that git ignores and a committed registry token reported, so that CI does not resolve versions nobody tested and a crates.io token does not live in history.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None
**Audit items:** 20, 24 (Z4-N13, Z4-N14)
**React Doctor reference:** `packages/core/src/checks/expo/check-gitignore.ts:13`, `packages/core/src/checks/expo/check-gitignore.ts:15` (files that must be committed but are ignored, abstaining when undetermined), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/utils/is-repository-secret-file-path.ts:5` (the package manager's credential file is a secret file).

**Acceptance Criteria:**
- [ ] When `Cargo.lock` exists at the workspace root of a workspace producing a binary, is absent from `git ls-files`, and `git check-ignore` answers ignored, the repo pass reports `rust_doctor::repo::ignored_lockfile`, reusing `probe_ignore` (`src/repo_hygiene.rs:413`). Amended 2026-10-19: a workspace nested below the repository root abstains, since the ignore file there is the enclosing repository's and an ignored nested lockfile is its choice, such as a fixture's.
- [ ] Given no git, or an undetermined ignore status, when scanned, then the rule abstains
- [ ] `.cargo/credentials.toml` and `.cargo/credentials`, matched as paths, join the secret file list (`src/repo_hygiene.rs:221`), gated on a `token = "..."` line under `[registry]` or `[registries.*]`
- [ ] Given an empty credentials template, when scanned, then nothing is reported
- [ ] Both land with fixtures, the closed-list test update, an evidence record for the new id and corpus measurements

#### US-044: Report CI steps that compile the workspace with secrets in scope
**Description:** As the engineer owning CI, I want a step that runs `cargo build`, `test` or `clippy` while a secret is in its scope reported, so that a dependency's `build.rs` cannot read a deploy token.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** None
**Audit items:** 18 (Z1a-N09, Z4-N10)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/build-pipeline-secret-boundary.ts:24` (a secret is injected only into the step that references it), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/build-pipeline-secret-boundary.ts:79` (an install with lifecycle scripts disabled is exempt), `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/build-pipeline-secret-boundary.ts:14`.

**Acceptance Criteria:**
- [ ] `rust_doctor::repo::secret_in_build_step` (security, P1) reads each tracked `.github/workflows/*.yml` under the existing size bound with an indentation walk, without a YAML dependency
- [ ] It reports a step whose `run` invokes `cargo build`, `test`, `check`, `clippy`, `run`, `bench`, `doc`, `install`, or `publish` without `--no-verify`, when the step, job or workflow `env` references `secrets.` or a `CARGO_REGISTRY_TOKEN` or `CARGO_REGISTRIES_*_TOKEN`
- [ ] The span is the `run` line, and the message names the secret's scope (step, job or workflow)
- [ ] The rule abstains on YAML anchors and on reusable-workflow calls
- [ ] Given this repository's `release.yml`, when scanned, then the finding is fixed in the same change by verifying in a step without the token and publishing with `--no-verify` in the token step, so the self-scan gate stays green
- [ ] The rule lands with fixtures, an evidence record and a corpus measurement

#### US-045: Publish a title and a not-a-defect check with every rule
**Description:** As an agent triaging a finding, I want each rule to state in a few words what it catches and when a finding is not a defect, so that I check the false-positive case before editing code.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None
**Audit items:** 30 (Z1a-N08)
**React Doctor reference:** `packages/oxlint-plugin-react-doctor/src/plugin/utils/rule.ts:30` (a short headline per rule), `packages/oxlint-plugin-react-doctor/src/plugin/utils/rule.ts:109` (`recommendationFor`), `packages/react-doctor/src/cli/utils/diagnostic-grouping.ts:96` (the agent is told to follow the false-positive check before fixing), `packages/core/src/runners/oxlint/validate-rule-registration.ts:32`.

**Acceptance Criteria:**
- [ ] `RuleDefinition` gains `title` (at most 8 words, seeded from the `catches` lines of `tests/rule_evidence.json`) and `not_a_defect_when` (one sentence)
- [ ] `validate_catalog` rejects an empty title or check, and a title longer than 8 words
- [ ] All 133 existing rules and every Release 3 rule carry both, reviewed by Arthur before the release. Amended 2026-10-19: Arthur delegated the review to the EP-006 review, which checked the 142 texts against Clippy 1.97.1 and rewrote the nine that described a case the lint does not report.
- [ ] `rules list --json` publishes both. The report embeds neither, so `SCHEMA_VERSION` does not move for this story.
- [ ] The handoff prompt prints the check under each rule's help, and the skill tells the agent to apply it before fixing
- [ ] Given `disabled_tls_verification`, when listed, then its check reads, in substance, "a test against a local server with a self-signed certificate"

---

## Functional Requirements

- FR-01: The system must NOT print, serialize or embed in a prompt the value of any credential a rule matched, on any surface.
- FR-02: Under `--scope lines`, a diagnostic with a path and no span must be in scope exactly when its file has a changed range.
- FR-03: Under any narrowed scope, a clone family must be kept when any of its members meets the scope, with that member as primary.
- FR-04: `--scope contextual` must keep a finding of an `Item` rule when its innermost enclosing function holds a changed range or a deletion point, and must never widen `Line` rules.
- FR-05: Discovery must never select a workspace outside the requested directory, and must never walk above the first directory holding `.git`.
- FR-06: `hook run cursor` must not scan when `status` is present and not `completed`, or when `loop_count` reaches the cap.
- FR-07: The installed git hook must exit 0 with a message when the binary is not on PATH, and must keep the gate's exit code when it is.
- FR-08: The Action must exit 0 on a non-`pull_request` event whose scan exited 1, and must fail on exit 2 for every event.
- FR-09: `render_markdown` must render every entry of `errors[]`, and must never print "No finding." for a failed scan.
- FR-10: Every non-local `uses:` the project ships must be pinned to a 40-hex SHA, and every checkout must set `persist-credentials: false`.
- FR-11: The count-up animation and line rewrites must not run when `--yes`, `--json`, any agent, hook or CI marker, or a non-terminal stream applies.
- FR-12: A native detector that cannot resolve a name must abstain, except where this PRD defines positive proof (glob-masked crate heads, typed numeric bindings, single same-module helpers).
- FR-13: Every confirmed false positive of a native rule must have a seed or a written reason, and every adjudication pass must cite the lines that decided it.
- FR-14: `--json` must be byte-identical with and without the overlap of US-023 and the cache of US-024 on the same inputs.
- FR-15: The side cache must store only complete base sides, and must treat any decode or identity mismatch as a miss.
- FR-16: No child process may outlive the run deadline or a handled SIGINT or SIGTERM.
- FR-17: The delta must pair across files only along a Git rename.
- FR-18: The baseline gate and the full gate must use the same weighting predicate.
- FR-19: `rules explain`, `why`, `setup --dry-run` and `rules list` must write nothing, and `rules explain` and `rules list` must spawn no process.
- FR-20: A rule matching a non-standard crate must declare a version gate, and must report nothing for a unit whose gate fails.
- FR-21: The binary must not open any network connection in any command this PRD adds or changes.

## Non-Functional Requirements

- **Performance:**
  - A warm full scan of this repository takes at most max(Clippy phase, native phases) plus 10% after US-023, measured by the US-022 harness on the same host.
  - A second baseline run on an unchanged base spawns 0 Cargo processes for the base side.
  - `--scope contextual` adds at most 15% to a warm `--scope lines` run on this repository.
  - The variant gate and the seed replay together finish within 10 seconds inside `cargo test`.
  - US-001 masking adds at most 1 ms per rendered frame.
- **Security:**
  - 0 credential bytes in any rendered surface, asserted per surface by tests.
  - 0 outbound connections from the binary.
  - 0 unpinned third-party actions in shipped YAML.
  - 0 checkouts holding persisted credentials.
  - YAML-bound inputs validated against `[A-Za-z0-9._/-]+` with no `..`.
- **Reliability:**
  - Every new failure mode produces a complete report with a `ReportError`.
  - Exit codes stay 0, 1 and 2, plus 130 on a handled signal and the opt-in warning code in 3 to 125.
  - A killed or interrupted scan leaves 0 processes of its groups alive and 0 snapshot directories in `$TMPDIR`.
- **Scalability:**
  - Discovery descends at most 4 levels and lists at most 10 candidates.
  - The side cache holds at most 20 entries.
  - The hunk map reads at most 1 MiB of diff, beyond which pairing falls back to line distance with a notice.
  - Workspaces of up to 200 members and 10,000 source files scan without a new limit being reached.
- **Maintainability:**
  - Every file stays under 1,000 lines and every `impl` under 500.
  - `src/main.rs` ends Release 1 with fewer lines than 999.
  - Every new document is registered in `docs/doc-budgets.json`.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Credential shorter than the prefix | Matched value under 8 characters | Whole value masked | `<redacted>` in the frame |
| 2 | Inherited dependency | `serde = { workspace = true }` | No span, file-level scope rule applies | None |
| 3 | Unparseable file under contextual | Item rule finding in a file with parse errors | Line filtering for that finding | None |
| 4 | Several nested workspaces | `src-tauri/` and `backend/` both hold workspaces | Exit 2, `ambiguous-workspace` | "Several Cargo workspaces under this path: src-tauri, backend. Pass one of them." |
| 5 | Workspace above an unrelated repository | Path inside a git repository under an outer `Cargo.toml` | Outer workspace never chosen | "no Cargo.toml found in this path or its ancestors" |
| 6 | Aborted Cursor turn | `{"status":"aborted"}` | No scan, empty stdout, exit 0 | None |
| 7 | Hook without binary | Teammate without rust-doctor on PATH | Commit proceeds | "rust-doctor is not on PATH: this commit was not judged." |
| 8 | Reinstall over a foreign hook | `--update` on a hook without the marker | Nothing written, exit 2 | "This hook was not written by rust-doctor: nothing was written." |
| 9 | Push with legacy findings | `push` event, scan exit 1 | Job passes, summary still rendered | Score and findings in the job summary |
| 10 | Failed scan on a pull request | Scope or preparation failure | Comment lists stage, code and message | "No finding was collected." plus the error section |
| 11 | Shallow pull request checkout | `fetch-depth: 1` | Base fetched; if the fetch fails, `shallow-clone` with remedy | "This clone is shallow: fetch with fetch-depth: 0, or pass --base to a fetched ref." |
| 12 | Clean pull request, opted out | `comment-on-clean: false`, no prior comment | No comment created | None |
| 13 | Glob from the crate itself | `use crate::prelude::*;` | Detector abstains as today | None |
| 14 | Recursive or ambiguous helper | Two `fn load` in scope, or recursion | Abstain or terminate without finding | None |
| 15 | Unknown seed directive | `// verdict: safe` | Test fails naming the seed | "regressions/x--y.rs: unknown verdict `safe` (expected quiet or fires)" |
| 16 | Corrupt side cache | Truncated `base-sides.json` | Miss, rewrite on store | None (`--verbose`: "Base side cache: miss") |
| 17 | Second Ctrl-C | Signal during cleanup | Immediate exit | None |
| 18 | Clippy lock held by rust-analyzer | `Blocking waiting for file lock` until the deadline | `deadline-exceeded` naming the lock | "Cargo waited on the build directory lock held by another Cargo or rust-analyzer." |
| 19 | Same-file swap inside one hunk | One finding fixed and one added in the same hunk and rule | Paired, not reported (accepted blind spot, documented) | None |
| 20 | No agent detected | `setup` with no agent CLI and no agent directory | Exit 2, nothing written | "No supported coding agent found (probed: claude, codex, cursor-agent, .claude/, .agents/, .codex/, .cursor/)." |
| 21 | Unsupported crate version | Gated rule, locked version outside its range | Rule published `crate-version-unsupported`, no finding | One line under `--verbose` |
| 22 | Oversized diff for the hunk map | Diff over 1 MiB | Pairing falls back to line distance | "The diff is too large for hunk pairing: pairing by line distance." |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | The masked frame hides context a user needs to locate the key | Low | Low | The first four characters and the exact span stay visible, as react-doctor keeps the type prefix |
| 2 | `--scope contextual` traps agents on legacy findings in touched functions | Medium | High | Opt-in only. The hooks stay on `lines`, and US-004 measures the extra findings before any default changes. |
| 3 | The side cache serves a stale base when a build script reads an untracked input | Low | High | Base sides are checked out trees. The key holds the toolchain, policy and Cargo environment variables, and `RUST_DOCTOR_NO_CACHE=1` disables it. The US-024 test proves byte identity. |
| 4 | Parallel passes reorder or race on output | Low | High | Normalization sorts, and every oracle fixture pins `--json` byte for byte before and after |
| 5 | Signal handling misbehaves across platforms | Medium | Medium | Unix only, async-signal-safe handler (flag and self-pipe), Windows unchanged and documented |
| 6 | Hunk pairing hides a new finding | Medium | High | US-028 measures hidden swaps on real history before US-029 is built, and the blind spot is documented |
| 7 | New security rules duplicate CodeQL and lose on precision | Medium | Medium | Each rule ships with a corpus measurement or `unproven`, a variant table and seeds, and the noise rate is published |
| 8 | The 133 title and check sentences drift into boilerplate | Medium | Low | `validate_catalog` bounds the length, and Arthur reviews them before Release 3 |
| 9 | Pinned SHAs in users' generated workflows age | High | Low | `ci install --update` refreshes them, and a bumpable Action ref is planned in the distribution PRD |
| 10 | Scope creep across 45 stories | High | Medium | Three releases, each shippable alone. The Disposition table fixes what is out. |
| 11 | `src/main.rs` exceeds its bound mid-epic | High | Medium | US-013 lands first in Release 1, and every new subcommand lives in its own module |
| 12 | The website republishes `rules list --json` and breaks on the new fields | Medium | Medium | Fields are additive. The release checklist updates rust-doctor-web with Release 1 (`diff_scope`) and Release 3 (`requires`, `title`, `not_a_defect_when`). |

## Non-Goals

- **Telemetry, crash reporting, a remote score API or any network lookup from the binary.** Excluded by `AGENTS.md`, permanently. The base fetch of US-010 and the comment post run in the workflow's shell, never in the binary.
- **Saved-report baselines (`--baseline <report.json>`).** They need published fingerprints, a source revision and a policy digest, so the schema would grow for good. US-024 obtains most of the cost cut with no schema change. Revisited in the report, scope and configuration PRD.
- **Switching the hooks to `--scope contextual`.** Revisited after one release of US-004's measurement.
- **A current-side result cache.** Build scripts and procedural macros read inputs git cannot see. Revisited once US-022 measures a warm Cargo replay.
- **A pure count-based delta.** It hides a same-file swap. US-028 may reopen it with evidence.
- **Inline review comments, commit statuses, GitLab and other providers.** The distribution PRD owns them.
- **Every item marked Later or "To measure" in the Gap Audit Disposition table.** That includes the opt-in tier and rule retirement, macro reparsing, control flow, cross-module resolution, the published JSON schema, and the TUI polish items.

## Files NOT to Modify

- `src/audit/density.rs` and `src/audit/tests/lambda_freeze.rs`: the core-v4 score model and its calibration. No story changes the formula.
- `src/policy/catalog/validate.rs` `TIER_WINDOWS`: new rules fit the existing windows.
- `src/git.rs` `GIT_ENVIRONMENT_OVERRIDES`: append only. US-025 and US-029 pass the deadline and new arguments through the existing bounded layer.
- `tests/corpus.json`: regenerated from the clone cache only, as `docs/catalog-and-corpus.md` prescribes, never edited by hand. US-021 adds evidence fields through the regeneration path.
- The frozen v7 archive projection in `src/report.rs` and `tests/fixtures/rule-scaling-kernel/v7-*.json`: they must keep projecting across schema 21, 22 and 23.
- Frozen precision records listed in `docs/doc-budgets.json`: new measurements (`docs/contextual-scope-2026-10.md`, `delta-pairing-2026-10.md` under `docs/`) go into new frozen files.
- The uncommitted EP-006 worktree is the base of Release 1. Its files are edited by EP-003 stories, never reverted.

## Technical Considerations

Framed as questions for engineering input:

- **Architecture, masking:** a mask range carried on the internal candidate and applied in `presentation::code_frame` is recommended over a catalog flag (`masks_span`), because only the repo pass knows the value's columns. Engineering to confirm the TUI reuses the same `CodeFrameLine` text, so one change covers both renderers.
- **Architecture, contextual scope:** finding the innermost `fn` needs a parse of the changed file. The recommendation is to reuse the source kernel's parsed units when the file is a reached unit, and to parse on demand otherwise. Does any reached-unit cache survive to projection time, or must projection parse again?
- **Architecture, overlap:** `ExecutionContext` must be `Sync` for `thread::scope`. Which of its members are not today, and is any of them interior-mutable?
- **Data model, schema 21 to 23:**
  - 21: `scope.mode: "contextual"`, `diff_scope` in `rules list --json`, and the `ambiguous-workspace` error code.
  - 22: `timings`, the delta's hunk-pairing counter and the `interrupted` code.
  - 23: `requires` and the `crate-absent` and `crate-version-unsupported` reasons.

  Should `diff_scope` also appear in `policy.rules[]` of the report? The recommendation is no: it is catalog data, published by `rules list`.
- **Dependencies:** `libc` as a normal dependency for US-026 is recommended over `signal-hook` or `ctrlc`, which would add crates to `Cargo.lock`. Engineering to confirm no feature flag of `libc` changes the locked build.
- **Delta:** if US-028 selects policy (c), US-029 changes to react-doctor's grouping, and `docs/subsystems/delta.md` records the accepted swap blind spot. Is the hunk map worth its second git call when the rename map already needs one? The recommendation is one `git diff --unified=0 --find-renames` that yields both.
- **Migration:**
  - `rust-doctor.toml` files stay valid.
  - Installed hooks keep their old body until `--update`. The 0.11.0 release note tells users to run `rust-doctor hook install git --update`.
  - Reports of schema 20 stay readable by `report markdown` through the existing version check.
  - Rollback is pinning the previous npm version.
- **Measurement:** US-004, US-015 to US-021, US-028 and every Release 3 rule need the corpus clone cache at `~/.cache/rust-doctor/corpus`. Engineering to confirm it is complete for all pinned repositories before Release 1 starts.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Rendered surfaces that print a detected credential | 2 (linear frame, viewer) | 0 | Month-1 (0.11.0) | Per-surface tests in `tests/repo_hygiene.rs` |
| Cargo declaration and lockfile rules reported under `--scope lines` when their file changed | 0 of 7 | 7 of 7 | Month-1 | `agent_loop` and `git_scope` tests |
| Problem 3 misfires reproducible | 7 | 0 | Month-1 | One regression test per misfire |
| Problem 4 shortfall cases neither firing nor carved | All of them | 0 | Month-1 | Variant gate in `cargo test` |
| Native false-positive verdicts with a seed or written reason | 0 of 147 | Every verdict on a rule Release 1 touches, then 147 of 147 | Month-1, then Month-6 | Seed directory against `tests/corpus.json` |
| Warm full-scan wall time on this repository | Sum of phases (measured by US-022) | At most max(Clippy, native) plus 10% | Month-6 (0.12.0) | `tests/performance.rs` |
| Base-side Cargo processes on a second unchanged baseline run | All members | 0 | Month-6 | US-024 integration test |
| False introductions per refactor-only commit pair | Measured by US-028 for policy (a) | At most the count US-028 records for the chosen policy | Month-6 | `delta-pairing-2026-10.md` under `docs/` replayed after US-029 |
| Catalogued rules with trigger evidence | 133 | 133 plus the Release 3 rules (about 10) | Month-6 (0.13.0) | README count and `rules list --json` |
| Rules publishing a title and a not-a-defect check | 0 of 133 | All | Month-6 | `validate_catalog` |

## Open Questions

- **Do the untagged parity releases ship first?** `Cargo.toml` is still at 0.7.0 although parity Releases 1 and 2 are DONE and EP-006 is IN_REVIEW. Arthur to decide before US-001 starts whether 0.8.0 to 0.10.0 are tagged separately or folded into 0.11.0. The release table and the schema numbers of this PRD depend on it.
- **Should EP-006 of the parity PRD be certified before or after EP-003 here?** Recommended: run EP-003 first and certify EP-006 against the amended criteria (US-016 of this PRD replaces parity US-029's "does not follow calls" criterion). Arthur to confirm before `/review-epic` on parity EP-006.
- **Resolved 2026-10-14: (d) ships.** Arthur chose (d) after reading the record; US-029's criteria were amended to it. **Which delta semantics ship? US-028's rule did not adopt (b).** On 125 commit pairs (b) removes 22 of the 29 false introductions (c) removes, 75.9 percent against the 80 required. (c) also adds 23 false introductions on module extractions without a Git rename; a fourth policy (d), evidence pairing then (c)'s count on the leftovers alone, removes all 29 and adds none. Neither hid a true introduction on this sample. Recommended: (d), with US-029's nearest-line ranking deciding which member of a group pairs, and its blind spot (a same-rule swap anywhere in one file) documented in place of (b)'s (a swap inside one hunk). Arthur to decide after reading `delta-pairing-2026-10.md` under `docs/`. US-029 waits on the answer.
- **Who writes and reviews the 133 title and check sentences of US-045?** Recommended: generated from `tests/rule_evidence.json` and the help text, then reviewed by Arthur in one pass. To be decided before Release 3.
- **Should the Action's commit status (react-doctor `action.yml:697`) come with US-008?** It needs `statuses: write`. Recommended: no, deferred to the distribution PRD. Arthur to confirm.
- **Is the Windows signal path worth `SetConsoleCtrlHandler` through `windows-sys`, which is already locked?** Recommended: defer until the Windows test leg is green (disposition item 116).
[/PRD]
