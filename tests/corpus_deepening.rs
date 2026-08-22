#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! What a scope measured at its whole subpopulation buys, and what it does not.
//!
//! Two healthy structural rules were the weakest scopes the record published:
//! five sites each, drawn under the pre-protocol default, carrying an interval
//! nearly six thousand basis points wide and ranking every repair the report
//! proposes for those rules. A five-site rate resolves steps of twenty points,
//! so it separates a rule with no observed false positive from one that has
//! some and settles nothing else.
//!
//! They are now drawn to the whole of their production subpopulation. That is
//! the end of what sampling can do for them: the draw takes every site the
//! corpus has, and what still bounds the interval is the corpus itself. This
//! crate is where that distinction is asserted rather than described, and where
//! the difference interval between the two populations is checked against the
//! bounds a reader recomputes.

mod support;

use support::corpus::comparison::{ComparisonVerdict, DifferenceMethod, verdict_of};
use support::corpus::interval::{NOMINAL_COVERAGE_SITES, difference_95, wilson_95};
use support::corpus::sampling::{stride, target_floor};
use support::corpus::{Population, PrecisionStatus, RulePrecision, artifact};

const NEAR_DUPLICATE: &str = "rust_doctor::structure::near_duplicate_function_body";
const OVERSIZED: &str = "rust_doctor::structure::oversized_unit";

/// The five sites each scope carried before the deepening, drawn on the
/// five-site default of the pre-protocol samples.
///
/// Frozen by position rather than by count. The deepened draw recomputes every
/// index from a population that is no longer the same size, so the question the
/// carry-over answers for a sampled scope has to be answered here for an
/// exhausted one: a draw that takes the whole subpopulation contains its own
/// predecessor or the record dropped verdicts that were paid for.
const PRIOR_OVERSIZED: [(&str, &str, u64); 5] = [
    ("anyhow", "src/error.rs", 1),
    ("bytes", "src/bytes_mut.rs", 624),
    ("ripgrep", "crates/globset/src/glob.rs", 1),
    ("ripgrep", "crates/printer/src/standard.rs", 884),
    ("serde_json", "src/de.rs", 1387),
];

const PRIOR_NEAR_DUPLICATE: [(&str, &str, u64); 5] = [
    ("async-channel", "src/lib.rs", 569),
    ("bytes", "src/bytes.rs", 1573),
    ("ripgrep", "crates/globset/src/lib.rs", 825),
    ("ripgrep", "crates/regex/src/literal.rs", 550),
    ("serde_json", "src/lexical/shift.rs", 8),
];

fn healthy(id: &str) -> RulePrecision {
    artifact()
        .precision
        .into_iter()
        .find(|rule| rule.id == id)
        .expect("the healthy population publishes this rule")
}

fn width(rule: &RulePrecision) -> u64 {
    rule.interval_high_basis_points.unwrap() - rule.interval_low_basis_points.unwrap()
}

// ---------------------------------------------------------------------------
// US-010 and US-011: the two weak healthy scopes are measured, not sampled
// ---------------------------------------------------------------------------

/// The draw is the whole production subpopulation, and the plan says so.
///
/// The target is what makes this checkable. A plan asking for the protocol's
/// twenty over a population of forty-six would describe a draw that did not
/// happen, and a plan asking for forty-six is a scope that took every site it
/// had: `target_floor` is what turns the protocol's number into the floor that
/// permits it rather than the quota that would refuse it.
#[test]
fn the_two_deepened_healthy_scopes_draw_their_whole_production_subpopulation() {
    let artifact = artifact();
    for (rule, observed) in [(OVERSIZED, 46u64), (NEAR_DUPLICATE, 30)] {
        let plan = artifact
            .adjudication
            .sampling_plan
            .iter()
            .find(|plan| plan.rule == rule && plan.population == Population::Healthy)
            .expect("the record publishes a healthy sampling plan for this rule");
        assert_eq!(plan.observed, observed, "{rule}");
        assert_eq!(plan.target, observed, "{rule}");
        assert!(plan.target >= target_floor(plan.observed), "{rule}");
        assert_eq!(plan.indices, stride(observed, observed), "{rule}");
        assert_eq!(plan.indices, (0..observed).collect::<Vec<u64>>(), "{rule}");
        // A draw that takes everything has nothing left to carry: a carried
        // position is one the stride did not select, and this stride selects
        // all of them.
        assert_eq!(plan.carried_over, Vec::<u64>::new(), "{rule}");
    }
}

/// Every site the plan drew was judged twice, and the sites the two passes
/// agreed on are the sites the rate rests on.
///
/// `doubly_judged` equal to `reviewed` is the invariant, not `reviewed` equal
/// to the population: a pair whose passes disagreed carries no reviewed site by
/// protocol, and the seven `oversized_unit` disagreements are exactly the
/// distance between the forty-six sites drawn and the thirty-nine published.
/// The two facts are asserted together because reading either alone reports the
/// escalation queue as sites nobody looked at.
#[test]
fn every_drawn_site_is_judged_twice_and_a_disagreement_publishes_no_verdict() {
    let artifact = artifact();
    for (rule, drawn, reviewed) in [(OVERSIZED, 46u64, 39u64), (NEAR_DUPLICATE, 30, 30)] {
        let measure = healthy(rule);
        assert_eq!(measure.status, PrecisionStatus::Measured, "{rule}");
        assert_eq!(measure.reviewed, reviewed, "{rule}");
        assert_eq!(measure.doubly_judged, measure.reviewed, "{rule}");

        let pairs: Vec<_> = artifact
            .adjudication
            .agreement
            .pairs
            .iter()
            .filter(|pair| pair.rule == rule && pair.population == Population::Healthy)
            .collect();
        assert_eq!(pairs.len() as u64, drawn, "{rule}");
        let agreeing = pairs.iter().filter(|pair| pair.agrees()).count() as u64;
        assert_eq!(agreeing, reviewed, "{rule}");

        // No agent settled a disagreement. An escalated pair keeps both
        // verdicts and publishes none, which is what leaves it in the queue.
        for pair in pairs.iter().filter(|pair| !pair.agrees()) {
            assert_eq!(pair.verdict(), None, "{}", pair.label());
        }
    }
}

/// The five sites each scope already carried are inside the deepened draw.
///
/// Asserted rather than assumed. An exhaustive draw contains every earlier one
/// as a matter of arithmetic, but only if the population it exhausts is the
/// population those sites came from, and the correction of August 2026 to the
/// production predicate is what makes that a question rather than a tautology.
#[test]
fn the_five_sites_each_scope_already_carried_are_retained() {
    let artifact = artifact();
    for (rule, prior) in [
        (OVERSIZED, PRIOR_OVERSIZED),
        (NEAR_DUPLICATE, PRIOR_NEAR_DUPLICATE),
    ] {
        for (repository, path, line) in prior {
            let judged = artifact
                .adjudication
                .agreement
                .pairs
                .iter()
                .any(|pair| {
                    pair.rule == rule
                        && pair.population == Population::Healthy
                        && pair.repository == repository
                        && pair.path == path
                        && pair.line == line
                });
            assert!(
                judged,
                "{rule} dropped a site its earlier sample drew: {repository}/{path}:{line}"
            );
        }
    }
}

/// The interval each scope publishes is the one its own sample supports, and it
/// is narrower than the five-site interval it replaced.
///
/// The width is asserted against a bound that holds whatever the rate, because
/// a target on the rate would be a target on the answer: what deepening buys is
/// precision, and precision is the width. Each bound is the widest a Wilson
/// interval gets at that scope's own denominator, which is the interval at
/// `x = n / 2`: 2993 over the thirty-nine verdicts `oversized_unit` publishes
/// and 3370 over the thirty of `near_duplicate_function_body`. The story fixed
/// 2748, the same bound at the forty-seven sites the population held before the
/// out-of-line `#[cfg(test)]` correction of August 2026 shrank it to forty-six,
/// and before the seven disagreements that scope escalated left thirty-nine
/// verdicts of the forty-six it drew. A bound stated at a denominator is
/// conservative at every larger one, so resolving an escalation cannot fail it.
#[test]
fn each_deepened_scope_publishes_a_narrower_interval_than_its_five_site_sample() {
    // The five-site interval both scopes published before: four false
    // positives in five sites, which is the widest a Wilson interval gets at
    // that denominator away from the ends.
    let (low, high) = wilson_95(4, 5);
    let before = high - low;
    assert_eq!(before, 5883);

    for (rule, bound) in [(OVERSIZED, 2993u64), (NEAR_DUPLICATE, 3370)] {
        let measure = healthy(rule);
        let (low, high) = wilson_95(measure.false_positives.unwrap(), measure.reviewed);
        assert_eq!(Some(low), measure.interval_low_basis_points, "{rule}");
        assert_eq!(Some(high), measure.interval_high_basis_points, "{rule}");
        assert!(
            width(&measure) <= bound,
            "{rule} publishes an interval {} wide against the {bound} its sample supports",
            width(&measure)
        );
        assert!(width(&measure) < before, "{rule}");
    }
}

/// `near_duplicate_function_body` cannot be deepened further, and the record
/// says the limit is the corpus.
///
/// The whole production subpopulation of that scope is thirty sites, under the
/// forty per arm the difference interval names as its nominal coverage floor.
/// No sampling decision moves that number: the draw already takes everything.
/// The record states it in prose and publishes it on the row, and this is where
/// the two are held to the same fact.
#[test]
fn the_scope_the_corpus_bounds_says_so_rather_than_asking_for_a_deeper_draw() {
    let artifact = artifact();
    let measure = healthy(NEAR_DUPLICATE);
    assert!(measure.reviewed < NOMINAL_COVERAGE_SITES);

    let row = artifact
        .agent_population
        .rate_comparison
        .iter()
        .find(|row| row.rule == NEAR_DUPLICATE)
        .expect("the comparison publishes this rule");
    assert!(row.below_nominal_coverage);
    assert_ne!(row.verdict, ComparisonVerdict::Separated);

    let prose = artifact.adjudication.sampling.as_str();
    assert!(prose.contains("the limit is the corpus, not the draw"), "{prose}");
}

// ---------------------------------------------------------------------------
// US-012: the difference between the two rates, with a verdict on it
// ---------------------------------------------------------------------------

/// The four intervals the record publishes, recomputed here from the counts.
///
/// Frozen by value rather than derived, because a test that recomputes the
/// bounds the same way the record does agrees with itself whatever the method
/// returns. These are the numbers a reader who disagrees has to disagree with.
const KNOWN: [(&str, i64, i64, ComparisonVerdict); 4] = [
    (
        "rust_doctor::structure::complex_function",
        -3437,
        282,
        ComparisonVerdict::Indistinguishable,
    ),
    (
        "rust_doctor::structure::duplicate_function_body",
        -3966,
        683,
        ComparisonVerdict::Indistinguishable,
    ),
    (
        NEAR_DUPLICATE,
        -3935,
        396,
        ComparisonVerdict::Indistinguishable,
    ),
    (OVERSIZED, -3725, -57, ComparisonVerdict::Separated),
];

/// Every comparison row publishes an interval, its method, and what it settles.
#[test]
fn each_compared_rule_publishes_a_difference_interval_and_a_verdict() {
    let artifact = artifact();
    let rows = &artifact.agent_population.rate_comparison;
    assert_eq!(rows.len(), KNOWN.len());
    for (row, (rule, low, high, verdict)) in rows.iter().zip(KNOWN) {
        assert_eq!(row.rule, rule);
        assert_eq!(row.difference_low_basis_points, low, "{rule}");
        assert_eq!(row.difference_high_basis_points, high, "{rule}");
        assert_eq!(row.verdict, verdict, "{rule}");
        assert_eq!(
            row.difference_method,
            DifferenceMethod::NewcombeHybridScore95,
            "{rule}"
        );
        // The interval is an interval for the same subtraction the gap states,
        // so the gap sits inside it and the sign of a separated row agrees with
        // the sign of its gap.
        assert!(
            (low..=high).contains(&row.gap_basis_points),
            "{rule} publishes a gap outside its own interval"
        );
    }
}

/// The interval comes from the counts, through the same Wilson bounds the
/// per-rule rates are published with.
///
/// Recomputed from `false_positives` and `reviewed` on both sides rather than
/// from the two rates, which is what makes this a check on the method and not
/// on the record's own arithmetic repeated.
#[test]
fn the_published_interval_is_the_hybrid_score_interval_of_the_two_samples() {
    let artifact = artifact();
    for row in &artifact.agent_population.rate_comparison {
        let healthy = healthy(&row.rule);
        let agent = artifact
            .agent_population
            .precision
            .iter()
            .find(|rule| rule.id == row.rule)
            .unwrap();
        let (low, high) = difference_95(
            (healthy.false_positives.unwrap(), healthy.reviewed),
            (agent.false_positives.unwrap(), agent.reviewed),
        );
        assert_eq!(row.difference_low_basis_points, low, "{}", row.rule);
        assert_eq!(row.difference_high_basis_points, high, "{}", row.rule);
        assert_eq!(row.verdict, verdict_of(low, high), "{}", row.rule);
    }
}

/// Both comparisons are strict, so a bound landing exactly on zero settles
/// nothing.
///
/// The three cases at the boundary, which is where a verdict written with the
/// wrong comparison differs from one written with the right one and nowhere
/// else. An interval touching zero places the two populations at the same rate:
/// the same strictness `separation` applies against the gate threshold.
#[test]
fn an_interval_touching_zero_is_indistinguishable_rather_than_separated() {
    assert_eq!(verdict_of(0, 500), ComparisonVerdict::Indistinguishable);
    assert_eq!(verdict_of(-500, 0), ComparisonVerdict::Indistinguishable);
    assert_eq!(verdict_of(0, 0), ComparisonVerdict::Indistinguishable);
    assert_eq!(verdict_of(1, 500), ComparisonVerdict::Separated);
    assert_eq!(verdict_of(-500, -1), ComparisonVerdict::Separated);
}

/// Two computations of the same difference are identical, bit for bit.
///
/// No float reaches the record: the bounds are integer basis points rounded
/// half away from zero, once, in `signed_basis_points`. A method rounding its
/// two bounds two ways is how an interval ends up wider on one side than the
/// arithmetic that produced it, and a record two machines disagree on is a
/// record neither of them can reproduce.
#[test]
fn two_computations_of_a_difference_interval_are_identical() {
    let samples = [
        (0u64, 1u64, 0u64, 1u64),
        (4, 5, 13, 39),
        (19, 30, 8, 20),
        (1, 40, 39, 40),
        (0, 0, 5, 5),
        (7, 7, 0, 7),
    ];
    for (first_positives, first_reviewed, second_positives, second_reviewed) in samples {
        let first = difference_95(
            (first_positives, first_reviewed),
            (second_positives, second_reviewed),
        );
        let second = difference_95(
            (first_positives, first_reviewed),
            (second_positives, second_reviewed),
        );
        assert_eq!(first, second);
        assert!(first.0 <= first.1);
        assert!((-10_000..=10_000).contains(&first.0));
        assert!((-10_000..=10_000).contains(&first.1));
    }
}

/// A row whose arms are small carries that fact and its verdict both.
///
/// Withholding the comparison where the sample is thin would withhold it
/// exactly where the reader most needs to know how little it settles, so the
/// coverage floor is published and never enforced. All four rows sit under it
/// today, and each still publishes a verdict.
#[test]
fn a_row_under_the_coverage_floor_publishes_the_fact_and_the_verdict_anyway() {
    let artifact = artifact();
    for row in &artifact.agent_population.rate_comparison {
        let healthy = healthy(&row.rule);
        let agent = artifact
            .agent_population
            .precision
            .iter()
            .find(|rule| rule.id == row.rule)
            .unwrap();
        assert_eq!(
            row.below_nominal_coverage,
            healthy.reviewed < NOMINAL_COVERAGE_SITES || agent.reviewed < NOMINAL_COVERAGE_SITES,
            "{}",
            row.rule
        );
        assert!(row.below_nominal_coverage, "{}", row.rule);
        assert_eq!(row.verdict, verdict_of(
            row.difference_low_basis_points,
            row.difference_high_basis_points,
        ), "{}", row.rule);
    }
}
