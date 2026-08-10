[PRD]
# PRD: Structural Slop Detection

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-08 | arthjean | Initial draft |

## Problem Statement

1. **The measured signature of agent-written code is structural, and rust-doctor is blind to it.** GitClear's 2025 study of 211 million changed lines found an 8-fold increase during 2024 in code blocks with five or more duplicated lines, commit-level duplicate-block prevalence rising from 0.45% (2022) to 6.66% (2024), and 2024 as the first year on record where copy-pasted lines exceeded moved lines while refactoring fell from 24.1% to 9.5%. rust-doctor's 43 rules are all per-site diagnostics. Not one of them can see a function cloned into six files.

2. **Clippy cannot close this gap, and the Rust community says so in its own words.** The `rust-lang/rust` LLM policy adopted 2026-08-05 by five teams frames the problem as review economics, not correctness: "Polished technical products no longer indicate effort and understanding." The Rust project's collected perspectives describe agent output as "bad structure, repetitive... a lot of duplicated information." Oleg Kubrakov (2026-04) states the tooling gap directly: existing review tools "spot errors like using wrong numeric type, but completely ignore clunky, verbose constructs."

3. **The Rust tools that do detect structure are new, fragmented, and unused, and one is abandoned.** Measured on crates.io on 2026-08-08: cargo-dupes 3,138 recent downloads (first release 2026-02), similarity-rs 1,485 (beta), cargo-duplicated 34. Mozilla's rust-code-analysis still draws 17,186 recent downloads but has not published since 2023-01. None of them produces a score, a CI gate, a baseline mode, or a unified reading alongside Clippy. rust-doctor already has all four.

4. **rust-doctor's own catalog head is measured as noise, and adding more per-site lints makes it worse.** `tests/corpus.json` adjudicates `clippy::indexing_slicing` at 5 false positives out of 5 reviewed on 246 findings, `unwrap_used` at 5 of 5 on 117, `panic_in_result_fn` and `string_slice` likewise. 11 of 16 measured rules exceed the 5% threshold. Structural findings do not have this problem: "these 38 lines appear three times" is a constatation, not an adjudication.

**Why now:** The practitioner evidence turned specific and dated in the last six months. Erik-Jan van de Wal (2026-08-05, ~20 years experience) itemized the failure modes in Rust specifically: `.clone()` sprinkled to escape the borrow checker, `Arc<Mutex<T>>` as a substitute for ownership reasoning, `unsafe impl` to satisfy `Send + Sync`, `unwrap` in a hot path, with 15 hours of cleanup making the AI-assisted path twice as expensive as writing it by hand. The competing tools are three to eight months old and none has found its audience yet. The window to be the aggregator rather than the fourth point tool is open now and will not stay open.

## Overview

This PRD adds a structural analysis pass to rust-doctor's existing scan. It does not create a separate crate, a separate binary, or a separate report. Structural findings enter the report as `rust_doctor::structure::*` diagnostics on the same path the seven native detectors already take, so they inherit the score, the gate, the category and tier system, the baseline mode, the delta computation and the terminal rendering without any of those subsystems being rewritten.

The flagship detector is duplicate-function detection through AST normalization. Each function, method and closure is parsed with `ra_ap_syntax`, normalized into a structural skeleton where identifiers become positional placeholders and literals are erased but typed, and hashed into a fingerprint. Functions sharing a fingerprint are exact structural clones regardless of naming; functions above a Sørensen-Dice similarity threshold are near-clones. A clone group becomes one diagnostic, not one per member, which is the decision that keeps the detector from dominating the report's top-three advice the way `indexing_slicing` currently does.

Around it sit four cheaper detectors that need no cross-file resolution: complexity hotspots (cyclomatic and cognitive per function), oversized units, orphan modules (a `.rs` file no `mod` declaration reaches, which rustc never mentions because it never compiles it), and an `#[allow]` census. Four already-warned Clippy lints sitting untriaged in the candidate queue are activated alongside them, because `excessive_nesting`, `type_complexity`, `cognitive_complexity` and `too_many_lines` are structural signal the toolchain already computes and rust-doctor currently discards.

One story is a measurement rather than a feature. No published corpus measures structural degradation on agent-written Rust; GitClear is multi-language and unsegmented, and the practitioner reports are anecdotes. `tests/corpus.json` is already the right instrument. Producing that number is publishable independently of the tool and is the strongest differentiator in this document.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Structural rules shipped in the catalog | 4 | 9 |
| Structural findings on a scan of rust-doctor itself | ≥ 10 clone groups or hotspots reported | 0 remaining above P3 threshold after remediation |
| Structural pass wall-clock overhead on a 1000-file workspace | ≤ 2.0s absolute and ≤ 15% of total scan | ≤ 1.0s absolute |
| Adjudicated false-positive rate of `duplicate_function_body` | ≤ 5% on 20 reviewed findings | ≤ 5% on 40 reviewed findings |
| Corpus populations measured | 1 (healthy public Rust) | 2 (healthy + agent-generated) |

## Target Users

### Vibecoder shipping Rust with an agent
- **Role:** Builds a Rust project primarily through Claude Code, Codex or Cursor. Reads compiler errors, does not read Clippy output, has never run `cargo clippy -- -D warnings`.
- **Behaviors:** Accepts agent diffs at a rate that outpaces review. Trusts a green `cargo check` as the completion signal.
- **Pain points:** The codebase grows structurally worse in ways nothing reports. The same helper exists four times under four names. Files reach 2,000 lines without anyone deciding they should.
- **Current workaround:** None. Occasionally asks the agent to "clean this up", which produces a fifth variant of the helper.
- **Success looks like:** One command names the six functions that are the same function, and the score moves when they are merged.

### Rust engineer supervising agent output
- **Role:** Experienced Rust developer, uses agents deliberately, reviews every diff. Van de Wal and Kubrakov are this persona.
- **Behaviors:** Already runs Clippy at pedantic level in pre-commit. Reads `cargo expand` output. Rejects designs, not just lines.
- **Pain points:** Review does not scale to the volume of generated code. Structural drift is invisible in a per-file diff and only surfaces months later. No tool tells them the agent reintroduced a helper that already existed.
- **Current workaround:** Manual architectural review, or installing cargo-dupes separately and reading a raw fingerprint list disconnected from every other signal.
- **Success looks like:** `--scope baseline` fails a PR that adds a clone of an existing function, with both locations named.

### Maintainer triaging agent-authored contributions
- **Role:** Open-source maintainer receiving PRs whose author may not understand them. The `rust-lang/rust` LLM policy exists for this person.
- **Behaviors:** Triages under time pressure. Needs to reject fast and defensibly.
- **Pain points:** Effort is no longer a readable signal. A polished PR may be structurally corrosive.
- **Current workaround:** Reads the whole diff, or closes on suspicion.
- **Success looks like:** A machine-readable structural verdict on the delta, citable in a review comment.

## Research Findings

Key findings that informed this PRD, gathered 2026-08-08:

### Competitive Context
- **cargo-dupes** (mpecan, first release 2026-02, 3,138 recent downloads): `syn`-based AST normalization with positional placeholders, typed literal erasure, opaque macros, 64-bit fingerprints, Sørensen-Dice near-duplicate scoring, `check` subcommand for CI. Author's stated motivation is Claude Code producing duplicate functions. **How we differ:** it emits a fingerprint list; we emit a scored, gated, baseline-aware diagnostic alongside 43 Clippy and native rules.
- **similarity-rs** (mizchi, 1,485 recent downloads, self-labeled beta): tree-sitter based, multi-language family. **How we differ:** Rust-specific, integrated, and not beta.
- **cargo-duplicated** (bircni, 34 recent downloads): text-based line matching, no AST. **How we differ:** renamed clones are the majority case in agent output and text matching cannot see them.
- **rust-code-analysis** (Mozilla, 17,186 recent downloads, no release since 2023-01): cyclomatic and cognitive complexity, library not cargo subcommand. **How we differ:** maintained, and complexity is one signal among several rather than the whole product.
- **cargo-machete / cargo-shear / cargo-udeps** (458k / 62k / 135k recent downloads): unused dependency detection. **How we differ:** we do not compete here. Explicit non-goal.
- **Market gap:** aggregation. Four separate installs, four output formats, no score, no gate, no baseline. Nobody has assembled them.

### Best Practices Applied
- **AST normalization over text matching** (cargo-dupes, published design): identifiers to positional placeholders, literals erased with type preserved, operators and control flow preserved exactly, macros opaque. Catches the renamed-clone case that dominates agent output.
- **Clone groups, not clone instances** (deslop-js clustering): the actionable unit is the family, and per-instance counting distorts any ranking built on occurrence counts.
- **Confidence tiers over pass/fail** (deslop-js): structural findings vary in certainty far more than lint hits; publish the certainty rather than forcing a binary.
- **Structural fingerprints for baseline stability** (novel here): position-derived fingerprints make every structural finding reappear after any edit above it, which destroys CI usability.

*Full research sources: GitClear AI Copilot Code Quality 2025; Inside Rust Blog 2026-08-05 LLM policy; rust-lang perspectives-on-llms Feb-27 summary; van de Wal 2026-08-05; Kubrakov 2026-04-06; pecan.si cargo-dupes part 1; crates.io API measurements 2026-08-08.*

## Assumptions & Constraints

### Assumptions (to validate)
- **HIGH: Structural findings on real Rust codebases are adjudicated as true positives at a materially better rate than the current catalog head.** Based on the reasoning that duplication is a constatation rather than a judgment, but unmeasured. US-018 validates this and the PRD's core premise fails without it.
- **HIGH: Agent-written Rust exhibits measurably more structural duplication than curated public Rust.** Based on GitClear's multi-language finding and two practitioner reports. No Rust-specific measurement exists. US-019 validates.
- **MEDIUM: `ra_ap_syntax` exposes enough structure to build a stable normalized tree without name resolution.** The seven existing native detectors resolve call provenance through manifest aliases, which is weaker than what normalization needs. US-005 proves or disproves it.
- **MEDIUM: dep/dev-dep misclassification is not already covered by cargo-shear.** Unverified; US-016 begins by checking.
- **LOW: `maintainability` at P3 is the right home for structural findings.** Follows from `TIER_WINDOWS` and from the judgment that spaghetti should not cap a score at 40.

### Hard Constraints
- Single crate, edition 2024, rustc 1.95 or later. No new workspace member.
- Dependencies pinned exactly (`=1.8.5` form) in `Cargo.toml`, `Cargo.lock` committed. Adding a dependency requires justification; `ra_ap_syntax` and `blake3` are already present and should carry this work.
- No network, no telemetry, no absolute paths in `--json`. Reports stay workspace-relative.
- Production code carries no `unwrap`, `expect`, `panic!` or `dbg!`. `tests/score_credibility_packs.rs` scans this repository and fails on any hit.
- Any change to the report shape bumps `SCHEMA_VERSION` in `src/report.rs` (currently 10).
- No catalogued rule may be `deny` by default (`no_catalogued_clippy_rule_is_denied_by_default`).
- Every catalogued rule needs an entry in `tests/rule_evidence.json` with a `catches` line and a pointer that resolves, enforced by `tests/rule_admission.rs`.
- `src/policy/catalog.rs` and `tests/corpus.json` must agree (`the_published_catalog_matches_the_shipped_policy`).
- Tier must sit inside its category's `TIER_WINDOWS` band: `maintainability` is P3 to P3, `dependencies` is P1 to P2.
- Every test running `cargo` or the built binary sets its own `CARGO_TARGET_DIR`.
- Integration test crates open with `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`.
- No code may be copied from `millionco/react-doctor` or `mpecan/cargo-dupes`. Both are licensed in ways incompatible with `MIT OR Apache-2.0`. Published algorithm descriptions may be reimplemented freely.

## Quality Gates

These commands must pass for every user story:
- `cargo build --release` - the crate compiles
- `cargo test` - the full suite including catalog, admission and oracle invariants
- `cargo clippy --all-targets --no-deps -- -D warnings` - must be clean, no exceptions
- `cargo run --release -- . --yes --verbose` - the tool runs on its own repository without error

## Epics & User Stories

### EP-001: Structural pass and report integration

Establish `src/structure/` as a producer of `Diagnostic` values on the same path the native detectors use, and make the report shape able to carry a finding that spans several locations.

**Definition of Done:** A trivial structural detector emits a diagnostic that appears in `--json` and in the terminal render, is scored, is gated, survives `--scope baseline` across an unrelated edit, and every existing test still passes.

#### US-001: Structural pass skeleton wired into the inspection session
**Description:** As a maintainer, I want a `src/structure/` module that receives the parsed source units and returns `Diagnostic` values so that every later detector has one integration point instead of its own.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a workspace, when `inspect` runs, then `structure::analyze` is invoked once with the same file set the source kernel already enumerates
- [ ] Given the module returns an empty vector, when the report is built, then the report is byte-identical to the pre-change report for the same workspace
- [ ] Given a file that fails to parse, when the pass runs, then the file is skipped, a `ReportError` with stage `structure` is recorded, and the pass completes for every other file
- [ ] Given a workspace with zero Rust files, when the pass runs, then it returns an empty vector without error
- [ ] Structural diagnostics carry `source: DiagnosticSource::RustDoctor`

#### US-002: Diagnostic carries related locations, schema bumped to 11
**Description:** As a consumer of `--json`, I want a finding that spans several sites to name all of them so that a duplication is actionable rather than a single unexplained span.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given a diagnostic with related locations, when `--json` is emitted, then a `related` array lists each additional `{path, span}` pair, workspace-relative
- [ ] Given a diagnostic with no related locations, when `--json` is emitted, then the `related` key is absent, not an empty array
- [ ] `SCHEMA_VERSION` is 11 and the frozen oracles under `tests/fixtures/` are regenerated to match
- [ ] Given a related location, when the terminal render runs in `--verbose`, then each additional location is printed as a `path:line:column` reference
- [ ] No related path is absolute and no related path escapes the workspace root

#### US-003: Structural findings use a position-independent fingerprint
**Description:** As a CI user, I want a structural finding to keep its identity when unrelated code above it changes so that `--scope baseline` reports what my change introduced rather than what my change shifted.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Given a structural diagnostic, when its fingerprint is computed, then the input tuple contains the structural hash of the finding and excludes `line_start`, `column_start`, `line_end` and `column_end`
- [ ] Given a file where 50 lines are inserted above a structural finding, when the scan reruns, then the fingerprint is unchanged
- [ ] Given a structural finding whose normalized content changes, when the scan reruns, then the fingerprint changes
- [ ] Given a baseline scan and a current scan across such an insertion, when the delta is computed, then the finding is classified `pre_existing`, not `introduced`
- [ ] Clippy and native non-structural diagnostics keep the existing `report.rs` fingerprint unchanged, proven by an unchanged oracle

#### US-004: Structural rules registered in the catalog under maintainability
**Description:** As a policy consumer, I want structural rules catalogued like every other rule so that `--rule`, `--category` and the gate treat them identically.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given the catalog, when `validate_catalog` runs, then every `rust_doctor::structure::*` rule sits inside its category's `TIER_WINDOWS` band
- [ ] Every structural rule declares `default_level: RuleLevel::Warn`; no structural rule defaults to error
- [ ] Given `--rule rust_doctor::structure::duplicate_function_body=off`, when the scan runs, then no diagnostic for that rule appears and the score is computed without it
- [ ] Given `--category maintainability=error`, when a structural finding exists, then it is reported at error severity and blocks if the blocking level allows
- [ ] `tests/corpus.json` is regenerated so `the_published_catalog_matches_the_shipped_policy` passes
- [ ] The rule count in `README.md` matches `src/policy/catalog.rs`

---

### EP-002: Duplicate function detection

Detect functions, methods and closures that are the same code under different names, group them into clone families, and report each family once.

**Definition of Done:** Running the tool on rust-doctor itself names at least one real clone family that a reviewer confirms should be merged, and the detector adds no more than 1.0s to that scan.

#### US-005: AST normalization of function bodies
**Description:** As the duplication detector, I want each function reduced to a structural skeleton so that two functions doing the same thing under different names compare equal.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given two functions differing only in function name, parameter names and local variable names, when both are normalized, then the normalized trees are equal
- [ ] Given two functions differing only in integer literal values, when both are normalized, then the trees are equal and the literal type is retained in the node
- [ ] Given two functions where one uses `if/else` and the other `match`, when both are normalized, then the trees differ
- [ ] Given two functions where one uses `+` and the other `*`, when both are normalized, then the trees differ
- [ ] Given a macro invocation, when it is normalized, then it becomes an opaque node and `println!("a")` and `println!("b")` normalize equal
- [ ] Given a function containing a syntax error region, when normalization runs, then the function is skipped without panicking and without aborting the file

#### US-006: Exact clone grouping by structural fingerprint
**Description:** As a user, I want functions that are structurally identical grouped into one finding so that I see families rather than a list of unrelated spans.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] Given N functions sharing a normalized tree, when the scan runs, then exactly one diagnostic is emitted with `occurrences` equal to N
- [ ] Given such a group, when the diagnostic is emitted, then `path` and `span` point at the first member in workspace-relative sorted order and `related` names the remaining N-1
- [ ] Given a function whose normalized tree has fewer nodes than the configured minimum, when the scan runs, then it is excluded from grouping
- [ ] Given two functions with identical bodies but different arities, when the scan runs, then they are not grouped, because the signature participates in the fingerprint
- [ ] Given a workspace with no clones, when the scan runs, then no `duplicate_function_body` diagnostic is emitted
- [ ] An entry for `rust_doctor::structure::duplicate_function_body` exists in `tests/rule_evidence.json` with a `catches` line and a pointer that `tests/rule_admission.rs` resolves

#### US-007: Near-duplicate detection by structural similarity
**Description:** As a user, I want functions that are almost the same reported too, because the near-clone is where the refactor actually hides.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-006

**Acceptance Criteria:**
- [ ] Given two functions differing by one added branch, when the scan runs at the default threshold, then they are reported as a near-duplicate group
- [ ] Given two unrelated functions of similar size, when the scan runs, then they are not grouped
- [ ] Given a pair already reported as an exact group, when near-duplicate scoring runs, then the pair is not reported twice
- [ ] The similarity score is emitted on the diagnostic and appears in `--json`
- [ ] Given a workspace of 1000 functions, when near-duplicate scoring runs, then it completes within the NFR budget without pairwise comparison of every pair
- [ ] `rust_doctor::structure::near_duplicate_function_body` has a `tests/rule_evidence.json` entry with a resolving pointer

#### US-008: Test, bench, example and build-script context handling
**Description:** As a user, I want duplication in tests weighed differently from duplication in shipped code, because a test helper repeated across cases is often deliberate.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-006

**Acceptance Criteria:**
- [ ] Given a clone family whose members are all in `#[cfg(test)]` modules or test targets, when the scan runs, then the diagnostic carries the existing `DiagnosticContext` mark and does not weigh on the score
- [ ] Given a clone family spanning both production and test code, when the scan runs, then it is unmarked and weighs on the score
- [ ] Given a clone family inside `build.rs`, when the scan runs, then it is marked as a build-script context
- [ ] Marked structural diagnostics remain published and counted in the occurrence totals, consistent with the existing rule for `println!` in `build.rs`

#### US-009: Duplication pass performance budget
**Description:** As a user, I want the structural pass to be fast enough that I do not notice it, because a scan I avoid running reports nothing.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] Given a synthetic workspace of 1000 files and 10,000 functions, when the structural pass runs, then it completes in ≤ 2.0s wall clock on the development machine
- [ ] Given the same workspace, when the full scan runs, then the structural pass accounts for ≤ 15% of total wall clock
- [ ] A benchmark test records the measurement and fails if the 1000-file budget regresses by more than 25%
- [ ] Given a workspace larger than the budget allows, when the pass exceeds its time bound, then it stops, reports what it covered, and marks the scan as not authoritative rather than hanging
- [ ] Memory used by the normalization pass is bounded and does not retain every parsed tree simultaneously

---

### EP-003: Complexity and size hotspots

Report the functions and files that are too large or too tangled to review, using both computed metrics and the Clippy lints already sitting untriaged in the candidate queue.

**Definition of Done:** A scan of rust-doctor names `report.rs` and `audit.rs` as oversized and reports a cognitive complexity hotspot, and `DECIDED_FLOOR` in `src/policy/coverage.rs` has risen.

#### US-010: Cyclomatic and cognitive complexity per function
**Description:** As a reviewer, I want a per-function complexity figure so that I can find the code that is expensive to reason about rather than merely long.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given a function with N branch points, when the pass runs, then the reported cyclomatic complexity is N+1
- [ ] Given two functions with equal cyclomatic complexity but different nesting depth, when the pass runs, then the more deeply nested one reports higher cognitive complexity, per the SonarSource nesting-weighted definition
- [ ] Given a function below both thresholds, when the pass runs, then no diagnostic is emitted for it
- [ ] Both figures appear in `--json` on the diagnostic
- [ ] Thresholds are overridable through `rust-doctor.toml` and the existing `--rule` mechanism
- [ ] `rust_doctor::structure::complex_function` has a `tests/rule_evidence.json` entry with a resolving pointer

#### US-011: Oversized units
**Description:** As a reviewer, I want functions, impl blocks, modules and files above a size threshold named so that unbounded growth is visible before it is irreversible.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given a file above the line threshold, when the scan runs, then one diagnostic names the file with its line count
- [ ] Given a function above the line threshold, when the scan runs, then one diagnostic names the function
- [ ] Given a scan of this repository, when the pass runs, then `src/report.rs` (2898 lines) and `src/audit.rs` (1445 lines) are both reported
- [ ] Given a generated file marked by a recognized generator header, when the scan runs, then it is excluded
- [ ] A single unit exceeding several thresholds produces one diagnostic, not one per threshold
- [ ] `rust_doctor::structure::oversized_unit` has a `tests/rule_evidence.json` entry with a resolving pointer

#### US-012: Activate queued structural Clippy lints
**Description:** As a maintainer, I want the structural lints Clippy already computes to be catalogued so that signal already reaching the report stops arriving without category, tier or help.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**
- [ ] `clippy::excessive_nesting`, `clippy::type_complexity`, `clippy::cognitive_complexity` and `clippy::too_many_lines` are each either catalogued with a category, tier and help, or added to `src/policy/rejected.json` with a closed class and a written reason
- [ ] Given the catalog, when `no_catalogued_clippy_rule_is_denied_by_default` runs, then it passes for each newly catalogued lint
- [ ] Each newly catalogued lint has a `tests/rule_evidence.json` entry whose `catches` line matches the toolchain description verbatim
- [ ] `DECIDED_FLOOR` in `src/policy/coverage.rs` is raised to the new decided count and `the_candidate_queue_is_published_and_coverage_never_regresses` passes
- [ ] `tests/corpus.json` is regenerated and the catalog-policy agreement test passes
- [ ] Given a fixture exercising each lint, when scanned, then the lint fires and is reported with its category

---

### EP-004: Rust-native slop signals

Detect the structural failure modes that exist in Rust specifically and that no JavaScript-derived tool can have, because they arise from Rust's module system, attribute system and manifest.

**Definition of Done:** All three P1 detectors ship with fixtures and evidence records, and at least one fires on a real external repository.

#### US-013: Orphan module files
**Description:** As a user, I want a `.rs` file that no `mod` declaration reaches to be reported, because Cargo never compiles it and nothing else will ever tell me it exists.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `src/helper.rs` with no `mod helper;` anywhere in the crate, when the scan runs, then one diagnostic names the file
- [ ] Given a file reached through a `#[path = "..."]` attribute, when the scan runs, then it is not reported
- [ ] Given a file reached only under a `#[cfg(...)]`-gated `mod` declaration, when the scan runs, then it is not reported
- [ ] Given `src/main.rs`, `src/lib.rs`, `build.rs`, and every declared binary, example, bench and integration test root, when the scan runs, then none is reported
- [ ] Given a workspace with several packages, when the scan runs, then module reachability is computed per package, not across the workspace
- [ ] `rust_doctor::structure::orphan_module_file` has a `tests/rule_evidence.json` entry with a resolving pointer

#### US-014: Allow-attribute census
**Description:** As a reviewer, I want every `#[allow]` in the codebase inventoried, because each one is a rule someone switched off locally and the census is the densest slop signal available.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given an `#[allow(...)]` or `#![allow(...)]` attribute in production code, when the scan runs, then it is counted and its lint names recorded
- [ ] Given an `#[allow]` carrying a `reason = "..."` argument, when the scan runs, then it is counted separately from unreasoned ones and does not produce a diagnostic
- [ ] Given an `#[expect(...)]` attribute, when the scan runs, then it is not reported, because it fails when the lint stops firing
- [ ] Given a crate-level allow in a test target or integration test crate, when the scan runs, then it is marked with the existing non-production context
- [ ] Given zero unreasoned allows, when the scan runs, then no diagnostic is emitted
- [ ] `rust_doctor::structure::unreasoned_allow_attribute` has a `tests/rule_evidence.json` entry with a resolving pointer

#### US-015: Feature flag inventory
**Description:** As a maintainer, I want features declared in `Cargo.toml` but never referenced in code, and `cfg(feature)` references to features that do not exist, so that dead configuration surface is visible.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given a feature declared in `[features]` and referenced by no `cfg(feature = "...")` and by no dependency's optional activation, when the scan runs, then one diagnostic names it
- [ ] Given a `cfg(feature = "x")` where `x` is declared nowhere, when the scan runs, then one diagnostic names it
- [ ] Given a `default` feature entry, when the scan runs, then it is never reported as unreferenced
- [ ] Given a feature referenced inside a `cfg(any(...))` or `cfg(all(...))` expression, when the scan runs, then the reference is recognized
- [ ] Given a workspace with several packages, when the scan runs, then features are resolved per package
- [ ] `rust_doctor::structure::unreferenced_feature` has a `tests/rule_evidence.json` entry with a resolving pointer

#### US-016: Dependency and dev-dependency misclassification
**Description:** As a maintainer, I want a crate declared under `[dependencies]` but used only from test, bench or example targets to be reported, so that the shipped dependency surface is honest.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015
**Status:** CANCELLED on 2026-08-08 by its own first acceptance criterion.

**Story notes (2026-08-08):** cargo-shear already covers this. Its README lists
"**Misplaced dependencies** (dev/build dependencies in wrong sections)" as one of
three things it detects, alongside unused dependencies and, as of the same
release, "**Unlinked source files** (Rust files not reachable from any module
tree)". Its stated limitation is narrow: "Misplaced dependency detection only
works for integration tests, benchmarks, and examples. Unit tests dependencies
within `#[cfg(test)]` cannot be detected as misplaced." The scenario this story
specifies, a crate used only from `#[cfg(test)]` code *and* `tests/`, is reached
through the `tests/` half, so cargo-shear reports it. The first acceptance
criterion says to cancel rather than implement in that case, and that is what
was done. Source: `github.com/Boshen/cargo-shear`, README read 2026-08-08.

This also dates the PRD's competitive section, which credits cargo-shear with
unused-dependency detection only. It now overlaps US-013 as well. US-013 shipped
regardless, because the differentiator this PRD names is aggregation: one score,
one gate, one baseline, not a fourth point tool.

**Acceptance Criteria:**
- [ ] Before implementation, cargo-shear and cargo-machete are checked for this capability and the finding is recorded in the story notes; if either already covers it, the story is CANCELLED rather than implemented
- [ ] Given a crate used only from `#[cfg(test)]` code and `tests/`, when the scan runs, then one diagnostic proposes moving it to `[dev-dependencies]`
- [ ] Given a crate used through a derive macro only, when the scan runs, then it is not reported, because derive usage is a known blind spot of path-based resolution
- [ ] Given a crate with no source reference at all, when the scan runs, then it is not reported by this rule, because unused-dependency detection is an explicit non-goal
- [ ] The diagnostic carries category `dependencies` and a tier inside the P1 to P2 window
- [ ] `rust_doctor::structure::misclassified_dependency` has a `tests/rule_evidence.json` entry with a resolving pointer

---

### EP-005: Precision measurement and publication

Measure what the structural rules actually do, on both healthy public Rust and on agent-generated Rust, and publish both numbers.

**Definition of Done:** `tests/corpus.json` carries adjudicated precision for every structural rule on the healthy population, per-rule findings and structural density for both populations, and the README states both figures. Adjudicating the agent population site by site is explicitly out of scope here, per US-020, and the README says so rather than implying a rate that was never read; it is future work, not silent debt.

#### US-017: Structural rules measured on the existing pinned corpus
**Description:** As a maintainer, I want the structural rules replayed against the ten pinned public repositories so that their noise rate on healthy Rust is published rather than asserted.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-006, US-010, US-011, US-013, US-014

**Acceptance Criteria:**
- [ ] Given `RUST_DOCTOR_CORPUS_DIR` and `RUST_DOCTOR_CORPUS_ARTIFACTS` set to paths outside this repository, when `cargo test --test corpus_precision` runs, then every structural rule receives a `measured` or `unobserved` status in `tests/corpus.json`
- [ ] Given at least 20 findings for `duplicate_function_body`, when adjudication runs, then at least 20 are reviewed and the false-positive rate in basis points is recorded
- [ ] Given a structural rule measured above the 5% threshold, when the gate is written, then it appears in `noisy_on_healthy_code` and remains admitted, consistent with the existing policy
- [ ] Given the environment variables are unset, when the test runs, then it returns silently
- [ ] `no_corpus_repository_is_committed_in_this_repository` still passes

#### US-018: Validate the structural precision assumption
**Description:** As a maintainer, I want the claim that structural findings are adjudicated better than the catalog head either confirmed or refuted, because the premise of this PRD rests on it.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-017

**Acceptance Criteria:**
- [ ] The adjudicated false-positive rate of `duplicate_function_body` on the pinned corpus is recorded and compared against the current head rates (`indexing_slicing` 10000bp, `unwrap_used` 10000bp)
- [ ] Given the structural rate exceeds 5000 basis points, when the comparison is written, then the finding is recorded as a refutation and the epic is marked BLOCKED pending a design revision
- [ ] The comparison, its sample size, and its confidence limits are written into `docs/` as a durable record
- [ ] The adjudication criterion used for structural findings is stated explicitly and added to `tests/corpus.json` alongside the existing criterion

#### US-019: Agent-generated Rust corpus population
**Description:** As a maintainer, I want a second corpus population of agent-written Rust so that rust-doctor can publish the delta between healthy and generated code, which is the measurement nobody else has.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-017

**Acceptance Criteria:**
- [ ] At least 5 public Rust repositories whose commit history documents agent authorship are pinned by commit in `tests/corpus.json` under a distinct population key
- [ ] The selection criterion for "agent-generated" is written down and is falsifiable from the repository record alone, not inferred from code style
- [ ] Given both populations, when the measurement runs, then `tests/corpus.json` carries per-rule findings for each population separately
- [ ] The structural finding density per thousand lines is computed for both populations and the ratio is recorded
- [ ] Given the ratio is at or below 1.0, when the result is written, then it is published as a refutation of the PRD's second assumption rather than omitted
- [ ] All corpus repositories are cloned from a local cache and no test touches the network

#### US-020: Documentation and rule count synchronization
**Description:** As a user reading the README, I want the rule count, the detector table and the structural section to match what ships so that the documentation is not a third source of truth.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-012, US-016

**Acceptance Criteria:**
- [ ] The README rule count matches the length of the catalog in `src/policy/catalog.rs`
- [ ] The README native detector table lists every `rust_doctor::structure::*` rule with its `catches` line
- [ ] `AGENTS.md` documents the structural pass, its fingerprint rule, and the constraint that structural rules default to warning
- [ ] The README states the measured structural precision on both corpus populations, or states explicitly that the second population is not yet measured
- [ ] Given the catalog changes, when `cargo test` runs, then a test fails if the README rule count no longer matches

## Functional Requirements

- FR-01: The system must run a structural analysis pass over every Rust source file in the scan scope during a normal scan, with no additional flag required.
- FR-02: The system must emit structural findings as `Diagnostic` values with `source: rust-doctor` and an identifier under the `rust_doctor::structure::` namespace.
- FR-03: The system must report a clone family as exactly one diagnostic whose `occurrences` equals the family size and whose `related` array names every member beyond the first.
- FR-04: The system must compute structural fingerprints from normalized content and must not include source positions in that computation.
- FR-05: When a structural rule is disabled through `--rule` or `rust-doctor.toml`, the system must omit its findings and compute the score without them.
- FR-06: The system must mark structural findings originating in test, bench, example and build-script targets with the existing non-production context, and must keep them published and counted.
- FR-07: The system must not emit any absolute filesystem path, environment variable or user identifier in a structural finding.
- FR-08: The system must NOT perform unused-dependency detection.
- FR-09: The system must NOT require network access, and must fail the scan rather than reach the network if a structural detector ever attempts it.
- FR-10: When the structural pass exceeds its time budget, the system must stop the pass, report the findings already collected, and mark the scan as not authoritative.
- FR-11: The system must degrade to a complete non-structural report when the structural pass fails entirely, recording a `ReportError` with stage `structure`.

## Non-Functional Requirements

- **Performance:** Structural pass completes in ≤ 2.0s wall clock on a 1000-file, 10,000-function workspace, and accounts for ≤ 15% of total scan wall clock. Near-duplicate scoring must not be pairwise-complete: comparisons per function are bounded by a candidate-nomination step.
- **Memory:** Peak additional resident memory attributable to the structural pass is ≤ 200 MB on the 1000-file benchmark; normalized trees are not all retained simultaneously.
- **Precision:** `duplicate_function_body` adjudicated false-positive rate ≤ 500 basis points on ≥ 20 reviewed findings from the pinned corpus. A rule above threshold is published with its rate, never silently suppressed.
- **Determinism:** Two scans of an unchanged workspace produce byte-identical `--json` output, including structural ordering. Clone family members are emitted in workspace-relative sorted path order.
- **Baseline stability:** Inserting 50 unrelated lines above a structural finding changes zero structural fingerprints.
- **Security and privacy:** Zero network calls, zero telemetry, zero absolute paths in any report field, enforced by the existing report tests.
- **Compatibility:** `SCHEMA_VERSION` 11 is a single additive bump. A consumer reading version 10 fields finds every one of them present and unchanged in version 11.
- **Reliability:** A parse failure in any single file never aborts the scan; the file is skipped and recorded.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Empty structural result | Workspace with no clones, no hotspots, no orphans | No structural section, score unaffected, scan reported as complete | — |
| 2 | Unparseable file | Syntax error, unstable syntax, or a file `ra_ap_syntax` rejects | File skipped, `ReportError` stage `structure` recorded, every other file processed | "Skipped 1 file the parser could not read" |
| 3 | Time budget exceeded | Workspace far larger than the benchmark | Pass stops, partial findings reported, scan marked not authoritative | "Structural analysis stopped at the time budget; results are partial" |
| 4 | Generated code | A file carrying a recognized generator header, or `target/` content | Excluded from every structural detector | — |
| 5 | Macro-heavy code | Function bodies dominated by macro invocations | Macros normalize opaque; a function whose normalized tree falls below the minimum node count is excluded from grouping | — |
| 6 | Deliberate duplication | Trait impls repeated across types, test fixtures repeated per case | Reported, but suppressible per rule through `rust-doctor.toml`, and test-context families do not weigh on the score | — |
| 7 | Single-package vs workspace | Virtual manifest with several members | Module reachability, feature resolution and dependency classification computed per package | — |
| 8 | Zero Rust files | Manifest present, `src/` empty | Structural pass returns empty without error, scan completes | — |
| 9 | Boundary: minimum unit size | A two-line function repeated 40 times | Excluded by the minimum node threshold, not reported | — |
| 10 | Boundary: enormous clone family | One function cloned 200 times | One diagnostic, `occurrences` 200, `related` truncated in the terminal render with a count, complete in `--json` | "and 197 more locations" |
| 11 | Conflicting overrides | `--rule ...=off` with `--category maintainability=error` | Command-line rule override wins over category override, matching existing precedence | — |
| 12 | Baseline against a ref with no structural data | Comparing against a commit built before schema 11 | Structural findings all classified `introduced`, with an explicit note rather than a silent delta | "Baseline predates structural analysis; all structural findings shown as new" |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Normalization is mistuned and every `match` arm reads as duplicated, or renamed clones slip through | High | High | US-005 acceptance criteria fix the normalization contract as testable equalities and inequalities before any grouping is written. Minimum node threshold is configurable. US-018 measures the result before the rule is promoted. |
| 2 | The precision premise is wrong and structural findings adjudicate no better than the catalog head | Medium | High | US-018 exists solely to test this and blocks EP-002 promotion on the result. A refutation is published, not buried. |
| 3 | Near-duplicate scoring is pairwise and does not scale | Medium | Medium | NFR forbids pairwise-complete comparison. US-009 sets a hard budget with a regression test and a stop-and-report path. |
| 4 | cargo-dupes reaches the audience first and defines the category | Medium | Medium | It has 3,138 recent downloads and emits a fingerprint list. The differentiator is aggregation with score, gate and baseline, none of which it has. Ship the integration, not a competing point tool. |
| 5 | Schema bump breaks a downstream consumer | Low | Medium | The change is additive: `related` is skipped when empty and no existing field moves. `SCHEMA_VERSION` bump is the declared contract. |
| 6 | Scope creep across 20 stories delays the still-unpublished tool further | High | High | EP-002 US-006 is the minimum shippable slice. EP-004 P2 stories and US-019 are explicitly deferrable. Publishing the crate is tracked outside this PRD and must not wait on it. |
| 7 | Deliberate duplication in trait impls produces reviewer fatigue | Medium | Medium | Test-context marking (US-008), configurable minimum node count, and per-rule suppression. Default level is warning, never error. |
| 8 | Agent-generated corpus selection is unfalsifiable and the headline measurement is worthless | Medium | High | US-019 requires the selection criterion to be verifiable from the repository record alone, explicitly forbidding style-based inference. |

## Non-Goals

- **Unused dependency detection.** cargo-machete (458k recent downloads) and cargo-shear (62k) own this. Revisit only if a user asks for a unified report and both tools remain external.
- **A separate `deslop-rs` crate or binary.** Extraction from a working module is cheap in Rust; two half-finished projects are not. Revisit once the structural pass has users.
- **Cross-crate or cross-workspace duplication.** Single-workspace analysis only. Revisit if a monorepo user asks.
- **Semantic equivalence.** Two functions that compute the same result through different structures are out of scope. This is structural similarity, not program equivalence.
- **Macro-expanded analysis.** Macros normalize to opaque nodes. Expanding them would require `cargo-expand` and a second compilation, which the time budget forbids.
- **Automatic fixes.** No `--fix`, no suggested refactoring diff. The tool reports; the agent or the human refactors.
- **A separate structural report section with its own scoring model.** Structural findings are diagnostics on the existing path. A parallel model would fork the score, the gate, the baseline and the render.
- **LSP or editor integration.** Out of scope for this PRD entirely.

## Files NOT to Modify

- `src/audit.rs` — the score model. Structural findings must flow through `aggregate_rules` unchanged. Any need to alter this file means the diagnostic shape is wrong, not the score.
- `src/delta.rs` — the baseline matcher. US-003 changes what fingerprint structural findings carry, not how the delta consumes fingerprints.
- `src/git_scope.rs` — scope resolution is orthogonal and already correct.
- `Cargo.toml` dependency table — no new dependency without an explicit decision; `ra_ap_syntax` and `blake3` are already present and must carry this work.
- `tests/corpus.json` by hand — regenerate it from the catalog, never hand-edit, or `the_published_catalog_matches_the_shipped_policy` will pass on a lie.
- `src/policy/coverage.rs` `DECIDED_FLOOR` except to raise it in US-012.

## Technical Considerations

- **Architecture:** Where does the structural pass sit relative to `execution::prepare` and the Clippy invocation? Recommended: a sibling of `source_kernel`, run on the same enumerated file set, independent of the Clippy subprocess so that a Clippy failure still yields structural findings. Engineering to confirm the file enumeration is reusable without a second walk.
- **Parser:** `ra_ap_syntax = 0.0.343`, already pinned. It is a lossless syntax tree, not a resolved HIR, which is sufficient for normalization and insufficient for name resolution. Alternative considered and rejected: `syn`, which cargo-dupes uses, because it would be a second parser in the same crate.
- **Normalization data model:** a `NormalizedNode` enum over the syntax kinds that matter, roughly 40 to 50 variants. Open trade-off: hashing the tree directly with `blake3` (already a dependency, 256-bit, collision-free in practice) versus a 64-bit hash as cargo-dupes uses. Recommended: `blake3`, because the fingerprint is also the baseline identity and a collision there is a wrong CI verdict.
- **Near-duplicate candidate nomination:** Sørensen-Dice over full trees is O(n²) in the number of functions. Recommended: bucket by normalized tree size and top-level shape first, score only within buckets. Engineering to confirm the bucketing preserves enough recall to be worth shipping, and to record what it drops.
- **Fingerprint plumbing:** `report.rs:1388` computes one fingerprint from `(source, code, path, span, base_severity, message)`. Structural findings need a variant that substitutes the structural hash for `span` and `message`. Open question: extend the existing function with an optional structural component, or add a parallel constructor. Recommended: the former, so there is one fingerprint definition and one oracle to freeze.
- **Migration:** `SCHEMA_VERSION` 10 to 11, additive only. Every frozen oracle under `tests/fixtures/` regenerates. Backward compatibility requirement: yes, a version-10 consumer must find every field it expects. Rollback plan: the structural pass is behind a catalog entry, so disabling all `rust_doctor::structure::*` rules returns the report to version-10 content under a version-11 header.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Structural rules in the catalog | 0 of 43 | 4 | Month-1 | `src/policy/catalog.rs` length, asserted by the catalog test |
| Structural rules in the catalog | 0 of 43 | 9 | Month-6 | Same |
| Clippy lints decided | 52 of 815 | 56 | Month-1 | `DECIDED_FLOOR` in `src/policy/coverage.rs` |
| Adjudicated FP rate, `duplicate_function_body` | N/A (new) | ≤ 500 basis points on ≥ 20 reviewed | Month-1 | `tests/corpus.json` precision entry |
| Corpus rules with `measured` status | 16 of 43 | 21 of 47 | Month-1 | `tests/corpus.json` status counter |
| Corpus populations | 1 | 2 | Month-6 | `tests/corpus.json` population keys |
| Structural finding density ratio, agent-generated over healthy | N/A (new) | Published, whatever the value | Month-6 | US-019 measurement |
| Structural pass wall clock, 1000-file benchmark | N/A (new) | ≤ 2.0s | Month-1 | Benchmark test assertion |
| Clone groups found on rust-doctor itself | Unknown | ≥ 1 confirmed by review as worth merging | Month-1 | Manual review of the self-scan |
| `SCHEMA_VERSION` | 10 | 11 | Month-1 | `src/report.rs` constant |

## Open Questions

- Should the terminal render give structural findings their own grouped section, or interleave them with Clippy diagnostics by severity? Arthur to decide before US-002 lands; it determines whether `render.rs` needs a second layout path or none at all.
- What minimum normalized node count excludes trivial functions without hiding real clones? Empirical, resolved by running US-005 against this repository before US-006 is written. cargo-dupes defaults to 10; that number is a starting hypothesis, not a citation.
- Does a structural finding in a `#[cfg(test)]` module deserve the existing non-production context mark, or a new context kind? Affects US-008 and the schema; resolve before the version-11 bump is frozen.
- Should `rust_doctor::structure::unreasoned_allow_attribute` fire per attribute or per file with a count? Per attribute is more actionable and far noisier on any real codebase. Resolve during US-014 with a measurement, not a preference.
- Is publishing the crate to crates.io a prerequisite for US-019, given that the agent-generated corpus measurement is the most publishable artifact here and lands better attached to an installable tool? Arthur to decide; it is a sequencing question outside this PRD's scope but it gates the value of its last epic.
[/PRD]
