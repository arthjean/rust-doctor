//! What the report tells the reader to repair first.
//!
//! The ranking key is the whole subject: what a rule costs the score, discounted by how often
//! the pinned corpus adjudicated it wrong. It has a file of its own so that every file of the
//! module stays under the thousand lines `oversized_unit` reports, which is what
//! `the_audit_holds_the_size_bound_it_scores_for` keeps true.
//!
//! Every case here is written against a real catalogued rule and the rate the shipped table
//! carries for it, except the three that put two samples of the same rate in competition: those
//! are the one question no pair of real rules asks, since no two catalogued rules share a rate
//! across two sample sizes.

use crate::audit::{BASIS_POINTS, DensityScope, RuleAggregate, ScoreDimension};
use crate::policy::UNMEASURED_NOISE_BASIS_POINTS;
use crate::report::Severity;

use super::scored;

/// One aggregate carrying nothing but what the ranking key reads.
fn ranked(id: &str, contribution: u64, noise: Option<u16>) -> RuleAggregate {
    RuleAggregate {
        id: id.to_owned(),
        effective_severity: Severity::Warning,
        category: None,
        dimension: Some(ScoreDimension::Reliability),
        scope: DensityScope::Workspace,
        tier: None,
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
        score.projected_rule_ids.first().map(String::as_str),
        Some("rust_doctor::cargo::duplicate_major_versions"),
        "the rule measured wrong on forty sites does not lead over the one measured right"
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

/// No rule ties at zero for want of resolution.
///
/// The raw rate collapsed twelve rules onto exactly 10000 basis points and every one of them to
/// an expected repair value of zero, which is a tie the reader cannot break and the ranking
/// cannot order. No smoothed rate reaches 10000, so the worst-measured rule of the catalog still
/// carries something.
#[test]
fn a_rule_measured_wrong_on_every_site_still_ranks_above_nothing() {
    let worst = ranked("clippy::panic", 1_000_000, Some(8571));
    assert!(worst.expected_repair_value() > 0);
    assert_eq!(
        worst.expected_repair_value(),
        1_000_000 * (BASIS_POINTS - 8571) / BASIS_POINTS
    );

    let score = scored(&[
        ("clippy::indexing_slicing", "reliability", Severity::Warning, 60),
        ("clippy::string_slice", "reliability", Severity::Warning, 12),
    ]);
    assert_eq!(
        score.projected_rule_ids,
        vec![
            "clippy::indexing_slicing".to_owned(),
            "clippy::string_slice".to_owned()
        ],
        "two rules measured wrong everywhere are ordered rather than tied away"
    );
    assert!(score.withheld_rule_ids.is_empty());
}

/// What the ranking drops is still named, loudest first.
///
/// A rule reaches the withheld list by having nothing left after the discount, which under the
/// smoothed rate means a contribution small enough that the surviving share rounds away rather
/// than a rate of exactly ten thousand. It is published all the same, so the absence reads as a
/// measurement and not as a defect.
#[test]
fn the_rules_the_ranking_dropped_are_published_loudest_first() {
    let noisy = ranked("clippy::indexing_slicing", 40, Some(9762));
    let quieter = ranked("clippy::string_slice", 30, Some(9762));
    assert_eq!(noisy.expected_repair_value(), 0);
    assert_eq!(quieter.expected_repair_value(), 0);
    assert!(noisy.contribution() > quieter.contribution());

    let rules = [quieter, noisy];
    let (projected, withheld) = crate::audit::rank_repairs(&rules);
    assert!(projected.is_empty());
    assert_eq!(
        withheld
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>(),
        vec!["clippy::indexing_slicing", "clippy::string_slice"]
    );
}

/// The discount ranks and never penalizes.
///
/// Two workspaces differing only in which rule fired score the same when the two rules cost the
/// same, whatever the corpus adjudicated for either. That is the whole separation between
/// `contribution`, which the score charges, and `expected_repair_value`, which orders the advice.
#[test]
fn the_rate_orders_the_advice_and_never_moves_the_score() {
    let noisy = scored(&[(
        "clippy::indexing_slicing",
        "reliability",
        Severity::Warning,
        12,
    )]);
    let quiet = scored(&[("clippy::unreachable", "reliability", Severity::Warning, 12)]);

    assert_eq!(noisy.value, quiet.value);
    assert_eq!(noisy.dimensions, quiet.dimensions);
    assert_eq!(
        ranked("clippy::indexing_slicing", 1_000_000, Some(9762)).expected_repair_value(),
        23_800
    );
    assert_eq!(
        ranked("clippy::unreachable", 1_000_000, Some(7143)).expected_repair_value(),
        285_700
    );
}
