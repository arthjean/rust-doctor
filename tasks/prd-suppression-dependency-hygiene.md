[PRD]
# PRD: Suppression Audit, Dependency Truth, and Repository Hygiene

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-10 | Arthur Jean | Initial draft |
| 1.1 | 2026-08-10 | Arthur Jean | US-004 cancelled on a measured toolchain behavior; the catalog target moves from 63 to 62 rules |

## Problem Statement

1. **The score can be raised by silencing the scanner instead of fixing the code.** A single `#![allow(clippy::all)]` at the top of `lib.rs`, or a `[lints.clippy] unwrap_used = "allow"` line in `Cargo.toml`, removes every diagnostic those rules would produce and leaves rust-doctor reporting a clean workspace. The catalog holds exactly one suppression rule today, `rust_doctor::structure::unreasoned_allow_attribute` (`src/policy/catalog.rs:478`), and it only asks whether an `allow` states a reason. It never asks whether the `allow` covers an entire crate, whether several are stacked on one item, whether the manifest turned a catalogued rule off, or whether the `allow` suppresses anything at all. An agent that cannot make a lint pass has one obvious move available, and the tool currently rewards it.

2. **Nothing in the tool answers what the manifest declares versus what the code uses.** `src/cargo_health.rs` reads dependency entries for pinning and duplication (`unpinned_git_dependency`, `unbounded_registry_dependency`, `duplicate_major_versions`, `path_dependency_outside_workspace`, `missing_lockfile`), and stops there. A crate declared and never referenced still compiles, still resolves, still ships in `Cargo.lock`, and costs every downstream build. A crate used only under `#[cfg(test)]` but declared in `[dependencies]` costs every consumer of the library the same way. `rustc`'s `unused_crate_dependencies` is allow-by-default because of structural false positives ([rust#72686](https://github.com/rust-lang/rust/issues/72686), [rust#78346](https://github.com/rust-lang/rust/issues/78346)), and cargo's own `[lints.cargo] unused_dependencies` ([cargo#16600](https://github.com/rust-lang/cargo/pull/16600)) is still nightly-gated: verified on 2026-08-10 against cargo 1.97.1, where the key produces `warning: unused manifest key lints.cargo (may be supported in a future version)`. The signal exists and no stable tool in the default toolchain emits it.

3. **The build profile is invisible to the scan.** `[profile.release]` ships with `overflow-checks = false`, `debug = false`, `strip = "none"` ([Cargo book, profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)), and a `.cargo/config.toml` can add `rustflags` that cap or silence lints for every build in the workspace. rust-doctor scores the source and ignores the settings under which that source is compiled and shipped, including settings that neutralize its own findings.

4. **The repository outside `.rs` files is unexamined.** `source_kernel::enumerate` (`src/source_kernel.rs:218`) starts from Cargo target entry points and follows `mod` declarations. It never walks a directory and never opens a non-Rust file. A `.env` tracked by git, a private key committed next to the manifest, or `target/` absent from `.gitignore` are all invisible to a tool whose README opens with "secrets-adjacent patterns like disabled TLS verification".

**Why now:** the structural pass shipped and measured (`docs/structural-precision-2026-08.md`), which proves the admission machinery scales past Clippy wrappers to native families. Problem 1 is the one that compounds: every rule added from here is worth exactly as much as the tool's ability to notice it was switched off. Fixing the suppression blind spot before growing the catalog further is cheaper than fixing it after.

## Overview

This PRD adds eleven rules across four families and one new producer, taking the catalog from 51 to 62 rules. It creates no new binary, no new crate, no configuration surface, and no report section: every finding is a `Diagnostic` on the existing path, subject to the existing policy overrides, gate, baseline, and score.

**Family 1, suppression audit,** extends the structural detector table (`src/structure.rs:187`) with two detectors and adds one manifest rule. The two read attributes the way `unreasoned_allow` already does (`src/structure.rs:489-539`). A fourth rule, dead-allow detection, was specified here and cancelled on 2026-08-10 before implementation; the amendment under US-004 records the measurement that killed it.

**Family 2, dependency truth,** adds a coarse crate-reference collector over the enumeration the source kernel already builds, and joins it per package against `package.dependencies`. This deliberately overturns an explicit Non-Goal of the previous PRD (`tasks/prd-structural-slop-detection.md:74`, "cargo-machete and cargo-shear own this"). The reason is factual and dated: the stable toolchain does not emit this diagnostic today, and the misclassification half of the family (a dependency used only in test code) is excluded from cargo's design by choice, so it will not become redundant even when `[lints.cargo]` stabilizes.

**Family 3, release profile hardening,** and **family 4, repository hygiene,** both read what no current pass reads. Profiles are absent from `cargo metadata` output, so the manifest is re-parsed with `toml` using `Spanned` to keep byte offsets for the diagnostic span. Repository hygiene enumerates through `git ls-files` rather than a filesystem walker, because the question asked is literally "is this file tracked", and because it excludes `target/` and ignored paths by construction rather than by a filter that can drift.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Catalogued rules | 62 (37 Clippy, 16 native, 9 structural) | 62, none withdrawn |
| A blanket crate-level suppression is visible in the report | A fixture carrying `#![allow(clippy::all)]` in `lib.rs` yields at least 1 diagnostic and a score below 100 | Same, plus measured on the pinned corpus |
| Adjudicated precision published per new rule | 11 of 11 rules carry a corpus verdict or an explicit `unproven` | Unchanged, re-measured after any rule edit |
| Security-tier rules admitted without a confirmed false positive | 2 of 2 (`tracked_secret_file`, `hardcoded_credential`) at 0 confirmed FP over 20 adjudicated sites | Same, re-measured on the agent corpus |
| Added scan wall-clock on this repository | +25% or less over the current `cargo run --release -- . --yes` | +25% or less |

## Target Users

### Vibecoder shipping Rust with an agent
- **Role:** builds and ships Rust without a senior reviewer, relies on an agent for most of the code.
- **Behaviors:** runs the scan, fixes the top rules, ships. Accepts what the agent proposes when the build turns green.
- **Pain points:** cannot tell an agent fixing a lint from an agent silencing it, because both end with a clean build and a passing scan. Has no idea which of the twelve crates in `Cargo.toml` are actually used.
- **Current workaround:** none. Trusts the score, which is exactly what this PRD makes trustworthy.
- **Success looks like:** the scan says "this allow covers the whole crate" or "serde_yaml is declared and never referenced" in the same list as everything else, with no extra tool to install.

### Rust engineer supervising agent output
- **Role:** reviews agent-authored branches, owns whether they merge.
- **Behaviors:** reads diffs, runs Clippy, occasionally runs cargo-machete or gitleaks by hand when something feels off.
- **Pain points:** `#[allow]` additions hide in a large diff and read as noise; four separate tools means four separate installations, four output formats, and no shared gate.
- **Current workaround:** grep for `allow(` in the diff, run `cargo machete` manually, trust that no one committed a `.env`.
- **Success looks like:** `--scope baseline` fails a pull request that adds a suppression or a stray dependency, in the same run that already checks the rest.

### Maintainer triaging agent-authored contributions
- **Role:** accepts or rejects outside contributions to a Rust project.
- **Behaviors:** wants one command whose output can be pasted into a review.
- **Pain points:** a contributor who silences a lint to make CI pass is indistinguishable from one who fixed the underlying problem.
- **Success looks like:** a suppression added by the contribution is reported as a finding of the contribution, not as pre-existing backlog.

## Research Findings

### Competitive Context
- **cargo-machete** (~458k recent downloads): textual search for the crate identifier under `src/`; no build required; documented false positives on macro-only use, build-script-only use, and renamed dependencies; ignore list via `[package.metadata.cargo-machete]`. *How we differ:* same syntactic posture, but the finding joins one score, one gate, and one baseline instead of a separate exit code.
- **cargo-shear** (~62k): `cargo_metadata` plus the rust-analyzer parser to extract real `use` paths; no macro expansion. *How we differ:* rust-doctor already parses with `ra_ap_syntax` and already holds the enumeration, so the collector is incremental work rather than a second parse of the workspace.
- **cargo-udeps**: compiler-driven, most accurate, requires nightly and a full build. *How we differ:* rust-doctor requires neither, and accepts the resulting abstentions as published noise rather than hiding them.
- **rustc `unused_crate_dependencies`**: allow-by-default precisely because dev-dependencies are flagged when compiling tests ([rust#129637](https://github.com/rust-lang/rust/issues/129637)) and doctest-only deps are missed ([rust#78346](https://github.com/rust-lang/rust/issues/78346)). *How we differ:* we treat those as the known false-positive classes to abstain on, and we measure the residue.
- **cargo `[lints.cargo] unused_dependencies`** ([cargo#16600](https://github.com/rust-lang/cargo/pull/16600), merged 2026-04-06): covers `[dependencies]` and `[build-dependencies]`, deliberately excludes dev-dependencies. Verified nightly-only on cargo 1.97.1 as of 2026-08-10. *How we differ:* available on stable today, and the dev-dependency half stays uncovered by cargo's own design.
- **gitleaks / TruffleHog**: gitleaks is fully offline (regex plus Shannon entropy); TruffleHog's precision edge comes from live API verification. One comparative study counted 635 gitleaks false positives against 370 for TruffleHog ([arXiv 2307.00714](https://arxiv.org/pdf/2307.00714)). *How we differ:* the no-network invariant forbids verification, so we ship only high-confidence prefixed patterns and no entropy-only trigger, and we accept lower recall as the price of a publishable precision number.
- **Clippy itself**: `clippy::allow_attributes` and `clippy::allow_attributes_without_reason` exist; `#[expect]` (stable since 1.81) makes rustc report an unfulfilled expectation. *How we differ:* nothing upstream reports a dead `#[allow]`, which is exactly what ESLint's `--report-unused-disable-directives` and Ruff's `RUF100` do for their ecosystems.

### Best Practices Applied
- **Suppression is a first-class quality signal, not metadata.** ESLint and Ruff both ship a dedicated unused-suppression check. Rust has the `#[expect]` half and not the `allow` half.
- **Publish the tradeoff, do not assert the defect.** The Rust Performance Book measures overflow checks at "a few percent" on integer-heavy code; a rule that calls their absence a defect is wrong. The help text names the cost and the rule sits at P3.
- **Ship an escape hatch before shipping the detector.** Every serious unused-dependency tool has one; rust-doctor's per-rule `off` override and `rust-doctor.toml` already provide it, so no new configuration surface is needed.
- **Prefer the mechanism that cannot drift.** `git ls-files` answers "tracked" exactly, where a filesystem walker plus an ignore filter answers it approximately.
- **Abstain loudly rather than guess.** A crate reference behind a macro or a `cfg` the tool cannot evaluate produces no finding, and the abstention classes are named in the help text.

*Full research sources: Cargo book (profiles), Rust Performance Book (build configuration), Clippy lint list, cargo#16600, rust#72686 / #78346 / #95513 / #129637, clippy#13348 / #16488, arXiv 2307.00714, cargo-machete and cargo-shear READMEs. Toolchain behavior verified locally on cargo 1.97.1 / rustc 1.97.1, 2026-08-10.*

## Assumptions & Constraints

### Assumptions (to validate)

- **HIGH: the suppression family adjudicates below 20% false positives on healthy public Rust.** Healthy code documents exemptions in ways no detector reads, which is exactly what sank the five measured structural rules to between 60% and 100% FP (`docs/structural-precision-2026-08.md`). Crate-level `allow` and permissive `[lints]` are structurally different, being manifest-level and file-level declarations rather than judgments about a call site, but this is reasoning, not measurement. US-017 validates. If it fails, the affected rules ship with a `noisy_on_healthy_code` verdict rather than being withdrawn.
- **HIGH: a syntactic crate-reference collector reaches usable precision without name resolution.** cargo-machete does less (plain text search) and is widely used; cargo-shear does roughly this. Neither publishes an FP rate. US-017 measures ours.
- **REFUTED on 2026-08-10: dead-allow detection can be scoped to catalogued, active rules without becoming useless.** The scoping was never the binding constraint. `rustc` honors `#[allow]` over a `-W` passed on the command line, so a live exemption and a dead one both leave zero diagnostics in their scope and the join has no discriminating signal at all. US-004 is cancelled; the amendment there records the measurement.
- **MEDIUM: high-confidence prefixed secret patterns produce zero false positives on the pinned corpus.** Ten curated public repositories are unlikely to carry a committed credential, so the likely outcome is `unproven` rather than a precision number. US-017 reports which.
- **LOW: `git ls-files` is available wherever a scan runs.** `src/git_scope.rs:208` already shells out to `git`, and `--scope files` already depends on it. A workspace outside a git repository yields no repository-hygiene findings and no error.

### Hard Constraints
- Single crate, edition 2024, rustc 1.95 or later. No new workspace member, no new binary.
- No new dependency. `ra_ap_syntax`, `cargo_metadata`, `toml` and `blake3` are already pinned and carry this work. `toml` already has the `serde` feature, which is what re-exports `serde_spanned::Spanned`.
- Dependencies pinned exactly (`= 1.8.5` form), `Cargo.lock` committed.
- No network, no telemetry. `--json` stays workspace-relative with no absolute path and no environment value. This is what forbids a Socket-style supply-chain score and what forbids TruffleHog-style secret verification.
- Production code carries no `unwrap`, `expect`, `panic!` or `dbg!`; `tests/score_credibility_packs.rs` enforces it by scanning this repository.
- No catalogued rule is `deny` by default (`no_catalogued_clippy_rule_is_denied_by_default`).
- Every new rule sits inside its category's `TIER_WINDOWS` band (`src/policy/catalog.rs:580-587`): security P0 to P1, correctness and dependencies P1 to P2, performance and reliability P2 to P3, maintainability P3 only.
- Every new rule carries a `tests/rule_evidence.json` entry with a `catches` line and a pointer that resolves, enforced by `tests/rule_admission.rs`.
- `src/policy/catalog.rs` and `tests/corpus.json` are regenerated together (`the_published_catalog_matches_the_shipped_policy`).
- Any change to the report shape bumps `SCHEMA_VERSION` in `src/report.rs` (currently 13).
- Every test that runs `cargo` or the built binary sets its own `CARGO_TARGET_DIR`. Integration test crates open with `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`.
- No code copied from cargo-machete, cargo-shear, gitleaks or react-doctor. Published algorithm descriptions may be reimplemented.

## Quality Gates

These commands must pass for every user story:
- `cargo build --release` - the crate compiles
- `cargo test` - the full suite including catalog, admission and oracle invariants
- `cargo clippy --all-targets --no-deps -- -D warnings` - must be clean, no exceptions
- `cargo run --release -- . --yes --verbose` - the tool runs on its own repository without error

## Epics & User Stories

### EP-001: Suppression audit

Make a suppression visible as a finding. Three rules: a crate-level or module-level `allow` that covers a whole file, several `allow` attributes stacked on one item, and a `[lints]` table that turns a catalogued rule off. A fourth, an `allow` that suppresses nothing, was cancelled on 2026-08-10 and its amendment stays below rather than being deleted.

**Definition of Done:** a fixture workspace whose only defect is a blanket `#![allow(clippy::all)]` produces at least one diagnostic and a score below 100, and all three rules are catalogued, evidenced and covered by tests.

**Risk 8, settled on 2026-08-10.** The self-scan answers the question the risk row asks. `crate_level_allow` fires twice on this repository, on `tests/presentation_nfr.rs` and `tests/support/mod.rs`, and both are true positives: each carries a genuine file-wide inner `allow`. Neither is a defect to correct, because both sit in a test target, carry the non-production context marker, and leave the score at 94, unchanged from the pre-epic run. `stacked_allow_attribute` and `permissive_lint_table` stay silent here: this repository's `[lints.clippy]` table sets no catalogued rule to `allow`, and its `#![cfg_attr(test, allow(...))]` lines are out of reach of every attribute detector by construction.

#### US-001: Crate-level and module-level allow detector
**Description:** As a reviewer, I want an `#![allow(...)]` that covers an entire file reported as a finding, so that a blanket suppression is as visible as the code it hides.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a file whose root carries `#![allow(clippy::unwrap_used)]`, when the structural pass runs, then `rust_doctor::structure::crate_level_allow` reports one finding at the attribute span naming the lints it covers
- [ ] Given `#![allow(...)]` inside an inline `mod` block, when the pass runs, then the finding is reported with the module as its subject
- [ ] Given an outer `#[allow(...)]` on a single item, when the pass runs, then this rule reports nothing, since US-002 and the existing `unreasoned_allow_attribute` own that case
- [ ] Given an inner attribute that is not `allow` (`#![deny(...)]`, `#![warn(...)]`, `#![doc = "..."]`), when the pass runs, then nothing is reported
- [ ] Given an inner `#![allow(..., reason = "...")]` with a stated reason, when the pass runs, then the finding is still reported, because the scope, not the justification, is what this rule judges, and the help text says so
- [ ] Given a file the parser rejects, when the pass runs, then the file is skipped and every other file is still analyzed
- [ ] Given a test, bench, example or build-script target, when a finding lands there, then it is reported with a context marker and does not weigh on the score
- [ ] The finding key contains no line, column or path, so `--scope baseline` does not move it when lines are inserted above

#### US-002: Stacked allow detector
**Description:** As a reviewer, I want an item carrying several suppressions at once reported, so that an accumulation of exemptions on one item reads as one deliberate signal.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given an item carrying two or more separate `#[allow(...)]` attributes, when the pass runs, then `rust_doctor::structure::stacked_allow_attribute` reports one finding for that item
- [ ] Given a single `#[allow(a, b, c, d)]` naming four or more lints, when the pass runs, then the same rule reports one finding, since one attribute listing four lints and four attributes are the same act
- [ ] Given an item with exactly one `#[allow]` naming one lint, when the pass runs, then nothing is reported
- [ ] Given `#[cfg_attr(test, allow(...))]`, when the pass runs, then it is not counted, and the limitation is stated in the rule help, matching the existing `unreasoned_allow` behavior (`src/structure.rs:486-488`)
- [ ] Given an item inside a test context, when a finding lands there, then it carries the context marker and does not weigh

#### US-003: Permissive lint table in the manifest
**Description:** As a reviewer, I want a `[lints]` table that switches a catalogued rule off reported as a finding, so that a suppression moved from the source into the manifest is not a way to hide from the scan.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a `Cargo.toml` with `[lints.clippy] unwrap_used = "allow"` where `clippy::unwrap_used` is catalogued and active, when cargo health runs, then `rust_doctor::cargo::permissive_lint_table` reports one finding naming the rule and the manifest
- [ ] Given `[lints.rust] unsafe_code = "allow"` naming a lint the catalog does not carry, when the pass runs, then nothing is reported, because the tool only judges what it would otherwise have said
- [ ] Given `[lints.clippy] unwrap_used = { level = "allow", priority = -1 }` in table form, when the pass runs, then it is recognized the same as the string form
- [ ] Given a workspace where a member declares `[lints] workspace = true`, when the pass runs, then the workspace root's `[workspace.lints]` is the table that is judged, and the finding names the root manifest once rather than once per member
- [ ] Given a manifest that `toml` cannot parse, when the pass runs, then a bounded `ReportError` at stage `dependencies` is recorded and the rest of the scan completes
- [ ] The diagnostic points at the offending key with a span derived from `Spanned`, and the path is workspace-relative

#### US-004: Dead allow detector (CANCELLED 2026-08-10)

**Amendment, 2026-08-10.** Cancelled before implementation, on a measurement rather than on a cost estimate. `rustc` applies `#[allow]` over a `-W` passed on the Clippy command line: a probe crate carrying `#[allow(clippy::unwrap_used)]` on one `unwrap` and a bare `unwrap` beside it, scanned with `-W clippy::unwrap_used` on clippy 1.97, reports the bare site and nothing else. A dead exemption and a live one therefore produce the same observation, zero diagnostics in scope, so the join this story specifies cannot separate them and its first two acceptance criteria are mutually unsatisfiable. ESLint's `--report-unused-disable-directives` and Ruff's `RUF100`, the precedents the Research Findings name, evaluate the rule first and check the directive second; rust-doctor holds only the second half.

Three routes were weighed and none is worth a P1 rule. A second Clippy pass under `--force-warn`, whose diagnostics decide dead against live and are never published, is correct by construction and costs a full second compilation of the workspace, against the +25 % wall-clock target. A single `--force-warn` pass with rust-doctor applying the suppression itself costs no time and moves the semantics of `#[allow]` out of the compiler and into the tool, across module scopes, `cfg_attr`, macro spans and tool lints, where one mistake publishes a finding the user correctly silenced. Reimplementing the 37 lints syntactically contradicts the producer model. The story is cut rather than approximated.

A cheaper rule covering most of the intent stays available and deliberately unspecified here: `#[expect]` is stable since 1.81 and rustc's own `unfulfilled_lint_expectations` reports an unmet expectation natively, so a rule stating that an `allow` could be an `expect`, which would then expire by itself, needs no join and no second pass. It is a different rule from the one below and belongs to a later PRD.

The original specification follows, unedited, as the record of what was intended.

**Description:** As a reviewer, I want an `#[allow]` that suppresses nothing reported, so that exemptions left behind after the code was fixed do not accumulate as permanent blind spots.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001, US-003

**Acceptance Criteria:**
- [ ] Given `#[allow(clippy::unwrap_used)]` on a function containing no `unwrap`, and `clippy::unwrap_used` active in the plan, when the report is assembled, then `rust_doctor::structure::dead_allow_attribute` reports one finding
- [ ] Given the same attribute on a function that does contain an `unwrap`, when the report is assembled, then nothing is reported
- [ ] Given `#[allow(clippy::needless_range_loop)]`, a lint rust-doctor does not run, when the report is assembled, then nothing is reported, and the help text states that only catalogued active rules are judged
- [ ] Given a scan where the Clippy pass failed or was disabled by policy, when the report is assembled, then this rule produces no finding at all rather than reporting every allow as dead
- [ ] Given a `--scope files` or `--scope baseline` run where Clippy only saw part of the workspace, when the report is assembled, then the rule only judges attributes inside the files Clippy actually scanned
- [ ] Given an `#[expect(...)]` rather than `#[allow(...)]`, when the report is assembled, then nothing is reported, since rustc's `unfulfilled_lint_expectations` already owns that case

---

### EP-002: Dependency truth

Answer what the manifest declares against what the code references. Two rules: a dependency nothing references, and a dependency only test code references.

**Definition of Done:** both rules are catalogued and evidenced, the collector abstains explicitly on the four known-hard classes (macro-only, build-script-only, `cfg`-gated, re-exported), and the previous PRD's Non-Goal is amended in writing.

#### US-005: Crate-reference collector over the enumeration
**Description:** As the dependency rules, I want a per-package set of crate names the source actually references, so that a manifest entry can be judged against evidence rather than against a text search.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a unit containing `use serde::Serialize;`, when the collector runs, then `serde` is recorded as referenced by that unit's packages
- [ ] Given a fully qualified path `::serde_json::to_string(...)` with no `use`, when the collector runs, then `serde_json` is recorded
- [ ] Given `extern crate libc;`, when the collector runs, then `libc` is recorded
- [ ] Given a macro invocation `serde_json::json!({})`, when the collector runs, then `serde_json` is recorded
- [ ] Given a dependency renamed in the manifest (`registry_alias = { package = "real_crate" }`), when the collector runs, then a reference to `registry_alias` credits the `real_crate` entry, matching the `rename` field already exercised in `src/cargo_health.rs` tests
- [ ] Given a hyphenated crate name in the manifest, when the collector runs, then its underscored form in source is matched
- [ ] Given a unit that fails to parse, when the collector runs, then that unit contributes no references and marks its packages as incompletely collected, so the dependency rules abstain for those packages rather than reporting every dependency as unused
- [ ] The collector reuses the existing enumeration from `source_kernel::enumerate` and triggers no second walk of the workspace

#### US-006: Unused dependency rule
**Description:** As a developer, I want a declared dependency that nothing references reported, so that the manifest describes what the code actually needs.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] Given a package declaring a dependency no unit of that package references, when the pass runs, then `rust_doctor::cargo::unused_dependency` reports one finding naming the dependency and the manifest
- [ ] Given a dependency referenced only from a build script, when the pass runs, then nothing is reported, because build-script sources are enumerated as their own target
- [ ] Given an optional dependency behind a feature the scan did not activate, when the pass runs, then nothing is reported
- [ ] Given a `-sys` crate or any dependency referenced nowhere but required for linking, when the pass runs, then it is reported, and the help text names the per-rule `off` override and `rust-doctor.toml` as the escape hatch
- [ ] Given a package whose collection was marked incomplete by US-005, when the pass runs, then no finding is produced for that package
- [ ] Given a workspace with several members, when the pass runs, then each member is judged against its own dependency table, and a dependency used by one member is not credited to another

#### US-007: Test-only dependency rule
**Description:** As a library author, I want a dependency used only by test code but declared in `[dependencies]` reported, so that consumers do not compile what only the test suite needs.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] Given a dependency in `[dependencies]` referenced only from units whose Cargo target is a test, bench or example, when the pass runs, then `rust_doctor::cargo::test_only_dependency` reports one finding
- [ ] Given the same dependency also referenced from a lib or bin target, when the pass runs, then nothing is reported
- [ ] Given a dependency referenced only inside an inline `#[cfg(test)] mod tests` block of a lib target, when the pass runs, then it is reported, and the finding names the inline test module as the only reference site
- [ ] Given a dependency already declared in `[dev-dependencies]`, when the pass runs, then it is never a candidate for this rule
- [ ] Given a dependency referenced nowhere at all, when the pass runs, then US-006 owns it and this rule stays silent, so a single manifest entry never produces two findings

---

### EP-003: Release profile hardening

Read the settings under which the code is compiled and shipped. Three rules covering the release profile and the workspace-wide rustflags.

**Definition of Done:** the manifest reader returns spans for profile keys, all three rules are catalogued and evidenced, and none of them is blocking by default.

#### US-008: Spanned manifest reader
**Description:** As the profile rules, I want the manifest re-parsed with byte spans, so that a finding about `[profile.release]` can point at the line that carries it.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a `Cargo.toml` with `[profile.release]`, when the reader runs, then the profile values and the byte span of each key are returned
- [ ] Given a byte offset, when converted, then the resulting line and column match what the existing `SourceSpan` helpers produce elsewhere in the crate
- [ ] Given a manifest larger than a bounded size limit, when the reader runs, then it refuses to read further and returns a bounded error rather than allocating without limit, matching the 4 MiB lockfile cap in `src/cargo_health.rs:21`
- [ ] Given a manifest that is not valid TOML, when the reader runs, then a bounded `ReportError` at stage `dependencies` is recorded, carrying no absolute path, and the rest of the scan completes
- [ ] Given a virtual workspace manifest with no `[package]`, when the reader runs, then `[profile.*]` is still read from it, since that is where a workspace declares profiles
- [ ] No new dependency is added; `toml` with its existing `serde` feature supplies `Spanned`

#### US-009: Release profile rules
**Description:** As a developer shipping a binary, I want release-profile settings that weaken the shipped artifact reported, so that the build configuration is part of the health picture.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-008

**Acceptance Criteria:**
- [ ] Given a workspace producing a binary with no `overflow-checks = true` under `[profile.release]`, when the pass runs, then `rust_doctor::cargo::unchecked_release_overflow` reports one finding whose help names the measured cost ("a few percent on integer-heavy code") so the reader can decide against it
- [ ] Given `overflow-checks = true`, or a workspace producing no binary target, when the pass runs, then nothing is reported
- [ ] Given `[profile.release] debug = true` or `debug = "full"` with `strip` unset or `"none"`, when the pass runs, then `rust_doctor::cargo::release_debug_symbols` reports one finding stating that absolute build paths ship inside the binary
- [ ] Given `strip = "symbols"` or `strip = true` alongside `debug = true`, when the pass runs, then nothing is reported
- [ ] Given a profile inherited through `[profile.release-lto] inherits = "release"`, when the pass runs, then only `[profile.release]` itself is judged, and the help states that inheriting profiles are not resolved
- [ ] Both rules default to `warn`, sit at tier P3 in `reliability`, and cap neither the dimension nor the overall score
- [ ] Given no `[profile.release]` section at all, when the pass runs, then `unchecked_release_overflow` still reports, since the Cargo default is `false`, and `release_debug_symbols` does not, since the Cargo default is `debug = false`

#### US-010: Permissive rustflags
**Description:** As a reviewer, I want a `.cargo/config.toml` that disables checks for every build reported, so that a workspace cannot silence the toolchain outside the manifest.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-008

**Acceptance Criteria:**
- [ ] Given a `.cargo/config.toml` containing `rustflags = ["--cap-lints", "allow"]`, when the pass runs, then `rust_doctor::cargo::permissive_rustflags` reports one finding naming the flag
- [ ] Given `rustflags = ["-A", "warnings"]` or `["-Awarnings"]`, when the pass runs, then it is reported in both spellings
- [ ] Given `rustflags = ["-C", "overflow-checks=off"]`, when the pass runs, then it is reported
- [ ] Given rustflags that carry none of the closed list of neutralizing flags, when the pass runs, then nothing is reported, and the closed list is stated in the rule help
- [ ] Given no `.cargo/config.toml`, or one outside the workspace root, when the pass runs, then nothing is reported and no error is raised
- [ ] Given a `.cargo/config.toml` that is unreadable or malformed, when the pass runs, then a bounded error is recorded and the scan completes
- [ ] The rule sits at tier P2 in `reliability`, because it neutralizes the scan itself rather than weakening the artifact

---

### EP-004: Repository hygiene

Open the repository, not only its Rust files. One new producer, one enumeration through git, three rules.

**Definition of Done:** the fifth producer is registered end to end (catalog, plan, execution, report, error stage), all three rules are catalogued and evidenced, and a workspace outside a git repository produces no finding and no error.

#### US-011: Repository pass wired as a fifth producer
**Description:** As the hygiene rules, I want a pass that enumerates the files git tracks, so that findings about non-Rust files reach the report on the same path as every other finding.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a git repository, when the pass runs, then it enumerates tracked paths through `git ls-files` reusing the invocation pattern of `src/git_scope.rs:208`, and never walks the filesystem
- [ ] Given a `Producer::Repo` variant, when the catalog is validated, then every rule whose id starts with `rust_doctor::repo::` is required to carry it, and any other prefix is refused, matching the existing per-producer prefix check
- [ ] Given a workspace that is not a git repository, when the pass runs, then it returns no finding, records no error, and the scan is still reported as complete and authoritative
- [ ] Given a repository with more tracked files than a stated bound, when the pass runs, then enumeration stops at the bound and records a bounded error, so a monorepo cannot turn a scan into a hang
- [ ] Given a pass failure of any kind, when the report is assembled, then a `ReportError` at stage `repo` is recorded, every other pass still reports, and the score drops its authoritative flag
- [ ] Given `git` is absent from `PATH`, when the pass runs, then it degrades to no findings with a recorded error rather than failing the scan
- [ ] Diagnostics from this pass carry a workspace-relative path and no span where the finding is about the file itself

#### US-012: Tracked secret file
**Description:** As a developer, I want a secret-bearing file that git tracks reported, so that a credential committed by mistake is caught before it spreads.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-011

**Acceptance Criteria:**
- [ ] Given a tracked `.env`, `.env.local`, `*.pem`, `*.key`, `id_rsa` or `credentials.json`, when the pass runs, then `rust_doctor::repo::tracked_secret_file` reports one finding per path
- [ ] Given `.env.example`, `.env.template` or `.env.sample`, when the pass runs, then nothing is reported
- [ ] Given a `.pem` under a directory whose path segment marks it as test material (`tests/`, `fixtures/`, `testdata/`), when the pass runs, then nothing is reported, since a test certificate is the fixture, not the leak
- [ ] Given an untracked `.env` present on disk, when the pass runs, then nothing is reported, because the file was correctly kept out of the repository
- [ ] The rule sits at tier P1 in `security`, so a hit caps the security dimension at 50 and the overall score at 65 without collapsing it to 40
- [ ] The finding names the path and never quotes the file contents, so the report leaks nothing that the report itself would then carry

#### US-013: Unignored build output
**Description:** As a developer, I want `target/` missing from `.gitignore` reported, so that a repository does not start committing build artifacts.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** Blocked by US-011

**Acceptance Criteria:**
- [ ] Given a workspace whose `.gitignore` does not cover `target/` and where `git check-ignore` confirms the target directory is not ignored, when the pass runs, then `rust_doctor::repo::unignored_build_output` reports one finding
- [ ] Given `target` ignored through a global gitignore or `.git/info/exclude` rather than the committed `.gitignore`, when the pass runs, then nothing is reported, because the question is whether builds stay out, not which file says so
- [ ] Given a workspace with no `.gitignore` at all and no git repository, when the pass runs, then nothing is reported
- [ ] Given a custom `target-dir` set in `.cargo/config.toml`, when the pass runs, then that directory is the one checked
- [ ] The rule sits at tier P3 in `maintainability` and caps nothing

#### US-014: Hardcoded credential
**Description:** As a developer, I want a constant shaped like a real credential reported, so that a key pasted into source is caught by the same scan as everything else.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-011

**Acceptance Criteria:**
- [ ] Given a string literal matching a closed list of high-confidence prefixed patterns (`AKIA`, `ghp_`, `github_pat_`, `sk-`, `xoxb-`, `-----BEGIN * PRIVATE KEY-----`), when the pass runs, then `rust_doctor::repo::hardcoded_credential` reports one finding at the literal
- [ ] Given a high-entropy string with no recognized prefix, when the pass runs, then nothing is reported, because entropy alone is the documented source of gitleaks' false positives and no network verification is available to offset it
- [ ] Given a match inside a test, fixture or example path, when the pass runs, then the finding carries the non-production context marker and does not weigh on the score
- [ ] Given a match in a file the pass could not read as UTF-8, when the pass runs, then the file is skipped and the scan completes
- [ ] The finding reports the path, the line, and the matched pattern name, and never the matched value
- [ ] The rule sits at tier P1 in `security`
- [ ] The pass reads only files git tracks, so a `.env` present but untracked is never scanned for this rule either

---

### EP-005: Admission, measurement and publication

Take all eleven rules through the repository's admission contract, move every hard-coded counter, and publish the measured precision.

**Definition of Done:** `cargo test` is green with 62 catalogued rules, every rule resolves an evidence pointer, `tests/corpus.json` matches the shipped policy, and the measured precision of the eleven rules is published.

#### US-015: Fixtures and evidence records for all eleven rules
**Description:** As the admission contract, I want every new rule to point at a place where a test has seen it fire, so that no rule ships on intent alone.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-007, US-010, US-014. US-004 was cancelled on 2026-08-10 and no longer blocks this story; EP-001's three shipped rules already carry their evidence records.

**Acceptance Criteria:**
- [ ] Given each of the eleven new rules, when `tests/rule_admission.rs` runs, then each has a `tests/rule_evidence.json` entry with a one-line `catches` and a pointer that resolves to either a frozen oracle naming the rule at an observed position or a named test asserting the finding
- [ ] Given a fixture workspace per family under `tests/fixtures/`, when the fixture tests run, then each rule fires on its positive case and stays silent on its stated negative cases
- [ ] Given an evidence entry whose pointer no longer resolves, when the admission test runs, then it fails, in both directions (catalogued without evidence, evidenced without catalog)
- [ ] Fixture workspaces carrying a deliberate `.env` or key-shaped literal are named so that no reader mistakes them for a real leak, and they carry no value that has ever been a real credential

#### US-016: Counters, catalog and README
**Description:** As a maintainer, I want every hard-coded count moved in one pass, so that the repository never states a rule total that disagrees with the catalog.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] `CATALOG: [&RuleDefinition; 51]` (`src/policy/catalog.rs:495`) becomes 62, entries sorted by id, and `validate_catalog` accepts the result
- [ ] The four `== 51` assertions (`tests/persistent_configuration_product_proof.rs:18` and `:421`, `tests/configuration_kernel.rs:74` and `:139`, `tests/score_credibility_kernel.rs:78`) are updated to 62
- [ ] `README.md:75` states "62 rules today: 37 selected Clippy lints, 16 native detectors and 9 structural rules", and the native detector table lists every new `rust_doctor::cargo::*` and `rust_doctor::repo::*` rule with its `catches` line
- [ ] `tests/corpus.json` is regenerated so `the_published_catalog_matches_the_shipped_policy` passes
- [ ] Given a run where a new field was added to any report structure, when the change lands, then `SCHEMA_VERSION` is bumped from 13 and the version test is updated; given no shape change, then it is deliberately left alone and the story says so
- [ ] `DECIDED_FLOOR` in `src/policy/coverage.rs:26` is left unchanged, since no Clippy lint was triaged by this PRD

#### US-017: Corpus precision measurement
**Description:** As a user deciding whether to trust a finding, I want each new rule's false-positive rate on healthy public Rust published, so that noise is disclosed rather than discovered.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-016

**Acceptance Criteria:**
- [ ] Given the ten pinned repositories replayed from the local clone cache, when `cargo test --test corpus_precision` runs with `RUST_DOCTOR_CORPUS_DIR` and `RUST_DOCTOR_CORPUS_ARTIFACTS` set outside this repository, then each of the eleven rules receives an adjudicated verdict or an explicit `unproven`
- [ ] Given a rule measured above the 5% threshold, when the corpus record is written, then its `gate` reads `noisy_on_healthy_code` and the rule stays active, matching the existing policy that no verdict removes a rule
- [ ] Given either `security`-tier rule with a confirmed false positive, when the gate runs, then default activation is refused for that rule and the refusal is recorded, per the zero-tolerance rule in AGENTS.md
- [ ] Given a rule the corpus never triggered, when the record is written, then it reads `unproven` and says so rather than implying a measured zero
- [ ] Given the measurement, when it is taken, then the sample size per rule and the adjudication criterion are recorded alongside the rate, so the number can be re-derived
- [ ] No corpus repository is committed to this repository (`no_corpus_repository_is_committed_in_this_repository` still passes)

#### US-018: Publish the measurement and amend the previous PRD
**Description:** As a reader of the project, I want the new measurement published and the superseded Non-Goal corrected, so that the written record does not contradict the shipped tool.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-017

**Acceptance Criteria:**
- [ ] Given the measurement from US-017, when it is published, then `docs/` carries a dated record in the shape of `docs/structural-precision-2026-08.md`, including the confidence interval or an explicit statement that the sample is too small for one
- [ ] Given `tasks/prd-structural-slop-detection.md:74`, which lists unused dependency detection as a Non-Goal, when this PRD ships, then that line carries a dated amendment naming this PRD and the reason, rather than being deleted
- [ ] Given `AGENTS.md`, when this PRD ships, then its rule-count and producer descriptions match the shipped catalog, including the new `repo` producer and error stage
- [ ] Given a reader who only reads the README, when they read the score section, then they learn that suppression is scored, which is the claim this PRD exists to make true

---

## Functional Requirements

- FR-01: The system must report an `#![allow(...)]` whose scope is a whole file or module as a diagnostic.
- FR-02: The system must report an item carrying two or more suppressions, or one suppression naming four or more lints.
- FR-03: The system must report a `[lints]` entry that sets a catalogued, active rule to `allow`.
- FR-04: Withdrawn on 2026-08-10 with US-004. The toolchain suppresses the very diagnostic this requirement would have joined against, so no observation separates a dead exemption from a live one. The number is retired rather than reused, so a reference to FR-04 made elsewhere still resolves to what it meant.
- FR-05: The system must report a declared dependency no source unit of its package references.
- FR-06: The system must report a `[dependencies]` entry referenced only from test, bench or example code.
- FR-07: The system must abstain from FR-05 and FR-06 for any package whose reference collection was incomplete.
- FR-08: The system must report a release profile without overflow checks, and a release profile that ships debug symbols unstripped.
- FR-09: The system must report workspace rustflags drawn from a closed list of checks-disabling flags.
- FR-10: The system must report a tracked file whose name marks it as secret-bearing, and a source literal matching a closed list of prefixed credential patterns.
- FR-11: The system must report `target/` not being ignored by git.
- FR-12: The system must NOT emit the value of any matched credential, in the terminal or in `--json`.
- FR-13: The system must NOT reach the network for any rule in this PRD.
- FR-14: The system must NOT fail a scan because the repository is not a git repository, has no `.cargo/config.toml`, or has no `[profile.release]`.
- FR-15: Every new rule must default to `warn` or lower and must remain switchable off through `--rule` and `rust-doctor.toml`.

## Non-Functional Requirements

- **Performance:** the repository hygiene pass completes in under 2 seconds on a repository with 10,000 tracked files, on the machine that produced the structural benchmark. The crate-reference collector adds no more than 15% to the wall clock of the existing enumeration, because it reuses that walk. Total added scan time on this repository is 25% or less over the current `cargo run --release -- . --yes`.
- **Bounded work:** repository enumeration stops at 50,000 tracked paths and 8 MiB per file read, mirroring the existing source-kernel limits (`src/source_kernel.rs:19-22`). The manifest reader refuses a `Cargo.toml` above 4 MiB, mirroring the lockfile cap.
- **Security:** no network call, no absolute path, no environment value, and no credential value in any report. The two `security`-tier rules sit at P1, which caps the security dimension at 50 and the overall score at 65, never at 40.
- **Precision:** each of the eleven rules ships with an adjudicated false-positive rate over at least 5 reviewed sites, or an explicit `unproven` verdict. A `security`-tier rule with one confirmed false positive is refused default activation.
- **Reliability:** any pass failure degrades to a complete report carrying a `ReportError` at its stage, with the authoritative flag dropped. No pass failure aborts the scan.
- **Baseline stability:** every structural finding's fingerprint is derived from normalized content, never from a source position, so inserting lines above a finding leaves `--scope baseline` unmoved.
- **Compatibility:** `--json` consumers reading schema 13 keep parsing, or `SCHEMA_VERSION` is bumped to 14 and the change is recorded.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Not a git repository | Workspace outside version control | Repository pass returns empty, no error, scan stays authoritative | none |
| 2 | `git` absent from PATH | Minimal container | Repository pass records a bounded error, every other pass reports | "Repository hygiene skipped: git was not available" |
| 3 | Clippy disabled or failed | Withdrawn 2026-08-10 with US-004 | No rule of this PRD reads the Clippy result, so no scenario depends on that pass having run | none |
| 4 | Partial scope | Withdrawn 2026-08-10 with US-004 | Same reason: no rule of this PRD is scoped by what Clippy actually scanned | none |
| 5 | Unparseable manifest | Malformed `Cargo.toml` or `.cargo/config.toml` | Bounded `ReportError` at stage `dependencies`, no absolute path, scan completes | "Manifest could not be parsed; dependency and profile rules were skipped" |
| 6 | Unparseable source unit | Syntax error or unstable syntax | Unit skipped; its packages marked incomplete so dependency rules abstain for them | "Skipped 1 file the parser could not read" |
| 7 | Enormous repository | More tracked files than the bound | Enumeration stops at the bound, partial findings reported, authoritative flag dropped | "Repository enumeration stopped at 50000 files; results are partial" |
| 8 | Boundary: zero dependencies | Package with an empty dependency table | No finding, no error | none |
| 9 | Boundary: every dependency unused | A scaffold crate that references nothing | One finding per dependency, occurrence multiplier applies, score reflects it once per rule | none |
| 10 | Deliberate suppression | A justified crate-level allow in a generated or vendored file | Reported, suppressible per rule through `rust-doctor.toml`; generated files already excluded by the structural pass | none |
| 11 | Test fixtures that look like leaks | This repository's own fixtures carry a `.env` and a key-shaped literal | Excluded by test-path context, and never weigh on the score; the self-scan test stays green | none |
| 12 | Workspace lints inheritance | Member declares `[lints] workspace = true` | Root `[workspace.lints]` judged once, not once per member | none |
| 13 | Baseline against a ref predating this schema | Comparing to a commit built before these rules existed | New-family findings all classified as introduced, with an explicit note rather than a silent delta | "Baseline predates these rules; all such findings shown as new" |
| 14 | Renamed dependency | `alias = { package = "real" }` | Reference to the alias credits the real entry; no false unused report | none |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Cargo stabilizes `[lints.cargo] unused_dependencies`, making US-006 redundant | Medium | Medium | US-007 (test-only) is excluded from cargo's design and stays unique. If cargo stabilizes, US-006's help text points at it and the rule stays as the no-nightly path. Verified nightly-only on cargo 1.97.1, 2026-08-10. |
| 2 | `hardcoded_credential` produces false positives and discredits the security dimension | Medium | High | Closed list of prefixed patterns only, no entropy trigger, test-path exclusion, P1 rather than P0, and the corpus gate refuses default activation on a confirmed FP. |
| 3 | `unchecked_release_overflow` reads as an opinion rather than a defect | High | Medium | Tier P3, category `reliability`, caps nothing, and help text names the measured cost so the reader can decline. The rule states a tradeoff, not a verdict. |
| 4 | The crate-reference collector misses a reference class and reports a used dependency as unused | Medium | High | Four abstention classes named in the help; incomplete collection suppresses the whole package; corpus measurement publishes the residue; per-rule `off` override documented in the finding itself. |
| 5 | Dead-allow scoping to catalogued rules leaves the rule almost never firing | Medium | Low | Materialized worse than written, and settled on 2026-08-10: the rule could not fire at all rather than rarely, so US-004 is cancelled instead of shipped `unproven`. |
| 6 | Eleven rules at once dilute the per-rule evidence quality | Medium | Medium | EP-005 is a full epic rather than a checklist step, and US-015 requires a positive and stated negative cases per rule before any counter moves. |
| 7 | The repository pass reads files outside the workspace | Low | High | `git ls-files` is run with the workspace root as its working directory and its output is filtered to workspace-relative paths, reusing the escape check already in `src/source_kernel.rs:246-276`. |
| 8 | Suppression rules fire on this repository and break the self-scan | Medium | Low | This repository's own `#![cfg_attr(test, allow(...))]` and `[lints.clippy]` entries are the first fixtures. If a rule fires here, either the rule is wrong or this repository is, and the story says which before it closes. |

## Non-Goals

- **A missing `#![forbid(unsafe_code)]` rule.** Considered and cut. Not having it is a preference, not a defect: the rule would fire on nearly every healthy crate in the corpus, which is the definition of noise. Revisit only if a user asks for a hardening profile that is opt-in rather than default.
- **A `panic = "unwind"` rule.** Considered and cut. Unwind is the Cargo default, the test harness depends on it, and `abort` is a deployment choice rather than a correction. Recommending `abort` by default would be wrong.
- **Supply-chain scoring in the style of Socket.dev.** Refused by contract, not by scope: it requires the network, and rust-doctor never reaches it. Locally available trust-surface signals (dependencies carrying a `build.rs` or a proc-macro, `[patch]` and `[replace]`) are a separate question from vulnerability and are not part of this PRD.
- **Entropy-based secret detection.** The published false-positive record of entropy-only detection is the reason, and network verification, the standard offset, is unavailable here.
- **Advisory-database checks.** `cargo audit` and `cargo deny` own RustSec, and both need a database this tool will not fetch.
- **The remaining unported deslop families.** Dead public API (`unusedExports`), duplicate type definitions, duplicate constants, identity wrappers. Real and deferred, not refused.
- **Ecosystem-specific rules** for tokio, axum, sqlx, serde or reqwest. The alias resolution that would carry them exists; the content is a separate PRD.
- **Product surface.** `why <file:line>`, `ci install` with a GitHub Action, LSP and editor integration. All out of scope here.
- **Automatic fixes.** No `--fix`, no suggested diff, for these rules as for every existing one.
- **Macro expansion.** A reference behind a macro that constructs a path is an abstention, not a target.

## Files NOT to Modify

- `src/policy/catalog.rs` and `tests/corpus.json` - paired invariant, edited together in US-016 only, never separately.
- `tests/rule_evidence.json` - paired with `tests/rule_admission.rs`, edited only through US-015.
- `SCHEMA_VERSION` in `src/report.rs:28` - bump only, and only if a report field changes.
- `TIER_WINDOWS` in `src/policy/catalog.rs:580` - widening a window is a policy decision, not an implementation detail. Every rule in this PRD fits an existing window.
- `src/policy/rejected.json` and `DECIDED_FLOOR` in `src/policy/coverage.rs:26` - monotonic, and untouched by this PRD since no Clippy lint is triaged here.
- `Cargo.toml` and `Cargo.lock` - exact pins; this PRD adds no dependency.
- `tests/fixtures/**` existing oracles - frozen; new fixtures go in new directories.
- `src/source_kernel/aliases.rs` - the per-unit scope-keyed resolver is load-bearing for the two existing source detectors. The crate-reference collector is a new, coarser reader beside it, not a change to it.
- `src/audit.rs` scoring constants - the tier ceilings and occurrence steps are the published score model; this PRD adds rules inside it, not changes to it.

## Technical Considerations

- **Architecture:** settled on 2026-08-10 by cancelling US-004. No rule of this PRD joins two producers, so `ExecutionResult` keeps its current shape and the structural pass publishes no attribute inventory. EP-001's three rules shipped on the existing paths, two as structural detectors and one inside cargo health.
- **Producer count:** adding `Producer::Repo` touches the enum (`src/policy/catalog.rs:12-17`), its id-prefix arm in `validate_catalog`, a field on `ExecutionResult` (`src/execution.rs:26-35`), a call in `execute_target` (near `src/execution.rs:322`, the manifest-level slot rather than the enumeration block), a mapping in `report::diagnostics_from_execution` (`src/report.rs:749`), and an error-stage arm near `src/report.rs:983-997`. Is a fifth producer the right unit, or should repository hygiene live under cargo health? Recommended: a separate producer, because it is the only pass that reads outside the Cargo model and users will want to switch it off as a unit.
- **Data model:** `Diagnostic.path` and `Diagnostic.span` are both `Option` (`src/report.rs:260-306`), so manifest-level and repo-level findings already fit with no schema change. Confirm before assuming no `SCHEMA_VERSION` bump is needed.
- **Manifest reading:** `cargo_metadata` exposes neither `[profile.*]` nor `[lints]`, verified against the pinned 0.23.1 source. `toml` with the existing `serde` feature re-exports `serde_spanned::Spanned`, which yields byte ranges, not line and column; a byte-offset-to-line converter is the only glue, and the crate already has the line-index machinery in `src/structure.rs`.
- **Enumeration:** `git ls-files` versus a bounded filesystem walker. Recommended: git, because "tracked" is the actual predicate and it excludes `target/` by construction. Trade-off: no findings at all outside a git repository, which the criteria accept explicitly.
- **Migration:** no data migration. The only compatibility question is whether `--scope baseline` against a pre-existing baseline should classify every new-family finding as introduced. Recommended: yes, with an explicit note, matching how the structural pass handled the same transition.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Catalogued rules | 51 | 62 | Month-1 | `src/policy/catalog.rs`, asserted by four tests |
| Score of a workspace whose only defect is `#![allow(clippy::all)]` | 100 with 0 diagnostics | below 100 with at least 1 diagnostic | Month-1 | Fixture test added in US-001 |
| New rules carrying a published precision verdict | 0 of 11 | 11 of 11 | Month-1 | `tests/corpus.json` gate field |
| Confirmed false positives on `security`-tier rules | not measured | 0 over 20 adjudicated sites | Month-1 | `cargo test --test corpus_precision` |
| Added scan wall clock on this repository | reference run | +25% or less | Month-1 | `cargo run --release -- . --yes`, three runs, median |
| Rules withdrawn after measurement | not applicable | 0 | Month-6 | Catalog diff |

## Open Questions

- ~~Should the dead-allow rule also judge an `#[allow]` naming a catalogued rule that policy switched **off** for this run?~~ Closed on 2026-08-10 without needing an answer: US-004 is cancelled, so no rule of this PRD reads the policy state of another.
- Does `unused_dependency` apply to `[build-dependencies]` in v1? Engineering to decide during US-006. Build scripts are enumerated as their own target, so the data exists; the risk is that a build dependency used only through an environment variable set by another crate is invisible.
- Should `permissive_lint_table` also fire when the manifest **raises** a catalogued rule to `deny`? Out of scope as written, since a stricter workspace is not a defect, but a `deny` on a rule rust-doctor also runs can turn a scan into a compilation failure, which the repository already treats as a hazard (`no_catalogued_clippy_rule_is_denied_by_default`). Arthur to decide whether that becomes a separate rule.
- Is 50,000 tracked paths the right enumeration bound? It is a guess anchored on the existing 20,000-unit source limit. Whoever implements US-011 should measure a large monorepo clone before pinning it.
[/PRD]
