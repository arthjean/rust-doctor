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

/// Sites per arm below which the difference interval says so.
///
/// The hybrid score interval keeps close to its nominal coverage far further
/// down than the Wald interval it replaces, which is the reason it is the
/// method here, but its coverage still dips under nominal on small arms
/// (Newcombe 1998, "Interval estimation for the difference between independent
/// proportions", Statistics in Medicine 17(8); Fagerland, Lydersen and Laake
/// 2011, "Recommended confidence intervals for two independent binomial
/// proportions", Statistical Methods in Medical Research 24(2)). Forty is the
/// floor this record adopts rather than a figure either paper states: what the
/// papers settle is that the dip exists and is a function of the arm size, and
/// what a record has to do with that is name a line and say on which side of it
/// each row sits.
///
/// The line is published, never enforced. A row below it carries the fact and
/// its verdict both: withholding the comparison of the scopes whose arms are
/// small would withhold it exactly where the reader most needs to know how
/// little the sample settles.
pub(crate) const NOMINAL_COVERAGE_SITES: u64 = 40;

/// The Newcombe hybrid score interval for `second - first`, in basis points.
///
/// Built from the two Wilson intervals rather than from a normal approximation
/// of the difference, which is the whole of Newcombe's method 10: the Wald
/// interval for a difference is symmetric around a point estimate and can reach
/// past the unit interval or collapse to a point at x = 0, and both failures
/// land on exactly the samples this record has. The two arms' own bounds are
/// what the slack on each side is composed from, so the difference interval
/// inherits the asymmetry the rates were measured with.
///
/// `l` and `u` are each arm's Wilson bounds and `p` its published rate:
/// the lower slack is `sqrt((p2 - l2)^2 + (u1 - p1)^2)` and the upper slack is
/// `sqrt((u2 - p2)^2 + (p1 - l1)^2)`. Derived from `wilson_95` rather than
/// recomputed here, because a second implementation of the same interval is a
/// second answer to the same question, and the record would carry both.
///
/// The published rate is the point each slack is measured from, not the Wilson
/// centre, so the interval is centred on the same gap the row publishes rather
/// than on a number appearing nowhere else in it.
pub(crate) fn difference_95(
    first: (u64, u64),
    second: (u64, u64),
) -> (i64, i64) {
    let (first_positives, first_reviewed) = first;
    let (second_positives, second_reviewed) = second;
    let p1 = rate_basis_points(first_positives, first_reviewed) as f64;
    let p2 = rate_basis_points(second_positives, second_reviewed) as f64;
    let (low_1, high_1) = wilson_95(first_positives, first_reviewed);
    let (low_2, high_2) = wilson_95(second_positives, second_reviewed);
    // Clamped at zero because a slack is a distance: the published rate sits
    // inside its own interval, which `interval_defects` refuses a record for
    // violating, and a negative term here would narrow the difference by the
    // amount that violation measures rather than by anything the sample says.
    let below_2 = (p2 - low_2 as f64).max(0.0);
    let above_1 = (high_1 as f64 - p1).max(0.0);
    let above_2 = (high_2 as f64 - p2).max(0.0);
    let below_1 = (p1 - low_1 as f64).max(0.0);
    let gap = p2 - p1;
    (
        signed_basis_points(gap - below_2.hypot(above_1)),
        signed_basis_points(gap + above_2.hypot(below_1)),
    )
}

/// A rate in integer basis points, truncated toward zero.
///
/// The one spelling of the point estimate, read both by the precision block
/// that publishes a rule's rate and by the difference interval that measures
/// its slack from that rate. Two spellings of the same division is how a
/// published gap ends up outside the interval computed beside it, which is
/// exactly the defect `comparison_defects` reports and nothing would have
/// produced.
pub(crate) fn rate_basis_points(false_positives: u64, reviewed: u64) -> u64 {
    if reviewed == 0 {
        return 0;
    }
    false_positives.saturating_mul(WHOLE) / reviewed
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

/// A signed difference in integer basis points, rounded half away from zero and
/// clamped to the whole scale in both directions.
///
/// `f64::round` is half away from zero by definition, which is the rule stated
/// once here rather than at each bound: two bounds rounded two ways is how an
/// interval ends up wider on one side than the arithmetic that produced it.
fn signed_basis_points(value: f64) -> i64 {
    let whole = WHOLE as f64;
    let rounded = value.clamp(-whole, whole).round();
    rounded as i64
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
