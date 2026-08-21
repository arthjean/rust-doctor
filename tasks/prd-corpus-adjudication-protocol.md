[PRD]
# PRD: The Corpus Adjudication Protocol

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-21 | arthjean | Initial draft |

## Problem Statement

1. **The protocol produces evidence the record cannot hold.** `.claude/skills/corpus-adjudicate/SKILL.md:110-127` mandates two independent passes, blind to each other, with disagreement escalated to a human and never tie-broken by an agent. It was applied once, on 2026-08-11, to three rules: 99 sites judged twice, 96 kept, 3 escalated. The artifact publishes 96 single verdicts where 192 were produced. `ReviewedSite` (`tests/support/corpus.rs:482-494`) carries one `verdict`, and its identity `(rule, repository, path, line)` is unique by construction: a duplicate identity puts the rule in `duplicated` and forces `PrecisionStatus::Incomplete`, which withholds the rate (`tests/support/corpus.rs:1234-1246,1279-1282`), and `tests/corpus_precision.rs:476-483` fails with "site relu deux fois". Recording the second pass is not merely unsupported, it destroys the measurement. Of the 273 published verdicts, 0 record both passes.

2. **Agreement and kappa are prose behind a length check.** `Adjudication::sampling` is a `String` (`tests/support/corpus.rs:399-401`), and the entire assertion on it is `assert!(artifact.adjudication.sampling.len() > 80)` (`tests/corpus_precision.rs:487`), one of three identical checks on `criterion`, `provenance` and `sampling`. That string currently ends: "Observed agreement was 35 of 35 on both Clippy rules, where the absence of variance leaves Cohen's kappa undefined, and 26 of 29 on complex_function, for a kappa of 0.53." Every number in that sentence can drift without a single test failing. By contrast the rate itself is recomputed and compared field by field: `assert_eq!(computed, artifact.precision)` (`tests/corpus_precision.rs:503`). The word `stride`, which names the sampling procedure that prose describes, appears in no test file in the repository.

3. **The escalation queue is a sentence in a document.** The three sites the two passes disagreed on (ripgrep `crates/ignore/src/dir.rs:340`, thiserror `impl/src/expand.rs:31` and `impl/src/expand.rs:221`) are excluded from `reviewed`, present in no field of `tests/corpus.json`, referenced by no test, and open ten days later. Nothing in the repository can tell a reader they exist. The one mechanism the protocol has for its own uncertainty leaves no trace.

4. **The sample floor cannot answer the question the gate asks.** `MINIMUM_REVIEWED_SITES = 5` (`tests/support/corpus.rs:498`) against `THRESHOLD_BASIS_POINTS = 500` (`tests/support/corpus.rs:20`). At n = 5 with zero false positives, the Wilson 95 % interval is [0 %, 43.4 %]. Reaching an upper bound at or below 5 % with a clean sample takes n = 73. Twenty of the 24 published healthy rates rest on 5 sites or fewer; only three carry an interval narrower than 30 points (`clippy::indexing_slicing` 40/40, `clippy::string_slice` 40/40, `rust_doctor::structure::complex_function` 27/31).

5. **The gate's only positive statement rests entirely on the five samples that cannot support it.** The gate publishes `verdict: passed` with 19 rules in `noisy_on_healthy_code` and 62 admitted. Five rules carry a measured rate and are not called noisy: `clippy::missing_safety_doc` (0/2), `clippy::ptr_arg` (0/1), `clippy::rc_buffer` (0/5), `clippy::stable_sort_primitive` (0/1), `rust_doctor::cargo::duplicate_major_versions` (0/1). Ten sites in total. Their Wilson intervals are [0, 65.8], [0, 79.3], [0, 43.4], [0, 79.3] and [0, 79.3]: not one of them separates the rule from the threshold it is being cleared against. Across all 24 rates, 19 are decisively above 5 %, 0 are decisively below, and the 5 that are indecisive are exactly the 5 the gate clears. Those five are also the only rules in the catalog carrying `CORPUS_NOISE` of 0, which is what gives them their full contribution in `expected_repair_value` (`src/audit.rs:678`).

6. **A verdict does not name its judge.** `Provenance` is `Agent | Human | Unrecorded` (`tests/support/corpus.rs:473-479`): 163 verdicts `agent`, 110 `unrecorded`, 0 `human`. Which model produced a verdict is recorded nowhere, so two passes of the same model cannot be told from two passes of different ones, and the self-preference bias of a judge evaluating the tool it serves cannot be measured even in principle.

7. **The population the tool exists for is unjudged where the score lands.** The agent population holds 5343 observed sites over 14 rules and publishes a rate for 4 of them. Recomputed from the pinned replay artifacts, in production context: `rust_doctor::structure::oversized_unit` has 828 sites and zero verdicts, `rust_doctor::structure::duplicate_function_body` 226 and zero, `rust_doctor::structure::near_duplicate_function_body` 204 and zero. Clippy is switched off on that population by the trust boundary, so those three rules plus `complex_function` are effectively the agent score. 1258 production sites, no verdict.

**Why now:** core-v3 shipped on 2026-08-20 and made the adjudicated rate load-bearing. `expected_repair_value` (`src/audit.rs:678`) discounts a rule's contribution by its measured noise, and both the report body's order and the score's projection read it. Before core-v3 the rate annotated a gate; it now ranks the advice the tool gives. Twenty of the 24 rates doing that ranking rest on 5 sites or fewer. The corpus also replays offline from a pinned local clone cache under a pinned toolchain, so deepening it is a reproduction run rather than a research project, and the catalog is at 62 rules with a live candidate queue behind it: every rule admitted after this point is admitted against a precision instrument nobody has calibrated.

## Overview

This PRD turns the adjudication protocol from prose that an agent is asked to follow into a record that a test can check. Three things change in the corpus artifact and its harness.

A judged pair becomes a first-class shape. `adjudication.agreement.pairs` holds one entry per doubly-judged site, carrying the site identity, its population and context, and two passes each with a verdict, a justification and the identity of the judge that produced it. `reviewed` is untouched: it keeps one verdict per site and stays the denominator of every rate. The two are coupled by an invariant rather than by a copy. A pair whose passes agree has exactly one matching `ReviewedSite`; a pair whose passes disagree has none, and that absence is what escalation means. No boolean says so.

The statistics stop being prose. `adjudication.agreement.coefficients` publishes, per `(rule, population)`, the number of pairs, the number that agreed, the 2x2 contingency table, Cohen's kappa in basis points, and Gwet's AC1 beside it. A pass with no variance yields `kappa_status: undefined_no_variance` and no value, never a fabricated 1.0. Every one of those numbers is recomputed from `pairs` by the same test that already recomputes the rate, so the sentence in `sampling` becomes a summary of checked data instead of the only place the data lives.

Every published rate gains its Wilson 95 % interval and a verdict on what that interval settles. `RulePrecision` gains `interval_low_basis_points`, `interval_high_basis_points`, `separation` (`above`, `below` or `indecisive` against the 500 bp threshold) and `doubly_judged`, the count of its sites backed by a recorded pair. `GateOutcome` gains `indecisive`, the list of rules it admits without evidence either way, which today is exactly the five it silently clears.

Two things follow. `schema_version` moves from 3 to 4 and, for the first time, is asserted against a constant rather than merely deserialized. And the three unjudged structural rules of the agent population are adjudicated at n = 20 each under the new protocol, which is both the first use of the recorded shape and the first rate the tool has for the code it exists for.

One story is anchoring rather than measurement. A site's `path:line` is only ever verified against a live scan under `RUST_DOCTOR_CORPUS_DIR`, in tests that return silently when the variable is unset, from a workflow that only fires manually. A blake3 digest over the identities of every published site, written by the reproduction run and checked offline by an always-on test, makes a hand-added site fail the suite until a reproduction covers it.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Verdicts produced after this PRD with both passes recorded | 100 % | 100 % |
| Published rates carrying a 95 % interval | 24 of 24 healthy, 7 of 7 agent | 100 % |
| Rules the gate clears with an indecisive interval, unnamed | 0 | 0 |
| Escalated sites tracked in data with both justifications | 3 of 3 | 100 % |
| Agent-population structural rules with zero verdicts | 0 of 4 | 0 |
| Quantities in `adjudication.sampling` that no test recomputes | 0 | 0 |
| Files of the corpus harness over the 1000-line bound the tool reports at | 0 | 0 |

## Target Users

**The adjudicator of last resort (Arthur).** Judges only what the two passes disagreed on. Today he has no queue: the three open escalations exist in a sentence. He needs the disagreeing pairs to be findable, each carrying both justifications so the disagreement is legible without reopening the code, and he needs the count of what is waiting to be a number a test can print.

**The orchestrating agent running `corpus-adjudicate`.** Fans out one judgment per site, twice, with no shared context between the two passes. Today it writes a `ReviewedSite` and drops the second verdict on the floor, then writes a sentence about agreement that nothing checks. It needs a shape that accepts what it produced, and a test that refuses a single-pass site so the protocol is enforced rather than requested.

**The reader of a rust-doctor report.** Never sees the corpus, but sees its consequence: the order of what to repair first, discounted by the adjudicated rate (`src/audit.rs:678`). A rate from one site and a rate from forty currently look identical to that reader's ranking. The interval does not change the ranking, and this PRD does not change it, but it makes the confidence of the ranking inspectable by whoever decides to change it.

## Research Findings

**Cohen's kappa collapses at high prevalence.** The prevalence paradox is the documented failure mode of kappa on a skewed binary scale: with 87 % of sites judged `true_positive`, expected agreement by chance is already high, so observed agreement of 90 % (26 of 29 on `complex_function`) yields a kappa of 0.53, which reads as "moderate" for two raters that agreed nine times in ten. Gwet's AC1 was designed against exactly this: its chance-agreement term does not inflate with prevalence. Publishing both is the standard response in the annotation literature, not choosing between them.

**Krippendorff's alpha buys nothing here.** It generalizes to any number of raters, any level of measurement and missing data. This protocol has exactly two raters, a nominal binary scale and no missing cells by construction, where alpha reduces to a variant of kappa. It is excluded, not deferred.

**LLM-as-a-judge biases that apply to this protocol.** Position bias is absent: a site is judged alone, not against an alternative. Verbosity bias is not directly applicable, but its mechanism, familiarity driving preference, is what produces self-preference bias, where a judge favors output resembling its own generation distribution. rust-doctor's adjudication has a sharper version of that problem: the judge evaluates the findings of the tool it is embedded in serving. Two passes of the same model reduce variance but not that bias, since both passes share the distribution. Two passes from different model families reduce it. That is the reason `independence` is recorded as a fact per pair rather than assumed.

**Wilson beats the alternatives at the sizes this corpus actually has.** The Wald interval gives [0, 0] at x = 0, which is not merely imprecise but false, and n = 1 is a real sample size in this artifact. Clopper-Pearson is exact and conservative but needs the incomplete beta function, a dependency this crate has no other use for. Wilson needs one square root, which IEEE-754 mandates correctly rounded, so an f64 computation rounded to integer basis points is byte-reproducible across runs and platforms, which `two_computations_of_the_precision_report_are_identical` requires.

**The codebase constraint that shapes everything.** Every corpus struct carries `serde(deny_unknown_fields)` and no optional field except where absence is itself a fact. The struct and the JSON have to move together, so every schema change here is a coordinated edit of `tests/support/corpus.rs`, `tests/corpus.json` and the synthetic fabricator `adjudication_of` at `tests/corpus_precision.rs:789`. That is a cost, and it is the mechanism that has kept the artifact honest.

## Assumptions & Constraints

- The 96 verdicts produced by the 2026-08-11 double pass cannot be recovered as pairs: only the aggregate agreement counts survive, in prose. They stay single-pass verdicts with `doubly_judged` counting 0, and the protocol applies forward. Recording them as pairs would mean inventing 96 second justifications.
- The three escalated sites are recoverable: `docs/top-rules-precision-2026-08.md` names them individually. They enter `pairs` with both verdicts and both justifications restored from that document, which is why they are a story and the 96 are not.
- Adjudication never runs unattended. No CI job produces a verdict, and no automated run resolves an escalation. The trust boundary already forbids scanning untrusted paths, and the protocol forbids an agent breaking a tie.
- The clone cache and the artifact scratch stay outside this repository, and no test reaches the network. `no_corpus_repository_is_committed_in_this_repository` stays green.
- Wilson bounds are stored as integer basis points. No float reaches the artifact.
- `MINIMUM_REVIEWED_SITES` stays 5. Raising it would unpublish 20 of 24 rates and break the exact bidirectional equality `the_noise_the_score_ranks_by_matches_the_adjudicated_rate` asserts against `CORPUS_NOISE`.

## Quality Gates

```bash
cargo build --release
cargo clippy --all-targets --no-deps -- -D warnings
cargo test
```

The corpus reproduction is a separate gate, run with both paths outside this repository:

```bash
RUST_DOCTOR_CORPUS_DIR=<clone cache outside this repository> \
RUST_DOCTOR_CORPUS_ARTIFACTS=<scratch outside this repository> \
cargo test --test corpus_precision
```

## Epics & User Stories

### EP-001: The record of a double pass

**Definition of done:** a doubly-judged site is a shape the artifact can hold, an escalated site is a fact derived from a disagreement rather than a boolean, every pass names its judge, and a verdict entering `reviewed` after this PRD without a pair behind it fails the suite.

---

#### US-001: Hold a doubly-judged site as a pair of passes

**As** the orchestrating agent, **I want** a shape that accepts both verdicts of a double pass **so that** the evidence the protocol produces stops being discarded at write time.

**Priority:** P0
**Size:** L
**Dependencies:** none

**Acceptance criteria:**
- [ ] `adjudication.agreement.pairs` holds `AdjudicatedPair { context, independence, line, passes, path, population, repository, rule }`, with `passes` exactly two entries of `Pass { judge, justification, verdict }`, all under `serde(deny_unknown_fields)` with no optional field
- [ ] A pair whose two passes agree has exactly one `ReviewedSite` of the same identity carrying that verdict; a test names the site when it does not
- [ ] A pair whose two passes disagree has no `ReviewedSite` of that identity; a test names the site when one exists
- [ ] Two pairs sharing `(rule, population, repository, path, line)` are refused the way `reviewed` refuses a duplicate, naming the identity
- [ ] `schema_version` moves from 3 to 4 and is asserted against a constant, closing the current free bump where the field is deserialized and never checked
- [ ] `adjudication_of` at `tests/corpus_precision.rs:789` fabricates the new block, and every existing test in `tests/corpus_precision.rs` still passes unchanged
- [ ] `cargo test` passes with the artifact carrying an empty `pairs` list before any pair is added

---

#### US-002: Track the three open escalations as data

**As** the adjudicator of last resort, **I want** the sites the two passes disagreed on to be findable in the artifact **so that** the protocol's own uncertainty leaves a trace instead of a sentence.

**Priority:** P0
**Size:** M
**Dependencies:** US-001

**Acceptance criteria:**
- [ ] The three sites of 2026-08-11 (ripgrep `crates/ignore/src/dir.rs:340`, thiserror `impl/src/expand.rs:31`, thiserror `impl/src/expand.rs:221`) enter `pairs` with both verdicts and both justifications restored from `docs/top-rules-precision-2026-08.md`
- [ ] Escalation is derived, never stored: no `escalated` field exists anywhere, and the count of open escalations is computed from disagreeing pairs
- [ ] The precision report publishes `escalations_open`, and a test recomputes it from `pairs`
- [ ] None of the three appears in `reviewed`, and the rates of `complex_function` on the healthy population are byte-identical to today's
- [ ] A pair carrying two identical verdicts but marked open anywhere fails the suite, since agreement and escalation cannot both hold

---

#### US-003: Name the judge and the independence of each pass

**As** a reader of the corpus record, **I want** each pass to say who produced it and how the two passes were kept apart **so that** self-preference bias is a measurable quantity rather than an assumption.

**Priority:** P0
**Size:** M
**Dependencies:** US-001

**Acceptance criteria:**
- [ ] `Pass::judge` is a non-empty model identifier, and a pair carrying an empty judge fails the suite naming the site
- [ ] `AdjudicatedPair::independence` is `separate_context` or `separate_model`, a closed vocabulary, and an unknown value fails deserialization
- [ ] A pair declaring `separate_model` whose two passes name the same judge fails the suite
- [ ] `Provenance` is unchanged and `ReviewedSite` gains no field, so the 110 `unrecorded` verdicts keep the only truthful value they have
- [ ] The precision report publishes the distinct judges behind each rate, sorted, the way `RulePrecision::provenance` already publishes provenances

---

#### US-004: Refuse a new single-pass verdict

**As** the maintainer, **I want** the protocol enforced rather than requested **so that** the next adjudication cannot quietly regress to one pass.

**Priority:** P0
**Size:** M
**Dependencies:** US-001, US-003

**Acceptance criteria:**
- [ ] `adjudication.protocol_cutoff` is a date, and every `ReviewedSite` whose rule and population were adjudicated after it must be backed by an agreeing pair
- [ ] The 177 verdicts predating the cutoff pass unchanged and are re-adjudicated by nothing
- [ ] A site added after the cutoff with no pair fails the suite, naming the site and the cutoff
- [ ] `RulePrecision` gains `doubly_judged`, the count of that rate's sites backed by a pair, recomputed by the test that recomputes the rate
- [ ] The failure message states which of the two conditions was violated, so a contributor does not have to read the harness to find out

---

### EP-002: What a rate is worth

**Definition of done:** every published rate carries its Wilson 95 % interval and a verdict on what that interval settles against the gate threshold, and the agreement statistics are recomputed by a test rather than written by hand.

---

#### US-005: Publish the Wilson 95 % interval beside every rate

**As** the reader of a rate, **I want** the interval it rests on **so that** a rate from one site and a rate from forty stop looking identical.

**Priority:** P0
**Size:** L
**Dependencies:** none

**Acceptance criteria:**
- [ ] `RulePrecision` gains `interval_low_basis_points` and `interval_high_basis_points`, present exactly when `false_positive_rate_basis_points` is, computed as the Wilson score interval at 95 %
- [ ] Bounds are integer basis points, rounded half away from zero, clamped to [0, 10000]; no float reaches the artifact
- [ ] `two_computations_of_the_precision_report_are_identical` covers the interval and stays byte-identical
- [ ] Both populations publish intervals: `precision` and `agent_population.precision`
- [ ] At x = 0, n = 1 the interval is not [0, 0]; the test asserts the published upper bound for `clippy::ptr_arg` is above 7000 basis points
- [ ] A rate whose stored interval disagrees with the recomputed one fails the suite naming the rule, the way `assert_eq!(computed, artifact.precision)` already fails on the rate

---

#### US-006: Recompute the agreement statistics instead of writing them

**As** the maintainer, **I want** kappa and agreement to be checked data **so that** the sentence in `sampling` stops being the only place they live.

**Priority:** P0
**Size:** L
**Dependencies:** US-001

**Acceptance criteria:**
- [ ] `adjudication.agreement.coefficients` publishes one row per `(rule, population)` with at least one pair: `pairs`, `agreed`, the 2x2 table, `kappa_basis_points`, `kappa_status`
- [ ] Every field is recomputed from `pairs` by a test and compared field by field, and a hand-edited coefficient fails naming the rule and the field
- [ ] A rule where one pass shows no variance publishes `kappa_status: undefined_no_variance` and no kappa value, never 1.0
- [ ] A `(rule, population)` with zero pairs has no row at all, rather than a row of zeros
- [ ] The `sampling` prose states no quantity that the coefficients block does not publish, and a test greps it for the digit patterns it is allowed to contain
- [ ] The three `assert!(len > 80)` checks at `tests/corpus_precision.rs:486-488` stay, since prose still has to exist, but they are no longer the only check on that field

---

#### US-007: Publish what each interval settles, and what the gate admits blind

**As** the reader of the gate, **I want** the rules it clears without evidence to be named **so that** `verdict: passed` stops resting on five samples that cannot support it.

**Priority:** P0
**Size:** M
**Dependencies:** US-005

**Acceptance criteria:**
- [ ] `RulePrecision` gains `separation`: `above` when the lower bound exceeds the threshold, `below` when the upper bound is under it, `indecisive` otherwise, present exactly when the rate is
- [ ] The comparison is strict on both sides, so a rate whose interval touches 500 basis points is `indecisive`
- [ ] `GateOutcome` gains `indecisive`, the sorted list of rules carrying a measured rate that is neither noisy nor decisively clean
- [ ] `the_published_gate_is_the_gate_recomputed_from_the_shipped_catalog` recomputes the new list, and a hand-edited one fails
- [ ] On today's data the list holds exactly `clippy::missing_safety_doc`, `clippy::ptr_arg`, `clippy::rc_buffer`, `clippy::stable_sort_primitive` and `rust_doctor::cargo::duplicate_major_versions`, and a test asserts it is not empty so the field cannot silently degenerate
- [ ] `MINIMUM_REVIEWED_SITES` is unchanged at 5, and no rate published today is withheld by this story

---

### EP-003: The judges

**Definition of done:** the corpus publishes a prevalence-robust agreement coefficient beside kappa, the sampling procedure is data rather than prose, and the skill that drives the protocol emits the recorded shape.

---

#### US-008: Publish Gwet's AC1 beside kappa

**As** the reader of an agreement figure, **I want** a coefficient that does not collapse at high prevalence **so that** 90 % agreement stops reading as "moderate".

**Priority:** P1
**Size:** M
**Dependencies:** US-006

**Acceptance criteria:**
- [ ] Each coefficient row gains `ac1_basis_points`, computed from the same 2x2 table, recomputed by the test
- [ ] AC1 is defined wherever the table has at least one pair, including the no-variance case where kappa is undefined, and the test asserts both behaviors on the same row
- [ ] A row where kappa is undefined and AC1 is absent fails the suite
- [ ] The `complex_function` healthy row publishes an AC1 materially above its kappa, and the test asserts the ordering rather than the values
- [ ] Negative AC1 is representable, so the field is signed and a hand-clamped negative value fails

---

#### US-009: Turn the sampling procedure into data

**As** the maintainer, **I want** the stride sample to be reproducible from the record **so that** "deterministic stride sampling" is a claim a test can replay.

**Priority:** P1
**Size:** M
**Dependencies:** US-001

**Acceptance criteria:**
- [ ] `adjudication.sampling_plan` publishes, per `(rule, population)`, the observed population size, the target, and the resulting indices rule `floor(i * n / k)` with `k = min(target, n)`
- [ ] A test replays the plan and asserts the reviewed sites of that rule are exactly the sites the stride selects, for every rule adjudicated after the protocol cutoff
- [ ] A plan whose target exceeds the observed population fails the suite naming the rule
- [ ] A rule adjudicated before the cutoff carries no plan, and its absence is a fact rather than an empty row
- [ ] The word `stride` appears in a test, which is the regression this story closes

---

#### US-010: Make the skill emit the recorded shape

**As** the orchestrating agent, **I want** `corpus-adjudicate` to write pairs **so that** the procedure and the record stop describing different protocols.

**Priority:** P1
**Size:** M
**Dependencies:** US-001, US-003, US-004, US-009

**Acceptance criteria:**
- [ ] `.claude/skills/corpus-adjudicate/SKILL.md` documents the pair shape, the judge field, the independence values and the cutoff rule, replacing the prose that describes what to compute by hand
- [ ] The skill's done-when clause requires the coefficients block to recompute clean rather than requiring a kappa figure to be stated
- [ ] The skill states that an escalation is never resolved by an agent, and that a disagreeing pair stays out of `reviewed` until a human verdict arrives
- [ ] A contract test asserts every JSON key the skill names exists in the harness structs, the way `tests/skill_contract.rs` asserts flags against `--help`
- [ ] The skill names no field the schema does not have, and a renamed field fails that test

---

### EP-004: The population the tool exists for

**Definition of done:** the three unjudged structural rules of the agent population carry a rate produced under the recorded protocol, with their intervals and separation published.

---

#### US-011: Adjudicate `oversized_unit` on the agent population

**As** the reader of an agent-written workspace's report, **I want** the largest structural rule of that population to have a measured rate **so that** its ranking rests on evidence from the code it fires on.

**Priority:** P1
**Size:** M
**Dependencies:** US-001, US-003, US-004, US-009

**Acceptance criteria:**
- [ ] 20 production-context sites are stride-sampled from the 828 observed, judged by two independent passes, and recorded as pairs
- [ ] Agreeing pairs produce `ReviewedSite` entries with `population: agent` and `context: production`; disagreeing pairs produce none and raise `escalations_open`
- [ ] Every justification names what was read, and a justification that could have been written without opening the file is grounds to rejudge the site
- [ ] The published rate, its interval and its separation appear under `agent_population.precision`, recomputed by the test
- [ ] `precision` for the healthy population is byte-identical to today's, and `CORPUS_NOISE` is unchanged

---

#### US-012: Adjudicate `duplicate_function_body` on the agent population

**As** the reader of an agent-written workspace's report, **I want** the clone rule to have a rate on that population **so that** the dimension core-v3 charges most heavily is not ranked blind.

**Priority:** P1
**Size:** M
**Dependencies:** US-011

**Acceptance criteria:**
- [ ] 20 production-context sites are stride-sampled from the 226 observed, judged twice, recorded as pairs
- [ ] The sample is drawn from production context only, and a site drawn from tests, benches or examples fails the context assertion the harness already makes
- [ ] A clone family is one site whatever its `related` array names, matching what the report publishes, and the justification says which member was read
- [ ] The rate, interval and separation are published and recomputed
- [ ] Its healthy rate of 4000 basis points is unchanged, so the two populations publish two rates from their own sites

---

#### US-013: Adjudicate `near_duplicate_function_body` on the agent population

**As** the reader of an agent-written workspace's report, **I want** the near-duplicate rule to have a rate on that population **so that** the last unjudged structural rule stops being ranked on a healthy-code estimate.

**Priority:** P1
**Size:** M
**Dependencies:** US-011

**Acceptance criteria:**
- [ ] 20 production-context sites are stride-sampled from the 204 observed, judged twice, recorded as pairs
- [ ] The justification distinguishes a near-duplicate that is a defect from deliberate parallel structure, since that distinction is the rule's whole failure mode
- [ ] The rate, interval and separation are published and recomputed
- [ ] Its healthy rate, published from 5 sites, is unchanged and its indecisive interval is now visible beside it
- [ ] A pair whose two passes split on that distinction escalates rather than being resolved toward the majority verdict of the rule
- [ ] After this story no structural rule of the agent population has zero verdicts, and a test naming the offending rule fails if one does

---

#### US-014: Publish what the three new rates settle

**As** the maintainer, **I want** the agent population's rates to state what n = 20 buys **so that** the decision about whether the score should read them is made against a measured interval.

**Priority:** P1
**Size:** S
**Dependencies:** US-011, US-012, US-013, US-007

**Acceptance criteria:**
- [ ] The three new rates carry an interval and a separation verdict, recomputed
- [ ] The precision report publishes the count of agent-population rules carrying a rate, and the test asserts it rose from 4 to 7
- [ ] `CORPUS_NOISE` is unchanged and `the_noise_the_score_ranks_by_matches_the_adjudicated_rate` passes untouched, since it reads the healthy rates alone
- [ ] The artifact records the comparison between each rule's healthy and agent rate, so the size of the gap is a published number rather than a reader's subtraction
- [ ] A rate whose separation is `indecisive` at n = 20 is published as such rather than deepened, since deepening it is a decision this PRD does not take

---

### EP-005: Anchoring the sites

**Definition of done:** a published site that no reproduction run has ever covered fails the suite, and a change to the corpus record is reproduced before it lands.

---

#### US-015: Refuse a site no reproduction has covered

**As** the maintainer, **I want** hand-added sites to fail offline **so that** `path:line` stops being verified only under an environment variable nobody sets.

**Priority:** P2
**Size:** M
**Dependencies:** US-001

**Acceptance criteria:**
- [ ] `adjudication.position_proof` publishes a blake3 digest over the sorted identities of every reviewed site and every pair, plus the toolchain and date of the run that wrote it
- [ ] An always-on test recomputes the digest from the artifact's own site list and fails when it disagrees with the stored one, naming the count of sites it hashed
- [ ] Only the gated reproduction run rewrites the stored digest, and it does so after confirming each site against a live scan
- [ ] The reproduction tests stop returning silently: an unset `RUST_DOCTOR_CORPUS_DIR` prints why the test is skipped rather than passing in silence
- [ ] Adding a site by hand without a reproduction run fails `cargo test`, and the failure says a reproduction is required

---

#### US-016: Reproduce the corpus on a pull request that edits it

**As** the maintainer, **I want** the one workflow that reproduces the record to fire when the record changes **so that** the pinned-inputs rationale for running it manually still holds.

**Priority:** P2
**Size:** S
**Dependencies:** US-015

**Acceptance criteria:**
- [ ] `.github/workflows/corpus.yml` gains a `pull_request` trigger scoped to `tests/corpus.json`, `tests/support/corpus.rs` and `tests/corpus_precision.rs`
- [ ] `workflow_dispatch` stays, since reproducing without a change is still the way to check a toolchain move
- [ ] The job restores the clone cache before anything else, and a cache miss fails with a message naming the cache key rather than cloning eighteen repositories silently
- [ ] `AGENTS.md` updates the sentence stating the workflow is manual, since it currently gives the rationale this story overrides
- [ ] A pull request touching none of the three paths does not trigger the job, asserted by the path filter rather than by a step condition

---

## Functional Requirements

| ID | Requirement | Stories |
|----|-------------|---------|
| FR-1 | A doubly-judged site is stored as a pair of passes in `adjudication.agreement.pairs`, never as a second verdict on `ReviewedSite` | US-001 |
| FR-2 | An agreeing pair has exactly one matching `ReviewedSite`; a disagreeing pair has none, and that absence is escalation | US-001, US-002 |
| FR-3 | Escalation is derived from disagreement and stored nowhere | US-002 |
| FR-4 | Every pass names its judge, and every pair names how the two passes were kept independent | US-003 |
| FR-5 | A verdict entering `reviewed` after the protocol cutoff must be backed by an agreeing pair | US-004 |
| FR-6 | Every published rate carries a Wilson 95 % interval in integer basis points | US-005 |
| FR-7 | Agreement, the contingency table, kappa and AC1 are recomputed from `pairs` and compared field by field | US-006, US-008 |
| FR-8 | Kappa is published as undefined when a pass has no variance, never as a value | US-006 |
| FR-9 | Every rate publishes what its interval settles against the gate threshold, and the gate names the rules it admits blind | US-007 |
| FR-10 | The stride sample is replayable from the record | US-009 |
| FR-11 | The skill emits the recorded shape and names no field the schema lacks | US-010 |
| FR-12 | The three unjudged structural rules of the agent population carry a rate produced under the protocol | US-011, US-012, US-013, US-014 |
| FR-13 | A published site not covered by a reproduction run fails the suite offline | US-015 |
| FR-14 | A pull request editing the corpus record reproduces it | US-016 |

## Non-Functional Requirements

| ID | Requirement | Measurement |
|----|-------------|-------------|
| NFR-1 | The precision report stays byte-reproducible, intervals and coefficients included | `two_computations_of_the_precision_report_are_identical` extended and green |
| NFR-2 | No float reaches the artifact; every statistic is an integer basis point | schema check plus `serde(deny_unknown_fields)` on every new struct |
| NFR-3 | Every file of the corpus harness is under the 1000 lines `oversized_unit` reports at | a new `the_corpus_harness_holds_the_size_bound_it_measures_for`, currently failing on `tests/support/corpus.rs` at 1486 and `tests/corpus_precision.rs` at 1543 |
| NFR-4 | No test reaches the network, and no corpus source is committed here | `no_corpus_repository_is_committed_in_this_repository` green |
| NFR-5 | The artifact carries no absolute path, no environment variable and no user data, including in justifications | a test scanning every new string field for a path separator prefix and a home directory |
| NFR-6 | Every new struct has no optional field except where absence is itself a fact, and that fact is documented at the field | review against `RulePrecision::provenance`, which sets the precedent |
| NFR-7 | The always-on suite gains no dependency on the clone cache: every new test runs from the artifact alone | `cargo test` green with both corpus environment variables unset |

## Edge Cases & Error States

| Case | Expected behavior | Story |
|------|-------------------|-------|
| A pair agrees but no `ReviewedSite` matches it | Suite fails naming the site identity | US-001 |
| A pair disagrees and a `ReviewedSite` of that identity exists | Suite fails: an escalated site cannot carry a published verdict | US-001 |
| Two pairs share one identity | Refused as a duplicate, naming the identity, the way `reviewed` refuses one | US-001 |
| A pair carries an empty judge | Suite fails naming the site | US-003 |
| A pair declares `separate_model` with the same judge twice | Suite fails | US-003 |
| A site is added to `reviewed` after the cutoff with no pair | Suite fails naming the site and the cutoff | US-004 |
| A rule adjudicated only before the cutoff | Passes unchanged, `doubly_judged` is 0, nothing is re-adjudicated | US-004 |
| x = 0, n = 1 | Interval is [0, 7930] basis points, separation `indecisive`, never [0, 0] | US-005, US-007 |
| A rate whose interval touches 500 basis points exactly | `indecisive`, since the comparison is strict on both sides | US-007 |
| One pass shows no variance across a rule | `kappa_status: undefined_no_variance`, no value; AC1 still defined | US-006, US-008 |
| A `(rule, population)` with zero pairs | No coefficient row at all, not a row of zeros | US-006 |
| A hand-edited coefficient, interval or gate list | Suite fails naming the rule and the field | US-006, US-007 |
| A sampling plan whose target exceeds the observed population | Suite fails naming the rule | US-009 |
| The skill names a JSON key the schema lacks | Contract test fails | US-010 |
| A sampled agent site turns out to sit in test context | Context assertion fails, the way the harness already checks `SiteContext::matches` | US-012 |
| A site is added by hand without a reproduction run | Digest mismatch, suite fails, message says a reproduction is required | US-015 |
| `RUST_DOCTOR_CORPUS_DIR` unset | Reproduction tests print why they are skipped instead of passing in silence | US-015 |
| The corpus workflow runs on a pull request with a cache miss | Job fails naming the cache key, never clones eighteen repositories silently | US-016 |

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Adjudication cost: 60 sites judged twice is 120 judgments plus the escalations they produce | Schedule | Stride-sample at 20 rather than census 828, 226 and 204; each judgment is stateless and site-local, so the fan-out is a workflow rather than a session |
| Self-preference bias: the judge evaluates the findings of the tool it serves | The rate is optimistic in the direction that flatters the catalog | `independence` recorded per pair, cross-family passes recommended, and no agent ever resolves a disagreement (US-003, US-004) |
| Kappa collapses at 87 % prevalence and reads as moderate for 90 % agreement | The agreement figure misleads whoever reads it | AC1 published beside it, with the ordering asserted rather than the values (US-008) |
| The two files this PRD edits are already the repository's largest violations of its own rule, measured at 1486 and 1543 lines | Every schema addition makes it worse | The agreement block and the interval arithmetic get files of their own under `tests/support/corpus/`, and NFR-3 gates it |
| Regenerating `tests/corpus.json` by hand unpins it | The artifact stops being a measurement | US-015's digest and US-016's pull-request trigger |
| Schema version 4 breaks a consumer | Low: the artifact has no external consumer, only tests | `schema_version` asserted against a constant for the first time (US-001) |
| n = 20 leaves the agent rates indecisive | The new rates cannot clear a rule either | Published as `indecisive` rather than deepened; deepening is Open Question 2, not a silent extension of scope |

## Non-Goals

- **No rule is admitted, retired or re-adjudicated in the catalog.** The 62 stay 62. `src/policy/catalog.rs` and `src/policy/rejected.json` are untouched.
- **The score is untouched.** core-v3, the λ table, the five dimensions and their weights, the tier ceilings and the three bands do not move. `src/audit/` is not modified.
- **`CORPUS_NOISE` values do not change.** The interval is published; the ranking still reads the point estimate. Switching the table to the agent rates is a product decision this PRD records as an open question and does not take.
- **The 177 single-pass verdicts are not re-adjudicated.** They keep their `provenance` and receive no fabricated second pass. The protocol applies forward from the cutoff.
- **No repository is added to either population.** Ten healthy and eight agent, at the pinned commits.
- **Adjudication never runs unattended.** No CI job produces a verdict, and no automated run resolves an escalation.
- **Krippendorff's alpha is excluded**, not deferred: two raters on a nominal binary scale is where it reduces to kappa.
- **`MINIMUM_REVIEWED_SITES` stays 5.** The honesty fix is the interval, not the floor.
- **No network, anywhere.** Not in a test, not in the harness, not in the tool.

## Files NOT to Modify

- `src/audit.rs`, `src/audit/density.rs`, `src/audit/source_inventory.rs` and the rest of `src/audit/`: the score is out of scope
- `src/policy/catalog.rs` and `src/policy/rejected.json`: no rule is admitted or turned down here
- `src/policy/noise.rs`: the `CORPUS_NOISE` table stays as measured
- `src/delta.rs` and `tests/fixtures/baseline/delta-oracle.json`
- `src/report.rs` and `SCHEMA_VERSION`: the corpus artifact has its own version, and this PRD moves that one
- `npm/` and `.github/workflows/release.yml`
- The `manifest`, `observations`, `lambdas`, `score_distribution`, `structural_density` and `toolchain` blocks of `tests/corpus.json`: this PRD adds to `adjudication`, `precision`, `agent_population.precision` and `gate`, and to nothing else

## Technical Considerations

**The pair lives beside `reviewed`, not inside it.** Recommended: a sibling `adjudication.agreement` block coupled to `reviewed` by an invariant. Rejected: a second verdict field on `ReviewedSite`, because the identity `(rule, repository, path, line)` is what `precision_of` uses to detect a double read and force `Incomplete` (`tests/support/corpus.rs:1234-1246`), and because an escalated site has no place in `reviewed` at all, so no field of that struct can hold it. Engineering to confirm the invariant is asserted in both directions, since only one of the two failure modes is intuitive.

**The artifact shape.** Recommended sketch, subject to the field names the implementation settles:

```json
"agreement": {
  "pairs": [{
    "context": "production", "independence": "separate_model",
    "line": 340, "path": "crates/ignore/src/dir.rs",
    "passes": [
      {"judge": "<model-id>", "justification": "...", "verdict": "false_positive"},
      {"judge": "<other-model-id>", "justification": "...", "verdict": "true_positive"}
    ],
    "population": "healthy", "repository": "ripgrep",
    "rule": "rust_doctor::structure::complex_function"
  }],
  "coefficients": [{
    "ac1_basis_points": 8800, "agreed": 26,
    "kappa_basis_points": 5300, "kappa_status": "defined",
    "pairs": 29, "population": "healthy", "rule": "...",
    "table": {"both_false_positive": 3, "both_true_positive": 23,
              "first_only_false_positive": 2, "second_only_false_positive": 1}
  }]
}
```

**Wilson, in integer basis points.** Recommended: compute `p ± z·√(p(1−p)/n + z²/4n²)` over `1 + z²/n` in f64 with `z = 1.959963985`, round half away from zero to basis points, clamp to [0, 10000]. Rejected: Wald, which returns [0, 0] at x = 0 and is therefore wrong at three of the five samples the gate currently clears; and Clopper-Pearson, which needs the incomplete beta function and a dependency the crate has no other use for. IEEE-754 mandates correctly rounded square root and arithmetic, so the result is byte-stable across platforms, which NFR-1 requires. Engineering to confirm the rounding mode is stated once and read from one place, not spelled at each of the four call sites.

**Kappa and AC1 from one table.** Recommended: build the 2x2 contingency table once per `(rule, population)` and derive both coefficients from it, so the two can never disagree about the same pairs. `kappa_status` is an enum rather than an `Option` carrying no reason, because "undefined because one pass had no variance" is a fact worth naming and is exactly the case the 2026-08-11 run hit twice.

**The harness has to pass the rule it measures for.** Measured on 2026-08-21 with the release binary: this repository's own scan reports `rust_doctor::structure::oversized_unit` on `tests/support/corpus.rs` (1486 lines) and `tests/corpus_precision.rs` (1543 lines), the two largest violations in the tree. `no_unit_of_this_crate_s_own_source_is_a_hotspot` does not name them because it filters `finding.path.starts_with("src/")` (`src/structure/tests.rs:392`). Recommended: split along the same seams the rest of the crate uses, `tests/support/corpus/agreement.rs` for the pair and coefficient shapes and `tests/support/corpus/interval.rs` for the statistics, with NFR-3 as the gate that keeps them there. Rejected: widening the self-scan filter to cover `tests/`, which would fail the suite on nine other files this PRD does not touch.

**The digest anchors what a gated test cannot.** Recommended: blake3 over the sorted `(population, repository, rule, path, line)` identities, written only by the reproduction run and checked offline by an always-on test. blake3 is already a dependency (the structural family identity uses it), so this adds none. Rejected: freezing the full position list as an oracle, which doubles the site list in the artifact and drifts the way the three hand-written catalog copies drifted.

**Everything moves together or not at all.** Every corpus struct carries `serde(deny_unknown_fields)`, so each schema story is a coordinated edit of `tests/support/corpus.rs`, `tests/corpus.json` and the synthetic fabricator `adjudication_of` (`tests/corpus_precision.rs:789`). That is the mechanism that has kept the artifact and its reader in step, and it is why `schema_version` moving from 3 to 4 must also become an asserted constant: today it is deserialized and never checked, so the bump costs nothing and proves nothing.

## Success Metrics

| Metric | Baseline | Target | Timeframe | How Measured |
|--------|----------|--------|-----------|--------------|
| Verdicts recording both passes | 0 of 273 | 100 % of verdicts produced after the cutoff | Month-1 | `doubly_judged` per rate, recomputed by the test |
| Published healthy rates carrying an interval | 0 of 24 | 24 of 24 | Month-1 | schema check on `RulePrecision` |
| Rules the gate clears with an indecisive interval, unnamed | 5 | 0 | Month-1 | `gate.indecisive`, recomputed and asserted non-empty |
| Escalated sites tracked in data | 0 of 3 | 3 of 3 | Month-1 | `escalations_open` recomputed from `pairs` |
| Agent-population structural rules with zero verdicts | 3 of 4 | 0 of 4 | Month-1 | count over `agent_population.precision` |
| Agent-population rules with a published rate | 4 of 14 | 7 of 14 | Month-1 | count over `agent_population.precision` |
| Quantities in `adjudication.sampling` recomputed by no test | all of them | 0 | Month-1 | the coefficients block plus the digit-pattern check on the prose |
| Corpus harness files over the 1000-line bound | 2 (1486, 1543) | 0 | Month-1 | `the_corpus_harness_holds_the_size_bound_it_measures_for` |

## Open Questions

1. **Does `CORPUS_NOISE` switch to the agent rates once the three structural rules carry one?** The report ranks for the user of this tool, whose code is the agent population, and `src/audit.rs:678` currently discounts by a rate measured on code nobody wants disturbed. AGENTS.md already states the switch is a product decision rather than a consequence of a number. This PRD produces the number and takes no decision.
2. **Is n = 20 enough on the agent population?** It separates a 40 % rate from 5 % with margin, and it cannot produce a `below` verdict for any rule. US-014 publishes what it actually settles, which is the input to deciding whether any rule is worth taking to n = 73.
3. **Should the second pass be required to come from a different model family?** US-003 records `independence` as a fact. Mandating `separate_model` is a follow-on once there are enough pairs of each kind to measure whether cross-family pairs disagree more than same-model ones. Mandating it first would be a rule with no measurement behind it, which is the failure this whole PRD is about.
4. **Does AC1 replace kappa in how the gate is read, or stay an annotation beside it?** This PRD publishes both and reads neither into a gate decision.
5. **What provenance does a site carry after Arthur resolves an escalation?** `human` is the obvious answer, but it loses the fact that two agents disagreed first, which is the most informative thing about that site. A third value naming the escalation is the alternative, and it costs a variant on a closed enum that 273 existing sites already deserialize against.
[/PRD]
