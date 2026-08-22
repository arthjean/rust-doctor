[PRD]
# PRD: The Measurement the Ranking Rests On

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-22 | arthjean | Initial draft |

## Problem Statement

1. **The published context misses half of its own rule, and the corpus is measured through it.** `test_context` (`src/structure.rs:659`) says what a test gate is: "Does this node sit under a `#[cfg(test)]` module or inside a `#[test]` function? Those are not a Cargo target, so no target kind names them, and they are exactly the non-production code a census must not charge for." It reads `node.ancestors()`, so it sees the inline form `#[cfg(test)] mod tests { ... }` and never the out-of-line form `#[cfg(test)] mod tests;`, whose gate sits in the declaring file and whose body is a separate unit. The unit-level answer does not cover it either: `SourceUnit::context` (`src/source_kernel.rs:244`) reads `TargetContexts` alone, built by `target_contexts` (`src/source_kernel/walk.rs:241`) from `DiagnosticContext::from_target_kinds` (`src/report.rs:363`), which maps only Cargo's `test`, `bench`, `example` and `custom-build` kinds. A library's own `#[cfg(test)]` modules are not a Cargo target. Measured on the pinned corpus: **31 of the 430 agent production duplication families, 7.2 percent, have every member in a file that is entirely test material**, plus 14 more that straddle the boundary; the healthy population has 0 of 68 and 2 straddling. `weighs` (`src/report.rs:379`) charges every one of them to the score.

2. **The record itself carries the contamination, and the test that was supposed to prevent it cannot see it.** `every_reviewed_structural_site_is_production_context` (`tests/corpus_precision.rs:671`) asserts every reviewed structural site has `SiteContext::Production`. It passes today over **11 of the 332 published reviewed sites that sit under a `tests`, `benches` or `examples` directory or on a `tests.rs` module file**, all in the agent population, all in vibesql, because `site.context` is copied from the same scan whose derivation is wrong. Four of the eleven carry `false_positive`, so they are load-bearing for the rates: agent `duplicate_function_body` is 6/19, agent `near_duplicate_function_body` 10/20, agent `oversized_unit` 1/20, agent `orphan_module_file` 1/16, and each loses sites when the classification is corrected. Seven of the 63 adjudicated pairs sit on the same paths. This repository reproduces the defect on itself: a `near_duplicate_function_body` family whose two members are `src/cargo_health/tests.rs` and `tests/cargo_health_product_proof.rs` is published with `context: null` and charged to a score of 89.

3. **Two of the four structural healthy rates rest on five sites, and only one of the four published population gaps is supported.** `rust_doctor::structure::oversized_unit` and `rust_doctor::structure::near_duplicate_function_body` are both 4/5 on healthy code, `provenance: ["unrecorded"]`, `doubly_judged: 0`, Wilson interval [3755, 9638], a width of 5883 basis points. Both ship as 8000 bp in `CORPUS_NOISE` (`src/policy/noise.rs:40,41`). The full healthy production subpopulations are 47 and 30 sites, so the samples are 11 percent and 17 percent of a population small enough to review whole. `agent_population.rate_comparison` publishes four gaps; recomputing each as a Newcombe hybrid score interval for the difference of two proportions gives `oversized_unit` [-9189, -2865], separated, and `near_duplicate_function_body` [-5590, 1695], `duplicate_function_body` [-3522, 2040] and `complex_function` [-3437, 282], all three spanning zero. Three of the four published gaps state a difference the record cannot distinguish from none.

4. **The ranking reads a point estimate, discards the sample size, and rewards never measuring.** `expected_repair_value` (`src/audit.rs:678`) is `contribution * (10000 - noise) / 10000`, and its `None` arm keeps the full contribution, documented at `src/policy/noise.rs:46` as "An unmeasured rule carries no discount: absence of a measurement is not evidence of noise." The consequences are arithmetic. **38 of the 62 catalogued rules have no measurement and are therefore ranked at full weight**, so measuring a rule can only ever lower its rank. **12 catalogued rules carry a rate of exactly 10000 bp and are therefore ranked at exactly zero**, tied, whatever they cost: `clippy::indexing_slicing` adjudicated 40/40 and `clippy::mem_forget` adjudicated 1/1 are ranked identically. And a rule cleared on one site is treated as certain: `rust_doctor::cargo::duplicate_major_versions` is 0/1, Wilson [0, 7935], `separation: indecisive`, and ships at 0 bp. Scanning this repository, the result is `projected_rule_ids: ["rust_doctor::cargo::duplicate_major_versions", "clippy::print_stdout", "rust_doctor::structure::near_duplicate_function_body"]`: a rule with 2 findings measured once, a rule with 1 finding never measured, and only then a rule with 26. `rust_doctor::structure::oversized_unit` at 14 findings and `rust_doctor::structure::duplicate_function_body` at 13 are ranked below both.

**Why now:** the record was just certified. EP-005 of the adjudication protocol closed on 2026-08-21 with `position_proof` anchoring all 332 reviewed sites and all 63 pairs to a run that located them, and `cargo test --test corpus_precision` reproduces both populations in 64.52 seconds from the pinned clone cache. The instrument is now trustworthy enough that what it measures is the binding constraint, and the same reproduction that certified it is what will republish it. The catalog is also about to grow: `.claude/skills/rule-candidate` and `.claude/skills/rule-admit` exist to drain the candidate queue, and every rule admitted from here is measured against a population that includes test code and is ranked by a key that punishes being measured at all. Fixing the instrument before the catalog grows is fixing it while 332 sites have to be relocated rather than a thousand.

## Overview

Three defects, one chain. A diagnostic is stamped with a context, the context decides whether it weighs on the score and whether it enters the corpus as a production site, the corpus site produces a rate, and the rate ranks what the report tells a reader to repair first. The chain is broken at the first link, thin at the third, and inverted at the fourth.

The first fix is structural and small. The walk at `src/source_kernel/walk.rs:358` already resolves out-of-line module declarations and queues the files they name with the declaring traversal's `Reachability`. The gate of the declaration is available exactly there and is currently dropped. Propagating it, joined into the reachability the unit accumulates rather than stamped on the unit, means `unanimous` (`src/source_kernel.rs:244`) still governs: a file reached both by a gated declaration and an ungated one abstains to production, which keeps the property the doc comment on that function calls the expensive mistake to lose. This was chosen over extending the path convention `path_contains_tests_segment` (`src/source_kernel.rs:558`) to the published context, because that helper matches a directory component and cannot match a `tests.rs` module file, which is 3 of the 31 contaminated families and is this repository's own case.

The second fix is measurement, not code. The two weak healthy scopes have production subpopulations of 47 and 30 sites, so the sample can become the population. Reviewing them whole takes 42 and 25 new sites at two passes each, 134 verdicts, and takes the Wilson width from 5883 basis points to at most 2748 and 3370 respectively, whatever the rate turns out to be. Stride containment was verified empirically: raising the target from 20 to 40 retains all five recorded `oversized_unit` sites, and 20 retains all five `near_duplicate` sites, so deepening extends the sample rather than replacing it. `PROTOCOL_TARGET` (`tests/support/corpus/sampling.rs:55`) is a single shared constant asserted for equality at `tests/corpus_agreement.rs:782` and `tests/corpus_statistics.rs:955`, so it has to become a floor with a per-scope target before either scope can exceed it.

The third fix is one formula. The ranking key becomes the Laplace-smoothed rate, `(false_positives + 1) / (reviewed + 2)`, which is the Beta(1,1) posterior mean. A rule the corpus never adjudicated lands at 5000 basis points, half weight, which is the same formula at n = 0, so measuring a rule can move it up or down and the inverted incentive disappears. A rule adjudicated 40/40 lands at 9762 and one adjudicated 1/1 at 6667, so the twelve-way tie at zero becomes an order. A rule cleared 0/1 lands at 3333 rather than 0, and one cleared 0/5 at 1429, so a clean sample of one stops being treated as proof. The Wilson lower bound was rejected on the research: it is negatively biased everywhere and structurally punishes unmeasured items, which is the same inversion in the other direction, and Laplace smoothing produces near-identical orderings at a fraction of the complexity. An empirical-Bayes prior fitted to the corpus was rejected because it would have to be fitted from 24 measured rules and would force a product decision this PRD does not take: the healthy pooled rate is 8447 bp and the agent pooled rate 3917 bp, so choosing the prior means choosing which population the shipped table mirrors. The score itself does not move. `CORPUS_NOISE` ranks, it never penalizes, and that stays true.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Published reviewed structural sites on a test path | 0 of 332 (from 11) | 0, enforced by a test the scan cannot launder |
| Agent production duplication families that are entirely test code | 0 of 430 (from 31) | 0, re-verified on every corpus reproduction |
| Catalogued rules ranked at full weight with no measurement | 0 of 62 (from 38) | 0 |
| Healthy Wilson interval width on the two weak structural scopes | at most 3370 basis points (from 5883) | at most 3370, sample equal to the population |
| Published population gaps carrying a separation verdict | 4 of 4 (from 0 published, 1 supported) | 4 of 4 |

## Target Users

### The developer reading the report
- **Role:** a Rust developer, often working with a coding agent, running `rust-doctor` on a workspace and reading the ranked list of what to repair first.
- **Behaviors:** scans, reads the score, reads the top three projected rules, and repairs in that order. Does not open `tests/corpus.json` and has no way to know a rule's rate rests on one site.
- **Pain points:** the top of the list is currently occupied by whichever rules the corpus happens not to have measured. On this repository that is a rule with 1 finding ranked above a rule with 26.
- **Current workaround:** ignore the ranking and repair by volume, which is the question the ranking was built to stop answering.
- **Success looks like:** the first three rules are the three worth the most points per unit of work, and a rule that has never been adjudicated is neither free nor damned.

### The catalog maintainer
- **Role:** decides which lints leave the candidate queue, which rules ship active, and what each rule's measured noise is.
- **Behaviors:** drives `.claude/skills/rule-candidate` and `.claude/skills/rule-admit`, reads `gate.verdict` and the per-rule intervals, regenerates `tests/corpus.json` from the gated reproduction.
- **Pain points:** admits rules against populations that include test code, and against rates whose intervals span most of the unit interval. The gate's only positive statements rest on samples of one to five sites.
- **Current workaround:** read the raw interval by hand and discount the verdict mentally.
- **Success looks like:** a rate whose sample is the population, an interval narrow enough to place a rule against the 5 percent threshold, and a difference interval that says whether the two populations really differ on that rule.

### The adjudicating agent
- **Role:** the double-pass judge driven by `.claude/skills/corpus-adjudicate`, which locates sites, judges them blind twice, and escalates disagreement.
- **Behaviors:** reads a sampled site, opens the file, decides true or false positive, writes a justification.
- **Pain points:** spends verdicts on sites that are test code presented as production, which are near-automatic false positives and teach the record nothing about the rule.
- **Current workaround:** none; the site arrives already classified.
- **Success looks like:** every sampled site is production code, so every verdict measures the rule rather than the classifier.

## Research Findings

Key findings that informed this PRD.

### Competitive Context
- **Google Tricorder** ranks and gates by an effective false positive rate, defined as any report the developer did not want to see and measured continuously through a "not useful" button. Analyzers above roughly 10 percent are disabled and the overall rate is held under 5 percent (Sadowski et al., ICSE 2015; *Lessons from Building Static Analysis Tools at Google*, CACM 2018). It gates on a running behavioral rate over very large volume, not on per-rule intervals over small hand-adjudicated samples.
- **SonarQube, Semgrep, CodeQL** publish rule metadata and severity, not a per-rule adjudicated false positive rate with a confidence interval.
- **Market gap:** no tool found publishes small-sample, interval-adjusted, per-rule false positive rates. rust-doctor's adjudicated corpus has no direct precedent to copy, which means the statistical treatment has to come from the estimation literature rather than from a competitor.

### Best Practices Applied
- **Ranking by a raw point estimate over unequal sample sizes is the failure mode the literature is about.** The standard responses are the Wilson lower confidence bound as a sort key (Evan Miller), Laplace smoothing, and empirical-Bayes shrinkage toward a fitted Beta prior (Robinson; Wheeler). Laplace and Wilson LCB produce near-identical orderings, and Laplace is recomputable by hand.
- **The Wilson lower bound is the wrong direction here.** It is negatively biased everywhere and structurally punishes unmeasured items, which is the same inverted incentive this PRD exists to remove. The posterior mean is the reconciliation: it neither zeroes noise for unmeasured rules nor bills them full weight, and measuring can move a rule either way.
- **Defaulting an unmeasured item to zero has no support in the literature.** Cold-start recommenders give every item the same prior and let data move it.
- **Newcombe's hybrid score and Agresti-Caffo are the recommended intervals for the difference of two independent proportions with unequal sample sizes** (Fagerland, Lydersen and Laake 2011); Wald is never recommended. Acceptable coverage needs roughly n at or above 30 for Agresti-Caffo and 40 for Newcombe per arm. At n = 5 the interval is honest but so wide that a separation verdict will almost never fire, which is the correct behavior of a gate rather than a defect.
- **Ranks estimated from small samples are unstable.** Goldstein and Spiegelhalter's league-table work and the BMJ IVF clinic study found large year-to-year rank swings with no significant change in the underlying rate, worst for the smallest units. Publish rates with their intervals, never bare ranks.

*Full research sources are listed in the session record; the primary citations are named inline above.*

## Assumptions & Constraints

### Assumptions (to validate)
- **Propagating the gate does not silence shipped code.** Based on the unanimity rule already in `SourceUnit::context`: a file reached both by a gated declaration and an ungated one abstains to production. US-005 and US-003 are what validate it.
- **A scope that loses sites can be re-sampled with containment.** Based on the stride being deterministic (`k = min(target, n)`, indices `floor(i * n / k)`) and on containment holding exactly when the new k is a multiple of the old. Removing sites changes n, so containment is not guaranteed by that argument and has to be measured. US-005 is the spike.
- **The healthy structural populations are uncontaminated.** Measured: 0 of 68 duplication families and 0 of 47 `oversized_unit` sites sit on a test path. This is why EP-002 does not depend on EP-001.
- **Correcting the context lowers the agent duplication rates rather than raising them.** Based on 4 of the 11 reclassified reviewed sites carrying `false_positive`. If it holds, the three unsupported gaps widen rather than close.

### Hard Constraints
- The tool reaches no network, uploads nothing, emits no telemetry. No test touches the network.
- Inspecting a workspace runs `cargo clippy` inside it. The corpus replays from a local clone cache outside this repository; no corpus code is ever committed here (`no_corpus_repository_is_committed_in_this_repository`).
- The toolchain is pinned to 1.97.1. `tests/corpus.json` records the Clippy version its measurement was taken under, and only the gated reproduction may rewrite `position_proof`.
- Every file of a module stays under the 1000 lines `oversized_unit` reports, and every impl under the 500 lines it reports at. The eleven `the_X_holds_the_size_bound` tests enforce it.
- Production code carries no `unwrap`, `expect`, `panic!` or `dbg!`.
- `SCHEMA_VERSION` in `src/report.rs` is bumped by any change to the report shape, and the frozen v7 archive keeps projecting.
- Dependencies stay pinned exactly and `Cargo.lock` stays committed. No new dependency is introduced by this PRD.

## Quality Gates

These commands must pass for every user story:
- `cargo clippy --all-targets --no-deps -- -D warnings` - the lint gate, must be clean
- `cargo test` - the full suite, offline, including the 11 size-bound tests and the position proof recomputation
- `RUST_DOCTOR_CORPUS_DIR=~/.cache/rust-doctor/corpus RUST_DOCTOR_CORPUS_ARTIFACTS=~/.cache/rust-doctor/corpus-artifacts cargo test --test corpus_precision` - the gated reproduction, required for every story that touches `tests/corpus.json`, `tests/corpus_precision.rs` or `tests/support/corpus/`

The Node launcher gates (`cd npm/rust-doctor && bun test tests`, `bun run smoke:packed`) are unaffected by this PRD and are not required for its stories.

## Epics & User Stories

### EP-001: The published context tells the truth about out-of-line test modules

Close the gap between what `test_context` says a test gate is and what the walk actually propagates, then republish the record measured through the corrected classification.

**Definition of Done:** no diagnostic whose every member is reached only through a `#[cfg(test)]` module declaration is published with a production context; the corpus reproduces with 0 reviewed sites on a test path; the four agent scopes that lose sites are back at their sampling target.

#### US-001: Propagate the test gate of an out-of-line module declaration through the walk
**Description:** As the source kernel, I want the `#[cfg(test)]` gate of a `mod name;` declaration to travel with the file it names, so that a unit reached only through a gated declaration knows it is test material.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a workspace where `lib.rs` declares `#[cfg(test)] mod tests;` and `src/tests/mod.rs` exists, when the walk enumerates, then the unit for `src/tests/mod.rs` records that the traversal reaching it was gated.
- [ ] Given a gated module that itself declares `mod helpers;`, when the walk descends, then the gate propagates transitively to `src/tests/helpers.rs`.
- [ ] Given `#[cfg(not(test))] mod production;`, when the walk enumerates, then the gate is not set: a negated predicate is not a test gate.
- [ ] Given `#[cfg(all(test, feature = "x"))] mod tests;`, when the walk enumerates, then the gate is set, and the grammar accepted is documented in one place beside the function that reads it.
- [ ] Given `#[cfg(feature = "test-util")] mod tests;`, when the walk enumerates, then the gate is not set: a feature whose name contains `test` is not the `test` predicate.
- [ ] Given a `#[path = "..."]` attribute on a gated declaration, when the walk resolves it, then the gate travels with the resolved file and not with the lexical one.
- [ ] Given the module depth limit is reached before a gated declaration resolves, when the walk refuses the file, then the refusal is recorded as `Limit::ModuleDepth` and no context question arises.
- [ ] Given the gate is recorded, then it is carried in the set the unit accumulates alongside `Reachability`, not stamped as a single field on the unit.

#### US-002: The published context reads the gate, and unanimity still governs
**Description:** As a report reader, I want a file that is test material by declaration to be published with a test context, so that it stops weighing on my score.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given a unit every traversal of which was gated, when `SourceUnit::context` is asked, then it answers `Some(DiagnosticContext::Tests)`.
- [ ] Given a unit reached both by a gated declaration and by an ungated one, when `SourceUnit::context` is asked, then it abstains and answers `None`, and the abstention is asserted by a named test rather than inferred.
- [ ] Given a unit reached only by a bench target and by a gated declaration, when `SourceUnit::context` is asked, then it abstains: two different non-production contexts are not unanimous.
- [ ] Given a diagnostic whose unit context is `Tests`, when the report is assembled, then `weighs` returns false and the diagnostic leaves the score and the gate while staying in the CLI surface.
- [ ] Given a duplication family straddling a gated file and a production file, when `unanimous_context` runs, then it abstains and the family is charged, because the duplication genuinely involves shipped code.
- [ ] Given `is_test_code` (`src/source_kernel.rs:268`), then its relationship to the new gate is stated in one place: either it reads the gate or its path fallback is documented as covering what the gate cannot.

#### US-003: A fixture that fails today and passes after the fix
**Description:** As a maintainer, I want a workspace fixture exercising every form of the out-of-line gate, so that the classification is proved rather than reasoned about.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given a fixture workspace under `tests/fixtures/`, when it is scanned, then it contains at minimum: an out-of-line `#[cfg(test)] mod tests;` resolving to `src/tests/mod.rs`, a `#[cfg(test)] mod tests;` resolving to `src/feature/tests.rs`, a nested gated module, and a file reached both gated and ungated.
- [ ] Given the fixture, when it is scanned before the fix, then at least one structural diagnostic is published with a production context on a gated file.
- [ ] Given the fixture, when it is scanned after the fix, then every diagnostic whose members are all gated carries `context: "tests"`, and the both-ways file still carries `context: null`.
- [ ] Given the fixture is scanned twice, then the two reports are identical, and `CARGO_TARGET_DIR` is set to a scratch directory keyed on the scanned path via `support::scan_target`.
- [ ] Given the fixture is added, then `tests/rule_evidence.json` is not disturbed: this fixture proves a classification, not that a rule fires.

#### US-004: This repository stops charging itself for its own test duplication
**Description:** As the maintainer of rust-doctor, I want the self-scan to stop counting a duplication between `src/cargo_health/tests.rs` and `tests/cargo_health_product_proof.rs` as production, so that the tool passes the rule it publishes.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Given this repository is scanned after the fix, when the diagnostics are read, then the `near_duplicate_function_body` family whose members are `src/cargo_health/tests.rs` and `tests/cargo_health_product_proof.rs` carries a non-null context.
- [ ] Given this repository is scanned after the fix, then no published diagnostic has a production context with every member under a `tests`, `benches` or `examples` directory or on a `tests.rs` module file, and a test asserts it.
- [ ] Given the self-scan score moves, when the change is reported, then the before and after values are both stated: the baseline is 89 over 35590 production lines with 58 source files.
- [ ] Given `no_unit_of_this_crate_s_own_source_is_a_hotspot` (`src/structure/tests.rs:392`) still filters on `src/`, then it is left alone by this story and the new assertion is separate.

#### US-005: Validate assumption: a scope that loses sites can be re-sampled with containment
**Description:** As a maintainer, I want to know before re-adjudicating whether the deterministic stride still contains the surviving recorded sites once the population shrinks, so that re-sampling extends the record instead of orphaning it.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Given the four affected agent scopes, when the corrected population is enumerated, then the new population size is recorded per scope and compared against the old.
- [ ] Given the new population and the protocol target, when the stride is recomputed, then the report states, per scope, how many of the surviving recorded sites the new stride retains and how many it drops.
- [ ] Given a scope where the new stride drops a surviving recorded site, then the spike states explicitly which mechanism will be used to keep it: raising the target to a containing multiple, or recording the retained sites as an explicit carry-over with its own justification.
- [ ] Given the spike produces no code, then its output is a written finding committed with the PRD status update, naming the per-scope numbers.
- [ ] Given containment cannot be achieved for a scope by any available mechanism, then the spike says so and US-007 is marked BLOCKED rather than proceeding on a broken sample.

#### US-006: Reproduce the corpus and republish the record under the corrected context
**Description:** As a maintainer, I want `tests/corpus.json` regenerated from a gated reproduction under the corrected classification, so that every published rate is measured over production code only.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Given the gated reproduction is run with both environment variables set, when it completes, then all tests of `corpus_precision` pass and `position-proof.json` is written beside the artifacts.
- [ ] Given the reproduction output, when the record is updated, then `position_proof` is copied from the run rather than hand-written, and `tests/corpus_position.rs` recomputes it offline on the next `cargo test`.
- [ ] Given the corrected classification, when the observations are recomputed, then the agent production duplication families that are entirely test code number 0, down from 31 of 430, and the count is stated in the commit message.
- [ ] Given a scope falls below `MINIMUM_REVIEWED_SITES` after reclassification, then its `PrecisionStatus` becomes `Incomplete` and its rate is withheld rather than published from a shortened sample.
- [ ] Given `structural_density` is recomputed, then both populations' line counts and densities move together and `ratio_milli` is republished, with the previous value 1509 stated for comparison.
- [ ] Given the record changes, then `the_published_catalog_matches_the_shipped_policy` and `the_noise_the_score_ranks_by_matches_the_adjudicated_rate` both pass, regenerating `CORPUS_NOISE` if the rates moved.
- [ ] Given the reproduction is run twice, then `two_computations_of_the_precision_report_are_identical` passes and the wall clock stays at or under 80 seconds on the machine that measured 64.52 seconds.

#### US-007: Re-adjudicate the agent scopes that fell below their sampling target
**Description:** As a maintainer, I want the four agent scopes that lost sites brought back to their target with newly sampled production sites, so that no published rate rests on a shortened sample.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-005, US-006

**Acceptance Criteria:**
- [ ] Given the corrected populations, when the four affected scopes are re-sampled, then agent `oversized_unit`, `near_duplicate_function_body`, `duplicate_function_body` and `orphan_module_file` each reach their sampling target again.
- [ ] Given each new site, when it is adjudicated, then it is judged by two independent passes per `.claude/skills/corpus-adjudicate`, and `doubly_judged` equals `reviewed` for each of the four scopes.
- [ ] Given a new pair whose two passes disagree, when the record is written, then the pair carries no reviewed site and `escalations_open` rises, rather than being tie-broken by an agent.
- [ ] Given the four scopes are re-adjudicated after the protocol cutoff, then `adjudicated_after_cutoff` names them and `protocol_defects` accepts the record.
- [ ] Given every rate moves, then the previous and new rate are both stated per scope, with the previous values 500, 5000, 3157 and 625 basis points.
- [ ] Given the re-adjudication completes, then the gated reproduction is run again and `position_proof` is rewritten by the run, never by hand.

#### US-008: The production-context test checks a fact the scan cannot launder
**Description:** As a maintainer, I want `every_reviewed_structural_site_is_production_context` to check something independent of the derivation it is auditing, so that a classifier defect cannot make the test pass over its own contamination.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-006

**Acceptance Criteria:**
- [ ] Given a reviewed site whose path lies under a `tests`, `benches` or `examples` directory, when the test runs, then it fails naming the site, regardless of the `context` the scan recorded on it.
- [ ] Given a reviewed site whose file name is `tests.rs`, when the test runs, then it fails naming the site.
- [ ] Given the record as it stands before US-006, when the test is run, then it fails on all 11 known sites, and that failure is demonstrated before the record is corrected.
- [ ] Given a repository that legitimately ships a production module under a directory named `tests`, then the test carries a documented, per-site allow list with a written justification per entry, and the list is empty on the current record.
- [ ] Given the test runs, then it stays offline and needs neither corpus environment variable.

---

### EP-002: The two weak healthy scopes are measured, not sampled

Take the two structural scopes whose healthy rate rests on five sites to their whole production subpopulation, and publish the difference interval that says whether a population gap is real.

**Definition of Done:** healthy `oversized_unit` and `near_duplicate_function_body` are reviewed at 47 and 30 sites with `doubly_judged` equal to `reviewed`; every rule measured in both populations publishes a difference interval and a separation verdict.

#### US-009: The sampling target becomes a floor with a per-scope value
**Description:** As a maintainer, I want a scope to be able to exceed the protocol target without failing the tests that pin it, so that a scope can be reviewed exhaustively.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a scope in `sampling_plan`, when its target is read, then it is the scope's own value, defaulting to `PROTOCOL_TARGET`.
- [ ] Given a scope whose target is below `PROTOCOL_TARGET`, when `sampling_defects` runs, then it reports a defect naming the scope and both numbers.
- [ ] Given `tests/corpus_agreement.rs:782` and `tests/corpus_statistics.rs:955,965`, when the constant becomes a floor, then those assertions compare against the scope's own target and still fail on a scope below the floor.
- [ ] Given a scope whose target exceeds its population, when the plan is built, then `k = min(target, n)` still holds and the plan records that the sample is the population.
- [ ] Given the change lands with no scope deepened yet, then the record is byte-identical and the gated reproduction passes unchanged.

#### US-010: Review healthy `oversized_unit` to its whole production subpopulation
**Description:** As a maintainer, I want every one of the 47 healthy production `oversized_unit` sites adjudicated twice, so that the rate the score ranks by rests on the population rather than on 11 percent of it.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-009

**Acceptance Criteria:**
- [ ] Given the healthy population, when the scope is deepened, then `reviewed` reaches 47 and equals the production subpopulation, with the plan recording `observed: 47, target: 47`.
- [ ] Given the five sites already on record, when the new stride runs, then all five are retained and the containment is asserted rather than assumed.
- [ ] Given every site, when it is adjudicated, then it is judged twice independently and `doubly_judged` equals 47.
- [ ] Given the deepened sample, when the Wilson interval is recomputed, then its width is at most 2748 basis points, down from 5883, whatever the rate.
- [ ] Given a pair whose two passes disagree, then it carries no reviewed site, `escalations_open` rises, and no agent tie-breaks it.
- [ ] Given the rate moves away from 8000 basis points, then `CORPUS_NOISE` is regenerated and `the_noise_the_score_ranks_by_matches_the_adjudicated_rate` passes.

#### US-011: Review healthy `near_duplicate_function_body` to its whole production subpopulation
**Description:** As a maintainer, I want every one of the 30 healthy production `near_duplicate_function_body` sites adjudicated twice, so that its rate stops resting on five sites.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-009

**Acceptance Criteria:**
- [ ] Given the healthy population, when the scope is deepened, then `reviewed` reaches 30 and equals the production subpopulation.
- [ ] Given the five sites already on record, when the new stride runs, then all five are retained and containment is asserted.
- [ ] Given every site, when it is adjudicated, then it is judged twice independently and `doubly_judged` equals 30.
- [ ] Given the deepened sample, when the Wilson interval is recomputed, then its width is at most 3370 basis points, down from 5883, whatever the rate.
- [ ] Given the population is 30, then the record states that this scope cannot reach the sample size Newcombe's interval needs for nominal coverage on this corpus, and that the limit is the corpus, not the sampling.
- [ ] Given a family is the unit of this rule, when a site is located, then it is anchored by `structural_identity` and the position proof recomputes over it.

#### US-012: Publish a difference interval and a separation verdict per comparable rule
**Description:** As a catalog maintainer, I want each `rate_comparison` row to carry an interval and a verdict, so that a published gap states whether the two populations can be told apart on that rule.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-010, US-011

**Acceptance Criteria:**
- [ ] Given a rule measured in both populations, when `rate_comparison` is built, then the row carries the difference interval in integer basis points beside `gap_basis_points`.
- [ ] Given the interval, when the verdict is derived, then it is `separated` when the interval excludes zero and `indistinguishable` when it contains it, with both comparisons strict and stated once.
- [ ] Given the interval is computed, then the method is named in the record beside the value, and the computation is derived from the same `wilson_95` the per-rule rates use rather than from a second implementation.
- [ ] Given a rule whose sample in either population is below the coverage the method needs, then the row carries that fact and the verdict is published anyway rather than withheld.
- [ ] Given two computations of the comparison block, then they are byte-identical, and the rounding is half away from zero in integer basis points on every platform.
- [ ] Given the current record, then recomputing reproduces the four known intervals: `oversized_unit` [-9189, -2865] separated, and `near_duplicate_function_body`, `duplicate_function_body` and `complex_function` all containing zero.

---

### EP-003: The ranking reads a smoothed rate, and an unmeasured rule is not free

Replace the raw point estimate in the ranking key with the Laplace-smoothed rate, so the sample size matters, the twelve-way tie at zero resolves, and measuring a rule can move it either way.

**Definition of Done:** `expected_repair_value` reads a smoothed rate for every catalogued rule, no rule is ranked at exactly zero or at full weight for want of a measurement, and the smoothing is frozen against the record the way lambda is.

#### US-013: `CORPUS_NOISE` carries the Laplace-smoothed rate, recomputed by its own test
**Description:** As a report reader, I want a rule's ranking discount to reflect how many sites its rate was measured on, so that a rule adjudicated once is not treated like a rule adjudicated forty times.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a rule with `false_positives` and `reviewed` on record, when `CORPUS_NOISE` is generated, then its value is `round((false_positives + 1) / (reviewed + 2) * 10000)` in integer basis points, rounded half away from zero.
- [ ] Given `the_noise_the_score_ranks_by_matches_the_adjudicated_rate` (`src/policy/noise.rs:66`), when it runs, then it recomputes the smoothing from the record's raw counts rather than comparing against the raw rate.
- [ ] Given the record keeps `false_positive_rate_basis_points` unsmoothed, then the raw rate stays what the record publishes and the smoothing lives only in the shipped table, with the difference documented where the table is declared.
- [ ] Given `clippy::indexing_slicing` at 40/40 and `clippy::mem_forget` at 1/1, when the table is generated, then they carry 9762 and 6667 basis points respectively rather than both carrying 10000.
- [ ] Given `rust_doctor::cargo::duplicate_major_versions` at 0/1, then it carries 3333 basis points rather than 0.
- [ ] Given a rule whose `PrecisionStatus` is not `measured`, then it has no entry in the table and the absence is what the ranking reads.

#### US-014: A rule the corpus never adjudicated is ranked at half its contribution
**Description:** As a report reader, I want an unmeasured rule to be ranked as unknown rather than as perfect, so that measuring a rule can raise its rank as well as lower it.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-013

**Acceptance Criteria:**
- [ ] Given a rule with no entry in `CORPUS_NOISE`, when `expected_repair_value` is computed, then the retained fraction is 5000 basis points, which is the same formula at `reviewed = 0`.
- [ ] Given two rules of equal contribution, one unmeasured and one measured at 2000 basis points, when they are ranked, then the measured one ranks higher.
- [ ] Given two rules of equal contribution measured at the same rate but on 5 and 40 sites, when they are ranked, then the one measured on 40 sites is further from the half-weight default, in the direction its data indicates.
- [ ] Given a rule measured at 10000 basis points on 5 sites, when it is ranked, then its expected repair value is strictly positive, so no two rules tie at zero for want of resolution.
- [ ] Given this repository is scanned before and after, then `projected_rule_ids` and `withheld_rule_ids` are recorded for both, with the baseline `["rust_doctor::cargo::duplicate_major_versions", "clippy::print_stdout", "rust_doctor::structure::near_duplicate_function_body"]` and `["clippy::indexing_slicing", "clippy::print_stderr", "clippy::string_slice"]`.
- [ ] Given the change lands, then `score.value` on this repository is unchanged at 89 and every dimension is unchanged: the rate ranks, it never penalizes.

#### US-015: The smoothing is frozen the way the lambda table is
**Description:** As a maintainer, I want the smoothing constant pinned and asserted against the record, so that changing how a rate is discounted cannot happen without a deliberate edit.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-013

**Acceptance Criteria:**
- [ ] Given the pseudo-counts, when they are declared, then they sit in one named constant beside `CORPUS_NOISE` with the Beta(1,1) rationale written once.
- [ ] Given the constant moves, when the suite runs, then a named test fails saying the smoothing changed and that every shipped rate has to be regenerated, in the shape of `src/audit/tests/lambda_freeze.rs`.
- [ ] Given the record is regenerated, then the smoothing constant is recorded in `tests/corpus.json` beside the toolchain and the lambda table, for the reason both of those are pinned.
- [ ] Given the constant is recorded, then a record whose smoothing disagrees with the shipped constant fails the reproduction naming both values.

#### US-016: The report names the sample a rule's rate rests on
**Description:** As a report reader, I want to see how many sites a rule's noise rate was measured on, so that I can weigh the ranking myself.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-013

**Acceptance Criteria:**
- [ ] Given a projected or withheld rule, when the report is rendered, then the sample size behind its rate is available to the reader, and a rule with no measurement is named as unmeasured rather than shown as a number.
- [ ] Given the JSON report gains a member, then `SCHEMA_VERSION` is bumped and `project_v11_wire_to_v7` strips it so the frozen v7 archive keeps projecting.
- [ ] Given the linear report gains text, then every file of `src/render/` stays under 1000 lines and `the_report_holds_the_size_bound_it_reports_for` passes.
- [ ] Given the terminal is 40 columns wide, when the report renders, then the added text does not push any row past the guard column and no frame wraps.
- [ ] Given a report is rendered for a scan that failed, then the added text is not rendered: `render_failure` stays two sections.

## Functional Requirements

- FR-01: The walk must record, per traversal reaching a unit, whether the module declaration that named it was gated by `#[cfg(test)]`, and must propagate that gate transitively to modules the gated file declares.
- FR-02: The published context of a unit must be the unanimous non-production context of every traversal reaching it, where a traversal's context is its Cargo target kind or its declaration gate, whichever is present.
- FR-03: The system must not publish a production context for a unit every traversal of which was gated.
- FR-04: The system must abstain to production for a unit reached by traversals that disagree, including two different non-production contexts.
- FR-05: A `#[cfg]` predicate must be read as a test gate only when it contains the bare `test` predicate at the top level or inside an `all(...)`; `not(test)` and a feature whose name merely contains `test` must not be read as one.
- FR-06: `every_reviewed_structural_site_is_production_context` must fail on a reviewed site whose path lies under a `tests`, `benches` or `examples` directory or whose file name is `tests.rs`, independently of the context the scan recorded.
- FR-07: Each scope in `sampling_plan` must carry its own target, defaulting to `PROTOCOL_TARGET`, and a target below that floor must be reported as a defect.
- FR-08: `rate_comparison` must publish, per rule measured in both populations, the interval for the difference of the two rates and a separation verdict derived from it.
- FR-09: `CORPUS_NOISE` must carry, per measured rule, `round((false_positives + 1) / (reviewed + 2) * 10000)` in integer basis points, and its test must recompute that from the record.
- FR-10: `expected_repair_value` must retain 5000 basis points for a rule with no entry in `CORPUS_NOISE`.
- FR-11: The system must NOT change what a diagnostic costs the score as a consequence of any change in this PRD: the discount applies to the ranking key alone.
- FR-12: The system must NOT rewrite `position_proof` outside the gated reproduction run.
- FR-13: A scope whose reviewed sample falls below `MINIMUM_REVIEWED_SITES` must publish `PrecisionStatus::Incomplete` and withhold its rate rather than publish it from a shortened sample.

## Non-Functional Requirements

- **Performance:** the gated corpus reproduction stays at or under 80 seconds on the machine that measured 64.52 seconds on 2026-08-21. The structural pass stays within its 10-second wall-clock budget per repository at the default setting, and the added per-declaration attribute read costs at most one syntax-node attribute scan per out-of-line `mod` declaration, which the walk already visits at `src/source_kernel/walk.rs:366`.
- **Determinism:** two computations of the precision report, of the comparison block and of the smoothed table are byte-identical. All rounding is half away from zero in integer basis points, and every square root goes through the IEEE-754 correctly-rounded path already used by `wilson_95`.
- **Measurement precision:** healthy `oversized_unit` publishes a Wilson interval at most 2748 basis points wide at n = 47, and healthy `near_duplicate_function_body` at most 3370 at n = 30, against 5883 for both today.
- **Coverage:** 0 of the published reviewed structural sites sit on a test path, against 11 of 332 today. 0 of the agent production duplication families are entirely test code, against 31 of 430, which is 7.2 percent.
- **Ranking resolution:** 0 catalogued rules have an expected repair value of exactly zero, against 12 today, and 0 are ranked at full weight for want of a measurement, against 38 today.
- **Score stability:** scanning this repository after EP-003 returns the same score value 89 and the same five dimension values (security 100, reliability 91, maintainability 93, performance 100, dependencies 51) as before it.
- **Size:** every file of `src/source_kernel/`, `src/structure/`, `src/policy/`, `src/audit/` and `src/render/` stays under 1000 lines and every impl block under 500, enforced by the eleven existing size-bound tests.
- **Security and privacy:** no network access in the tool or in any test; no absolute path, environment variable or user data in a `--json` report; no corpus repository committed in this repository.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Both-ways file | A file reached by `#[cfg(test)] mod tests;` and by an ungated `mod helpers;` | Abstain: `context` stays `null` and the file weighs on the score | none, the diagnostic is published as production |
| 2 | Public test helpers | A crate that ships `src/tests/` as public API with no `#[cfg(test)]` on the declaration | Stays production; the gate is the declaration, never the directory name | none |
| 3 | Negated gate | `#[cfg(not(test))] mod production;` | Not a test gate; the file stays production | none |
| 4 | Feature named like a gate | `#[cfg(feature = "test-util")] mod tests;` | Not a test gate; the file stays production | none |
| 5 | Compound gate | `#[cfg(all(test, feature = "x"))] mod tests;` | A test gate | none |
| 6 | Redirected gate | `#[cfg(test)] #[path = "../shared/mod.rs"] mod tests;` where the target is also reached ungated | Gate travels with the resolved file, then abstains on unanimity | none |
| 7 | Depth limit | A gated declaration below `Limit::ModuleDepth` | The file is never enumerated; the refusal is recorded as a limit, not as a context | the existing limit error names `ModuleDepth` |
| 8 | Straddling family | A duplication family with one gated member and one production member | Abstain: the family is charged, because the duplication involves shipped code | none |
| 9 | Sample falls under the floor | A scope drops below `MINIMUM_REVIEWED_SITES` after reclassification | `PrecisionStatus::Incomplete`, rate withheld, rule named | the gate names the rule as incomplete rather than clean |
| 10 | Stride loses a recorded site | The population shrinks and the new stride no longer contains a reviewed site | The run fails naming the sites it dropped, rather than silently orphaning them | "reproduction required", naming the count |
| 11 | Never adjudicated | A rule with no `CORPUS_NOISE` entry | Ranked at half its contribution, never at zero and never at full | the report names it as unmeasured |
| 12 | Cleared on one site | A rule at 0/1 | 3333 basis points, not 0 | none |
| 13 | Exhausted population | A scope whose target exceeds its population | `k = min(target, n)`; the plan records that the sample is the population | none |
| 14 | Disagreeing new pair | A re-adjudicated pair whose two passes disagree | No reviewed site is published, `escalations_open` rises, no agent tie-breaks | the record shows the pair as escalated |
| 15 | Smoothing constant moved | The pseudo-counts are edited without regenerating the table | A named test fails saying the smoothing changed and every rate must be regenerated | the failure names both values |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Propagating the gate silences shipped code in a crate whose gated module is really production | Medium | High | Unanimity abstains on any disagreement (US-002); the fixture covers the both-ways case (US-003); the reproduction reports the reclassified count per repository for review before the record is written (US-006) |
| 2 | Removing sites moves the agent rates enough to change the two-population conclusion | Medium | Medium | 4 of the 11 reclassified sites are false positives, so the agent duplication rates fall rather than rise; no rate is republished until the scope is back at target (US-007), and the previous value is stated beside the new one |
| 3 | Stride containment fails once the population shrinks, orphaning recorded sites | Medium | High | US-005 measures it before any re-adjudication starts, and US-007 is marked BLOCKED rather than proceeding on a broken sample |
| 4 | Smoothing reorders the report on every workspace and surprises existing users | Low | Medium | The score does not move at all, which US-014 asserts on this repository; only the repair order changes, and it changes toward the evidence. The change is stated in the release note |
| 5 | Healthy `near_duplicate_function_body` cannot reach the sample size Newcombe's interval needs, because its whole population is 30 | High | Low | Publish the verdict with its sample size and record that the limit is the corpus rather than the sampling (US-011, US-012); adding repositories is a separate decision |
| 6 | EP-003 lands before EP-001 or EP-002 and the shipped table has to be regenerated twice | High | Low | `the_noise_the_score_ranks_by_matches_the_adjudicated_rate` fails loudly rather than drifting; sequence EP-003 last, and accept one regeneration if not |
| 7 | The corrected classification changes enough diagnostics that frozen oracles across the suite move | Medium | Medium | The oracles that cover whole reports are already documented as moving with the catalog; regenerate in one pass and state the count |
| 8 | The re-adjudication of four agent scopes plus two healthy scopes is a large volume of agent verdicts | High | Medium | 134 verdicts for EP-002 and the four agent scopes for EP-001, all driven by `.claude/skills/corpus-adjudicate` rather than by hand; the double pass is what the skill already does |

## Non-Goals

- **Switching `CORPUS_NOISE` to mirror the agent rates rather than the healthy ones.** The healthy pooled rate is 8447 basis points and the agent pooled rate 3917, so the choice materially changes every ranking. AGENTS.md calls it a product decision and not a consequence of a number; it stays one, and only `oversized_unit` currently has a separated difference to argue from.
- **Adding repositories to either population.** The corpus stays at ten healthy and eight agent repositories pinned by commit.
- **Closing the four open escalations.** `agreement_defects` refuses both an agreeing pair with no reviewed site and an escalated site carrying a verdict, so closing them needs an arbitration mechanism this PRD does not design. They stay open and counted.
- **Changing what a diagnostic costs the score.** The rate ranks, it never penalizes. No lambda, no weight, no ceiling and no severity moves here.
- **Changing the 5 percent threshold, the gate's verdict rules, or which rules ship active.** `TIER_WINDOWS` and the admission debt are untouched.
- **A hierarchical Bayesian model or an empirical-Bayes prior fitted to the corpus.** Rejected as fragile at 24 measured rules and as forcing the population decision above. Revisit if the measured set passes roughly 50 rules.
- **Extending the path convention `path_contains_tests_segment` to the published context.** It cannot match a `tests.rs` module file and it fires on a directory name rather than on a declaration.

## Files NOT to Modify

- `src/audit/density.rs` - the core-v3 curve, the lambda table and the per-producer denominators are calibrated and frozen by `src/audit/tests/lambda_freeze.rs`. This PRD moves the ranking, never the score.
- `src/policy/catalog.rs` and `src/policy/catalog/validate.rs` - no rule is admitted, retired or retiered here, and no tier window moves.
- `src/delta.rs` and `tests/fixtures/baseline/delta-oracle.json` - the baseline pairing and its 32-case oracle are untouched; a structural finding is still matched on `structural_identity`.
- `npm/rust-doctor/` and `.github/workflows/release.yml` - the launcher and the publishing surface are unaffected.
- `src/skill.rs` and `skills/rust-doctor/` - the agent skill documents flags and rule ids, neither of which changes; `tests/skill_contract.rs` would catch it if they did.
- `tests/rule_evidence.json` - this PRD proves a classification and a ranking, not that a rule fires on its pattern.

## Technical Considerations

- **Architecture:** the gate is a property of a traversal, not of a file. Recommended: carry it in the set the unit accumulates beside `Reachability` (`src/source_kernel/walk.rs:161`), so `unanimous` keeps deciding and no second merge rule appears. Engineering to confirm whether it belongs inside `Reachability` itself or as a parallel member of the same tuple, given `target_key` is what `TargetContexts` is keyed by.
- **Data Model:** does the gate deserve its own `DiagnosticContext` variant, or does it reuse `Tests`? Reusing `Tests` keeps the published vocabulary closed and needs no schema bump. A distinct variant would let a reader tell a Cargo test target from a `#[cfg(test)]` module, at the cost of a new closed-vocabulary member and a `SCHEMA_VERSION` bump. Recommended: reuse `Tests`.
- **API Design:** `is_test_code` (`src/source_kernel.rs:268`) currently ORs the target kind with the path convention. Once the gate exists, does the path fallback still earn its place? Recommended: keep it, since it covers an ungated `src/tests/` directory that no declaration marks, but state in one comment what each of the three sources covers that the others do not.
- **Migration:** `tests/corpus.json` is regenerated by the gated reproduction, never edited. `position_proof` is copied from `position-proof.json`. Backward compatibility of the JSON report is required: any added member bumps `SCHEMA_VERSION` and is stripped by `project_v11_wire_to_v7`. Rollback is a revert plus one reproduction run.
- **Sequencing:** EP-002 does not depend on EP-001, because the healthy population has zero contamination as measured (0 of 68 duplication families, 0 of 47 `oversized_unit` sites). The two can run in parallel. EP-003 is mechanically independent of both, but landing it last saves one regeneration of `CORPUS_NOISE`.
- **Dependencies:** none added. The difference interval is derived from the existing `wilson_95` in `tests/support/corpus/interval.rs`, and the smoothing is integer arithmetic.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Reviewed structural sites on a test path | 11 of 332 | 0 | Month-1 | `every_reviewed_structural_site_is_production_context` after US-008 |
| Agent production duplication families that are entirely test code | 31 of 430 (7.2 %) | 0 | Month-1 | recount over `agent-replay/reports` in the gated reproduction |
| Catalogued rules ranked at full weight with no measurement | 38 of 62 | 0 | Month-1 | `expected_repair_value` over `catalog()` with no `CORPUS_NOISE` entry |
| Catalogued rules whose expected repair value is exactly zero | 12 | 0 | Month-1 | same computation |
| Healthy `oversized_unit` reviewed sites, doubly judged | 5 reviewed, 0 doubly judged | 47 and 47 | Month-1 | `tests/corpus.json` `precision` and `agreement` |
| Healthy `near_duplicate_function_body` reviewed sites, doubly judged | 5 reviewed, 0 doubly judged | 30 and 30 | Month-1 | same |
| Widest healthy Wilson interval among the four structural scopes | 5883 basis points | at most 3370 | Month-1 | `interval_low_basis_points` and `interval_high_basis_points` |
| Population gaps carrying an interval and a verdict | 0 of 4 published | 4 of 4 | Month-1 | `agent_population.rate_comparison` |
| Gated reproduction wall clock | 64.52 s | at most 80 s | Month-1 | `cargo test --test corpus_precision` on the same machine |
| Self-scan score, before and after EP-003 | 89 | 89, unchanged | Month-1 | `cargo run --release -- . --yes --json` |
| Rules whose ranking changes on this repository | not measured | recorded, before and after | Month-1 | `projected_rule_ids` and `withheld_rule_ids` |

## Open Questions

- Does a distinct `DiagnosticContext` variant for a `#[cfg(test)]` module earn a `SCHEMA_VERSION` bump, or is reusing `Tests` the right closed vocabulary? Owner: Arthur, before US-002 lands, since it decides whether the schema moves.
- What is the exact accepted `#[cfg]` grammar for a test gate beyond `test` and `all(test, ...)`? `any(test, feature = "x")` is ambiguous: the module compiles outside tests when the feature is on. Owner: Arthur, before US-001 lands; the conservative reading is that `any(...)` is not a gate.
- Healthy `near_duplicate_function_body` has a whole production population of 30, below the roughly 40 per arm Newcombe needs for nominal coverage. Is publishing an under-covered verdict with its sample size the right answer, or should that row publish the interval with no verdict? Owner: Arthur, before US-012 lands.
- Should the four re-adjudicated agent scopes be re-sampled from the corrected population from scratch, or extended from the surviving sites? US-005 measures containment; the product question is whether a record whose sample changed composition should say so. Owner: Arthur, after US-005 reports.
- Once EP-002 lands, `oversized_unit` may be the only rule with a separated population difference, or a second may join it. Does one separated rule justify moving that rule alone to the agent rate in `CORPUS_NOISE`, against the consistency of the table mirroring one population? Owner: Arthur, after US-012; this PRD deliberately does not decide it.
[/PRD]
