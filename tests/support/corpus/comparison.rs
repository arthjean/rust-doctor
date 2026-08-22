//! What the two populations say about the same rule.
//!
//! A rate on healthy code and a rate on agent-written code are two answers to
//! two different questions, and the record has published them side by side
//! since the agent population existed. Side by side is not the same as
//! compared: the distance between them is the whole reason both are measured,
//! and until now that distance was a subtraction each reader performed alone,
//! over sixty-two rows, against two lists that do not carry the same rules.
//!
//! So the record publishes the subtraction. One row per rule measured on both
//! populations, carrying the two rates and the signed gap between them, and
//! the count of rules the agent population has a rate for at all, which is the
//! number that says how much of the structural catalog is still ranked on a
//! healthy-code estimate.
//!
//! Both are recomputed from the precision lists rather than written by hand,
//! for the reason every other figure of this record is: a number nobody
//! recomputes is a number that survives the measurement it described.

use serde::{Deserialize, Serialize};

use super::interval::{NOMINAL_COVERAGE_SITES, difference_95};
use super::{PrecisionStatus, RulePrecision};

/// One rule's rate on both populations, and the distance between them.
///
/// Published only for a rule measured on both sides. A rule measured on one is
/// not a gap of the rate's own size against zero: the other population never
/// observed it, and an absence is not a rate of nought. `measured_rules` is
/// what says how many rules that is.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RateComparison {
    pub(crate) agent_basis_points: u64,
    /// Whether either arm holds fewer sites than `NOMINAL_COVERAGE_SITES`.
    ///
    /// Published on the row rather than left to the reader to derive from the
    /// two precision blocks, because the fact qualifies this interval and a
    /// qualification a reader has to go and assemble is a qualification most
    /// readers never see.
    pub(crate) below_nominal_coverage: bool,
    /// The upper bound of the difference interval, in signed basis points.
    pub(crate) difference_high_basis_points: i64,
    /// The lower bound of the difference interval, in signed basis points.
    pub(crate) difference_low_basis_points: i64,
    /// The method the two bounds come from, named beside them.
    ///
    /// A closed vocabulary rather than free text, and on the row rather than
    /// once for the block, because an interval whose method is not published is
    /// a pair of numbers a reader cannot check: three methods in common use
    /// give three different answers on these samples, and two of them give an
    /// answer that reaches outside the scale.
    pub(crate) difference_method: DifferenceMethod,
    /// Agent rate minus healthy rate, in basis points. Signed, because the
    /// sign is the finding: a rule wrong more often on the code this tool
    /// exists for than on the code it was calibrated against is the case the
    /// comparison exists to make visible.
    pub(crate) gap_basis_points: i64,
    pub(crate) healthy_basis_points: u64,
    pub(crate) rule: String,
    /// What the difference interval settles about the two populations.
    pub(crate) verdict: ComparisonVerdict,
}

/// The interval the two bounds of a row were computed with.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DifferenceMethod {
    /// Newcombe's method 10, the hybrid score interval, at 95 %.
    ///
    /// Spelled out rather than left to `rename_all`, which renders the trailing
    /// digits as `score95`: the record names the confidence level the way
    /// `wilson_95` does, separated from the method it qualifies.
    #[serde(rename = "newcombe_hybrid_score_95")]
    NewcombeHybridScore95,
}

/// What a difference interval settles about the two populations of one rule.
///
/// Both comparisons are strict and are stated once, in `verdict_of`. An
/// interval whose bound lands exactly on zero contains zero: the sample places
/// the two populations at the same rate rather than apart, which is the same
/// strictness `separation` applies against the gate threshold.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ComparisonVerdict {
    /// The interval contains zero: the two samples cannot be told apart.
    Indistinguishable,
    /// The interval excludes zero: the two populations differ on this rule.
    Separated,
}

/// What an interval excluding zero settles, stated once.
pub(crate) fn verdict_of(low: i64, high: i64) -> ComparisonVerdict {
    if low > 0 || high < 0 {
        ComparisonVerdict::Separated
    } else {
        ComparisonVerdict::Indistinguishable
    }
}

/// Rules of one population carrying a published rate.
pub(crate) fn measured_rules(precision: &[RulePrecision]) -> u64 {
    precision
        .iter()
        .filter(|rule| rule.status == PrecisionStatus::Measured)
        .count() as u64
}

/// One comparison row per rule measured on both populations, in the order the
/// agent precision list publishes, which is the catalog's.
pub(crate) fn rate_comparison(
    healthy: &[RulePrecision],
    agent: &[RulePrecision],
) -> Vec<RateComparison> {
    agent
        .iter()
        .filter_map(|measure| {
            let agent_rate = rate_of(measure)?;
            let counterpart = healthy.iter().find(|rule| rule.id == measure.id)?;
            let healthy_rate = rate_of(counterpart)?;
            // Healthy first, agent second, so the interval is an interval for
            // agent minus healthy: the same subtraction, in the same order, as
            // the gap published beside it.
            let (low, high) = difference_95(
                (counterpart.false_positives?, counterpart.reviewed),
                (measure.false_positives?, measure.reviewed),
            );
            Some(RateComparison {
                agent_basis_points: agent_rate,
                below_nominal_coverage: counterpart.reviewed < NOMINAL_COVERAGE_SITES
                    || measure.reviewed < NOMINAL_COVERAGE_SITES,
                difference_high_basis_points: high,
                difference_low_basis_points: low,
                difference_method: DifferenceMethod::NewcombeHybridScore95,
                gap_basis_points: i64::try_from(agent_rate).unwrap_or(i64::MAX)
                    - i64::try_from(healthy_rate).unwrap_or(i64::MAX),
                healthy_basis_points: healthy_rate,
                rule: measure.id.clone(),
                verdict: verdict_of(low, high),
            })
        })
        .collect()
}

/// The rate a measured row publishes, and nothing from a row that publishes
/// none: a withheld rate read as zero is the one arithmetic this block must
/// never perform.
fn rate_of(measure: &RulePrecision) -> Option<u64> {
    if measure.status == PrecisionStatus::Measured {
        measure.false_positive_rate_basis_points
    } else {
        None
    }
}

/// Closed defects of the published comparison, each naming the rule and the
/// field.
///
/// Field by field rather than one comparison of the whole list, for the reason
/// `interval_defects` is written that way: a reader handed "the comparison
/// block disagrees" has to diff the rows by hand to find the number that
/// moved, and the number that moved is the finding.
pub(crate) fn comparison_defects(
    healthy: &[RulePrecision],
    agent: &[RulePrecision],
    published_measured_rules: u64,
    published_comparison: &[RateComparison],
) -> Vec<String> {
    let mut defects = Vec::new();

    let expected_count = measured_rules(agent);
    if published_measured_rules != expected_count {
        defects.push(format!(
            "the agent population publishes measured_rules {published_measured_rules} \
             against {expected_count} rules carrying a rate"
        ));
    }

    let expected = rate_comparison(healthy, agent);
    if published_comparison.len() != expected.len() {
        defects.push(format!(
            "the agent population publishes {} comparison rows against {} recomputed",
            published_comparison.len(),
            expected.len()
        ));
    }
    for (published, recomputed) in published_comparison.iter().zip(expected.iter()) {
        let id = recomputed.rule.as_str();
        if published.rule != recomputed.rule {
            defects.push(format!(
                "the comparison publishes {} where {id} is recomputed",
                published.rule
            ));
            continue;
        }
        if published.agent_basis_points != recomputed.agent_basis_points {
            defects.push(format!(
                "{id} publishes an agent rate of {} against {} recomputed",
                published.agent_basis_points, recomputed.agent_basis_points
            ));
        }
        if published.healthy_basis_points != recomputed.healthy_basis_points {
            defects.push(format!(
                "{id} publishes a healthy rate of {} against {} recomputed",
                published.healthy_basis_points, recomputed.healthy_basis_points
            ));
        }
        if published.gap_basis_points != recomputed.gap_basis_points {
            defects.push(format!(
                "{id} publishes a gap of {} against {} recomputed",
                published.gap_basis_points, recomputed.gap_basis_points
            ));
        }
        if published.difference_low_basis_points != recomputed.difference_low_basis_points {
            defects.push(format!(
                "{id} publishes difference_low_basis_points {} against {} recomputed",
                published.difference_low_basis_points, recomputed.difference_low_basis_points
            ));
        }
        if published.difference_high_basis_points != recomputed.difference_high_basis_points {
            defects.push(format!(
                "{id} publishes difference_high_basis_points {} against {} recomputed",
                published.difference_high_basis_points, recomputed.difference_high_basis_points
            ));
        }
        if published.difference_method != recomputed.difference_method {
            defects.push(format!(
                "{id} publishes difference_method {:?} against {:?} recomputed",
                published.difference_method, recomputed.difference_method
            ));
        }
        if published.below_nominal_coverage != recomputed.below_nominal_coverage {
            defects.push(format!(
                "{id} publishes below_nominal_coverage {} against {} recomputed",
                published.below_nominal_coverage, recomputed.below_nominal_coverage
            ));
        }
        // Recomputed from the published bounds rather than from the recomputed
        // ones, so a verdict edited to disagree with the interval printed above
        // it is named here even where the two bounds themselves are honest.
        let expected_verdict = verdict_of(
            published.difference_low_basis_points,
            published.difference_high_basis_points,
        );
        if published.verdict != expected_verdict {
            defects.push(format!(
                "{id} publishes verdict {:?} against {expected_verdict:?} its own interval settles",
                published.verdict
            ));
        }
        if published.difference_low_basis_points > published.difference_high_basis_points {
            defects.push(format!(
                "{id} publishes a difference interval [{}, {}] whose bounds are inverted",
                published.difference_low_basis_points, published.difference_high_basis_points
            ));
        }
        if !(published.difference_low_basis_points..=published.difference_high_basis_points)
            .contains(&published.gap_basis_points)
        {
            defects.push(format!(
                "{id} publishes a gap of {} outside its own difference interval [{}, {}]",
                published.gap_basis_points,
                published.difference_low_basis_points,
                published.difference_high_basis_points
            ));
        }
    }

    defects
}
