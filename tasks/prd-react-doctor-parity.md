[PRD]
# PRD: react-doctor Parity: a Score That Hides Nothing, an Agent Loop That Enforces Itself, and the Rule Families Clippy Cannot See

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-09-25 | Arthur Jean | Initial draft, from the parity audit of 2026-09-25 (98 verified gaps, 799 citations re-read) |
| 1.1 | 2026-09-25 | Arthur Jean | EP-001 review: US-003's test-target criterion amended to the default-target scope the scan already has |

## Problem Statement

rust-doctor is the Rust transposition of react-doctor (`/home/arthur/dev/react-doctor`, commit `83466a8fa`). On 2026-09-25, eleven audit zones compared both repositories, with every cited `file:line` re-read and every absence claim re-searched. The audit found 98 gaps once cross-zone duplicates were merged. The zone list is tracked in `TASKS.md`. Seven problems carry most of the weight.

1. **The tool reports less than `cargo clippy` does.** The Clippy invocation opens with `SILENCE_UNCATALOGUED` (`src/execution/clippy.rs:51`, `-A clippy::all`) and adds `-W` only for the 37 catalogued Clippy lints. On toolchain 1.97.1 that silences all 67 members of Clippy's deny-by-default `correctness` group (0 catalogued), 76 of 82 `suspicious` lints and 33 of 38 `perf` lints. The effect was reproduced on 2026-09-25 against a scratch file with `clippy-driver` 1.97.1: `if a == a` produces no output under `-A clippy::all`, an error under plain Clippy, and a warning under `-A clippy::all -W clippy::eq_op`. The barrier that keeps these lints out of the candidate queue (`src/policy/coverage.rs:173` "minus the ones the toolchain denies by default, which no scan can carry", filter at `src/policy/coverage.rs:179`) rests on a rejection reason written before `-A clippy::all` existed (`src/policy/rejected.json`, class `deny-by-default`: "dropping its -W restores Clippy's refusal"). With `-A clippy::all` first, a dropped `-W` now silences the lint instead. A workspace containing bugs Clippy calls certain gets a clean correctness dimension.

2. **One ordinary compiler warning voids the score, and nothing says why.** Any diagnostic whose code the catalog does not know is kept (`src/report/normalize.rs:34`, "An uncatalogued warning is kept, and costs the score its authoritative flag downstream") and flips `diagnostics_are_authoritative` (`src/audit.rs:819`). A single `unused_imports` in shipped code therefore turns the headline score "partial". The report publishes only `score.authoritative: bool` (`src/audit.rs:147`), and the terminal prints one sentence that conflates two causes (`src/render.rs:723`, "Score is partial because the scan did not complete or contains unscored findings."). In the pinned agent population, 2 of 8 reports (`ostendo`, `vibesql`) are non-authoritative, and neither says why.

3. **A scan that fails or hangs explains nothing.**
   - Cargo's stderr is discarded (`src/execution/clippy.rs:150`, `.stderr(Stdio::null())`), so a lockfile or registry failure publishes "Clippy exited with status 101" (`src/report/assembly.rs:480`).
   - Nothing bounds `cargo clippy` in time (`src/execution/clippy.rs:110` waits unconditionally). A hung `build.rs`, or a Cargo lock held by rust-analyzer, hangs the scan.
   - Clippy runs without `--keep-going` (`src/execution/clippy.rs:31`), so one broken member hides every other member's findings.
   - The Clippy child inherits the caller's `RUSTFLAGS` (`src/execution/clippy.rs:152` sets only `CARGO_TARGET_DIR`). A `-Dwarnings` exported by CI turns catalogued warnings into build failures.
   - `-W` is passed for every catalogued lint regardless of what the installed Clippy knows (`src/execution/clippy.rs:66`).

4. **The agent loop is manual.** rust-doctor is positioned for Rust written by coding agents, yet:
   - Every scope reads the working tree. No scan can judge what `git commit` is about to record (`src/git_scope.rs:57`).
   - The narrowest filter is the whole file (`src/git_scope.rs:257`).
   - `--base` is mandatory (`src/main.rs:457`), and the shipped skill hard-codes `--base main` (`skills/rust-doctor/SKILL.md:59`).
   - Interactive mode opens whenever stdin and stdout are terminals. That ignores agent, hook and CI-provider markers, and it treats `CI=false` as CI (`src/main.rs:482`).
   - The skill installs for Claude Code only and refuses to refresh itself after an upgrade (`src/skill.rs:19`, `src/skill.rs:64`).
   - No hook makes an agent rescan before it declares its work done.

5. **Precision controls are all or nothing.** `rust-doctor.toml` accepts `blocking`, `[categories]`, `[rules]` and two thresholds under `deny_unknown_fields` (`src/configuration.rs:29`). No path can be ignored and no rule can be relaxed for one directory. The 25 native `rust_doctor::*` rules cannot be silenced at a single site: "There is no suppression comment." (`skills/rust-doctor/SKILL.md:115`). Generated code is recognized only by the structure pass (`src/structure.rs:664`), and `.gitattributes` is never read. The workspace gets one score, with no per-member breakdown and no way to scan one member (`src/execution/clippy.rs:33`, `--workspace`).

6. **CI stops at a job log.** The only CI surface is a workflow the interactive report writes from a fixed template (`src/tui/workflow.rs:22`, `branches: [main]`). It tracks `dtolnay/rust-toolchain@stable` while the repository pins 1.97.1 because Clippy output is the product. It runs `--scope full` on push and exits with the scan status (`src/tui/workflow.rs:88`), so pre-existing errors can turn the default branch red. No Action, no pull request comment and no step outputs exist.

7. **The rule families where Rust services actually fail are missing.**
   - Blocking the executor from `async` code: the catalog covers only `await_holding_lock`, `await_holding_refcell_ref` and `unused_async`, and Clippy has no general lint ([clippy#10794](https://github.com/rust-lang/rust-clippy/issues/10794)).
   - SQL assembled with `format!`: no detector.
   - Undocumented `unsafe`: only `missing_safety_doc` (`src/policy/catalog.rs:208`).
   - Known-vulnerable crates in `Cargo.lock`: `src/cargo_health/resolution.rs:45` parses the lockfile and only checks it for missing entries.
   - The native detectors cannot follow a `let` binding (`src/source_kernel/aliases.rs:6`), so both shipped security detectors miss `let mut cmd = Command::new("sh")`.
   - Recall is not measured anywhere (`docs/suppression-precision-2026-08.md:224`).

**Why now:** v0.7.0 shipped the core-v4 score model, so the score is calibrated and published. Problem 1 undermines what that score claims: a user who runs plain `cargo clippy` next to rust-doctor sees defects the scan called clean. The agent tools also moved. Claude Code documents a blocking `Stop` hook with a loop guard (`stop_hook_active`, [hooks reference](https://code.claude.com/docs/en/hooks)), Cursor ships a `stop` hook ([Cursor hooks](https://cursor.com/docs/agent/hooks)) and Codex added hooks ([Codex config](https://developers.openai.com/codex/config-reference)). The enforcement loop react-doctor sells (`packages/react-doctor/src/cli/utils/install-agent-hooks.ts:48`) is now buildable for Rust with no agent-specific integration beyond a settings entry.

## Overview

This PRD is the whole parity program, phased into three releases so that each ships something coherent. The first release restores trust in the score and makes the agent loop work. The second adds precision controls and a CI integration that reports where reviewers look. The third adds the rule families Clippy cannot express, plus the kernel capability they need. The audit's other gaps each get an explicit destination in the Parity Audit Disposition table below, so the program has one written answer for every gap the audit verified.

| Release | Version | Epics | Stories | Theme |
|---------|---------|-------|---------|-------|
| 1 | 0.8.0 | EP-001, EP-002, EP-003 | US-001 to US-019 (19) | A score that hides nothing, a scan that fails loudly, an agent loop |
| 2 | 0.9.0 | EP-004, EP-005 | US-020 to US-027 (8) | Precision controls and pull-request-native CI |
| 3 | 0.10.0 | EP-006 | US-028 to US-033 (6) | Rule families Clippy cannot see |

Key decisions, each argued in Research Findings and Technical Considerations:

- **Admit Clippy's correctness group as one pack**, reusing the pack-oracle admission path `tests/rule_admission.rs` already accepts. The alternatives were sixty-five single-rule admissions or a new group-level catalog entry. The toolchain's deny-by-default classification is already a curation by Clippy's maintainers. This PRD re-measures it on the pinned corpus instead of trusting it.
- **Publish uncatalogued compiler diagnostics as a visible, unscored population.** They should neither void the score nor be silenced. The score gains a closed vocabulary of reasons for being non-authoritative.
- **Execution gets:**
  - a run deadline that kills Cargo's whole process group;
  - bounded, scrubbed stderr in every stage error;
  - `--keep-going`, with the unlinted packages named;
  - lint-level `RUSTFLAGS` tokens stripped, and only when present, so the target cache survives;
  - a probe of the installed Clippy's lint set, so an unknown lint is reported as not evaluated instead of silently skipped;
  - a baseline target directory that persists under Cargo's own target directory.
- **The agent loop adds** `--staged` (the index replaces the worktree), `--scope lines`, local default-branch detection, an agent- and hook-aware interactive check, multi-agent skill installation with an explicit `--update`, a git pre-commit hook installer, and an end-of-turn agent hook. By default the agent hook goes into the untracked `.claude/settings.local.json`.
- **CI adds** a non-interactive `ci install` and a `report markdown` command that renders a saved report without rescanning. On top of those sit a composite Action and a fork-safe comment job. The binary never reaches the network: the comment is posted by the workflow's token in a job that never checks out pull request code.

It adds no network access, no telemetry, no new dependency and no fix subcommand. `ra_ap_syntax`, `toml` (parse feature), `serde_json`, `clap` and `cargo_metadata` (which re-exports `semver`, `cargo_metadata-0.23.1/src/lib.rs:101`) carry every story.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Clippy `correctness` lints a scan reports where plain `cargo clippy` reports them (0 of 67 today on 1.97.1) | Every std-triggerable member, the count US-001 measures | 67 of 67 decided: admitted, or in the candidate queue with the external crate its trigger needs |
| Non-authoritative reports that publish a machine-readable reason (0 of 2 in the agent population today) | 2 of 2, and 0 reports voided by an uncatalogued warning alone | Every non-authoritative report in the corpus carries at least one reason |
| Failed scans (exit 2) whose error carries Cargo's own cause (0% today) | 100% of the failure fixtures in US-007 | 100% |
| A pre-commit gate that judges only what the commit records | `--staged` ships, with a hook installer for 4 hook setups | Documented in the skill, exercised by a CI test |
| Agents that receive the skill and an end-of-turn rescan | Claude Code skill and Stop hook; Codex and Cursor skill targets confirmed or explicitly excluded | Claude Code and Cursor hooks shipped |
| Pull request feedback without opening a log | n/a (Release 2) | Action with outputs and a sticky comment, dogfooded on this repository |
| New catalogued rule families with trigger evidence | n/a (Release 3) | Blocking in async, SQL from `format!`, vulnerable and unmaintained dependencies |

## Target Users

### Vibecoder shipping Rust with an agent
- **Role:** builds and ships Rust written mostly by Claude Code, Codex or Cursor, without a senior reviewer.
- **Behaviors:** runs `rust-doctor .`, reads the score, asks the agent to fix the top findings, ships when the score looks good.
- **Pain points:**
  - A clean score can hide `if a == a`, which plain `cargo clippy` would refuse.
  - One stray `unused_imports` turns the score "partial" with no explanation.
  - A failing scan prints a status code.
  - The agent forgets to rescan before saying it is done.
- **Current workaround:** runs `cargo clippy` separately, re-runs rust-doctor by hand, reads the agent's diff.
- **Success looks like:** the agent cannot finish a turn that introduced findings, and a scan that fails says what Cargo said.

### The coding agent itself
- **Role:** the process that edits the code and runs the tool (Claude Code, Codex, Cursor), usually without a real terminal, and sometimes under a PTY.
- **Behaviors:** follows `skills/rust-doctor/SKILL.md`, runs `rust-doctor . --json --scope baseline --base main`, parses the JSON.
- **Pain points:**
  - `--base main` fails on `master` and `trunk` repositories.
  - Under a PTY it may open a TUI it cannot drive.
  - An old installed skill documents flags the binary no longer has.
  - A baseline run recompiles every dependency each time.
- **Current workaround:** the user edits the command, or the agent retries with other flags.
- **Success looks like:** a flagless `rust-doctor . --json --scope lines` works in any repository, is never interactive under an agent, and returns a reason whenever the score is partial.

### Engineer owning CI and review
- **Role:** decides what gates a merge on a Rust repository, often reviewing agent-authored pull requests.
- **Behaviors:** runs Clippy with `-D warnings` in CI, reads job logs, adds `#[allow]` or `#[expect]` when a lint is wrong.
- **Pain points:**
  - The generated workflow can turn the default branch red on legacy findings, and it tracks an unpinned Clippy.
  - Findings live in a log nobody opens.
  - A noisy generated directory can only be silenced by turning the rule off everywhere.
- **Current workaround:** hand-written workflows, `cargo-deny` and `cargo-audit` as separate jobs, `grep` for `allow(` in diffs.
- **Success looks like:** one Action whose outputs feed branch protection, one sticky comment per pull request, and path-scoped exceptions that carry a reason.

## Research Findings

### Competitive Context
- **react-doctor** (the reference): 906 registered rules, 688 on by default, grouped Bugs 496, Performance 136, Maintainability 116, Accessibility 102, Security 56 (`packages/oxlint-plugin-react-doctor/src/plugin/core-rule-registry-data.json`). It ships `--staged`, `--scope lines`, default-base detection, agent Stop hooks, multi-agent skill installation, a composite Action with a sticky comment, a scan-result cache, a run deadline and a fuzz and evaluation harness. **How we differ:** rust-doctor keeps a local, reproducible score and adjudicated per-rule precision, which react-doctor does not publish. This PRD takes react-doctor's loop and integration surfaces and leaves its telemetry, its remote score API and its network lookups behind, all excluded by `AGENTS.md:38`.
- **cargo-audit and cargo-deny:** audit `Cargo.lock` against the RustSec advisory database, which they fetch and cache locally. On this machine, `~/.cargo/advisory-db` holds 1,158 advisories (258 unmaintained, 193 unsound, 6 notice, 13 withdrawn), last updated 2026-08-01, and `~/.cargo/advisory-dbs/` holds cargo-deny's copy. **How we differ:** we read a cache those tools already populated and never fetch, so the no-network invariant holds. The finding joins one score, one gate and one baseline.
- **clippy-sarif with reviewdog, and GitHub code scanning:** convert Clippy's JSON into pull request annotations. **How we differ:** our comment carries the score and the introduced and fixed counts that baseline scope already computes, not only line annotations.
- **Qlty CLI:** a multi-tool runner with changed-file scanning and vendored-path exclusion. **How we differ:** a single-purpose Rust scanner with a calibrated score, with the same `linguist-vendored` signal adopted.
- **dylint:** the escape hatch for lints Clippy refuses, blocking-in-async among them, at the cost of per-toolchain lint builds. **How we differ:** our native detectors parse with `ra_ap_syntax` and build nothing, which the trust boundary requires (`AGENTS.md:33`).
- **Market gap:** no Rust tool detects SQL built with `format!` or blocking calls in `async` code on stable without compiling custom lints. No Rust scanner ships an agent end-of-turn gate.

### Best Practices Applied
- **Pre-commit judges the index, not the worktree** (pre-commit-rust, lefthook, cargo-husky all run on staged content). `--staged` materializes the index the way `src/baseline.rs:213` already materializes a revision.
- **Changed-line filtering intersects diagnostic spans with `git diff -U0` hunks.** This is how Qlty and reviewdog filter, and how react-doctor does it (`packages/core/src/parse-changed-line-ranges.ts:7`).
- **Every suppression states a reason, and a stale suppression is reported.** Rust stabilized `#[expect(lint, reason = "...")]` in 1.81 with `unfulfilled_lint_expectations`, and ESLint and Ruff (`RUF100`) report unused directives. The native suppression directive of US-021 follows this shape.
- **Never install another project's hook, and never run privileged CI on untrusted code.** GitHub's guidance forbids `pull_request_target` combined with a checkout of pull request code ([Securely using pull_request_target](https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target)). A comment job therefore runs from `workflow_run` on an artifact.
- **Trailing command-line lint flags win over earlier ones, and in-source `#[allow]` wins over `-W`.** Verified on 1.97.1. It is the basis of the pack admission and of the `RUSTFLAGS` handling.

### Parity Audit Disposition

Every gap the 2026-09-25 audit verified, merged across zones, and where it goes. "Later" means a named follow-on PRD. "Won't" states the reason.

| Audit gap | Disposition |
|---|---|
| Z1-01 Clippy `correctness`/`suspicious` silenced by `-A clippy::all` | US-001, US-002, US-003; `suspicious` members enter the candidate queue through US-002 and follow the normal `rule-candidate` triage |
| Z1-05, Z3-03 rustc lints uncatalogued, uncatalogued warnings void the score | US-004 |
| Z3-05 no machine-readable reason for a partial score | US-005 |
| Z3-09 JSON report does not name the tool version | US-005 (tool version); schema publication Later |
| Z5-03, Z5-09 Cargo stderr discarded, `cargo metadata` read unbounded | US-007 |
| Z5-02 no deadline on Clippy or the run | US-008 |
| Z5-05 no `--keep-going` | US-009 |
| Z5-06, Z5-07 caller `RUSTFLAGS`, installed Clippy's lint set | US-010 |
| Z5-01 no cache between runs (worst on baseline) | US-011 for the baseline side; a whole-scan result cache is Later |
| Z8-01 no progress while scanning | US-012 |
| Z7-02 interactive check ignores agents, hooks and CI providers | US-013 |
| Z6-03, Z6-09 no default base, no shallow-clone diagnosis | US-014 |
| Z6-02, Z6-05 no `lines` scope, untracked files dropped | US-015 |
| Z6-01 no staged scan | US-016 |
| Z6-07 no git hook installer | US-017 |
| Z7-03, Z9-04 skill for Claude Code only, never refreshed | US-018 |
| Z7-04 no end-of-turn agent hook | US-019 |
| Z3-01 no path-scoped ignores or overrides | US-020 |
| Z3-02 no site-level suppression for native rules | US-021 |
| Z3-06, Z4-09, Z6-08 generated and vendored code not excluded | US-022 |
| Z3-08, Z7-07, Z6-04 no member selection, no per-member score, scope narrows the report but not the work | US-023 (member selection narrows Clippy's work); file-level work narrowing is Later |
| Z7-01, Z9-02 no `ci install`, generated workflow gates the backlog on an unpinned Clippy | US-024 |
| Z9-01, Z11-11 no reusable Action or pull request feedback; checkouts persist the token | US-025, US-026, US-027 |
| Z2-01 no local binding resolution | US-028 |
| Z1-04 no async-blocking family | US-029 |
| Z1-02 no injection and web-hardening family | US-030 covers SQL; JWT, CORS, weak crypto and path traversal are Later |
| Z4-01 no advisory check | US-031 |
| Z1-03 no unsafe and FFI soundness family | US-032 |
| Z10-02, Z10-05 no recall measurement, no fuzzing | US-033 (variant gate); full fuzzing is Later |
| Z1-06, Z4-04 secret detection limited to a closed prefix list | Later: secrets PRD (each pattern measured on the corpus before shipping) |
| Z1-07 Clippy `perf` group silenced | Later: `rule-candidate` triage of the queue US-002 reopens |
| Z1-08, Z1-09 swallowed errors, numeric casts | Later: both families need the opt-in tier (Z1-10) first |
| Z1-10 no opt-in rule tier | Later: catalog-tier PRD |
| Z1-11, Z10-06 no rule retirement lifecycle | Later: catalog-tier PRD (same mechanism: an id that resolves and produces nothing) |
| Z1-12 no crate-specific packs | Partly US-029 and US-030; tokio, axum and serde packs Later |
| Z1-13, Z1-14 thinner rule metadata, no binary-size family | Later |
| Z2-02, Z2-03, Z2-04, Z2-05 macros, control flow, cross-module resolution, single-kind detector contract | Later: source-kernel PRD, after US-029 proves the incremental approach |
| Z2-06 textual test classification | Later: source-kernel PRD |
| Z2-07, Z2-08, Z2-09 no detector authoring guide, hand-maintained registry, private harness | Later: contributor PRD |
| Z3-04, Z3-07 per-surface controls, audit mode through `#[allow]` | Later (Z3-07 requires a second `--force-warn` pass, rejected once on cost in `tasks/prd-suppression-dependency-hygiene.md`, US-004) |
| Z3-10, Z3-11, Z3-12 selector aliases, config discovery, rule options | Later |
| Z4-02, Z4-03, Z4-05 dev and build dependencies, Cargo source hardening, CI secrets boundary | Later: dependency-hygiene follow-on |
| Z4-06, Z4-07, Z4-08 block clones, multi-workspace discovery, unused cross-crate `pub` | Later |
| Z5-04, Z5-08, Z11-10 serial native passes, no performance harness, no phase timing | Later: performance PRD |
| Z6-06, Z6-10 explicit changed-file list, LFS and checkout filters | Later |
| Z7-05, Z7-06, Z8-11 `rules explain`, `why`, CLI config editing, human rule list | Later: CLI ergonomics PRD |
| Z7-08, Z8-05 handoff prompt lost on failure, no per-rule diagnostics directory | Later |
| Z7-09, Z8-02, Z8-03, Z8-04 `--score`, one finding by default, color flags, terminal width | Later: CLI ergonomics PRD |
| Z8-06 to Z8-10 Windows glyphs, highlighting, context labels, viewer polish, hyperlink detection | Later |
| Z9-03 lint-plugin or editor feed | Later |
| Z9-05, Z9-06 audit-then-plan skill, documented library API | Later |
| Z9-07, Z9-08 musl and Windows ARM binaries, preview builds | Later: release engineering |
| Z10-01, Z10-03, Z10-04, Z10-07 corpus size, base-versus-head diff, regression seeds, agent patch audit | Later: measurement PRD |
| Z11-01 to Z11-09 Windows tests, fmt gate, contributor hook, Clippy matrix, product-thinking pass, release authorization, packaged crate check, skills, error model | Later: engineering-system PRD (Z11-01 and Z11-02 are already recorded as deferrals in `docs/ci-workflows.md:10`) |

*Full research sources: the parity audit of 2026-09-25 (zone tracker `TASKS.md`); Claude Code [hooks](https://code.claude.com/docs/en/hooks), [settings](https://code.claude.com/docs/en/settings), [env vars](https://code.claude.com/docs/en/env-vars) and [skills](https://code.claude.com/docs/en/skills); [Cursor hooks](https://cursor.com/docs/agent/hooks); [Codex config](https://developers.openai.com/codex/config-reference) and [skills](https://developers.openai.com/codex/skills); [RustSec advisory-db](https://github.com/rustsec/advisory-db); [cargo-audit](https://github.com/rustsec/rustsec/blob/main/cargo-audit/README.md); [cargo-deny](https://github.com/EmbarkStudios/cargo-deny); [clippy#10794](https://github.com/rust-lang/rust-clippy/issues/10794); [RFC 3389](https://rust-lang.github.io/rfcs/3389-manifest-lint.html); the Cargo book (`--keep-going`, `CARGO_ENCODED_RUSTFLAGS`). Verified locally on 2026-09-25: the `eq_op` flag order on clippy 1.97.1, the group sizes from `clippy-driver -W help` on 1.97.1, `--keep-going` in `cargo check --help` on 1.97.1, `CLAUDECODE` set in Claude Code's Bash shells, and the RustSec front-matter layout in `~/.cargo/advisory-db`.*

## Assumptions & Constraints

### Assumptions (to validate)
- **HIGH: the correctness group adds almost no findings on healthy code and real findings on agent code.** Deny-by-default lints fail `cargo clippy` in any repository that runs it in CI, so healthy repositories should carry none. Agent repositories may. US-006 measures both populations before release. A member with a confirmed false positive on healthy code is demoted before the release, not after.
- **HIGH: most correctness members trigger with `std` alone.** Some (for example `invalid_regex`) need an external crate, which fixtures cannot fetch because tests never touch the network. US-001 counts them. If fewer than 50 of 67 are std-triggerable, the Month-1 goal is restated in the Changelog.
- **HIGH: a deny-by-default lint switched off by `--rule <id>=off` is allowed, not denied, under the current flag order.** Reproduced for `eq_op` on 1.97.1. US-001 repeats it on the declared minimum toolchain 1.95.
- **MEDIUM: the Codex and Cursor skill locations are stable and documented.** Claude Code's are documented ([skills](https://code.claude.com/docs/en/skills)). The shared `.agents/skills` directory has no official specification, although react-doctor writes it (`packages/react-doctor/tests/install-react-doctor.test.ts:491`) and it exists on this machine. US-019 ships only targets confirmed against official documentation at implementation time.
- **MEDIUM: a Stop hook blocking through exit code 2 with stderr is stable.** It is documented as the recommended form on the hooks page, and the JSON output form changed wording between fetches. US-020 uses exit code 2 only.
- **MEDIUM: persisting the baseline target under Cargo's target directory keeps registry dependencies fresh across runs, even though the snapshot tree moves.** Registry crates are fingerprinted by package id and source, not by the workspace path. US-012 measures it from Cargo's `fresh` flags.
- **LOW: `CLAUDECODE`, `CODEX_SANDBOX` and `CURSOR_AGENT` are the markers those agents set in child shells.** `CLAUDECODE=1` is documented ([env vars](https://code.claude.com/docs/en/env-vars)) and observed here. The other two come from react-doctor's list (`packages/react-doctor/src/cli/utils/is-ci-environment.ts:66`) and research, and are unconfirmed in official docs. A missing marker only means the TTY check still applies.

### Hard Constraints
- Single crate, edition 2024, `rust-version = "1.95"`, CI toolchain pinned to 1.97.1 in every workflow (`docs/ci-workflows.md:38`). No new workspace member and no new binary.
- **No new dependency.** Globs are matched by an in-crate matcher of under 150 lines. JSON settings files are read and written with `serde_json`. Advisories are parsed with `toml` (features `parse`, `serde`, `std`) and matched with `cargo_metadata::semver`. Workflow YAML is written from a template string.
- Dependencies stay pinned exactly and `Cargo.lock` stays committed.
- **No network, no telemetry, no upload** (`AGENTS.md:38`). The advisory database is only read from a local cache, never fetched. The pull request comment is posted by the workflow's token in a separate job, never by the binary.
- **`--json` stays workspace-relative:** no absolute path, no environment value, no home directory. This applies to every new field: stderr excerpts, the advisory database label, hook and skill paths.
- **Writes into the workspace happen only on an explicit command** (`skill install`, `hook install`, `ci install`), print exactly what they write, and never write over a file rust-doctor did not create. The one carve-out is the JSON merge of US-020, which adds one entry and keeps every other key. Scans themselves never write outside Cargo's target directory.
- Production code carries no `unwrap`, `expect`, `panic!` or `dbg!` (`tests/score_credibility_packs.rs`).
- Every file stays under 1000 lines and every `impl` under 500 (the fourteen `the_X_holds_the_size_bound` tests).
- Every new rule has a trigger record in `tests/rule_evidence.json` and sits inside its category's `TIER_WINDOWS` band (`src/policy/catalog/validate.rs:33`). `src/policy/catalog.rs` and `tests/corpus.json` are regenerated together, and the README rule count moves with the catalog.
- **`SCHEMA_VERSION` moves once per release:** 17 to 18 in Release 1 (first shape change: US-004), 18 to 19 in Release 2, 19 to 20 in Release 3. Every shape change inside a release lands under that release's number before its tag. The frozen v7 archive keeps projecting.
- Every test that runs `cargo` or the binary sets its own `CARGO_TARGET_DIR` through `support::scan_target`, and no test touches the network.
- `skills/rust-doctor/SKILL.md` is updated with every new flag and subcommand (`tests/skill_contract.rs` ties it to `--help`).
- No code copied from react-doctor, cargo-audit, cargo-deny or reviewdog. Published formats and documented behavior may be reimplemented.

## Quality Gates

These commands must pass for every user story:
- `cargo build --release` - the crate compiles
- `cargo test` - the full suite, including the catalog, admission, schema and size-bound invariants
- `cargo clippy --all-targets --no-deps -- -D warnings` - clean, no exceptions
- `bash scripts/verify-doc-budgets.sh` - every document within its word ceiling, and every new document registered
- `bash scripts/verify-doc-refs.sh` - every path the documents cite resolves
- `cd npm/rust-doctor && bun test tests` - the launcher still resolves and runs the binary
- `cargo run --release -- . --yes --verbose` - the tool scans its own repository without error

## Epics & User Stories

### EP-001: A score that hides nothing

**Release:** 1 (0.8.0). Stop silencing the defects Clippy calls certain, stop letting an uncatalogued warning void the score, and say why a score is partial whenever it is.

**Definition of Done:**
- A production-code `if a == a` lowers the correctness dimension and the score.
- A lone `unused_imports` leaves `score.authoritative` true.
- Every non-authoritative score in the corpus carries at least one reason from the closed vocabulary.
- `tests/corpus.json` is regenerated, and the score deltas are recorded in a frozen document.

#### US-001: Validate assumption: a deny-by-default lint can be carried and switched off
**Description:** As the catalog maintainer, I want the behavior of deny-by-default Clippy lints under the current flag order measured on the pinned and minimum toolchains, so that the barrier at `src/policy/coverage.rs:173` is retired on evidence, not on reasoning.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a scratch crate outside the repository containing `let a = 1; if a == a {}`, when it is checked with `cargo +1.97.1 clippy -- -A clippy::all -W clippy::eq_op`, then `clippy::eq_op` is reported at level `warning` and `build-finished.success` is true
- [ ] Given the same crate with `-A clippy::all` and no `-W clippy::eq_op`, when it is checked, then `eq_op` is not reported and the build succeeds
- [ ] Given toolchain 1.95, when both checks are repeated, then the observations match those on 1.97.1, or the difference is recorded as the minimum toolchain the pack of US-003 requires
- [ ] Given each of the 67 members of `clippy::correctness` on 1.97.1, when a minimal trigger is attempted with `std` only, then each member is classified as `std`, `needs-crate:<name>` or `untriggerable`, with the trigger source kept for US-003
- [ ] Given a member whose `std` trigger does not compile or does not fire on 1.95, when classified, then it is marked `toolchain-gated` and excluded from the Release 1 pack
- [ ] The classification and both flag-order observations are recorded in `docs/correctness-group-2026-09.md`, registered as `frozen` in `docs/doc-budgets.json`
- [ ] Given an observation that contradicts the first criterion (the lint stays an error), when the spike closes, then US-002 and US-003 are marked CANCELLED with an amendment quoting the measurement, and the Changelog records it

#### US-002: Reopen the candidate queue to deny-by-default lints
**Description:** As the catalog maintainer, I want deny-by-default Clippy lints to reach the candidate queue and the stale rejection reasons to be re-decided, so that the correctness group can be triaged like any other lint.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given the 1.97.1 lint universe, when `coverage::queue` builds the queue, then uncatalogued and unrejected deny-by-default lints appear in it, and the filter at `src/policy/coverage.rs:179` no longer tests `ToolchainLevel::Deny`
- [ ] Given the two `rejected.json` entries of class `deny-by-default`, when re-decided, then each either leaves `rejected.json` for admission in US-003 or is re-rejected under another class with a reason that holds whatever the flag order
- [ ] The test asserting that no catalogued Clippy rule is denied by default is replaced by a test asserting that a catalogued deny-by-default rule set to `off` is absent from the Clippy command, and that its trigger produces no diagnostic and no build failure
- [ ] Given a scan with `--rule clippy::eq_op=off` on a workspace containing a trigger, when it runs, then the exit code is 0 or 1 by the gate, never 2
- [ ] Every sentence in `docs/catalog-and-corpus.md` and `.claude/skills/rule-candidate/SKILL.md` that says a deny-by-default lint cannot be carried, or that a warned lint reaches the report uncatalogued, is corrected

#### US-003: Admit Clippy's correctness group as one pack
**Description:** As a developer, I want every certain bug Clippy knows to be reported and scored by rust-doctor, so that a clean rust-doctor score is never dirtier under plain `cargo clippy`.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Every member US-001 classified `std` has a `RuleDefinition` in `src/policy/catalog.rs` with category `correctness`, tier `P1`, default level `Warn`, in alphabetical position, and `validate_catalog` accepts the catalog
- [ ] `tests/fixtures/score-credibility/packs/correctness/` holds one crate that triggers every admitted member in production code, and an oracle that names each member in an observed position
- [ ] `tests/rule_evidence.json` carries one entry per admitted member pointing at that oracle, with `catches` equal to the toolchain's published description (enforced by `what_a_clippy_rule_catches_is_what_the_toolchain_says_it_catches`)
- [ ] Given a workspace whose `src/lib.rs` contains `let a = 1; if a == a {}`, when it is scanned, then `clippy::eq_op` is reported at severity warning, the correctness dimension is below 100, and so is the overall score
- [ ] Given the same trigger in an integration test target, when scanned, then it is reported with the `tests` context and does not weigh
- [ ] Every hard-coded counter listed in `.claude/skills/rule-admit/SKILL.md` moves in the same change, and the README rule count equals the catalog length
- [ ] Given a member classified `needs-crate` or `toolchain-gated`, when the queue is printed, then it stays in the candidate queue, and its classification is quoted in the admission commit message
- [ ] Given a toolchain older than the one a member requires, when the scan runs, then the member is handled by US-011 (not evaluated), never by a build failure

**Amendment, 2026-09-25 (EP-001 review).** The fifth criterion rests on a false premise. The scan runs Clippy on Cargo's default targets (`BASE_ARGS` in `src/execution/clippy.rs`, no `--all-targets`), so an integration test target is never linted and its trigger is not reported at all, with or without the `tests` context. `no_cargo_test_target_reaches_the_report` already records this for the panic pack. The criterion is restated as: the same trigger in an integration test target produces no diagnostic and weighs nothing, proven by `a_self_comparison_in_shipped_code_lowers_the_correctness_dimension` and the pack oracle's empty `test_context`. Linting test targets would change what the score judges and stays outside this PRD. The last criterion names US-011 where it means US-010, the lint-set probe.

#### US-004: Publish uncatalogued compiler diagnostics as an unscored population
**Description:** As a developer, I want warnings the catalog does not describe shown and labeled as unscored, so that one `unused_imports` neither vanishes nor voids my score.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a production-code `unused_imports` warning, when the report is assembled, then the diagnostic appears in `diagnostics` with `unscored: "uncatalogued"`, weighs nothing, and `score.authoritative` stays true when nothing else voids it
- [ ] Given an uncatalogued Clippy lint enabled by the workspace's own `[lints.clippy]` table (for example `clippy::pedantic`), when the scan runs, then its diagnostics get the same treatment
- [ ] Given a catalogued rule switched off by policy, when its diagnostic arrives, then it is still dropped, as today (`src/report/normalize.rs` keeps that branch)
- [ ] Given an uncatalogued diagnostic at level `error` (a compile error), when the report is assembled, then it is not a note: the Clippy stage fails as today, and US-005 reports `stage-failed`
- [ ] The linear report prints uncatalogued diagnostics after the scored findings, under the heading "Compiler notes (not scored)", with their count in the summary line. The interactive report lists them after scored findings, with the same label.
- [ ] `SCHEMA_VERSION` moves from 17 to 18, the frozen v7 archive still projects, and the schema tests pin the new field's serialization
- [ ] `docs/subsystems/reporting.md` and `docs/subsystems/audit.md` state the new rule: an uncatalogued diagnostic is published and never weighed

#### US-005: Say why a score is not authoritative
**Description:** As an agent or CI step reading `--json`, I want the causes of a partial score as closed values, so that I can tell a broken toolchain from a timeout without re-deriving it from errors.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**
- [ ] `audit.score.reasons` is an array drawn from the closed vocabulary `scan-incomplete`, `stage-failed`, `category-mapping-conflict`, `deadline-exceeded`, `packages-unlinted`, `rules-not-evaluated`, serialized kebab-case from a Rust enum
- [ ] Given an authoritative score, when serialized, then `reasons` is an empty array
- [ ] Given a failed stage, when the report is assembled, then `reasons` contains `stage-failed` and `errors[]` still names the stage and code
- [ ] Given a rule whose category mapping conflicts (the `mapping_conflict` path in `src/audit.rs`), when the report is assembled, then `reasons` contains `category-mapping-conflict`
- [ ] The sentence at `src/render.rs:723` is replaced by one line per reason, each naming its cause and its next step, and the interactive report shows the same lines
- [ ] `toolchain` gains `rust_doctor`, the version of the binary that produced the report
- [ ] The two non-authoritative agent-population reports (`ostendo`, `vibesql`) each carry at least one reason when replayed in US-006

#### US-006: Regenerate the corpus under the new catalog and record what moved
**Description:** As the catalog maintainer, I want the pinned corpus replayed under the admitted pack and the unscored population, so that every score change this release causes is measured and published before the tag.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003, US-004, US-005

**Acceptance Criteria:**
- [ ] `tests/corpus.json` is regenerated from the clone cache as `docs/catalog-and-corpus.md` describes, and `the_published_catalog_matches_the_shipped_policy` passes
- [ ] `docs/parity-release-1-2026-10.md` (registered `frozen`) records, for each of the 18 corpus repositories, the correctness findings, the score before and after, and `authoritative` before and after with its reasons
- [ ] Given an admitted correctness member that fires on healthy code, when its sites are adjudicated with `corpus-adjudicate`, then a confirmed false positive demotes the member to the candidate queue before release, with the site quoted in the amendment
- [ ] Given a member that fires only on the agent population, when recorded, then it is listed with its site count and needs no demotion
- [ ] Given a `score_distribution` gate failure after regeneration, when this story closes, then the lambda table is unchanged, the failure is recorded, and the release is blocked on an Open Question rather than on a silent retune

---

### EP-002: A scan that fails loudly and finishes on time

**Release:** 1 (0.8.0). Every failure names its cause, no scan can hang forever, one broken member does not hide the rest, and the caller's environment cannot turn warnings into build failures.

**Definition of Done:**
- The failure fixtures (bad lockfile, broken member, sleeping build script, `RUSTFLAGS=-Dwarnings`, older toolchain) each produce a complete report whose `errors[]` or `reasons` names the cause, with no absolute path in `--json`.
- A second baseline run on an unchanged base recompiles no registry dependency.

#### US-007: Surface Cargo's stderr in the stage error, bounded and scrubbed
**Description:** As a developer whose scan failed, I want the error to carry what Cargo said, so that I can fix a lockfile, registry or lock problem without rerunning Cargo by hand.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The Clippy child's stderr is piped and drained concurrently with stdout through `collect_bounded` (`src/bounded_read.rs:22`) with a 65,536-byte limit, so a verbose build can never deadlock the pipes
- [ ] Given a Clippy run that exits non-zero without `build-finished.success: true`, when the report is assembled, then the `clippy` stage error message ends with the last lines of stderr, capped at 1,020 bytes, with control characters removed as `bounded_stderr` does (`src/main.rs:521`)
- [ ] Given stderr containing the workspace root, `CARGO_HOME` or `HOME`, when it is published, then those prefixes are replaced by `.`, `$CARGO_HOME` and `~`, and a test scans `--json` for the fixture's absolute paths and finds none
- [ ] Given a `Cargo.lock` that fails to parse (fixture), when scanned, then the error message contains Cargo's own sentence about the lockfile
- [ ] Given a successful run, when the report is assembled, then nothing from stderr is published
- [ ] `cargo metadata` runs through the same bounded, scrubbed path: its output is read through `collect_bounded` and parsed with `MetadataCommand::parse`, and its failure publishes Cargo's cause instead of "cargo metadata exited with an error" (`src/scan_target.rs:166`)

#### US-008: Bound the run with `--max-duration`
**Description:** As a CI owner or an agent hook, I want a wall-clock bound on the whole scan that kills Cargo cleanly, so that a hung build script degrades into a report instead of a hung job.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] `--max-duration <SECONDS>` accepts 1 to 86,400 and is refused otherwise with a clap error. Without the flag the run has no deadline, as today.
- [ ] Given a workspace whose `build.rs` sleeps 120 seconds and `--max-duration 5`, when scanned, then the process exits within 7 seconds with exit code 2, `errors[]` contains stage `clippy` code `deadline-exceeded`, and `reasons` contains `deadline-exceeded`
- [ ] On Unix, the Clippy child is spawned in its own process group and the whole group is killed at the deadline. A test asserts that no process of that group survives. On Windows, the child is killed and the limitation is documented in `docs/subsystems/execution.md`.
- [ ] Diagnostics collected from stdout before the deadline are kept in the report, marked by the incomplete status
- [ ] Given the deadline passes while native passes are pending, when the run continues, then passes not yet started are skipped with the same reason, and the structure pass budget (`src/structure.rs:74`) is capped by the remaining time
- [ ] The flag is documented in `skills/rust-doctor/SKILL.md` and the skill contract test passes

#### US-009: Keep going past a broken member and name what went unlinted
**Description:** As a developer mid-refactor, I want the members that compile to be linted even when one does not, so that one red crate does not erase the report for the others.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] `--keep-going` is part of the Clippy base arguments, and the two command-length assertions and the command-shape test are updated in the same change
- [ ] Given a two-member workspace where `a` fails to compile and `b` carries a catalogued finding, when scanned, then `b`'s finding is reported, `errors[]` contains stage `clippy` code `packages-unlinted` naming `a` by package name, and `reasons` contains `packages-unlinted`
- [ ] Given a member that fails only because a dependency failed, when scanned, then it is named too
- [ ] Given a workspace where every member compiles, when scanned, then no `packages-unlinted` error exists
- [ ] Package names are published, never paths

#### US-010: Tolerate the caller's lint flags and the installed Clippy's lint set
**Description:** As a developer running the scan in a CI job that exports `RUSTFLAGS=-Dwarnings`, or on an older toolchain, I want the scan to keep its own lint levels and to say which rules it could not evaluate, so that my environment cannot fail the build or silently shrink the catalog.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] Given `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` containing lint-level tokens (`-D`, `-F`, `--deny`, `--forbid`, `--cap-lints`, in joined or separate form), when Clippy is spawned, then those tokens and their values are removed, the remaining flags are passed through `CARGO_ENCODED_RUSTFLAGS`, and the removed lint names are listed under `toolchain.removed_lint_flags`
- [ ] Given `RUSTFLAGS` without lint-level tokens, when Clippy is spawned, then the environment is passed untouched and no `CARGO_ENCODED_RUSTFLAGS` is set, so Cargo's fingerprints match the user's own builds
- [ ] Given `RUSTFLAGS=-Dwarnings` and a workspace with one catalogued warning, when scanned, then the finding is a warning, the build succeeds, and the exit code is decided by the gate
- [ ] Given `[build] rustflags = ["-D", "warnings"]` in `.cargo/config.toml`, when scanned on 1.97.1, then the observed behavior is recorded in `docs/subsystems/execution.md`, and if it fails the build, the same stripping is applied by passing the config's flags minus the lint-level tokens through `CARGO_ENCODED_RUSTFLAGS`
- [ ] Before Clippy runs, the installed `clippy-driver`'s lint list is read under a 10-second timeout with the parser `src/policy/coverage.rs` uses in tests, and catalogued Clippy rules it does not list get no `-W` flag
- [ ] Each such rule is published in `policy.rules[]` with `not_evaluated: "unknown-to-toolchain"`, and `reasons` contains `rules-not-evaluated`
- [ ] Given the lint-list probe fails or times out, when Clippy runs, then every catalogued `-W` is passed with `-A unknown_lints` appended, and an informational error at stage `toolchain` code `lint-list-unavailable` is recorded without voiding the score
- [ ] Each of the three version probes (`src/execution.rs:449-451`) is bounded by a 10-second timeout, and a timed-out probe records stage `toolchain` code `probe-timeout`

#### US-011: Keep the baseline side's dependency build between runs
**Description:** As an agent running `--scope baseline` after every change, I want the base side to reuse its compiled dependencies, so that each baseline run no longer pays a cold build of the whole dependency graph.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The base side's `CARGO_TARGET_DIR` is `<cargo metadata target_directory>/rust-doctor/baseline` instead of `<temp root>/target` (`src/baseline.rs:160`), and the temporary source tree is still removed after every run
- [ ] Given two consecutive `--scope baseline --base <ref>` runs on an unchanged base, when the second runs, then every registry dependency's `compiler-artifact` message on the base side reports `fresh: true`
- [ ] Given a target directory that cannot be created or written, when the baseline runs, then it falls back to the temporary target as today, and no error is recorded
- [ ] Given two concurrent baseline runs on the same workspace, when both run, then Cargo's build-directory lock serializes them and both reports are complete
- [ ] `skills/rust-doctor/SKILL.md` and `docs/subsystems/delta.md` state that a scan writes only build artifacts under Cargo's target directory, as `cargo clippy` itself does, which amends "the tool never writes into a workspace it scans"

#### US-012: Show progress on stderr while Clippy runs
**Description:** As a developer waiting on a cold scan, I want to see what the scan is doing, so that minutes of compilation do not read as a hang.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given stderr is a terminal and `TERM` is not `dumb`, when the scan runs, then one line is rewritten in place at most every 100 ms, showing the phase (`dependencies`, `linting <package> k/N`, `native passes`) from Cargo's `compiler-artifact` records and producer completion
- [ ] The progress line is cleared before the report, or any error, is printed
- [ ] Given stderr is not a terminal (agent, CI, hook), when the scan runs, then nothing is rewritten, and at most one line per phase is printed, 5 lines in total
- [ ] Given `--json`, when stdout is not a terminal, then stdout contains only the JSON document
- [ ] The static `Scanning Rust files...` line (`src/main.rs:310`) is replaced by the first phase line

---

### EP-003: The agent loop

**Release:** 1 (0.8.0). An agent and a commit can each ask "what did I just introduce?" without flags that depend on the repository, and the tool installs the checks that make them ask it.

**Definition of Done:**
- In a repository whose default branch is `master`, the three command forms `rust-doctor . --json --scope lines`, `rust-doctor . --json --staged --scope lines` and `rust-doctor . --json --scope baseline` succeed with no `--base`.
- `rust-doctor hook install git` and `rust-doctor hook install claude` produce working hooks.
- The skill installs and refreshes for every confirmed agent target.

#### US-013: Treat agents, git hooks and CI providers as non-interactive
**Description:** As an agent calling `rust-doctor` under a PTY, I want the tool to recognize me, so that I get the linear report instead of a TUI I cannot drive.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `interactions_allowed` (`src/main.rs:470`) returns false when any of these variables is set and not empty:
  - agent markers `CLAUDECODE`, `CODEX_SANDBOX`, `CURSOR_AGENT`;
  - hook markers `GIT_DIR`, `GIT_INDEX_FILE`;
  - CI provider variables `GITHUB_ACTIONS`, `GITLAB_CI`, `BUILDKITE`, `CIRCLECI`, `TF_BUILD`, `JENKINS_URL`, `TEAMCITY_VERSION`.
- [ ] Given `CI` set to `false`, `0` or the empty string (case-insensitive) with both TTYs, when run, then the interactive report may open. Any other value keeps it closed.
- [ ] Given `CLAUDECODE=1` with both TTYs, when `rust-doctor .` runs, then the linear report prints
- [ ] The list lives in one constant, and `skills/rust-doctor/SKILL.md` names it where it explains when the interactive report opens
- [ ] Given a variable set to the empty string, when checked, then it counts as unset

#### US-014: Resolve the base branch when `--base` is omitted
**Description:** As an agent following the skill, I want the base resolved from the repository, so that one command works whether the default branch is `main`, `master` or `trunk`.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `--scope files`, `--scope lines` or `--scope baseline` without `--base`, when the scope resolves, then the base is the first of these that `git rev-parse --verify` accepts: the target of `refs/remotes/origin/HEAD`, `origin/main`, `origin/master`, `main`, `master`
- [ ] Given the checked-out branch is the resolved base branch, when the scope resolves, then the base becomes `HEAD`, which scopes the scan to uncommitted work
- [ ] The resolved ref is printed on the terminal and published in the report's `scope` block
- [ ] Given no candidate resolves (detached HEAD, no remote), when the scan runs, then it fails with stage `scope` code `base-undetected` and a message naming `--base`
- [ ] Given the merge base fails and `git rev-parse --is-shallow-repository` prints `true`, when the scan runs, then the error code is `shallow-clone` and the message names `fetch-depth: 0` instead of `merge-base-unavailable`
- [ ] `skills/rust-doctor/SKILL.md` drops `--base main` from its post-change command, and every git call goes through `src/git.rs` with `GIT_ENVIRONMENT_OVERRIDES` applied

#### US-015: `--scope lines` and `--include-untracked`
**Description:** As an agent or reviewer, I want findings limited to the lines a change touched, so that editing one line of a large file does not surface the file's whole backlog, without paying baseline's second compilation.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-014

**Acceptance Criteria:**
- [ ] `--scope lines` diffs `git diff --unified=0 --no-color --no-ext-diff <merge-base>` against the working tree, parses the new-side ranges of every hunk, and keeps a diagnostic when its primary span `[line, line_end]` intersects a range of its file
- [ ] A hunk with a new-side count of 0 (pure deletion) contributes no range, and a renamed file is keyed by its new path
- [ ] A diagnostic with no path or no span is dropped, as the files scope drops it today
- [ ] Given `--include-untracked` with `files` or `lines`, when the scope resolves, then `git ls-files --others --exclude-standard` is unioned into the selection, and an untracked file counts as wholly changed
- [ ] Given untracked Rust files and no `--include-untracked`, when the report renders, then one terminal line states how many untracked Rust files were compiled but not reported, and names the flag
- [ ] Diff output is read under the existing `DIFF_OUTPUT_LIMIT` (`src/git_scope.rs`), and exceeding it fails with stage `scope` code `diff-too-large`
- [ ] Given a finding on line 40 and a change on lines 10 to 12 of the same file, when scanned with `--scope lines`, then the finding is excluded, and with `--scope files` it is included

#### US-016: `--staged`: scan what the commit will record
**Description:** As a developer running a pre-commit gate, I want the scan to judge the index, so that unstaged edits neither create nor hide findings in the commit.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-011, US-015

**Acceptance Criteria:**
- [ ] `--staged` is accepted with `--scope files`, `lines` or `baseline`, and refused with `full` by a validation error naming the combination
- [ ] The index is materialized into a temporary tree through `checkout-index` (the mechanism of `src/baseline.rs:213`), honoring `GIT_INDEX_FILE` when git set it for a hook and reading `.git/index` otherwise. Every producer runs on that tree with `CARGO_TARGET_DIR` at `<target_directory>/rust-doctor/staged`, and paths are mapped back to workspace-relative form.
- [ ] With `files` or `lines`, the changed set is `git diff --cached` against `HEAD` (or `--base` when given); with `baseline`, the base side is `HEAD` (or `--base`) and the current side is the index tree
- [ ] Given a staged file with a defect and an unstaged edit that fixes it, when scanned with `--staged`, then the defect is reported
- [ ] Given a staged clean file with an unstaged defect, when scanned with `--staged`, then nothing is reported
- [ ] Given an unreadable or locked index, when scanned, then the scan fails with stage `scope` code `index-unavailable` and exit code 2, never with an empty pass
- [ ] Given unmerged index entries, when scanned, then the scan fails with code `index-conflicted`
- [ ] Given no staged Rust change, when scanned, then the report has no in-scope finding, exit code 0, and a terminal line saying nothing staged was judged

#### US-017: Install a git pre-commit hook that respects the repository's hook manager
**Description:** As a developer, I want one command that wires `--staged` into my commits the way my repository already runs hooks, so that the gate becomes a habit without breaking existing hooks.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-016

**Acceptance Criteria:**
- [ ] `rust-doctor hook install git [--dry-run]` writes a hook running `rust-doctor . --yes --staged --scope lines`, which inherits the workspace's configured blocking level
- [ ] The target depends on what the repository already uses:
  - `core.hooksPath` set: the file goes into that directory;
  - `.cargo-husky/hooks/` present: the file goes there;
  - `.pre-commit-config.yaml` or `lefthook.yml` present: the exact snippet is printed and nothing is written, since no YAML is edited;
  - none of these: `.git/hooks/pre-commit` is written.
- [ ] Given a pre-commit file that exists and was not written by rust-doctor, when installing, then nothing is written, and the line to add is printed
- [ ] Given a file carrying rust-doctor's marker comment, when installing again, then nothing is written, and the output says it is already installed
- [ ] `--dry-run` prints the target and the content, and writes nothing
- [ ] Given a directory that is not a git repository, when installing, then it fails with a message and exit code 2

#### US-018: Install and refresh the skill for every confirmed agent
**Description:** As a user of Codex, Cursor or Claude Code, I want the skill installed where my agent reads it, and refreshed after each upgrade, so that the agent never follows instructions for flags the binary no longer has.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `rust-doctor skill install [--agent claude|codex|cursor|all] [--update] [--dry-run]` defaults to `claude`, so the current behavior is unchanged
- [ ] Target directories are a closed table in `src/skill.rs`, with `.claude/skills/rust-doctor/` for Claude Code. The Codex and Cursor targets are confirmed against each agent's official documentation during implementation, with the URL cited in a comment. A target not confirmed is not shipped, and its `--agent` value is refused with a message saying so.
- [ ] Given an existing target without `--update`, when installing, then nothing is written, as today, and the message names `--update`
- [ ] Given `--update` and an existing target whose `SKILL.md` front matter declares `name: rust-doctor`, when installing, then every embedded document is rewritten, and the output names the version replaced and the version written
- [ ] Given `--update` and a target whose `SKILL.md` declares another name, or has none, when installing, then nothing is written
- [ ] Every installed copy records the binary version that wrote it, and `tests/skill_contract.rs` still ties the source skill to `--help`
- [ ] Given a target path with a symlinked component, when installing, then nothing is written through the link, and the message names the link
- [ ] Printed paths are workspace-relative

#### US-019: Install an end-of-turn hook that rescans the agent's changes
**Description:** As a user of a coding agent, I want the agent's turn to end only after a rescan of what it changed, so that regressions are fixed before the agent says it is done.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-008, US-013, US-015

**Acceptance Criteria:**
- [ ] `rust-doctor hook install claude [--shared] [--dry-run]` adds a `hooks.Stop` entry running `rust-doctor hook run claude`, with a `timeout` of 600 seconds. It writes `.claude/settings.local.json` by default and `.claude/settings.json` with `--shared`.
- [ ] Given an existing settings file, when installing, then it is parsed as JSON, every other key and hook is kept, the entry is added only when no entry with the same command exists, and the exact entry is printed before the file is written
- [ ] Given an existing settings file that is not valid JSON, when installing, then nothing is written and the command exits 2 with the parse position
- [ ] `rust-doctor hook run claude` reads the hook's JSON from stdin. When `stop_hook_active` is true, it exits 0 immediately.
- [ ] Otherwise, `hook run claude` runs the equivalent of `--scope lines --base HEAD --include-untracked --blocking warning --max-duration 540`, then:
  - gate fails: exits 2 and prints to stderr up to 10 findings (rule id, `path:line`, message) followed by the rescan command;
  - gate passes: exits 0.
- [ ] Given the scan itself fails (exit 2), not a gate failure, when the hook runs, then it exits 0 with a one-line stderr note, so a broken toolchain never traps the agent
- [ ] `rust-doctor hook install cursor` writes a `stop` entry into `.cursor/hooks.json` with the same merge rules, and `rust-doctor hook run cursor` prints `{"followup_message": ...}` when findings exist, since Cursor's stop hook cannot block
- [ ] The hook command names the binary without a path, and installation prints a warning when `rust-doctor` is not on `PATH`
- [ ] Nothing in a scan installs a hook: only the `hook install` subcommand writes, and only under the scanned workspace root

---

### EP-004: Precision controls

**Release:** 2 (0.9.0). Every exception is narrow, stated and visible: a path, a site with a reason, or a generated file declared as such, and a large workspace can be judged one member at a time.

**Definition of Done:**
- A workspace can ignore one directory, relax one rule under one path, silence one native finding with a reason, and exclude `linguist-generated` files, each visible in the report.
- A single member can be scanned and scored.

#### US-020: Path-scoped ignores and per-path overrides
**Description:** As a CI owner, I want to ignore a directory or relax a rule under one path in `rust-doctor.toml`, so that one noisy directory does not force a rule off everywhere.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `rust-doctor.toml` accepts `[ignore] paths = [...]` and any number of `[[overrides]]` tables, each with `paths = [...]` and optional `rules` and `categories` using the top-level selector grammar
- [ ] Globs are relative to the workspace root and support `*`, `**` and `?`. A malformed glob fails with stage `policy` code `invalid-glob`, naming the key and the entry's index, never echoing its value.
- [ ] A diagnostic whose path matches an ignore glob is removed from the report and the score, and `scan.ignored` counts them (outside `summary`, which `Summary::from_diagnostics` derives)
- [ ] A diagnostic under a matching override path takes the override's level. Later overrides win over earlier ones, and a CLI `--rule` or `--category` still wins over the file.
- [ ] The source-kernel walk and the structure pass skip ignored files; Clippy still compiles them, and the documentation says so
- [ ] An override naming an unknown rule is refused, as the top-level `[rules]` table refuses it (`src/configuration.rs:119`)
- [ ] `SCHEMA_VERSION` moves to 19 for this release

#### US-021: Site-level suppression for native rules, with a reason and an audit
**Description:** As a developer with one justified native finding, I want to silence that site with a stated reason, so that I do not turn the rule off for the whole workspace.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A line comment `// rust-doctor: allow(<rule-id>[, <rule-id>]) -- <reason>` suppresses the named `rust_doctor::*` findings whose primary span starts on the comment's line or on the next line
- [ ] In `Cargo.toml` and `.cargo/config.toml`, the TOML comment `# rust-doctor: allow(<rule-id>) -- <reason>` on the line above a key suppresses the finding at that key
- [ ] Given a directive with a missing or empty reason, when the report is assembled, then it suppresses nothing and is listed with status `missing-reason`
- [ ] Given a directive naming a `clippy::*` id, when the report is assembled, then it suppresses nothing and is listed with status `use-expect`, with help naming `#[expect(<lint>, reason = "...")]`
- [ ] Given a directive naming an unknown rule, when the report is assembled, then it is listed with status `unknown-rule` and the closest catalogued id within edit distance 3 as a hint
- [ ] Given a directive that suppressed no finding in a scanned file, when the report is assembled, then it is listed with status `unused`
- [ ] `suppressions[]` publishes path, line, rule ids and status, the terminal prints one summary line, and a suppressed finding weighs nothing
- [ ] Directives are found only in comment tokens (`ra_ap_syntax` for Rust, `toml` comments for manifests), so the same text inside a string literal is ignored

#### US-022: Exclude generated and vendored code from every producer
**Description:** As a developer with committed codegen output, I want files my repository declares generated or vendored left out of the score, so that code nobody edits by hand does not grade the code people do.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `git check-attr -z --stdin linguist-generated linguist-vendored` runs over the tracked files through `src/git.rs`, and a path whose value is `set` or `true` is excluded from every producer's report and score
- [ ] Values `unset`, `false` and `unspecified` leave the path in the scan
- [ ] The generator-header test (`src/structure.rs:664`) applies to diagnostics from every producer, not only the structure pass
- [ ] `scan.excluded_generated` counts excluded diagnostics, and none of them is listed
- [ ] Given a workspace outside git, or a failing `check-attr`, when scanned, then only the header test applies, and no error is recorded
- [ ] Given a `Cargo.toml` marked `linguist-generated`, when scanned, then its cargo-health findings are excluded like any other path

#### US-023: Select workspace members and publish a score per member
**Description:** As the owner of one crate in a large workspace, I want to scan and score my member alone, so that a single number stops hiding which crate regressed.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `--package <NAME>`, repeatable, runs Clippy with one `-p <NAME>` per package instead of `--workspace`, and limits native producers to those packages' units
- [ ] Given an unknown package name, when the scan starts, then it fails with stage `policy` code `unknown-package`, listing up to 20 member names
- [ ] `audit.packages[]` publishes, per scanned member, a score computed by the core-v4 model over that member's production lines and findings, and the workspace score keeps its current definition
- [ ] The verbose linear report prints one line per member with its score and worst tier
- [ ] Given `--package`, when the repository-hygiene pass would run, then it is skipped, its rules are published with `not_evaluated: "package-scoped"`, and `reasons` contains `rules-not-evaluated`
- [ ] A finding with no package attribution counts only in the workspace score

---

### EP-005: CI that reports where reviewers look

**Release:** 2 (0.9.0). CI setup is one command an agent can run. Findings, score and delta reach the pull request without anyone opening a log, and no step runs privileged on untrusted code.

**Definition of Done:**
- This repository's own pull requests show a sticky rust-doctor comment and Action outputs produced by the Action from the checkout.
- `rust-doctor ci install` output passes `bash -n`, and every rust-doctor invocation it contains parses against the binary's `--help`.

#### US-024: `rust-doctor ci install`
**Description:** As a CI owner or an agent, I want to write the CI workflow non-interactively with its gate, branch and toolchain chosen, so that CI setup no longer needs the interactive report and does not turn the default branch red on legacy findings.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-014

**Acceptance Criteria:**
- [ ] `rust-doctor ci install [--blocking <LEVEL>] [--branch <NAME>] [--toolchain <VERSION>] [--update] [--dry-run]` writes `.github/workflows/rust-doctor.yml`, and the interactive report's menu item calls the same writer with the same defaults
- [ ] The branch defaults to the branch US-014 resolves, and the toolchain defaults to the version the release was validated on (1.97.1 for 0.9.0), embedded as a constant
- [ ] Pull requests run `--scope baseline` with the base passed through the environment, never interpolated. Pushes to the branch run `--scope full --blocking none`, so they report without failing.
- [ ] Checkout steps set `persist-credentials: false` and the workflow declares `permissions: contents: read`
- [ ] Given an existing workflow file, when installing without `--update`, then nothing is written. With `--update`, it is rewritten only when it carries rust-doctor's marker comment.
- [ ] Given no resolvable branch and no `--branch`, when installing, then it fails with a message naming `--branch`
- [ ] A test renders the template, runs `bash -n` on its script step, and parses each rust-doctor command line in it with the binary's clap definition

#### US-025: Render a saved report as Markdown
**Description:** As a CI step, I want to turn a saved `--json` report into Markdown without rescanning, so that summaries and comments come from one tested renderer.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `rust-doctor report markdown <FILE> [--limit <N>]` reads a report and prints Markdown containing:
  - score, label, and `authoritative` with its reasons;
  - in baseline scope, the introduced and fixed counts;
  - a table of up to N findings (default 20): rule, `path:line`, severity, message;
  - a link for each rule to `https://rust-doctor.com/rules/<id>`.
- [ ] Output is deterministic for a given report and pinned by a golden test
- [ ] Message text is escaped for Markdown and HTML (pipes, backticks, angle brackets, newlines), so a finding message cannot inject markup into a comment
- [ ] Given a report whose `schema_version` differs from the binary's, when rendered, then it exits 2 with a message naming both versions
- [ ] Given a file over 64 MiB or invalid JSON, when rendered, then it exits 2 with a message, and nothing is printed to stdout
- [ ] The command runs no scan, spawns no process, and opens only the file named

#### US-026: A composite GitHub Action with outputs and a job summary
**Description:** As a CI owner, I want `uses: arthjean/rust-doctor@v0` with outputs, so that branch protection and later steps can read the score and delta without parsing logs.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-024, US-025

**Acceptance Criteria:**
- [ ] `action.yml` at the repository root declares these inputs: `version`, `toolchain`, `scope`, `base`, `blocking`, `working-directory`, `args`. The toolchain defaults to the embedded validated version.
- [ ] The steps run in order:
  - install the toolchain with Clippy;
  - install rust-doctor at `version` through the npm launcher;
  - run `rust-doctor <dir> --yes --json > report.json`, keeping the exit code;
  - append `rust-doctor report markdown report.json` to `$GITHUB_STEP_SUMMARY`;
  - upload `report.json` as artifact `rust-doctor-report`;
  - exit with the scan's exit code.
- [ ] The outputs `score`, `authoritative`, `introduced`, `fixed` and `exit-code` are read from `report.json` with `jq`
- [ ] Every ref and input reaches shell steps through `env`, never through `${{ }}` inside `run`
- [ ] Given a scan that exits 2, when the Action runs, then the summary is still written, the artifact still uploaded, the outputs still set, and the step fails
- [ ] A workflow in this repository runs the Action from the checkout (`uses: ./`) on pull requests, and `docs/publishing.md` states how the floating `v0` tag moves on release

#### US-027: A fork-safe sticky pull request comment
**Description:** As a reviewer, I want one comment on the pull request carrying the score, the delta and the introduced findings, updated on each push, so that I see the result without opening the job, including on forks.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-026

**Acceptance Criteria:**
- [ ] `.github/workflows/rust-doctor-comment.yml` is triggered by `workflow_run` of the scan workflow and has `permissions: pull-requests: write` in that job only. It downloads `rust-doctor-report`, renders it with `report markdown`, and upserts one comment identified by the marker `<!-- rust-doctor -->` through `gh api`.
- [ ] The comment job never checks out pull request code and never runs `cargo`
- [ ] Given a pull request from a fork, when the scan workflow completes, then the comment is posted, because `workflow_run` runs in the base repository's context
- [ ] Given a second push to the same pull request, when the comment job runs, then the existing comment is edited, not duplicated
- [ ] Given no artifact (the scan failed before writing it), when the comment job runs, then it posts nothing and succeeds with a notice
- [ ] `ci install` (US-024) can emit this workflow with `--comment`, and the README states that the binary never reaches the network, while this job uses the workflow's token

---

### EP-006: Rule families Clippy cannot see

**Release:** 3 (0.10.0). The failures where Rust services actually break (blocking the executor, SQL injection, undocumented `unsafe`, vulnerable crates) get rules. The kernel gains the one capability those detectors need, and every native detector is held to its verdict under rewrites.

**Definition of Done:**
- The new rules are catalogued with trigger evidence and corpus measurements.
- Both shipped security detectors fire through a `let` binding.
- Every native detector passes the variant gate in `cargo test`.

#### US-028: Resolve local bindings in the source kernel
**Description:** As a native detector author, I want an identifier resolved to the initializer of its `let` binding in the same body, so that detectors stop missing the most common way code is written.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The detector `Context` resolves an identifier to the initializer of the nearest preceding `let` binding of that name in an enclosing block of the same function body
- [ ] It also resolves `const` items of the same module
- [ ] Given a binding that is reassigned, bound in a pattern, taken from a parameter, or shadowed in a branch, when resolved, then the answer is indeterminate and detectors abstain
- [ ] Given `let mut cmd = std::process::Command::new("sh"); cmd.arg("-c").arg(format!("echo {user}")).spawn();`, when scanned, then `rust_doctor::source::dynamic_shell_command` reports it. The fixture case previously recorded as an expected silence flips to positive, with `tests/rule_evidence.json` unchanged.
- [ ] Given `const INSECURE: bool = true;` and `reqwest::Client::builder().danger_accept_invalid_certs(INSECURE)`, when scanned, then the TLS detector reports it
- [ ] The source-kernel pass's wall-clock time on this repository grows by at most 10%, measured the way `src/structure/benchmark.rs` measures the structure pass

#### US-029: Native detector: blocking call inside async code
**Description:** As a developer of an async service, I want blocking calls inside `async` code reported, so that I find the executor stalls before they reach production.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-028

**Acceptance Criteria:**
- [ ] `rust_doctor::source::blocking_call_in_async` (category `reliability`, tier `P2`) reports a call resolving to one of these paths:
  - `std::thread::sleep`;
  - `std::fs::{read, read_to_string, write, copy, remove_file, create_dir_all}`;
  - `std::fs::File::{open, create}`;
  - `std::net::TcpStream::connect`;
  - `std::process::Command::{output, status}`;
  - any `reqwest::blocking::` item.
- [ ] Reports are limited to calls inside an `async fn` body or an `async` block, and excluded when inside a closure passed to `spawn_blocking` or `block_in_place`
- [ ] Given `tokio::fs::read` or `tokio::time::sleep` inside async code, when scanned, then nothing is reported
- [ ] Given `std::fs::read` in a synchronous function called from async code, when scanned, then nothing is reported, and the help text states that the rule does not follow calls
- [ ] Given a finding inside a `#[tokio::test]` or `#[test]` function in a test target, when scanned, then it carries the test context and does not weigh
- [ ] Positive and negative fixtures, a `tests/rule_evidence.json` entry, the counters of `.claude/skills/rule-admit/SKILL.md`, the README count and a corpus measurement or an explicit `unproven` all land in the same change

#### US-030: Native detector: SQL text built with `format!` or concatenation
**Description:** As a developer of a database-backed service, I want SQL assembled from runtime values reported, so that injection is caught where it is written.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-028

**Acceptance Criteria:**
- [ ] `rust_doctor::source::sql_built_from_format` (category `security`, tier `P1`) reports calls whose SQL argument is a `format!` with at least one non-literal argument, a `+` or `push_str` concatenation of a non-literal, or a local binding to either through US-028. The calls covered:
  - `sqlx::{query, query_as, query_scalar, raw_sql}`;
  - `diesel::sql_query`;
  - `rusqlite::Connection::{execute, prepare, query_row}`;
  - `postgres::Client::{query, execute, batch_execute}`;
  - `tokio_postgres::Client::{query, execute, batch_execute}`.
- [ ] Method forms (`conn.execute(...)`) are reported only when the receiver resolves through US-028 to a constructor of a listed type. Otherwise the detector abstains.
- [ ] Given `sqlx::query!("...")` or `sqlx::query("SELECT 1")`, when scanned, then nothing is reported
- [ ] Given `format!` whose arguments are all literals or `const` items, when scanned, then nothing is reported
- [ ] The help text names parameter binding (`.bind()`, `query!`) as the fix, and the rule lands with its fixtures, evidence, counters and README count

#### US-031: Offline advisory check against a local RustSec database
**Description:** As a developer, I want known-vulnerable and unmaintained crates in my `Cargo.lock` reported from a database already on my machine, so that the dependency finding I expect first is part of the same score, without the tool reaching the network.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-010

**Acceptance Criteria:**
- [ ] `rust_doctor::cargo::vulnerable_dependency` (category `security`, tier `P1`) and `rust_doctor::cargo::unmaintained_dependency` (category `dependencies`, tier `P2`) read a local advisory database, located in this order: `--advisory-db <PATH>`; `$CARGO_HOME/advisory-db` (cargo-audit); the newest directory under `$CARGO_HOME/advisory-dbs/` (cargo-deny). It is never fetched.
- [ ] Each crates.io package in `Cargo.lock` is matched against the advisories filed under its name. A package is affected when its version matches neither a `patched` nor an `unaffected` requirement, evaluated with `cargo_metadata::semver`. Withdrawn advisories are skipped.
- [ ] Advisories with no `informational` field or with `informational = "unsound"` feed `vulnerable_dependency`, `unmaintained` feeds `unmaintained_dependency`, and `notice` is skipped
- [ ] A finding points at the package's entry in `Cargo.lock` and names the advisory id and the patched requirement
- [ ] Given no database, when scanned, then both rules are published with `not_evaluated: "advisory-db-missing"`, the score's authority is unaffected, and one terminal line says how to populate a database
- [ ] Given a database whose newest advisory date is more than 30 days old, when scanned, then a notice states its age
- [ ] `--json` publishes the database source as `cargo-audit`, `cargo-deny` or `explicit`, with its date, never its path
- [ ] Given a malformed advisory, an advisory file over 1 MiB, or more than 20,000 advisory files, when read, then the offending files are skipped and counted, and the scan completes
- [ ] Tests use a fixture database under `tests/fixtures/` and never touch the network or `$CARGO_HOME`

#### US-032: Triage and admit the unsafe and FFI soundness lints
**Description:** As a reviewer of code containing `unsafe`, I want the lints that make each `unsafe` block state its invariant reported, so that the one defect class the compiler does not stop is visible in the score.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] `clippy::undocumented_unsafe_blocks`, `clippy::multiple_unsafe_ops_per_block` and `clippy::missing_transmute_annotations` each go through `rule-candidate` triage and end with a recorded decision: admission, or rejection with a reason in `src/policy/rejected.json`
- [ ] So do the transmute lints of the `suspicious` group left in the queue by US-003
- [ ] An admitted lint sits in category `correctness`, tier `P2`, with an admission fixture under `tests/fixtures/rule-admission/`, an evidence record, the moved counters and the README count
- [ ] Given a workspace with no `unsafe`, when scanned, then no admitted lint fires, which a test asserts on this repository's self-scan
- [ ] An admitted lint's healthy-corpus sites are adjudicated, and the rate is published as the catalog publishes rates today

#### US-033: Variant gate: every native detector keeps its verdict under rewrites
**Description:** As the catalog maintainer, I want each native detector exercised against verdict-preserving rewrites of its canonical cases, so that a detector silently missing a common variant fails `cargo test` instead of shipping.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-028

**Acceptance Criteria:**
- [ ] Every native source detector declares a canonical positive and a canonical negative, and the gate applies these rewrites:
  - rename bindings;
  - alias the import (`use x as y`);
  - use the fully qualified path;
  - wrap in a block;
  - move into an inline module;
  - extract the receiver into a local binding;
  - reformat whitespace and comments.
- [ ] For each rewrite, the positive still fires, or the pair is listed in the detector's `known_misses` with a one-line reason
- [ ] For each rewrite, the negative stays silent
- [ ] A native source detector without a declared variant table fails the gate
- [ ] The gate runs inside `cargo test`, touches no network, and completes in under 5 seconds on this repository

## Functional Requirements

- FR-01: The Clippy invocation must carry `-W` for every active catalogued Clippy rule the installed toolchain knows, after `-A clippy::all`, and must never pass `-W` for a rule the toolchain does not list.
- FR-02: A catalogued deny-by-default lint set to `off` must be absent from the invocation, and its trigger must not fail the build.
- FR-03: The system must publish every diagnostic whose code the catalog does not know with `unscored: "uncatalogued"`, and must never weigh it or let it void the score.
- FR-04: When the score is not authoritative, the system must publish at least one reason from the closed vocabulary. When it is authoritative, `reasons` must be empty.
- FR-05: Every stage error caused by a Cargo process must carry a bounded, scrubbed excerpt of that process's stderr.
- FR-06: When `--max-duration` elapses, the system must terminate every process it spawned for the scan and emit a complete report with `deadline-exceeded`.
- FR-07: When a workspace member fails to compile, the system must lint every member that compiles and name the ones it could not lint.
- FR-08: The system must remove lint-level tokens from caller-provided rustflags before invoking Clippy, and must leave rustflags untouched when there are none.
- FR-09: `--scope files`, `lines` and `baseline` must resolve a base from local git when `--base` is omitted, and must fail with `base-undetected` when none resolves.
- FR-10: With `--staged`, no producer may read a file from the working tree.
- FR-11: The system must not open the interactive report when an agent, hook or CI-provider marker is set.
- FR-12: `skill install`, `hook install` and `ci install` must write only the files they print, must never write through a symlink, and must never overwrite a file rust-doctor did not create, except for the JSON merge of `hook install claude` and `hook install cursor`, which adds one entry and keeps every other key.
- FR-13: `hook run claude` must exit 0 when `stop_hook_active` is true, or when the scan fails for a reason other than the gate.
- FR-14: A path ignored by configuration, or declared `linguist-generated` or `linguist-vendored`, must not weigh on any score, and the count of excluded diagnostics must be published.
- FR-15: A native suppression directive must suppress nothing unless it names a native rule and carries a non-empty reason.
- FR-16: `report markdown` must escape every piece of text that comes from a diagnostic.
- FR-17: The Action and the comment workflow must never use `pull_request_target`, and must never interpolate a ref or input into a `run` block.
- FR-18: The advisory check must never open a network connection. When no local database exists, it must publish its rules as not evaluated.
- FR-19: The binary must not open any network connection in any command this PRD adds.

## Non-Functional Requirements

- **Performance:**
  - A warm full scan of this repository after Release 1 takes at most 10% longer than 0.7.0 on the same machine.
  - A second consecutive baseline run on an unchanged base recompiles 0 registry dependencies.
  - A warm `--staged --scope lines` run takes at most 1.5 times a warm `--scope lines` run on the same tree.
  - The progress line redraws at most every 100 ms.
  - `hook run claude` finishes within 540 seconds, under Claude Code's 600-second default hook timeout.
  - `report markdown` renders a 10,000-diagnostic report in under 500 ms.
- **Security:**
  - Zero outbound network connections from the binary, asserted by the existing no-network test policy.
  - No absolute path, home directory or environment value in `--json`, asserted over every new field by fixture tests.
  - Workspace writes only through the three `install` subcommands.
  - No `pull_request_target` anywhere.
  - Comment permissions scoped to one job.
- **Reliability:**
  - Every new failure mode produces a complete report with a `ReportError`, and exit codes stay 0 (complete, gate passed), 1 (gate failed or incomplete) and 2 (failed).
  - Stderr pipes are drained concurrently, so no pipe of up to 65,536 bytes can deadlock.
  - A killed scan leaves no child process of its group alive.
- **Scalability:**
  - Workspaces of up to 200 members and 10,000 source files scan without any new limit being reached.
  - Diff, stderr and advisory reads stay bounded (1 MiB diff, 65,536 bytes stderr, 1 MiB per advisory, 20,000 advisories).
- **Maintainability:** every file stays under 1000 lines and every `impl` under 500, and every new document is registered in `docs/doc-budgets.json`.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Empty staged set | `--staged` with nothing staged | Report with zero in-scope findings, exit 0 | "No staged Rust change to judge." |
| 2 | Long cold build | First scan of a large workspace | Progress line on a terminal, one line per phase elsewhere | "Linting <package> (k/N)" |
| 3 | Cargo failure | Unparsable `Cargo.lock` | Stage `clippy` error with Cargo's scrubbed stderr, exit 2 | Cargo's own sentence, prefixed "Cargo reported:" |
| 4 | Hung build script | `build.rs` sleeps past `--max-duration` | Process group killed, partial report, `deadline-exceeded`, exit 2 | "The scan stopped at the 5 s limit set by --max-duration." |
| 5 | Shallow clone | CI checkout at depth 1 with `--scope baseline` | Error `shallow-clone`, exit 2 | "This clone is shallow: fetch with fetch-depth: 0, or pass --base to a fetched ref." |
| 6 | No resolvable base | Detached HEAD, no remote, no `--base` | Error `base-undetected`, exit 2 | "No base branch found: pass --base <REF>." |
| 7 | Locked or conflicted index | `--staged` during a merge conflict | Error `index-conflicted`, exit 2, never an empty pass | "The index has unmerged entries: resolve them before a staged scan." |
| 8 | Broken member | One of several members fails to compile | Others linted, `packages-unlinted` names it | "Not linted because it did not compile: <package>" |
| 9 | Caller lint flags | `RUSTFLAGS=-Dwarnings` exported | Lint-level tokens removed, warnings stay warnings | "Ignored lint flags from RUSTFLAGS: -D warnings" |
| 10 | Old toolchain | A catalogued lint unknown to the installed Clippy | Rule published as not evaluated, `rules-not-evaluated` | "N rules were not evaluated by clippy <version>." |
| 11 | Existing settings file | `.claude/settings.local.json` with other hooks | One entry added, all others kept, entry printed first | "Added a Stop hook to .claude/settings.local.json: <entry>" |
| 12 | Invalid settings JSON | Settings file that does not parse | Nothing written, exit 2 | "Could not parse .claude/settings.local.json at line L, column C: nothing was written." |
| 13 | Hook loop | Agent stops again after a blocked stop | `stop_hook_active` true, so exit 0 | None |
| 14 | Toolchain broken inside the hook | `hook run claude` and the scan exits 2 | Exit 0, one stderr note, agent not trapped | "rust-doctor could not scan (see errors); not blocking this turn." |
| 15 | Someone else's skill | `--update` on a directory whose SKILL.md is not rust-doctor's | Nothing written | "This directory holds another skill: nothing was written." |
| 16 | Missing advisory database | No cargo-audit or cargo-deny cache | Rules not evaluated, score authority unaffected | "No local RustSec database: run cargo audit once to create one." |
| 17 | Stale advisory database | Newest advisory older than 30 days | Scan runs, notice printed | "The advisory database is 55 days old." |
| 18 | Suppression without reason | `// rust-doctor: allow(rust_doctor::source::disabled_tls)` | Nothing suppressed, status `missing-reason` | "A suppression needs a reason after --." |
| 19 | Fork pull request | Pull request from a fork | Comment posted by the `workflow_run` job, no pull request code checked out | Sticky comment |
| 20 | Oversized diff | `git diff` output over 1 MiB with `--scope lines` | Error `diff-too-large`, exit 2 | "The change is too large for --scope lines: use --scope files or baseline." |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | The correctness pack changes healthy-corpus scores and breaks the `score_distribution` gate | Low | High | US-006 measures before the tag. A failure blocks the release on an Open Question and never triggers a silent lambda retune. |
| 2 | Doubling the catalog (62 to about 127) reads as abandoning curation | Medium | Medium | The pack is exactly Clippy's deny-by-default group, curated upstream, and every member carries toolchain-published evidence and a corpus measurement. The README says so in one line. |
| 3 | Clippy renames or removes a catalogued lint between releases | High | Medium | US-010 probes the lint list at runtime and publishes unknown rules as not evaluated. CI stays pinned to 1.97.1. |
| 4 | An installed hook becomes an attack vector (a malicious repository shipping its own hook) | Low | High | Only the explicit `hook install` subcommand writes. The local settings file is the default. The entry is printed before writing, and a scan never installs anything. |
| 5 | Claude Code's Stop hook schema changes | Medium | Medium | Exit code 2 with stderr, the documented and recommended form. `stop_hook_active` loop guard. The hook exits 0 on any tool failure. |
| 6 | Codex or Cursor skill locations are not officially documented | Medium | Low | US-018 ships only confirmed targets and refuses the others with a message. |
| 7 | The staged snapshot doubles build cost on large workspaces | Medium | Medium | A dedicated persistent target (`rust-doctor/staged`) keeps dependencies fresh, with the 1.5x NFR measured on this repository. |
| 8 | The website (rust-doctor-web) republishes the rule count and model and drifts | High | Medium | The rules page generates from `rules list`. The release skill's website step is part of each release. The model gate already compares the model name. |
| 9 | The comment workflow is misconfigured by users into `pull_request_target` | Low | High | Shipped templates only. The README and `docs/publishing.md` forbid the pattern, citing GitHub's guidance. |
| 10 | A local advisory database gives a false sense of coverage when stale | Medium | Medium | Its age is published, with a notice past 30 days and not-evaluated status when absent. |
| 11 | Scope creep across 33 stories | High | Medium | Three releases, each shippable alone. The Disposition table fixes what is out, and each later PRD is named. |

## Non-Goals

- **Telemetry, crash reporting, a remote score API or any network lookup** (Sentry, Axiom, Socket.dev, the GitHub API from the binary). Excluded by `AGENTS.md:38`, permanently.
- **A fix subcommand or any automatic edit of scanned code.** `skills/rust-doctor/SKILL.md` states that none is coming. The installers write configuration only.
- **Admitting the `suspicious`, `perf`, `complexity`, `style` or `pedantic` groups wholesale.** US-002 reopens the queue, and those lints follow the normal one-by-one `rule-candidate` triage, because unlike `correctness` they are warn-level and their noise is unmeasured.
- **A catalog of rustc lints (`deprecated`, `unused_must_use`, `rust_2024_compatibility`).** US-004 makes them visible and unscored. Scoring them needs a new producer prefix and a later PRD.
- **An audit mode that sees through `#[allow]`.** It requires a second `--force-warn` compilation, rejected once on cost (`tasks/prd-suppression-dependency-hygiene.md`, US-004 amendment).
- **Every gap marked "Later" or "Won't" in the Parity Audit Disposition table**, including the engineering-system gaps (Windows test port, fmt gate, Clippy matrix), which a separate PRD owns.
- **GitLab CI, Bitbucket and other providers for `ci install` and the Action.** The Markdown renderer of US-025 is provider-neutral, so a later story can reuse it.

## Files NOT to Modify

- `src/audit/density.rs`, `src/audit/ranking.rs` and the lambda table in `tests/corpus.json`: the core-v4 model and its calibration. US-005 and US-023 add fields and a per-member aggregation, never a change to the formula.
- `tests/corpus.json`: regenerated only in US-006 and in the admission stories of Release 3, from the clone cache, as `docs/catalog-and-corpus.md` prescribes. Never edited by hand.
- `src/policy/catalog/validate.rs` `TIER_WINDOWS`: new rules fit the existing windows. Widening one is a decision about what the score means, and is outside this PRD.
- `src/git.rs` `GIT_ENVIRONMENT_OVERRIDES`: append only. US-016 honors a hook-provided `GIT_INDEX_FILE` by passing it explicitly to the one call that needs it, never by removing it from the list.
- The frozen v7 archive projection in `src/report.rs`: it must keep projecting across schema 18, 19 and 20.
- `docs/*-2026-08.md` precision records: frozen measurement records. New measurements go into new frozen files.

## Technical Considerations

Framed as questions for engineering input:

- **Architecture, the correctness pack:** one pack fixture crate triggering about 60 lints may interact, for example one lint's trigger suppressing another's. The recommendation is one function per lint, each under `#[allow]` of every other correctness lint. Engineering to confirm the oracle still observes each member once.
- **Architecture, the unscored population:** US-004 recommends a separate `unscored` field over a new `DiagnosticContext` variant, because `context` means target kind today (`src/report.rs:420`). Engineering to confirm `weighs()` stays the single place that decides.
- **Architecture, killing the process group:** `std::os::unix::process::CommandExt::process_group(0)` (stable since 1.64) plus `libc::killpg` would need `libc` as a normal dependency, while it is dev-only today. An alternative is to spawn Cargo under `setsid` semantics through `process_group(0)` and kill the group by sending `SIGKILL` to the negative pid through `std::process::Command::new("kill")`. Engineering to choose the dependency-free path, or to argue for `libc` against the no-new-dependency constraint.
- **Data model, schema 18:** new fields are `diagnostics[].unscored`, `audit.score.reasons`, `toolchain.rust_doctor`, `toolchain.removed_lint_flags` and `policy.rules[].not_evaluated`. Should `not_evaluated` be an object carrying a reason and detail, instead of a string? The recommendation is a string drawn from a closed enum, matching `errors[].code`.
- **API design, the CLI surface:** `--staged` is a modifier, not a scope value, because it composes with three scopes. `hook` and `ci` become subcommands beside `rules` and `skill`. Engineering to confirm the exit-code and clap oracle (`src/policy.rs`) is updated once per release.
- **Dependencies, globs:** a matcher for `*`, `**` and `?` of under 150 lines is recommended over `globset`, to keep the no-new-dependency constraint. Does any user need brace expansion or negation? If so, Release 2 reopens the question.
- **Migration:** `rust-doctor.toml` files from 0.7 stay valid, since every new table is optional. Reports of schema 17 are not readable by `report markdown` of a newer binary, which is refused explicitly. Rollback is to pin the previous npm version. No data migration.
- **Measurement:** US-006 and the Release 3 admissions need the clone cache at `~/.cache/rust-doctor/corpus`. Engineering to confirm the cache is complete for all 18 repositories before starting US-006.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Clippy `correctness` members reported by a scan | 0 of 67 (1.97.1) | Every std-triggerable member (US-001 count) | Month-1 (0.8.0) | `rust-doctor rules list --json`, count of ids in the pack |
| Correctness members decided (admitted or queued with a reason) | 0 of 67 | 67 of 67 | Month-6 | `docs/correctness-group-2026-09.md` against the catalog |
| Agent-population reports that are non-authoritative with no reason | 2 of 8 | 0 of 8 | Month-1 | `tests/corpus.json` after US-006 |
| Failure fixtures whose error carries Cargo's cause | 0 of 5 | 5 of 5 | Month-1 | EP-002 integration tests |
| Second baseline run, registry dependencies recompiled | All of them (cold temporary target) | 0 | Month-1 | `fresh` flags in Cargo's JSON stream, US-011 test |
| Agent integrations with an end-of-turn rescan | 0 | Claude Code and Cursor | Month-1 | `hook install` tests |
| Pull requests on this repository with a rust-doctor comment | 0% | 100% of pull requests touching `src/` | Month-6 | GitHub comments marked `<!-- rust-doctor -->` |
| Native detectors passing the variant gate | 0 of 2 (no gate) | Every native source detector | Month-6 | `cargo test` variant-gate test |
| Catalogued rules | 62 | About 127 after Release 1, plus 4 to 7 in Release 3 | Month-6 | README count and `rules list` |

## Open Questions

- **Should the admitted correctness members all sit at tier P1?** The pack assigns `P1` uniformly. Some members flag useless rather than wrong code (`approx_constant`), which could argue for `P2`. Arthur to decide before US-003 starts. Only the pack's tier table depends on it.
- **What happens if US-006 finds the `score_distribution` gate failing?** Retune the lambdas under a new model name (core-v5), or ship with the gate relaxed and recorded? Arthur to decide before the 0.8.0 tag. The release waits on the answer.
- **Should the persistent baseline and staged targets be pruned?** They grow with toolchain changes. Recommended: `cargo clean` covers them, since they live under the target directory, and nothing is added. Engineering to confirm during US-011.
- **Is the `workflow_run` comment job acceptable against the no-network positioning?** The binary stays offline and the workflow posts with GitHub's token. Arthur to confirm before US-027, or to cut the comment and keep the job summary and outputs only.
- **Codex and Cursor skill targets:** which directories do their current official docs name? This is answered inside US-018 and fixes that story's target table.
[/PRD]
