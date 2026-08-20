[PRD]
# PRD: core-v3 Score Recalibration

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-20 | arthjean | Initial draft |

## Problem Statement

1. **The score cannot distinguish the two populations the corpus was built to compare.** `tests/corpus.json` pins ten healthy public repositories and eight agent-authored ones. Under core-v2 the healthy median is 93.5 and the agent median 92.5: one point. The corpus already publishes the verdict on itself, `score_distribution.collapsed_into_one_band: true`, `minimum: 90`, `maximum: 100`, with all ten healthy repositories in a single band. A score whose whole purpose is to tell a reader where their workspace stands returns the same answer for `smol` and for a 698,000-line agent-generated codebase.

2. **The model has an arithmetic floor of 63 that no amount of defects can breach.** Every one of the 62 catalogued rules ships at `default_level: warn`, so `severity_penalty_quarters(Severity::Error) = 6` at `src/audit.rs:602` is never exercised and the maximum a single rule can cost is `3 × OCCURRENCE_CEILING` = 12 quarters (`src/audit.rs:568,573`). Against the 400-quarter scale of `dimension_score` at `src/audit.rs:892`, the worst attainable dimension is bounded by its rule count: reliability holds 25 rules and floors at 25, maintainability 16 rules floors at 52, security 6 floors at 82, performance 10 floors at 70, dependencies 5 floors at 85. Weighted 4/3/2/2/2 over 13, the absolute worst score the model can emit with no P0 or P1 rule firing is 63. Every catalogued rule is P2 or P3 in practice, so 63 is the real floor.

3. **Repairing findings does not move the score, which inverts the tool's own advice.** `occurrence_multiplier` at `src/audit.rs:580` saturates at 4 for any count above 20. A rule firing 246 times and a rule firing 21 times cost the same. Measured across the 18 corpus repositories, repairing 90 % of every site moves core-v2 by 0 to 1 point. The report ranks what to fix first by `expected_repair_value` (`src/audit.rs:648`), then the score refuses to acknowledge the repair.

4. **The score is not scale-invariant, and its dominant signal is repository size.** No denominator exists anywhere in `src/audit.rs`: `SourceFileInventory` is computed at `src/audit/source_inventory.rs:17`, published, and passed to `share_url` at `src/audit.rs:297`, but `score()` at `src/audit.rs:690` never reads it. The Spearman correlation between repository size and published score across the 18 corpus repositories is **−0.794**. The three largest hold the three lowest scores (vibesql 698k lines → 87, claudes-c-compiler 187k → 88, ripgrep 53k → 91); the three smallest hold the three highest (smol 1.7k → 100, async-channel 2.3k → 99, hexyl 2.8k → 98). Duplicating a workspace ten times, changing nothing about its quality, moves core-v2 by −2 wherever it is not already saturated.

**Why now:** The catalog reached 62 rules and the corpus reached two populations. Both instruments are now precise enough to measure the score, and both say the same thing. The measurement is also cheap to redo: `tests/corpus.json` replays from a pinned local clone cache under a pinned toolchain, so recalibrating is a reproduction run, not a research project. Doing it before the catalog grows again is doing it while the frozen oracles are small enough to regenerate in one pass. Every rule admitted after this point would be admitted against a scale nobody has checked.

## Overview

This PRD replaces the core-v2 penalty with a continuous one. A dimension's score becomes `round(100 · exp(−D / λ))`, where `D` is the density of distinct scored sites in that dimension, severity-weighted, over a denominator chosen by the producer that raised each finding. Per-site producers (`clippy`, `source-kernel`, `structure`) divide by production kilolines with a 2.0 floor; workspace-scoped producers (`cargo-health`, `repo`) divide by 1, because a missing lockfile is not less serious in a large repository. The five dimensions, their 4/3/2/2/2 weights over 13, the tier ceilings and the three bands are unchanged.

The denominator does not exist yet and is the load-bearing new input. It is production source lines, counted during the walk `source_kernel::enumerate` already performs, because that walk is the one place that already decides production from test code per unit through `SourceUnit::is_test_code`. Counting there makes the numerator population and the denominator population the same by construction rather than by convention. The count is stored in the audit block rather than recomputed, because `report.rs:200,211` replay `rebuild_for_scope` as a validity check and the score has to stay a pure function of stored inputs.

Three things follow mechanically. `SCORE_MODEL` moves to `core-v3` and `SCHEMA_VERSION` from 14 to 15, which the v7 archive projection at `tests/support/mod.rs:153` tracks through a single literal. The enumeration at `src/execution.rs:371` becomes unconditional, because a Clippy-only plan currently produces no enumeration and would therefore produce no denominator, degrading through the existing incomplete-inventory path. And `tests/corpus.json` is regenerated, with `score_distribution` redefined to publish spread rather than to record its own collapse.

One story is a measurement rather than a feature. Density per kloc has a documented failure mode: it rewards verbosity, because padding the denominator raises the score. The counter-argument specific to rust-doctor is that its own catalog penalizes the gaming vector, `duplicate_function_body`, `near_duplicate_function_body` and `oversized_unit` all charging for exactly the code someone would add to dilute their density. That is a plausible argument, not a measured one, so US-018 measures it.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Score spread across the 18 corpus repositories | ≥ 20 points | ≥ 25 points, ≥ 2 bands occupied |
| Separation between healthy and agent medians | ≥ 5 points | ≥ 8 points |
| Score gain from repairing 90 % of sites, median across the corpus | ≥ 8 points | ≥ 10 points |
| Spearman correlation between repository size and score | \|ρ\| ≤ 0.30 | \|ρ\| ≤ 0.20 |
| Score change under 10x whole-workspace duplication | 0 points | 0 points |
| Corpus repositories in the top band | ≤ 17 of 18 | ≤ 14 of 18 |

## Target Users

### Vibecoder reading a score for the first time
- **Role:** Runs `npx rust-doctor@latest` on a project an agent wrote most of. Has no calibration for what a Rust codebase should score.
- **Behaviors:** Reads the number and the band, not the rule list. Reruns after asking the agent to fix things.
- **Pain points:** Scores 92 on a codebase with 5,296 structural findings across 989k lines and concludes there is nothing to do. Fixes forty findings, reruns, sees 92 again, and concludes the tool does not work.
- **Current workaround:** Ignores the score and reads the finding count, which has no denominator either and so punishes them for having a large project.
- **Success looks like:** The first scan lands somewhere below the top band, and the second scan after a real repair session lands measurably higher.

### Maintainer running the tool in CI
- **Role:** Uses the generated `.github/workflows/rust-doctor.yml`, or `--scope baseline` on pull requests the way `dogfood.yml` does.
- **Behaviors:** Sets a threshold and expects it to mean something. Compares the score across commits.
- **Pain points:** Cannot pick a threshold: every healthy repository scores between 90 and 100, so 90 gates nothing and 95 fails on noise. The score drifts downward as the repository grows, so a threshold set in month one fails in month six for reasons unrelated to quality.
- **Current workaround:** Gates on the diagnostic count or on the delta instead, which is why the score is decorative in CI today.
- **Success looks like:** A threshold that survives the repository tripling in size, and a score that moves when the pull request is actually better or worse.

### Arthur calibrating the catalog
- **Role:** Admits rules through `.claude/skills/rule-admit` and adjudicates precision through `.claude/skills/corpus-adjudicate`.
- **Behaviors:** Measures a rule's false-positive rate before shipping it. Reads `tests/corpus.json` as the record of what the tool is worth.
- **Pain points:** Cannot tell whether a newly admitted rule improved the score's discriminating power, because the score has none to improve. Cannot tell whether a rule is worth its noise, because its cost to the score saturates at 21 occurrences regardless.
- **Current workaround:** Judges rules on adjudicated precision alone, which answers whether the rule is right and not whether it matters.
- **Success looks like:** Admitting a rule changes the corpus spread by a measurable amount, so the catalog can be grown against evidence.

## Research Findings

### Competitive Context

No shipped code-quality product uses exponential decay for its score. SonarQube's SQALE debt ratio is linear and normalized by `ncloc`, banded A-E at 5/10/20/50 %, but its reliability and security ratings are worst-severity rather than density: one blocker issue sets the rating regardless of size. Code Climate publishes a 0-4 GPA aggregated from per-file grades. CodeScene's Code Health is a continuous 1-10 aggregated LOC-weighted across files.

Two conclusions. First, LOC normalization is the industry norm for debt-style metrics, which supports the denominator. Second, exponential decay has no prior art, so λ cannot be imported from anywhere and has to be calibrated against this corpus and frozen against it. The tier ceilings this PRD keeps unchanged are the worst-severity mechanism SonarQube uses for its reliability rating, so core-v3 keeps both mechanisms rather than choosing one: density sets the value, tier sets the ceiling.

### Best Practices Applied

Density per kloc has a documented failure mode: it rewards verbosity. The standard mitigations are to normalize by a complexity measure instead of raw lines, or to pair the density metric with a size metric. rust-doctor already ships the second: `oversized_unit`, `complex_function`, `duplicate_function_body` and `near_duplicate_function_body` charge for exactly the code that would dilute a density. This PRD keeps raw physical lines as the denominator, because the alternative requires agreeing on what a comment is and a second traversal, and validates the mitigation empirically in US-018 rather than assuming it.

Scale invariance is the property the industry norm buys and core-v2 lacks. Duplicating a workspace must not change its score. This is asserted directly rather than inferred.

### Existing Codebase Constraints

`rust_line_count` already exists at `tests/support/corpus.rs:202`, recursing over `.rs` files and summing `source.lines().count()`. It counts blanks, comments and test files, which is the wrong population for the score but the right definition of a line: this PRD reuses the definition and changes the population.

`SourceUnit::source()` at `src/source_kernel.rs:209` returns the already-decoded text, so a per-unit line count during the walk costs one `lines().count()` per file already in memory. `WalkCounters` at `src/source_kernel.rs:145` is where the tallies already live.

`report.rs:200,211` compare the stored audit against `rebuild_for_scope`. Any input the score reads must be a stored field of the audit block, or the report fails its own validity check.

`src/execution.rs:371` gates `source_kernel::enumerate` on `source_rules || structure_rules || dependency_truth`. A plan with only Clippy rules active has no enumeration, so it would have no denominator.

51 of the 97 diagnostics on a self-scan of this repository carry `context: "tests"` and do not weigh: `aggregate_rules` at `src/audit.rs:790` excludes them from `scored_occurrences` while keeping them in `occurrences`. The core-v3 numerator inherits this and the denominator must match it.

## Assumptions & Constraints

### Assumptions (to validate)

- **A1 (HIGH):** The catalog's own size and duplication rules deter denominator padding well enough that verbosity is not a profitable strategy. Validated by US-018.
- **A2 (MEDIUM):** λ calibrated on the healthy population transfers to the agent population. The agent corpus is scanned with every Clippy rule off (`corpus.json` `trust_boundary`), so λ_reliability and λ_maintainability cannot be calibrated against it. Recorded as a limitation, not resolved.
- **A3 (MEDIUM):** Physical lines are a good enough denominator that switching to non-blank non-comment lines would not change any band. Not validated in this PRD; recorded in Open Questions.
- **A4 (LOW):** The pre-implementation simulation is directionally right. It used all-context numerators and all-lines denominators because the corpus record does not carry the production/test split per rule. Both approximations point the same way and partly cancel. The exact numbers come from US-011, not from this document.

### Hard Constraints

- The five dimensions, their weights 4/3/2/2/2 over 13, the tier dimension ceilings (P0→20, P1→50, P2→75, P3→none), the overall ceilings (P0→40, P1→65) and the three bands (Great ≥ 75, Needs work ≥ 50, Critical below) are unchanged. Only the value fed into the ceiling changes.
- The score must remain a pure function of the stored audit block and the report's diagnostics, because `report.rs:200,211` replay it as a validity check.
- `--json` reports stay workspace-relative: no absolute path, no environment variable, no user data. A line count is a number and carries none of those.
- The tool reaches no network. The corpus replays from a local clone cache outside this repository.
- Both `RUST_DOCTOR_CORPUS_DIR` and `RUST_DOCTOR_CORPUS_ARTIFACTS` sit outside this repository, and no corpus repository is ever committed here.
- Every test that runs `cargo` or the built binary sets `CARGO_TARGET_DIR` to its own scratch directory through `support::scan_target`.
- The frozen v7 archive keeps projecting: `project_v11_wire_to_v7` drops the whole `audit` member, so a new audit field is invisible to it and only the literal `14` at `tests/support/mod.rs:153` moves.
- Every file of `src/audit/` stays under the 1000 lines `oversized_unit` reports, tests included, per `the_audit_holds_the_size_bound_it_scores_for`.
- No production code carries `unwrap`, `expect`, `panic!` or `dbg!`.

## Quality Gates

Run before any story is called complete:

```
cargo build --release
cargo clippy --all-targets --no-deps -- -D warnings
cargo test
```

The corpus reproduction is a separate gate, run once per recalibration story in EP-003:

```
RUST_DOCTOR_CORPUS_DIR=<clone cache outside this repository> \
RUST_DOCTOR_CORPUS_ARTIFACTS=<scratch outside this repository> \
cargo test --test corpus_precision
```

## Epics & User Stories

### EP-001: The denominator

Produce a production source-line count during the walk that already runs, publish it in the audit block, and make it available on every plan.

**Definition of Done:** Every scan, including a Clippy-only plan, publishes a production line count in `--json`, the count is stable across two runs of the same workspace, and no existing test changes behavior beyond the schema bump.

#### US-001: Count production source lines during the source-kernel walk
**Description:** As the score, I want the number of production Rust lines the scan enumerated so that a density has a denominator counted over the same population its numerator is charged on.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a workspace, when `source_kernel::enumerate` runs, then the resulting `Enumeration` publishes the total lines of every unit for which `SourceUnit::is_test_code` is false
- [ ] Given a unit already decoded by the walk, when its lines are counted, then the count is `source().lines().count()` and the file is not read a second time
- [ ] Given a file the walk skipped for a per-file byte limit, when the count is produced, then that file contributes zero lines and the enumeration records that the count is a floor
- [ ] Given the global byte budget is exhausted mid-walk, when the count is produced, then it covers the units actually loaded and is marked incomplete
- [ ] Given a workspace with zero production Rust files, when the count is produced, then it is zero and no error is raised
- [ ] Given the same workspace scanned twice, when both counts are produced, then they are equal

#### US-002: Enumerate unconditionally so every plan has a denominator
**Description:** As a user running a Clippy-only policy, I want my score computed on the same scale as everyone else's so that switching off the native rules does not silently change what the number means.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given a plan with every `rust_doctor::*` rule off, when `inspect` runs, then `source_kernel::enumerate` still runs and the line count is published
- [ ] Given such a plan, when the report is built, then no `rust_doctor::*` diagnostic appears and the score is authoritative
- [ ] Given the enumeration fails, when the report is built, then a `ReportError` at stage `source` is recorded, the report is complete, and the authoritative flag is dropped
- [ ] Given a workspace previously scanned with the native rules on, when it is rescanned with them off, then the published line count is identical
- [ ] The wall-clock cost of enumerating on a Clippy-only plan is recorded in the story's completion note for a workspace of at least 50k lines

#### US-003: Publish the line count in the audit block
**Description:** As the report's own validity check, I want the denominator stored rather than recomputed so that replaying `rebuild_for_scope` reproduces the stored score exactly.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given a scan, when `--json` is emitted, then the audit block carries the production line count alongside `source_files`
- [ ] Given a stored report, when `rebuild_for_scope` is replayed on it, then the rebuilt audit equals the stored audit, which is what `report.rs:200,211` already assert
- [ ] Given an inventory the walk marked incomplete, when the audit block is built, then the completeness flag is carried on the inventory itself and not recovered from `score.authoritative`
- [ ] Given a scan whose inventory is incomplete, when the score is computed, then the authoritative flag is false
- [ ] `share_url` carries the line count and no absolute path, environment variable or user data

#### US-004: Freeze what counts as a counted line
**Description:** As a future maintainer, I want the counted-line definition pinned to a fixture so that changing it is a moved assertion rather than a silent recalibration of every score ever published.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Given a fixture workspace with a known mix of blank lines, comment lines, a file without a trailing newline, and a `tests/` target, when it is scanned, then the published line count equals a value frozen in the test
- [ ] Given a file whose last line has no newline terminator, when it is counted, then that line is counted once
- [ ] Given a file under a `tests/` target, when the count is produced, then its lines are excluded
- [ ] Given a `#[cfg(test)]` module inside a production file, when the count is produced, then the file's lines are counted in full and the exclusion is per unit, not per module, with that decision stated in the test's name or a comment
- [ ] Given a non-UTF-8 file, when the walk reaches it, then it contributes zero lines and the enumeration is marked a floor

---

### EP-002: The core-v3 penalty

Replace the occurrence step function with a density, the linear dimension score with an exponential one, and make the denominator depend on the producer.

**Definition of Done:** `SCORE_MODEL` reads `core-v3`, duplicating a workspace ten times moves the score by zero points, repairing sites moves it at every density, and the tier ceilings and weights are provably unchanged.

#### US-005: Dimension density from distinct scored sites
**Description:** As the score, I want each dimension's numerator to be its distinct scored sites weighted by severity so that a finding counted once in the report is charged once by the score.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Given a rule firing at N distinct sites in production code, when its numerator contribution is computed, then it is `N × severity_weight` with `warn` weighing 1 and `error` weighing 2
- [ ] Given a diagnostic whose `context` marks it outside production code, when the numerator is computed, then it contributes nothing, while still appearing in `occurrences` and in the report body
- [ ] Given a structural family diagnostic naming K members through `related`, when the numerator is computed, then it contributes one site, not K
- [ ] Given a diagnostic whose category maps to no dimension, when the numerator is computed, then it contributes nothing and the authoritative flag is dropped
- [ ] Given a diagnostic with `Severity::Unknown`, when the numerator is computed, then it contributes nothing and the authoritative flag is dropped
- [ ] `OCCURRENCE_STEPS` and `OCCURRENCE_CEILING` are removed and no test references them

#### US-006: Exponential dimension score and the lambda table
**Description:** As a user repairing findings, I want the score to move at every density so that fixing forty things is visible whether I had fifty or five hundred.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] Given a dimension density `D` and its `λ`, when the dimension score is computed, then it equals `round(100 · exp(−D / λ))` clamped to 0..=100
- [ ] Given `D = 0`, when the dimension score is computed, then it is 100
- [ ] Given `D = λ`, when the dimension score is computed, then it is 37
- [ ] Given two densities where the second is strictly larger, when both are scored, then the second score is less than or equal to the first, and strictly less whenever the two round differently
- [ ] The λ table declares one value per dimension with a doc comment stating that λ is the density costing 63 points, and no λ is zero
- [ ] The computation uses no `unwrap`, `expect`, `panic!` or floating-point value that can reach `NaN` from a finite density and a non-zero λ

#### US-007: Denominator chosen by the producer that raised the finding
**Description:** As a maintainer of a large workspace, I want a missing lockfile to cost the same as it costs a small workspace so that workspace-level facts are not diluted by size.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-006

**Acceptance Criteria:**
- [ ] Given a finding from `clippy`, `source-kernel` or `structure`, when its density contribution is computed, then it is divided by production kilolines
- [ ] Given a finding from `cargo-health` or `repo`, when its density contribution is computed, then it is divided by 1
- [ ] Given a rule whose producer cannot be resolved from the catalog, when the density is computed, then it is treated as workspace-scoped and the authoritative flag is dropped
- [ ] The per-producer split is derived from the catalog's existing `producer` field, and no new field is added to `RuleDefinition`
- [ ] Given the same workspace with its Rust source tripled by duplication, when both scans are compared, then the `dependencies` dimension is identical

#### US-008: Kiloline floor and the degenerate zero-line case
**Description:** As the author of a 120-line crate, I want three findings not to read as a catastrophic density so that a small project is not punished for being small.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] Given a workspace under 2000 production lines, when the density is computed, then the denominator is 2.0 kilolines
- [ ] Given a workspace of 120 lines with 3 distinct maintainability sites, when it is scored, then the maintainability dimension is the value the floor produces, `round(100 * exp(-1.5 / lambda_maintainability))`, which is 61 under the provisional lambda of 3.0, rather than the 0 the same three sites score against their own 0.12 kilolines. The absolute value is not a target here: US-014 freezes the lambda table against the measured corpus spread, and this criterion is the floor doing its work, not the calibration
- [ ] Given a workspace with zero production lines, when the audit is built, then no score is published, which is the existing `(source_files > 0).then(...)` gate at `src/audit.rs:279` extended to the line count
- [ ] Given a workspace with zero production lines and zero diagnostics, when the report is built, then it is valid and carries no score rather than a 100
- [ ] The floor constant is named, carries a doc comment stating the case it protects, and is asserted in a test naming that case

#### US-009: Redefine what a rule contributes so repair ranking survives
**Description:** As a reader of the report body, I want the ranking of what to fix first to reflect the new penalty so that the order I am given and the points I am promised agree.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] Given a rule that fired, when its contribution is computed, then it is the number of points the weighted score recovers if that rule's sites are removed, computed on the core-v3 penalty
- [ ] Given a rule the corpus adjudicated at a non-zero noise rate, when `expected_repair_value` is computed, then the contribution is discounted by that rate exactly as today
- [ ] Given a rule adjudicated at 10000 basis points of noise, when the report body is ordered, then it is not ranked first, which is what `a_rule_the_corpus_measured_wrong_is_not_ranked_first` already asserts
- [ ] Given the top three ranked rules, when `projected_after_top_three` is computed, then it equals the score with those three rules' sites removed and is published only when the scan is authoritative
- [ ] Given a rule whose sites all sit outside production code, when the ranking is built, then it contributes zero and is not ranked

#### US-010: Bump the model and the schema
**Description:** As a consumer of `--json`, I want the model name and schema version to say the score changed so that a stored report is never silently reinterpreted under a different scale.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-008, US-009

**Acceptance Criteria:**
- [ ] `SCORE_MODEL` is `core-v3` and `AuditScore::is_valid` at `src/audit.rs:406` refuses a report claiming `core-v2`
- [ ] `SCHEMA_VERSION` is 15 and the assertion at `src/report.rs:192` follows
- [ ] The literal `14` in the prefix at `tests/support/mod.rs:153` becomes `15` and the frozen v7 archive still projects, proving no historical field disappeared or changed type
- [ ] Every frozen oracle under `tests/fixtures/` that carries a schema version or a model name is regenerated, and `cargo test` is green
- [ ] `README.md`'s rule count is unchanged, because this PRD admits no rule

#### US-011: Prove scale invariance and monotonic repair
**Description:** As a maintainer, I want the two properties this model exists for asserted directly so that a future edit that loses either fails a test rather than a corpus run.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-010

**Acceptance Criteria:**
- [ ] Given a synthetic report and a second one where every per-site finding and every production line are multiplied by ten, when both are scored, then the two values are equal
- [ ] Given a synthetic report and a second with 90 % of its sites removed, when both are scored, then the second is strictly greater whenever the first is below 100
- [ ] Given a synthetic report whose production lines are doubled with no new finding, when both are scored, then the second is greater than or equal to the first
- [ ] Given a synthetic report with a single `cargo-health` finding and a second where only the line count is multiplied by ten, when both are scored, then the `dependencies` dimension is identical
- [ ] Given a report at every tier, when the ceilings are applied, then the published ceiling matches `tier_dimension_ceiling` and `tier_overall_ceiling` unchanged
- [ ] Given a synthetic report whose diagnostics reach `DIAGNOSTIC_LIMIT`, when it is scored, then the computation completes, the value stays in 0..=100, and no arithmetic overflows

---

### EP-003: The recalibration

Regenerate the corpus under core-v3, freeze λ against the spread it produces, and re-freeze the local oracles.

**Definition of Done:** `tests/corpus.json` records an 18-repository measurement under core-v3, `score_distribution` publishes spread rather than collapse, and λ is pinned to that measurement by a test.

#### US-012: Regenerate tests/corpus.json under core-v3
**Description:** As the record of what the tool is worth, I want the corpus measured under the shipped model so that every rate and every score in it describes the binary that ships.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-011

**Acceptance Criteria:**
- [ ] Given the pinned clone cache and the pinned toolchain, when `corpus_precision` is replayed, then all 18 observations are regenerated with `model: "core-v3"`
- [ ] Given each observation, when it is written, then it carries the production line count the scan measured, so the density is reproducible from the record alone
- [ ] `the_published_catalog_matches_the_shipped_policy` passes against the regenerated file
- [ ] `the_noise_the_score_ranks_by_matches_the_adjudicated_rate` passes: `CORPUS_NOISE` is unchanged, because this PRD adjudicates nothing
- [ ] `each_population_publishes_its_own_rate_from_its_own_sites` passes, and no Clippy rule carries an `agent` rate
- [ ] `no_corpus_repository_is_committed_in_this_repository` passes
- [ ] Both corpus paths sit outside this repository and no test touches the network

#### US-013: Redefine score_distribution to publish spread
**Description:** As the reader of the corpus, I want the distribution block to report the spread the model achieves so that the record measures the score instead of recording that it collapsed.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-012

**Acceptance Criteria:**
- [ ] Given the regenerated corpus, when `score_distribution` is written, then it publishes minimum, maximum, spread, median and per-band counts for the healthy population
- [ ] Given the agent population, when its distribution is written, then it publishes its own block with the same members, which is `null` today
- [ ] Given both populations, when the separation is written, then the difference between the two medians is published as a number
- [ ] `collapsed_into_one_band` is retained and is false, or is replaced by a threshold assertion naming the minimum spread the model must achieve
- [ ] `the_corpus_score_distribution_is_published_with_its_spread` at `tests/corpus_precision.rs:922` asserts the new shape byte for byte
- [ ] Given a future model whose spread falls below the recorded threshold, when the corpus is replayed, then the test fails and names the spread it measured

#### US-014: Freeze the lambda table against the measured spread
**Description:** As a future maintainer, I want λ pinned to the measurement that justified it so that changing a λ is a deliberate recalibration rather than a tuning knob.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-013

**Acceptance Criteria:**
- [ ] Given the λ table and the regenerated corpus, when the freeze test runs, then it recomputes every corpus score from the recorded densities and compares against the recorded scores
- [ ] Given a λ changed without regenerating the corpus, when the test runs, then it fails and names the dimension whose λ moved
- [ ] Given the corpus regenerated without changing λ, when the test runs, then it passes
- [ ] The test states in a comment that λ_reliability and λ_maintainability are calibrated on the healthy population alone, because the agent population is scanned with Clippy off
- [ ] The corpus records the λ table it was measured under, alongside the toolchain version it already records

#### US-015: Re-freeze the local audit oracle
**Description:** As the guard on the local CLI experience, I want the frozen audit oracle to describe core-v3 while the core-v2 file survives as the archive of what changed.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-011

**Acceptance Criteria:**
- [ ] A new `tests/fixtures/local-cli-experience/audit-core-v3.json` is written and is what the test compares against
- [ ] `tests/fixtures/local-cli-experience/audit-core-v2.json` is retained unchanged and is referenced by a comment naming it as the archive of the previous model
- [ ] Given each of the 18 embedded scores, when the oracle is regenerated, then each entry carries `model: "core-v3"` and the line count its density was computed from
- [ ] Given the oracle, when it is compared, then the comparison is field by field, not a string equality on the whole file
- [ ] Every fixture regenerated in this story was produced under `support::scan_target`, so no two fixtures shared a target directory
- [ ] Given a stale oracle regenerated under an inherited target directory, when the test replays it, then the cache hit is detectable rather than reading as a scan that found nothing

#### US-016: Record the dogfood delta on this repository
**Description:** As the maintainer of this repository, I want the score change on rust-doctor itself recorded so that the recalibration is honest about what it does to the project that ships it.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-012

**Acceptance Criteria:**
- [ ] Given a scan of this repository under core-v3, when the score is published, then the value, the five dimensions and the production line count are recorded in the story's completion note
- [ ] Given the recorded score, when it is compared against the core-v2 value of 94, then the delta and the dimension responsible for it are named
- [ ] Given `dogfood.yml` on a pull request, when it runs in baseline scope, then it still passes, or the threshold it enforces is moved in the same commit with the reason stated
- [ ] `no_unit_of_this_crate_s_own_source_is_a_hotspot` still passes
- [ ] Every file of `src/audit/` remains under 1000 lines, which `the_audit_holds_the_size_bound_it_scores_for` asserts
- [ ] Given the recorded score falls below the band this repository claims, when the story completes, then the fact is written into the completion note rather than absorbed by moving a threshold silently

---

### EP-004: What reads the score

Update every surface that renders, transmits or documents the score, and measure the one risk this model carries.

**Definition of Done:** No surface says `core-v2`, the two reports render the new value correctly at their minimum widths, and the verbosity risk has a measured answer.

#### US-017: The two reports render the recalibrated score
**Description:** As a reader of either report, I want the score block to render the new value and its band correctly so that a wider spread does not break a layout tuned to a 13-point range.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-011

**Acceptance Criteria:**
- [ ] Given a score of 0, 37, 73 and 100, when the linear report renders at `MIN_WIDTH`, then the bar fill, the face and the label are correct for each
- [ ] Given the same four values, when the interactive report renders at 40 columns, then the block fits and `score_block::MIN_BLOCK_COLUMNS` is respected
- [ ] Given a failed scan, when the report renders, then it is still the two sections `render_failure` produces and no score block is drawn
- [ ] Given a score in each of the three bands, when the label is computed, then it matches `score_label` unchanged
- [ ] Both reports read the same `score_block` model, and no second copy of the bar arithmetic is introduced

#### US-018: The share URL and the agent skill carry the new model
**Description:** As an agent driving the tool, I want the skill to describe the score it will actually see so that its advice matches the number.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-011

**Acceptance Criteria:**
- [ ] Given a report, when `share_url` is built, then its payload names `core-v3` and carries the line count, with no absolute path and no user data
- [ ] Given `skills/rust-doctor/SKILL.md`, when it describes the score, then it names the current model and no retired one
- [ ] Given `skills/rust-doctor/references/expert-review.md`, when it explains what a score means, then every threshold it states matches the shipped bands
- [ ] `tests/skill_contract.rs` passes: every long flag the skill documents exists in `--help`, every rule id it names is in `catalog()`, and the rule count it states matches
- [ ] Given a `ShareError`, when the URL cannot be built, then the report still renders and no panic occurs

#### US-019: Spike: measure whether verbosity is a profitable strategy
**Description:** As the author of this model, I want the documented failure mode of density metrics measured on this catalog so that A1 is a number rather than an argument.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-012

**Acceptance Criteria:**
- [ ] Given a corpus repository, when its production source is padded with N lines of plausible filler that trips no catalogued rule, then the score change is recorded for N at 10 %, 50 % and 100 % of the original size
- [ ] Given the same repository padded with N lines of duplicated existing functions, then the score change is recorded for the same three values of N, and the structural rules that fired are named
- [ ] Given both curves, when they are compared, then the story states whether padding is profitable and at what padding-to-detection ratio it stops being so
- [ ] If padding is profitable at any measured ratio, then a follow-up is filed naming the mitigation considered and the reason it is or is not adopted now
- [ ] The spike commits no corpus repository into this repository and touches no network

## Functional Requirements

| ID | Requirement | Stories |
|----|-------------|---------|
| FR-1 | The scan publishes a count of production Rust lines, counted during the source-kernel walk over the units `SourceUnit::is_test_code` reports false for | US-001 |
| FR-2 | The enumeration runs on every plan, including one with every native rule off | US-002 |
| FR-3 | The line count is a stored field of the audit block and the score reads only stored inputs | US-003 |
| FR-4 | A dimension's numerator is its distinct scored sites weighted by severity, excluding non-production sites and counting a structural family once | US-005 |
| FR-5 | A dimension's score is `round(100 · exp(−D / λ))`, clamped to 0..=100 | US-006 |
| FR-6 | Findings from `clippy`, `source-kernel` and `structure` divide by production kilolines; findings from `cargo-health` and `repo` divide by 1 | US-007 |
| FR-7 | The kiloline denominator has a floor of 2.0, and a workspace with zero production lines publishes no score | US-008 |
| FR-8 | A rule's contribution is the points the weighted score recovers if its sites are removed, discounted by its adjudicated noise for ranking | US-009 |
| FR-9 | `SCORE_MODEL` is `core-v3` and `SCHEMA_VERSION` is 15 | US-010 |
| FR-10 | The tier dimension ceilings, the overall ceilings, the five weights and the three bands are unchanged | US-011 |
| FR-11 | `tests/corpus.json` records 18 observations under core-v3, each with the line count its density used | US-012 |
| FR-12 | `score_distribution` publishes spread, median and per-band counts for both populations and the separation between them | US-013 |
| FR-13 | The λ table is frozen against the corpus measurement and recorded in it | US-014 |

## Non-Functional Requirements

| ID | Requirement | Measurement |
|----|-------------|-------------|
| NFR-1 | Counting lines adds no additional file read | Zero new `read_to_string` calls in the walk, verified by inspection and by the unchanged `WalkCounters::files_read` on a fixture |
| NFR-2 | Enumerating unconditionally costs at most 15 % of total scan wall clock on a Clippy-only plan | Measured on a workspace of at least 50k lines, recorded in US-002 |
| NFR-3 | Score spread across the 18 corpus repositories is at least 20 points | `score_distribution.spread` in the regenerated `tests/corpus.json` |
| NFR-4 | Healthy and agent medians differ by at least 5 points | The separation member US-013 adds |
| NFR-5 | Repairing 90 % of sites moves the median corpus score by at least 8 points | Computed in US-012 from the recorded densities |
| NFR-6 | Duplicating a workspace 10x changes the score by exactly 0 points | Asserted in US-011 |
| NFR-7 | Every file of `src/audit/` stays under 1000 lines, tests included | `the_audit_holds_the_size_bound_it_scores_for` |
| NFR-8 | The score computation performs no allocation per diagnostic beyond what `aggregate_rules` already performs | Verified by inspection of the density accumulation |

## Edge Cases & Error States

| Case | Expected behavior | Story |
|------|-------------------|-------|
| Zero production Rust lines | No score published; report valid; existing `(source_files > 0).then(...)` gate extended | US-008 |
| Workspace under 2000 production lines | Denominator floors at 2.0 kilolines | US-008 |
| Enumeration fails | `ReportError` at stage `source`, report complete, authoritative flag dropped | US-002 |
| Global byte budget exhausted mid-walk | Line count covers the units loaded, marked incomplete, authoritative flag dropped | US-001, US-003 |
| Non-UTF-8 file | Contributes zero lines, count marked a floor | US-004 |
| File without trailing newline | Its last line counted once | US-004 |
| Diagnostic outside production code | Counted in `occurrences`, excluded from the numerator, ranked at zero | US-005, US-009 |
| Diagnostic with no catalogued category | Contributes nothing; authoritative flag dropped | US-005 |
| Diagnostic with `Severity::Unknown` | Contributes nothing; authoritative flag dropped | US-005 |
| Rule whose producer cannot be resolved | Treated as workspace-scoped; authoritative flag dropped | US-007 |
| Structural family naming K members | One site, not K | US-005 |
| `--scope changed` with a small changed set | Numerator is the scoped subset, denominator is the stored workspace count; the score rises, matching core-v2's direction | US-003 |
| A rule adjudicated at 100 % noise | Ranked last, not first | US-009 |
| Failed scan | Two sections, no score block | US-017 |
| `share_url` fails | Report still renders, no panic | US-018 |

## Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Density rewards verbosity: padding the denominator raises the score | Medium | High | The catalog already charges for the padding vector through `oversized_unit`, `duplicate_function_body` and `near_duplicate_function_body`. US-019 measures the padding-to-detection ratio rather than assuming it. |
| λ has no external reference and is fitted to 18 repositories | High | Medium | λ is frozen against the corpus by US-014 and recorded inside it, so moving one is a moved assertion. The corpus is the only calibration instrument that exists; the alternative is an unmeasured constant. |
| λ_reliability cannot be calibrated on the agent population, scanned with Clippy off | Certain | Medium | Calibrate on the healthy population alone and write the limitation into the corpus and into the freeze test. Do not infer a reliability rate for agent code from a Clippy-off scan. |
| Every score every user has ever seen changes | Certain | Medium | `SCORE_MODEL` and `SCHEMA_VERSION` both move, so a stored report is never reinterpreted under the wrong scale. The v7 archive keeps projecting. |
| The pre-implementation simulation used all-context numerators and all-line denominators, so its numbers are approximate | Certain | Low | The simulation sets the design, not the constants. US-012 produces the numbers the model ships with, and the goals table is expressed as thresholds rather than as predicted values. |
| Unconditional enumeration slows a Clippy-only scan | Low | Low | NFR-2 bounds it at 15 % and US-002 records the measurement. The walk is already the shared pass every other producer reads. |
| Regenerating every frozen oracle hides an unintended change | Medium | Medium | Oracles are compared field by field, the v7 projection proves no historical field disappeared, and `audit-core-v2.json` is kept as the archive to diff against. |
| 17 of 18 repositories still read "Great" | High | Low | Bands are a hard constraint of this PRD. The month-6 goal lowers the top-band count to 14 of 18, which is a band question to answer after the scale is trustworthy, not before. |

## Non-Goals

- Changing the five dimensions, their weights, the tier ceilings or the three band thresholds. Only the value fed into the ceiling changes.
- Admitting, retiring or re-adjudicating any rule. `CORPUS_NOISE` is unchanged and `README.md`'s rule count does not move.
- Switching the denominator to non-blank non-comment lines, to a complexity measure, or to any weighting of lines. Physical lines, one definition, frozen.
- Making the score comparable across languages or against SonarQube, Code Climate or CodeScene numbers.
- Adding a per-rule weight, a per-rule λ or any configurable scoring knob. λ is per dimension and is not user-settable.
- Changing what `--scope baseline` reports as introduced, inherited or fixed. `src/delta.rs` is untouched.
- Publishing a second score or a legacy score alongside core-v3.

## Files NOT to Modify

- `src/delta.rs` and `src/delta/`: the baseline pairing is independent of the score, and its 32-case oracle stays frozen.
- `src/policy/catalog.rs`, `src/policy/noise.rs`, `src/policy/rejected.json`: no rule is admitted, retired or re-adjudicated.
- `src/git.rs`, `src/git_scope.rs`: the scope resolution is unchanged.
- `src/source_kernel/detectors.rs`, `src/structure/` detector families, `src/cargo_health.rs`, `src/repo_hygiene.rs`: no producer changes what it reports.
- `tests/fixtures/baseline/delta-oracle.json`: frozen.
- `tests/fixtures/local-cli-experience/audit-core-v2.json`: retained as the archive of the previous model.
- `npm/rust-doctor/` and `.github/workflows/release.yml`: publishing is untouched.
- `LICENSE-MIT`, `LICENSE-APACHE`.

## Technical Considerations

- **Where the denominator is produced:** `source_kernel::enumerate` loads and parses each reachable file once and `SourceUnit::source()` at `src/source_kernel.rs:209` hands back the decoded text, so the count is one `lines().count()` per unit already in memory, and `SourceUnit::is_test_code` is the production/test policy that already exists. Recommended: count there, and extend `WalkCounters` at `src/source_kernel.rs:145` rather than adding a parallel tally. Rejected alternative: `src/audit/source_inventory.rs`, which reads Cargo's dep-info and never reads source, so it would need a walk of its own and would become a second answer to a question the kernel already answers. Engineering to confirm the count survives a unit several targets reach, where `unanimous` already abstains on disagreement.
- **Where the denominator is stored:** `report.rs:200,211` compare the stored audit against `rebuild_for_scope`, so any input the score reads must be a stored field. Recommended: a sibling of `source_files` on the audit block, with the inventory's own completeness flag carrying incompleteness rather than `score.authoritative` recovering it, which is the precedent the audit block already set. Engineering to confirm no `--scope` path can rebuild against a different count than the one stored.
- **Floating point in a crate that had none in the score:** the exponential is `f64`, and the only value reaching it is `density / λ` with a non-zero λ and a density bounded by `DIAGNOSTIC_LIMIT`, so `NaN` is unreachable. Recommended: round to `u8` at the boundary so every stored score stays an integer and `rebuild_for_scope` stays byte-reproducible. Open trade-off: a fixed-point implementation would remove the argument entirely at the cost of a table; engineering to decide whether the argument is worth the table.
- **What is deleted and what is untouched:** `OCCURRENCE_STEPS` and `OCCURRENCE_CEILING` go, `severity_penalty_quarters` becomes a weight of 1 or 2, `dimension_score` becomes the exponential. Recommended: leave `capped`, `worse_tier`, `tier_dimension_ceiling`, `tier_overall_ceiling`, `dimension_weight_twice`, `weighted_score` and `score_label` untouched, which is what makes the tier and band constraint checkable by inspection instead of by argument. Engineering to confirm no test reaches the removed constants.
- **How the per-producer split is read:** the catalog's `producer` field already exists and `validate_catalog` already refuses an identifier whose prefix disagrees with it. Recommended: derive the split from that field alone and add nothing to `RuleDefinition`. Open question for engineering: whether the split belongs in `src/audit.rs` or beside `Producer` in `src/policy/catalog.rs`, given that it is a scoring decision expressed over a policy concept.
- **Migration:** `SCHEMA_VERSION` 14 to 15, additive only. `project_v11_wire_to_v7` drops the whole `audit` member, so a new audit field is invisible to the v7 archive and only the literal at `tests/support/mod.rs:153` moves. Rollback plan: none. `SCORE_MODEL` moving to `core-v3` is what makes a stored core-v2 report refuse to validate rather than be reinterpreted, and there is no dual-model path, which is a Non-Goal.
- **How this lands under the size bound:** `the_audit_holds_the_size_bound_it_scores_for` keeps every file of `src/audit/` under 1000 lines, tests included, and the module is already three files because it was once 1624. Recommended: if the density accumulation and its tests push a file over, split it rather than relax the test. Engineering to decide whether the λ table and the density arithmetic want a file of their own.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Score spread across the 18 corpus repositories | 13 points (87-100) | ≥ 20 points | Month-1 | `score_distribution.spread` in `tests/corpus.json` |
| Score spread across the 18 corpus repositories | 13 points | ≥ 25 points | Month-6 | Same |
| Healthy median minus agent median | 1.0 point (93.5 vs 92.5) | ≥ 5 points | Month-1 | The separation member US-013 adds |
| Healthy median minus agent median | 1.0 point | ≥ 8 points | Month-6 | Same |
| Median score gain from repairing 90 % of sites | 0 to 1 point | ≥ 8 points | Month-1 | Recomputed in US-012 from the recorded densities |
| \|Spearman ρ\| between repository size and score | 0.794 | ≤ 0.30 | Month-1 | Computed over the 18 recorded line counts and scores |
| Score change under 10x whole-workspace duplication | −2 points | 0 points | Month-1 | Asserted in US-011 |
| Corpus repositories in the top band | 18 of 18 | ≤ 17 of 18 | Month-1 | `score_distribution.bands` |
| Corpus repositories in the top band | 18 of 18 | ≤ 14 of 18 | Month-6 | Same |
| `SCORE_MODEL` | `core-v2` | `core-v3` | Month-1 | `src/audit.rs` constant |
| `SCHEMA_VERSION` | 14 | 15 | Month-1 | `src/report.rs` constant |
| Padding-to-detection ratio at which verbosity stops paying | Unmeasured | Published, whatever the value | Month-1 | US-019 measurement |

## Open Questions

1. Should `score_distribution` keep `collapsed_into_one_band` as a boolean, or replace it with a minimum-spread threshold the corpus run asserts against? The second is a stronger gate and turns the record into an instrument; the first is what exists. US-013 accepts either and the choice is made there.
2. Would non-blank non-comment lines change any band? A3 assumes not. Answering it costs a second pass over already-decoded text and is cheap; it is out of scope here because it would need its own frozen definition of a comment, and the current definition is already frozen by US-004.
3. Should the agent population get its own λ once the trust boundary allows a Clippy scan of it, or should it never be scanned with Clippy at all? The trust boundary says never, so the question is whether a second λ table calibrated on native-only scans is worth having. Not answered here.
4. At what point do the bands move? All 18 repositories still read "Great" at the recommended λ except one. The bands are a hard constraint of this PRD, and the month-6 goal is the trigger to revisit them once the scale itself is trusted.
5. Does `--scope changed` want the workspace denominator or the changed-file denominator? This PRD keeps the workspace one, matching core-v2's direction, and records the question rather than settling it.
[/PRD]
