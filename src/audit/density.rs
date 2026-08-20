//! The core-v3 penalty: how much of a workspace a dimension's findings cover, and what that
//! coverage is worth out of a hundred.
//!
//! A dimension carries a density rather than a count: its distinct scored sites, weighted by
//! severity, over a denominator the producer that raised each finding chooses. Per-site
//! producers divide by production kilolines, so duplicating a workspace changes nothing;
//! workspace-scoped ones divide by one, because a missing lockfile is not less serious in a
//! large repository. The density then decays exponentially, which is what makes a repair
//! visible at every scale instead of only under twenty findings.
//!
//! It lives beside the block rather than inside it so that every file of the module stays under
//! the thousand lines `oversized_unit` reports.

use std::collections::BTreeSet;

use crate::policy::{Producer, RuleTier};
use crate::report::Severity;

use super::{
    RuleAggregate, ScoreDimension, ScoreDimensions, capped, dimension_weight_twice,
    tier_dimension_ceiling, worse_tier,
};

/// Smallest denominator a per-site density is ever divided by, in kilolines.
///
/// It protects the small crate: a 120-line workspace with three findings has a raw density of
/// 25 per kiloline, which decays to nothing in every dimension, and the reader is told their
/// crate is broken for holding three findings. Two kilolines is the size below which the
/// denominator stops shrinking, so a small project is scored on what it holds rather than on
/// how little of it there is.
pub(super) const KILOLINE_FLOOR: f64 = 2.0;

/// Lines in a kiloline. The denominator is a density per thousand lines, not per line, because
/// a per-line rate is a number no reader can hold.
const LINES_PER_KILOLINE: f64 = 1000.0;

/// What one distinct scored site of this severity adds to its dimension's numerator.
///
/// `Unknown` has no weight at all: a severity the catalog could not resolve is not a small
/// penalty, it is the absence of a measurement, and `aggregate_rules` drops the authoritative
/// flag over it rather than charging a guess.
pub(super) const fn severity_weight(severity: Severity) -> Option<u64> {
    match severity {
        Severity::Error => Some(2),
        Severity::Warning | Severity::Info => Some(1),
        Severity::Unknown => None,
    }
}

/// What a producer's findings are counted against.
///
/// The split is read off the catalog's own `producer` field, so a rule joins one side or the
/// other by being catalogued rather than by being listed a second time here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DensityScope {
    /// One finding per site in the source the producer read: `clippy`, the source kernel and
    /// the structural pass. Divided by production kilolines.
    PerKiloline,
    /// One finding about the workspace itself: `cargo-health` and the repository pass. Divided
    /// by one, so a large workspace does not dilute a fact that is true of it once.
    Workspace,
}

impl DensityScope {
    pub(super) const fn of(producer: Producer) -> Self {
        match producer {
            Producer::Clippy | Producer::SourceKernel | Producer::Structure => Self::PerKiloline,
            Producer::CargoHealth | Producer::Repo => Self::Workspace,
        }
    }
}

/// The denominators one scan divides its densities by.
///
/// One scan has one scale, computed from the line count the audit block publishes, so every
/// pass over the same rules divides by the same thing whatever it removed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Scale {
    kilolines: f64,
}

impl Scale {
    pub(super) fn of(production_lines: usize) -> Self {
        Self {
            kilolines: (production_lines as f64 / LINES_PER_KILOLINE).max(KILOLINE_FLOOR),
        }
    }

    /// What a finding of this scope is divided by. Never zero, and never below one, so a
    /// density is finite for any finite numerator.
    pub(super) fn divisor(self, scope: DensityScope) -> f64 {
        match scope {
            DensityScope::PerKiloline => self.kilolines,
            DensityScope::Workspace => 1.0,
        }
    }
}

/// The density that costs a dimension 63 points, in sites per denominator.
///
/// It is the whole calibration: a dimension sitting exactly at its λ scores 37, twice its λ
/// scores 14, and half of it scores 61. Security is the tightest, because one security finding
/// per two kilolines is already a workspace in trouble; reliability is the loosest, because the
/// correctness lints fire densely on code nobody would call broken. No λ may be zero, which is
/// what `no_lambda_is_zero` holds: a zero λ divides by nothing and takes every density to
/// infinity.
pub(super) const fn lambda(dimension: ScoreDimension) -> f64 {
    match dimension {
        ScoreDimension::Security => 1.0,
        ScoreDimension::Reliability => 10.0,
        ScoreDimension::Maintainability => 3.0,
        ScoreDimension::Performance => 4.0,
        ScoreDimension::Dependencies => 3.0,
    }
}

/// `100 · exp(−D / λ)`, before any rounding.
///
/// This is the one place the curve is written down. The published score rounds it to a whole
/// point per dimension, and the repair ranking reads it unrounded, because on a large workspace
/// one repaired site is worth a hundredth of a point and rounding first collapses the order into
/// ties.
///
/// A finite density over a non-zero λ cannot reach `NaN`: `exp` of a finite negative is in
/// `(0, 1]`, and `exp` of `-inf` is `0`. The clamp is what makes that a guarantee rather than an
/// argument.
pub(super) fn decayed(density: f64, dimension: ScoreDimension) -> f64 {
    let value = 100.0 * (-density / lambda(dimension)).exp();
    value.clamp(0.0, 100.0)
}

/// The dimension score the report publishes: the curve rounded to a whole point.
pub(super) fn dimension_score(density: f64, dimension: ScoreDimension) -> u8 {
    let rounded = decayed(density, dimension).round();
    // `decayed` is clamped to `0..=100` and `round` keeps it there, so the conversion is total.
    // It is still written as a fallible one rather than an `as`, since a silent truncation is
    // exactly the defect this block exists to report on.
    u8::try_from(rounded as i64).unwrap_or(100)
}

/// Millionths of a point, the unit a contribution is held in.
///
/// Contributions are only ever compared against one another, never published, and whole points
/// cannot separate them: on a large workspace repairing one site is worth a hundredth of a
/// point, so a whole-point contribution would rank every rule equal at zero.
const CONTRIBUTION_SCALE: f64 = 1_000_000.0;

pub(super) fn calculate_dimensions(
    rules: &[RuleAggregate],
    removed: &BTreeSet<String>,
    scale: Scale,
) -> ScoreDimensions {
    ScoreDimensions::from_fn(|dimension| {
        let state = DimensionState::of(rules, removed, scale, dimension);
        capped(
            dimension_score(state.density, dimension),
            state.worst_tier.and_then(tier_dimension_ceiling),
        )
    })
}

/// What one dimension carries over the rules a scoring pass kept: how dense its findings are,
/// and the worst tier among them.
///
/// The published dimensions and the repair ranking both read this. They part company one step
/// later, at the rounding: the report shows whole points, and the ranking cannot, because on a
/// large workspace one repaired site is worth a hundredth of a point and rounding first
/// collapses every rule into a tie at zero.
struct DimensionState {
    density: f64,
    worst_tier: Option<RuleTier>,
}

impl DimensionState {
    fn of(
        rules: &[RuleAggregate],
        removed: &BTreeSet<String>,
        scale: Scale,
        dimension: ScoreDimension,
    ) -> Self {
        rules
            .iter()
            .filter(|rule| rule.dimension == Some(dimension) && !removed.contains(&rule.id))
            .fold(
                Self {
                    density: 0.0,
                    worst_tier: None,
                },
                |state, rule| Self {
                    density: state.density
                        + rule.numerator as f64 / scale.divisor(rule.scope),
                    worst_tier: worse_tier(state.worst_tier, rule.scoring_tier()),
                },
            )
    }
}

/// The weighted score before any dimension is rounded and before any tier ceiling is applied,
/// which is what a contribution is the difference of.
///
/// The ceilings are deliberately left out. They are a statement about the worst tier a dimension
/// carries, not about how dense it is, and they pin a dimension exactly when it is healthy: a
/// single `P2` rule holds reliability at 75 for every density under 2.9 sites per kiloline. Read
/// through the ceiling, every repair in that dimension recovers nothing, every contribution is
/// zero, `rank_repairs` filters the whole dimension out and the report tells a reader with real
/// findings that there is nothing worth fixing. What the ranking needs is the density relief a
/// repair buys, so that is what it measures. The promise the report makes is unaffected:
/// `projected_after_top_three` rescores through `ScoredState`, ceilings included.
fn uncapped_score(rules: &[RuleAggregate], removed: &BTreeSet<String>, scale: Scale) -> f64 {
    let numerator: f64 = ScoreDimension::ALL
        .into_iter()
        .map(|dimension| {
            let state = DimensionState::of(rules, removed, scale, dimension);
            decayed(state.density, dimension) * dimension_weight_twice(dimension) as f64
        })
        .sum();
    numerator / 13.0
}

/// What removing each rule's sites recovers, in the order the rules are held.
///
/// One pass per rule over the same five dimensions: the whole set has to be rescored without it,
/// because under core-v3 a rule's cost is not a property of the rule. A rule that cannot be
/// scored recovers nothing by construction, and is skipped rather than rescored.
pub(super) fn contributions(rules: &[RuleAggregate], scale: Scale) -> Vec<u64> {
    let base = uncapped_score(rules, &BTreeSet::new(), scale);
    rules
        .iter()
        .map(|rule| {
            if !rule.is_scorable() {
                return 0;
            }
            let removed = BTreeSet::from([rule.id.clone()]);
            let recovered = uncapped_score(rules, &removed, scale) - base;
            // Removing findings can only raise the score, so a negative here is floating-point
            // noise around zero and not a rule worth repairing.
            let scaled = (recovered * CONTRIBUTION_SCALE).round();
            if scaled.is_finite() && scaled > 0.0 {
                scaled as u64
            } else {
                0
            }
        })
        .collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    /// λ is the density costing 63 points, and a dimension sitting at it scores 37.
    #[test]
    fn a_dimension_at_its_lambda_scores_thirty_seven() {
        for dimension in ScoreDimension::ALL {
            assert_eq!(dimension_score(0.0, dimension), 100);
            assert_eq!(dimension_score(lambda(dimension), dimension), 37);
        }
    }

    /// A zero λ divides by nothing and takes every density to infinity.
    #[test]
    fn no_lambda_is_zero() {
        for dimension in ScoreDimension::ALL {
            let value = lambda(dimension);
            assert!(value > 0.0 && value.is_finite(), "{dimension:?} has λ {value}");
        }
    }

    /// The curve never rises with density, and it moves at every density rather than saturating.
    #[test]
    fn a_denser_dimension_never_scores_higher() {
        for dimension in ScoreDimension::ALL {
            let mut previous = 100u8;
            let mut density = 0.0;
            while density < 40.0 {
                let value = dimension_score(density, dimension);
                assert!(value <= previous, "{dimension:?} rose at density {density}");
                previous = value;
                density += 0.05;
            }
        }
        // Strictly less wherever the two round differently, which is the whole point of the
        // curve: the step function it replaced answered the same number for 21 sites and 246.
        assert!(dimension_score(1.0, ScoreDimension::Reliability) > dimension_score(4.0, ScoreDimension::Reliability));
    }

    /// A density no arithmetic can survive still lands inside the published range.
    #[test]
    fn an_absurd_density_stays_inside_the_published_range() {
        for density in [1e9, f64::MAX, f64::INFINITY] {
            for dimension in ScoreDimension::ALL {
                assert_eq!(dimension_score(density, dimension), 0);
            }
        }
    }

    /// The floor protects the small crate: a 120-line workspace is divided by two kilolines,
    /// not by 0.12, so three findings are three findings rather than a catastrophe.
    #[test]
    fn a_small_workspace_is_divided_by_the_floor() {
        let small = Scale::of(120);
        assert_eq!(small.divisor(DensityScope::PerKiloline), KILOLINE_FLOOR);
        assert_eq!(small.divisor(DensityScope::Workspace), 1.0);
        assert_eq!(Scale::of(0), small, "an empty workspace never divides by zero");
        assert_eq!(Scale::of(2000), small, "the floor is exactly two kilolines");
        assert!(Scale::of(20_000).divisor(DensityScope::PerKiloline) > small.divisor(DensityScope::PerKiloline));
    }

    /// The split is a total function of the catalog's own producer field.
    #[test]
    fn every_producer_names_its_denominator() {
        for (producer, scope) in [
            (Producer::Clippy, DensityScope::PerKiloline),
            (Producer::SourceKernel, DensityScope::PerKiloline),
            (Producer::Structure, DensityScope::PerKiloline),
            (Producer::CargoHealth, DensityScope::Workspace),
            (Producer::Repo, DensityScope::Workspace),
        ] {
            assert_eq!(DensityScope::of(producer), scope);
        }
    }

    /// An error site weighs twice a warning, and an unresolved severity weighs nothing at all.
    #[test]
    fn severity_weighs_a_site_or_refuses_to() {
        assert_eq!(severity_weight(Severity::Error), Some(2));
        assert_eq!(severity_weight(Severity::Warning), Some(1));
        assert_eq!(severity_weight(Severity::Info), Some(1));
        assert_eq!(severity_weight(Severity::Unknown), None);
    }
}
