//! What the report tells the reader to repair first.
//!
//! The ranking is the whole subject: the points a repair gives back through the ceilings the
//! published value takes, discounted by how often the pinned corpus adjudicated the rule wrong,
//! and the rules that discount withholds. It has a file of its own so that every file of the
//! module stays under the thousand lines `oversized_unit` reports, which is what
//! `the_audit_holds_the_size_bound_it_scores_for` keeps true.
//!
//! Every case here is written against a real catalogued rule and the rate the shipped table
//! carries for it, except the ones that put two samples of the same rate in competition: those
//! are the one question no pair of real rules asks, since no two catalogued rules share a rate
//! across two sample sizes.

use crate::audit::{BASIS_POINTS, DensityScope, RuleAggregate, ScoreDimension};
use crate::policy::{RuleTier, UNMEASURED_NOISE_BASIS_POINTS};
use crate::report::Severity;

use super::{diagnostics_for, scored};
use crate::audit::{Audit, AuditScore};
use crate::report::Status;

/// The same profile over twenty kilolines, the size at which a handful of sites is a density
/// rather than a catastrophe: on the hundred-line floor sixty sites saturate every dimension and
/// the ceiling is not what holds the score.
fn scored_over_twenty_kilolines(rules: &[(&str, &str, Severity, usize)]) -> AuditScore {
    Audit::build(20, 20_000, Status::Complete, &diagnostics_for(rules))
        .score
        .expect("twenty kilolines is a scorable workspace")
}

/// One aggregate carrying nothing but what the ranking key reads.
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
        noise,
        contribution,
    }
}

/// The rule that fires most is not the rule worth fixing first.
///
/// `clippy::indexing_slicing` is adjudicated wrong on all forty sites the pinned corpus reviewed,
/// while `rust_doctor::cargo::duplicate_major_versions` was right on the one it showed. Ranking
/// by contribution alone put the noisy rule first because volume is exactly what it has the most
/// of, which is advice to go and change correct code.
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

/// An unmeasured rule is ranked as unknown, not as perfect.
///
/// It keeps half its contribution, which is the same smoothing read at no observations. Under the
/// raw rate it kept all of it, so the 38 rules the corpus has never adjudicated all ranked ahead
/// of every rule it ever confirmed, and withholding a rate promoted the rule instead of demoting
/// it.
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

/// The same rate on more sites is a stronger claim, and the key says so.
///
/// Five sites all adjudicated wrong and forty sites all adjudicated wrong are the same raw rate
/// and not the same evidence. The smoothing is what separates them: both sit above the
/// unmeasured middle, and the larger sample sits further from it, in the direction its data
/// indicates.
#[test]
fn the_same_rate_on_more_sites_moves_further_from_the_default() {
    let five = ranked("clippy::panic", 1_000_000, Some(8571));
    let forty = ranked("clippy::indexing_slicing", 1_000_000, Some(9762));
    let unmeasured = ranked("rust_doctor::repo::tracked_secret_file", 1_000_000, None);

    assert!(forty.expected_repair_value() < five.expected_repair_value());
    assert!(five.expected_repair_value() < unmeasured.expected_repair_value());
}

/// A rule the corpus found wrong more often than right is withheld, and still named.
///
/// The threshold is the rate, not the value left after the discount: under the value no rule
/// was ever withheld, since no smoothed rate reaches ten thousand and a millionth of a point
/// survives any discount. Two such rules are published loudest first, so the absence from the
/// projection reads as a measurement and not as a defect.
#[test]
fn a_rule_wrong_more_often_than_right_is_withheld_and_still_named() {
    let worst = ranked("clippy::panic", 1_000_000, Some(8571));
    assert!(worst.is_withheld());
    assert_eq!(
        worst.expected_repair_value(),
        1_000_000 * (BASIS_POINTS - 8571) / BASIS_POINTS,
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

/// The rule holding the ceiling is named first, whatever the others' volume.
///
/// One `P1` site holds the score at 65. Read from the density relief alone, three `P3` rules
/// with dozens of sites each led the projection and the projection promised the 65 the reader
/// already had. The marginal gain through the ceilings is what puts the `P1` rule first, and
/// the promise moves.
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

/// Two rules sharing the ceiling are both named: the first lifts nothing alone, the second lifts
/// it on top of the first.
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

/// The rate discounts the charge of a measured rule, and never invents one.
///
/// Two workspaces differing only in which rule fired, both rules of the same tier and dimension,
/// no longer score the same: the one the corpus found wrong on forty sites out of forty costs a
/// fortieth of the one it found wrong on five out of five. Neither is free, since the smoothing
/// keeps a share of every measured rule, and the sample is printed wherever the rule is named.
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
    assert_eq!(
        ranked("clippy::indexing_slicing", 1_000_000, Some(9762)).expected_repair_value(),
        23_800
    );
    assert_eq!(
        ranked("clippy::unreachable", 1_000_000, Some(7143)).expected_repair_value(),
        285_700
    );
}

/// An unmeasured rule is charged whole: a measurement can only lower a charge.
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
