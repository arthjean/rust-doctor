use serde::{Deserialize, Serialize};

use super::RulePrecision;

const Z_95: f64 = 1.959_963_985;

const WHOLE: u64 = 10_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Separation {
    Above,
    Below,
    Indecisive,
}

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

pub(crate) const NOMINAL_COVERAGE_SITES: u64 = 40;

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

pub(crate) fn rate_basis_points(false_positives: u64, reviewed: u64) -> u64 {
    if reviewed == 0 {
        return 0;
    }
    false_positives.saturating_mul(WHOLE) / reviewed
}

pub(crate) fn separation(low: u64, high: u64, threshold: u64) -> Separation {
    if low > threshold {
        Separation::Above
    } else if high < threshold {
        Separation::Below
    } else {
        Separation::Indecisive
    }
}

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

fn signed_basis_points(value: f64) -> i64 {
    let whole = WHOLE as f64;
    let rounded = value.clamp(-whole, whole).round();
    rounded as i64
}

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
