use super::*;
use crate::audit::density::{decayed, dimension_score};

const BY_VALUE: &str = "clippy::large_types_passed_by_value";
const BUILD_STEP: &str = "rust_doctor::repo::secret_in_build_step";

fn with_causes(code: &str, category: &str, causes: &[(Option<&str>, usize)]) -> Vec<Diagnostic> {
    let mut diagnostics = diagnostics_for(&[(
        code,
        category,
        Severity::Warning,
        causes.iter().map(|(_, count)| count).sum(),
    )]);
    let mut assigned = diagnostics.iter_mut();
    for (cause, count) in causes {
        for diagnostic in assigned.by_ref().take(*count) {
            diagnostic.cause = cause.map(str::to_owned);
        }
    }
    diagnostics
}

fn rule<'a>(aggregation: &'a RuleAggregation, id: &str) -> &'a RuleAggregate {
    aggregation
        .rules
        .iter()
        .find(|rule| rule.id == id)
        .expect("the rule should be aggregated")
}

#[test]
fn three_hundred_and_three_sites_of_one_cause_and_seven_singles_are_charged_on_the_curve() {
    let diagnostics = with_causes(
        BY_VALUE,
        "performance",
        &[(Some("UiColors (292 byte)"), 303), (None, 7)],
    );
    let aggregation = aggregate_rules(20_000, 1, &diagnostics);
    let charged = rule(&aggregation, BY_VALUE).sites.total();
    assert!(
        (charged - (1.0 + 303f64.ln() + 7.0)).abs() < 0.001,
        "{charged} sites charged"
    );

    let score = Audit::build(20, 20_000, Status::Complete, &diagnostics)
        .score
        .expect("twenty kilolines is a scorable workspace");
    assert_eq!(score.projected_rule_ids, [BY_VALUE.to_owned()]);
    let repaired = Audit::build(20, 20_000, Status::Complete, &[])
        .score
        .expect("a clean workspace is scorable");
    assert_eq!(
        score.projected_after_top_three,
        Some(repaired.value),
        "the projection removes the rule at the charge the score took"
    );
    let expected = dimension_score(
        (1.0 + 303f64.ln() + 7.0) * 0.5 / 20.0,
        ScoreDimension::Performance,
    );
    assert_eq!(score.dimensions.performance, expected);
}

#[test]
fn a_rule_with_no_declared_cause_is_charged_its_core_v6_sites() {
    let diagnostics = diagnostics_for(&[("clippy::rc_buffer", "performance", Severity::Warning, 12)]);
    let aggregation = aggregate_rules(20_000, 1, &diagnostics);
    assert_eq!(rule(&aggregation, "clippy::rc_buffer").sites.total(), 12.0);
}

#[test]
fn a_lone_workspace_finding_is_charged_the_one_site_core_v6_charged() {
    let measured = scored(&[(
        "rust_doctor::cargo::unchecked_release_overflow",
        "dependencies",
        Severity::Warning,
        1,
    )]);
    let noise = crate::policy::corpus_measurement("rust_doctor::cargo::unchecked_release_overflow")
        .map(crate::policy::CorpusMeasurement::noise_basis_points)
        .expect("the rule is measured");
    let core_v6 = 1.0 * (1.0 - f64::from(noise) / 10_000.0);
    assert_eq!(
        measured.dimensions.dependencies,
        dimension_score(core_v6, ScoreDimension::Dependencies)
    );

    let duplicate = scored(&[(
        "rust_doctor::cargo::duplicate_major_versions",
        "dependencies",
        Severity::Warning,
        1,
    )]);
    let aggregation = aggregate_rules(100, 1, &diagnostics_for(&[(
        "rust_doctor::cargo::duplicate_major_versions",
        "dependencies",
        Severity::Warning,
        1,
    )]));
    assert_eq!(
        rule(&aggregation, "rust_doctor::cargo::duplicate_major_versions").sites.total(),
        1.0,
        "weight(1) is 1, so the curve leaves a lone finding where core-v6 charged it"
    );
    assert_eq!(
        duplicate.dimensions.dependencies, 75,
        "the unmeasured prior halves the core-v6 charge, so the P2 ceiling binds"
    );
}

#[test]
fn one_variable_in_three_workflows_weighs_what_one_workflow_weighs() {
    let three = Audit::build(1, 100, Status::Complete, &with_causes(BUILD_STEP, "security", &[(Some("ANALYTICS_KEY"), 3)]))
        .score
        .expect("a scored audit");
    let one = Audit::build(1, 100, Status::Complete, &with_causes(BUILD_STEP, "security", &[(Some("ANALYTICS_KEY"), 1)]))
        .score
        .expect("a scored audit");
    assert_eq!(three.dimensions.security, one.dimensions.security);
    assert_eq!(three.value, one.value);

    let distinct = Audit::build(
        1,
        100,
        Status::Complete,
        &with_causes(BUILD_STEP, "security", &[(Some("A"), 1), (Some("B"), 1), (Some("C"), 1)]),
    )
    .score
    .expect("a scored audit");
    let expected = decayed((1.0 + 3f64.ln()) * 4.0 * 0.5, ScoreDimension::Security).round();
    assert_eq!(f64::from(distinct.dimensions.security), expected.min(50.0));
}

#[test]
fn an_unmeasured_p1_workspace_finding_is_held_at_its_full_tier() {
    let score = scored(&[(BUILD_STEP, "security", Severity::Warning, 1)]);
    assert_eq!(
        decayed(4.0 * 0.5, ScoreDimension::Security).round(),
        61.0,
        "the prior lifts a lone P1 site from 37 to 61 before the ceiling"
    );
    assert!(score.dimensions.security <= 50, "{:?}", score.dimensions);
    assert_eq!(score.dimensions.security, 50);
    assert!(score.value <= 65, "{}", score.value);
    assert_eq!(score.worst_tier, Some(RuleTier::P1));
}
