//! What a rate is worth.
//!
//! A rate from one site and a rate from forty read identically until the
//! interval they rest on is published beside them. Twenty of the twenty-four
//! healthy rates rest on five sites or fewer, and the only positive statement
//! the gate makes rests entirely on the five rates whose sample cannot support
//! it: at x = 0, n = 1 the sample is compatible with a rule that is wrong four
//! times in five. The interval is what says so, and the separation is what the
//! interval settles against the published threshold.
//!
//! Every bound is an integer basis point. No float reaches the artifact: the
//! record derives `Eq`, and a bound stored as a float is a record two machines
//! can disagree on.

use serde::{Deserialize, Serialize};

use super::RulePrecision;

/// Two-sided standard normal quantile at 95 %.
const Z_95: f64 = 1.959_963_985;

/// One whole, in basis points.
const WHOLE: u64 = 10_000;

/// What an interval settles against the gate threshold.
///
/// The comparison is strict on both sides, so a bound landing exactly on the
/// threshold settles nothing: a rule the interval touches at 5 % is a rule the
/// sample places at the threshold, not under it and not past it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Separation {
    /// The whole interval sits past the threshold: the rule is noisy, measured.
    Above,
    /// The whole interval sits under it: the rule is clean, measured.
    Below,
    /// The interval spans the threshold: the sample settles nothing either way.
    Indecisive,
}

/// The Wilson score interval at 95 %, in integer basis points.
///
/// Wilson rather than Wald, which returns `[0, 0]` at x = 0 and is therefore
/// not merely imprecise but false at three of the five samples the gate
/// currently clears; and rather than Clopper-Pearson, which is exact and
/// conservative but needs the incomplete beta function, a dependency this crate
/// has no other use for. IEEE-754 mandates correctly rounded arithmetic and
/// square root, so the f64 computation rounded to basis points is byte-stable
/// across runs and platforms, which the reproducibility of the report requires.
///
/// A sample with no denominator bounds nothing, so it yields the whole scale
/// rather than the point `[0, 0]` a Wald computation would publish there.
pub(crate) fn wilson_95(false_positives: u64, reviewed: u64) -> (u64, u64) {
    if reviewed == 0 || false_positives > reviewed {
        return (0, WHOLE);
    }
    let n = reviewed as f64;
    let p = false_positives as f64 / n;
    let z_squared = Z_95 * Z_95;
    let denominator = 1.0 + z_squared / n;
    let center = (p + z_squared / (2.0 * n)) / denominator;
    let margin =
        Z_95 * (p * (1.0 - p) / n + z_squared / (4.0 * n * n)).sqrt() / denominator;
    (
        basis_points(center - margin),
        basis_points(center + margin),
    )
}

/// What the interval settles against a threshold, both comparisons strict.
pub(crate) fn separation(low: u64, high: u64, threshold: u64) -> Separation {
    if low > threshold {
        Separation::Above
    } else if high < threshold {
        Separation::Below
    } else {
        Separation::Indecisive
    }
}

/// A proportion in integer basis points, rounded half away from zero and
/// clamped to the unit interval.
///
/// Stated once and read from here rather than spelled at each call site: two
/// bounds rounded two ways is how an interval ends up wider on one side than
/// the arithmetic that produced it.
fn basis_points(value: f64) -> u64 {
    let clamped = value.clamp(0.0, 1.0);
    let scaled = (clamped * WHOLE as f64).round();
    if scaled <= 0.0 {
        0
    } else if scaled >= WHOLE as f64 {
        WHOLE
    } else {
        scaled as u64
    }
}

/// Closed defects of the published intervals, each naming the rule and the
/// field.
///
/// Field by field rather than one comparison of the whole row, because a reader
/// handed "the precision block disagrees" has to diff sixty-two rows by hand to
/// find the one number that moved, and the number that moved is the finding.
/// The presence coupling is checked in both directions: a rate with no bounds
/// is a rate nobody can weigh, and bounds with no rate are bounds around
/// nothing.
pub(crate) fn interval_defects(precision: &[RulePrecision], threshold: u64) -> Vec<String> {
    let mut defects = Vec::new();
    for rule in precision {
        let id = rule.id.as_str();
        let published = (
            rule.interval_low_basis_points,
            rule.interval_high_basis_points,
            rule.separation,
        );
        let Some(rate) = rule.false_positive_rate_basis_points else {
            if published != (None, None, None) {
                defects.push(format!("{id} publishes bounds with no rate"));
            }
            continue;
        };
        let Some(false_positives) = rule.false_positives else {
            defects.push(format!("{id} publishes a rate with no false positive count"));
            continue;
        };
        let (low, high) = wilson_95(false_positives, rule.reviewed);
        let expected = (Some(low), Some(high), Some(separation(low, high, threshold)));
        if published.0 != expected.0 {
            defects.push(format!(
                "{id} publishes interval_low_basis_points {:?} against {low} recomputed",
                published.0
            ));
        }
        if published.1 != expected.1 {
            defects.push(format!(
                "{id} publishes interval_high_basis_points {:?} against {high} recomputed",
                published.1
            ));
        }
        if published.2 != expected.2 {
            defects.push(format!(
                "{id} publishes separation {:?} against {:?} recomputed",
                published.2, expected.2
            ));
        }
        if !(low..=high).contains(&rate) {
            defects.push(format!(
                "{id} publishes a rate of {rate} outside its own interval [{low}, {high}]"
            ));
        }
    }
    defects
}
