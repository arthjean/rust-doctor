use crate::audit::{DensityScope, RuleAggregate, ScoreDimension};
use crate::policy::{RuleTier, UNMEASURED_NOISE_BASIS_POINTS};
use crate::report::Severity;

use super::{diagnostics_for, scored};
use crate::audit::{Audit, AuditScore};
use crate::report::Status;

fn scored_over_twenty_kilolines(rules: &[(&str, &str, Severity, usize)]) -> AuditScore {
    Audit::build(20, 20_000, Status::Complete, &diagnostics_for(rules))
        .score
        .expect("twenty kilolines is a scorable workspace")
}

fn ranked(id: &str, contribution: u64, noise: Option<u16>) -> RuleAggregate {
    RuleAggregate {
        id: id.to_owned(),
        effective_severity: Severity::Warning,
        category: None,
        dimension: Some(ScoreDimension::Reliability),
        scope: DensityScope::Workspace,
        tier: Some(RuleTier::P3),
        occurrences: 1,
        numerator: 1,
        owned: 0,
        noise,
        contribution,
    }
}

#[test]
fn a_rule_the_corpus_measured_wrong_yields_the_lead_to_a_quieter_one() {
    let score = scored(&[
        ("clippy::indexing_slicing", "reliability", Severity::Warning, 60),
        (
            "rust_doctor::cargo::duplicate_major_versions",
            "dependencies",
            Severity::Warning,
            2,
        ),
    ]);

    assert_eq!(
        score.projected_rule_ids,
        vec!["rust_doctor::cargo::duplicate_major_versions".to_owned()],
        "the rule measured wrong on forty sites does not lead over the one measured right"
    );
    assert_eq!(
        score.withheld_rule_ids,
        vec!["clippy::indexing_slicing".to_owned()]
    );
}

#[test]
fn an_unmeasured_rule_is_ranked_at_half_its_contribution() {
    let unmeasured = ranked("rust_doctor::repo::tracked_secret_file", 1_000_000, None);
    assert_eq!(unmeasured.expected_repair_value(), 500_000);
    assert_eq!(UNMEASURED_NOISE_BASIS_POINTS, 5000);
    assert!(!unmeasured.is_withheld(), "the middle is ranked, not withheld");

    let quiet = ranked("clippy::rc_buffer", 1_000_000, Some(2000));
    assert!(
        quiet.expected_repair_value() > unmeasured.expected_repair_value(),
        "a rule measured quiet ranks above one nobody has measured"
    );
}

#[test]
fn the_same_rate_on_more_sites_moves_further_from_the_default() {
    let mut five = ranked("clippy::panic", 0, Some(8571));
    let mut forty = ranked("clippy::indexing_slicing", 0, Some(9762));
    let mut unmeasured = ranked("rust_doctor::repo::tracked_secret_file", 0, None);
    for rule in [&mut five, &mut forty, &mut unmeasured] {
        rule.numerator = 10;
    }

    assert!(forty.charged() < five.charged());
    assert!(five.charged() < unmeasured.charged());
    assert_eq!(
        forty.expected_repair_value(),
        five.expected_repair_value(),
        "two measured rules of equal contribution are worth the same: the rate is already in it"
    );
}

#[test]
fn a_measured_rule_is_discounted_once_and_ranks_above_an_unmeasured_peer() {
    let score = scored(&[
        ("clippy::useless_vec", "performance", Severity::Warning, 1),
        ("clippy::ptr_arg", "performance", Severity::Warning, 1),
    ]);
    assert_eq!(
        score.projected_rule_ids,
        vec!["clippy::ptr_arg".to_owned(), "clippy::useless_vec".to_owned()]
    );

    let measured = ranked("clippy::ptr_arg", 1_000_000, Some(3333));
    let unmeasured = ranked("clippy::useless_vec", 1_000_000, None);
    assert_eq!(measured.expected_repair_value(), 1_000_000);
    assert_eq!(unmeasured.expected_repair_value(), 500_000);
}

#[test]
fn the_ceiling_a_measured_rule_holds_is_released_whole() {
    let score = scored(&[
        (
            "clippy::undocumented_unsafe_blocks",
            "correctness",
            Severity::Warning,
            1,
        ),
        ("clippy::useless_vec", "performance", Severity::Warning, 1),
    ]);
    assert_eq!(score.dimensions.reliability, 75);
    assert_eq!(
        score.projected_rule_ids.first().map(String::as_str),
        Some("clippy::undocumented_unsafe_blocks")
    );
    assert_eq!(score.projected_after_top_three, Some(100));
}

#[test]
fn a_rule_wrong_more_often_than_right_is_withheld_and_still_named() {
    let worst = ranked("clippy::panic", 1_000_000, Some(8571));
    assert!(worst.is_withheld());
    assert_eq!(
        worst.expected_repair_value(),
        1_000_000,
        "withholding does not zero the value, it declines to rank it"
    );
    let at_the_middle = ranked("clippy::todo", 1_000_000, Some(5000));
    assert!(!at_the_middle.is_withheld(), "the threshold is strict");

    let score = scored(&[
        ("clippy::string_slice", "reliability", Severity::Warning, 12),
        ("clippy::indexing_slicing", "reliability", Severity::Warning, 60),
    ]);
    assert!(score.projected_rule_ids.is_empty());
    assert_eq!(score.projected_after_top_three, None);
    assert_eq!(
        score.withheld_rule_ids,
        vec![
            "clippy::indexing_slicing".to_owned(),
            "clippy::string_slice".to_owned()
        ],
        "two withheld rules are named loudest first"
    );
}

#[test]
fn the_rule_holding_the_ceiling_is_named_first() {
    let score = scored_over_twenty_kilolines(&[
        ("clippy::await_holding_lock", "correctness", Severity::Warning, 1),
        ("clippy::todo", "correctness", Severity::Warning, 30),
        ("clippy::dbg_macro", "maintainability", Severity::Warning, 20),
        ("clippy::useless_vec", "performance", Severity::Warning, 15),
    ]);

    assert_eq!(score.applied_ceiling, Some(65));
    assert_eq!(score.value, 65);
    assert_eq!(
        score.projected_rule_ids.first().map(String::as_str),
        Some("clippy::await_holding_lock")
    );
    assert_eq!(score.projected_rule_ids.len(), 3);
    let projected = score
        .projected_after_top_three
        .expect("an authoritative score with a projection names its value");
    assert!(projected > 65, "the promise moves once the ceiling is named: {projected}");
}

#[test]
fn two_rules_sharing_a_ceiling_are_both_named() {
    let score = scored_over_twenty_kilolines(&[
        ("clippy::await_holding_lock", "correctness", Severity::Warning, 1),
        ("clippy::unimplemented", "correctness", Severity::Warning, 1),
        ("clippy::dbg_macro", "maintainability", Severity::Warning, 40),
        ("clippy::useless_vec", "performance", Severity::Warning, 40),
        ("clippy::todo", "correctness", Severity::Warning, 40),
    ]);

    assert_eq!(score.applied_ceiling, Some(65));
    let named: Vec<&str> = score.projected_rule_ids.iter().map(String::as_str).collect();
    assert!(named.contains(&"clippy::await_holding_lock"), "{named:?}");
    assert!(named.contains(&"clippy::unimplemented"), "{named:?}");
    assert!(
        score.projected_after_top_three.is_some_and(|value| value > 65),
        "{:?}",
        score.projected_after_top_three
    );
}

#[test]
fn the_rate_discounts_the_charge_of_a_measured_rule() {
    let forty = scored(&[(
        "clippy::indexing_slicing",
        "reliability",
        Severity::Warning,
        12,
    )]);
    let five = scored(&[("clippy::unwrap_used", "reliability", Severity::Warning, 12)]);
    let clean = scored(&[]);

    assert!(forty.value > five.value, "{} vs {}", forty.value, five.value);
    assert!(five.value < clean.value);
    assert_eq!(clean.value, 100);
    let mut indexing = ranked("clippy::indexing_slicing", 0, Some(9762));
    indexing.numerator = 10_000;
    assert_eq!(indexing.charged(), 238.0);
    let mut unreachable = ranked("clippy::unreachable", 0, Some(7143));
    unreachable.numerator = 10_000;
    assert_eq!(unreachable.charged(), 2857.0);
}

#[test]
fn an_unmeasured_rule_is_charged_whole() {
    let mut rule = ranked("rust_doctor::repo::tracked_secret_file", 0, None);
    rule.numerator = 3;
    assert_eq!(rule.charged(), 3.0);
    rule.tier = Some(RuleTier::P1);
    assert_eq!(rule.charged(), 12.0, "a P1 site weighs four P3 sites");
    rule.noise = Some(2500);
    assert_eq!(rule.charged(), 9.0, "a measured rule keeps the share the corpus confirmed");
}
