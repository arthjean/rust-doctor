use serde::{Deserialize, Serialize};

use super::interval::{NOMINAL_COVERAGE_SITES, difference_95};
use super::{PrecisionStatus, RulePrecision};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RateComparison {
    pub(crate) agent_basis_points: u64,
    pub(crate) below_nominal_coverage: bool,
    pub(crate) difference_high_basis_points: i64,
    pub(crate) difference_low_basis_points: i64,
    pub(crate) difference_method: DifferenceMethod,
    pub(crate) gap_basis_points: i64,
    pub(crate) healthy_basis_points: u64,
    pub(crate) rule: String,
    pub(crate) verdict: ComparisonVerdict,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DifferenceMethod {
    #[serde(rename = "newcombe_hybrid_score_95")]
    NewcombeHybridScore95,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ComparisonVerdict {
    Indistinguishable,
    Separated,
}

pub(crate) fn verdict_of(low: i64, high: i64) -> ComparisonVerdict {
    if low > 0 || high < 0 {
        ComparisonVerdict::Separated
    } else {
        ComparisonVerdict::Indistinguishable
    }
}

pub(crate) fn measured_rules(precision: &[RulePrecision]) -> u64 {
    precision
        .iter()
        .filter(|rule| rule.status == PrecisionStatus::Measured)
        .count() as u64
}

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

fn rate_of(measure: &RulePrecision) -> Option<u64> {
    if measure.status == PrecisionStatus::Measured {
        measure.false_positive_rate_basis_points
    } else {
        None
    }
}

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
