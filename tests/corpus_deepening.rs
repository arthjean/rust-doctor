#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use support::corpus::comparison::{ComparisonVerdict, DifferenceMethod, verdict_of};
use support::corpus::interval::{NOMINAL_COVERAGE_SITES, difference_95, wilson_95};
use support::corpus::sampling::{stride, target_floor};
use support::corpus::{Population, PrecisionStatus, RulePrecision, artifact};

const NEAR_DUPLICATE: &str = "rust_doctor::structure::near_duplicate_function_body";
const OVERSIZED: &str = "rust_doctor::structure::oversized_unit";

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

#[test]
fn the_two_deepened_healthy_scopes_draw_their_whole_production_subpopulation() {
    let artifact = artifact();
    for (rule, observed) in [(OVERSIZED, 46u64), (NEAR_DUPLICATE, 29)] {
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
        assert_eq!(plan.carried_over, Vec::<u64>::new(), "{rule}");
    }
}

#[test]
fn every_drawn_site_is_judged_twice_and_a_disagreement_publishes_no_verdict() {
    let artifact = artifact();
    for (rule, drawn, reviewed) in [(OVERSIZED, 46u64, 39u64), (NEAR_DUPLICATE, 29, 29)] {
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

        for pair in pairs.iter().filter(|pair| !pair.agrees()) {
            assert_eq!(pair.verdict(), None, "{}", pair.label());
        }
    }
}

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

#[test]
fn each_deepened_scope_publishes_a_narrower_interval_than_its_five_site_sample() {
    let (low, high) = wilson_95(4, 5);
    let before = high - low;
    assert_eq!(before, 5883);

    for (rule, bound) in [(OVERSIZED, 2993u64), (NEAR_DUPLICATE, 3418)] {
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

const KNOWN: [(&str, i64, i64, ComparisonVerdict); 4] = [
    (
        "rust_doctor::structure::complex_function",
        -3437,
        282,
        ComparisonVerdict::Indistinguishable,
    ),
    (
        "rust_doctor::structure::duplicate_function_body",
        -3282,
        1383,
        ComparisonVerdict::Indistinguishable,
    ),
    (
        NEAR_DUPLICATE,
        -4076,
        110,
        ComparisonVerdict::Indistinguishable,
    ),
    (OVERSIZED, -3882, -504, ComparisonVerdict::Separated),
];

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
        assert!(
            (low..=high).contains(&row.gap_basis_points),
            "{rule} publishes a gap outside its own interval"
        );
    }
}

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

#[test]
fn an_interval_touching_zero_is_indistinguishable_rather_than_separated() {
    assert_eq!(verdict_of(0, 500), ComparisonVerdict::Indistinguishable);
    assert_eq!(verdict_of(-500, 0), ComparisonVerdict::Indistinguishable);
    assert_eq!(verdict_of(0, 0), ComparisonVerdict::Indistinguishable);
    assert_eq!(verdict_of(1, 500), ComparisonVerdict::Separated);
    assert_eq!(verdict_of(-500, -1), ComparisonVerdict::Separated);
}

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
