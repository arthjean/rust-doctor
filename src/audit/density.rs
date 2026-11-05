use std::collections::BTreeSet;

use crate::policy::{Producer, RuleTier};
use crate::report::Severity;

use super::ceilings::{capped, tier_dimension_ceiling, tier_overall_ceiling, worse_tier};
use super::{
    BASIS_POINTS, RuleAggregate, ScoreDimension, ScoreDimensions, dimension_weight_twice,
};

impl RuleAggregate {
    pub(super) fn charged(&self) -> f64 {
        self.charge(self.numerator)
    }

    pub(super) fn charge(&self, sites: u64) -> f64 {
        let kept = self
            .noise
            .map_or(BASIS_POINTS, |noise| BASIS_POINTS.saturating_sub(u64::from(noise)));
        let weighted = sites.saturating_mul(tier_weight(self.tier));
        weighted as f64 * kept as f64 / BASIS_POINTS as f64
    }

    fn density(&self, scale: Scale) -> f64 {
        match self.scope {
            DensityScope::PerKiloline => self.charged() / scale.divisor(self.scope),
            DensityScope::Workspace => {
                let owned = self.owned.min(self.numerator);
                self.charge(self.numerator - owned) + self.charge(owned) / scale.members()
            }
        }
    }
}

pub(super) const KILOLINE_FLOOR: f64 = 2.0;

const LINES_PER_KILOLINE: f64 = 1000.0;

pub(super) const fn severity_weight(severity: Severity) -> Option<u64> {
    match severity {
        Severity::Error => Some(2),
        Severity::Warning => Some(1),
        Severity::Info => Some(0),
        Severity::Unknown => None,
    }
}

pub(super) const fn tier_weight(tier: Option<RuleTier>) -> u64 {
    match tier {
        Some(RuleTier::P0) => 8,
        Some(RuleTier::P1) => 4,
        Some(RuleTier::P2) => 2,
        Some(RuleTier::P3) | None => 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DensityScope {
    PerKiloline,
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Scale {
    kilolines: f64,
    members: f64,
}

impl Scale {
    pub(super) fn of(production_lines: usize, members: usize) -> Self {
        Self {
            kilolines: (production_lines as f64 / LINES_PER_KILOLINE).max(KILOLINE_FLOOR),
            members: members.max(1) as f64,
        }
    }

    pub(super) const fn members(self) -> f64 {
        self.members
    }

    pub(super) fn divisor(self, scope: DensityScope) -> f64 {
        match scope {
            DensityScope::PerKiloline => self.kilolines,
            DensityScope::Workspace => 1.0,
        }
    }
}

pub(super) const fn lambda(dimension: ScoreDimension) -> f64 {
    match dimension {
        ScoreDimension::Security => 4.0,
        ScoreDimension::Reliability => 10.0,
        ScoreDimension::Maintainability => 3.0,
        ScoreDimension::Performance => 4.0,
        ScoreDimension::Dependencies => 6.0,
    }
}

pub(super) fn decayed(density: f64, dimension: ScoreDimension) -> f64 {
    let value = 100.0 * (-density / lambda(dimension)).exp();
    value.clamp(0.0, 100.0)
}

pub(super) fn dimension_score(density: f64, dimension: ScoreDimension) -> u8 {
    let rounded = decayed(density, dimension).round();
    u8::try_from(rounded as i64).unwrap_or(100)
}

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
                    density: state.density + rule.density(scale),
                    worst_tier: worse_tier(state.worst_tier, rule.scoring_tier()),
                },
            )
    }
}

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

pub(super) fn capped_score_micro(
    rules: &[RuleAggregate],
    removed: &BTreeSet<String>,
    scale: Scale,
) -> u64 {
    let mut worst_tier = None;
    let numerator: f64 = ScoreDimension::ALL
        .into_iter()
        .map(|dimension| {
            let state = DimensionState::of(rules, removed, scale, dimension);
            worst_tier = worse_tier(worst_tier, state.worst_tier);
            let ceiling = state
                .worst_tier
                .and_then(tier_dimension_ceiling)
                .map_or(100.0, f64::from);
            decayed(state.density, dimension).min(ceiling) * dimension_weight_twice(dimension) as f64
        })
        .sum();
    let overall = worst_tier
        .and_then(tier_overall_ceiling)
        .map_or(100.0, f64::from);
    let value = (numerator / 13.0).min(overall);
    let scaled = (value * CONTRIBUTION_SCALE).round();
    if scaled.is_finite() && scaled > 0.0 {
        scaled as u64
    } else {
        0
    }
}

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

    #[test]
    fn a_dimension_at_its_lambda_scores_thirty_seven() {
        for dimension in ScoreDimension::ALL {
            assert_eq!(dimension_score(0.0, dimension), 100);
            assert_eq!(dimension_score(lambda(dimension), dimension), 37);
        }
    }

    #[test]
    fn no_lambda_is_zero() {
        for dimension in ScoreDimension::ALL {
            let value = lambda(dimension);
            assert!(value > 0.0 && value.is_finite(), "{dimension:?} has λ {value}");
        }
    }

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
        assert!(dimension_score(1.0, ScoreDimension::Reliability) > dimension_score(4.0, ScoreDimension::Reliability));
    }

    #[test]
    fn an_absurd_density_stays_inside_the_published_range() {
        for density in [1e9, f64::MAX, f64::INFINITY] {
            for dimension in ScoreDimension::ALL {
                assert_eq!(dimension_score(density, dimension), 0);
            }
        }
    }

    #[test]
    fn a_small_workspace_is_divided_by_the_floor() {
        let small = Scale::of(120, 1);
        assert_eq!(small.divisor(DensityScope::PerKiloline), KILOLINE_FLOOR);
        assert_eq!(small.divisor(DensityScope::Workspace), 1.0);
        assert_eq!(Scale::of(0, 1), small, "an empty workspace never divides by zero");
        assert_eq!(Scale::of(2000, 1), small, "the floor is exactly two kilolines");
        assert!(Scale::of(20_000, 1).divisor(DensityScope::PerKiloline) > small.divisor(DensityScope::PerKiloline));
    }

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

    #[test]
    fn severity_weighs_a_site_or_refuses_to() {
        assert_eq!(severity_weight(Severity::Error), Some(2));
        assert_eq!(severity_weight(Severity::Warning), Some(1));
        assert_eq!(severity_weight(Severity::Info), Some(0));
        assert_eq!(severity_weight(Severity::Unknown), None);
    }

    #[test]
    fn every_tier_doubles_the_one_below_it() {
        assert_eq!(tier_weight(Some(RuleTier::P0)), 8);
        assert_eq!(tier_weight(Some(RuleTier::P1)), 4);
        assert_eq!(tier_weight(Some(RuleTier::P2)), 2);
        assert_eq!(tier_weight(Some(RuleTier::P3)), 1);
        assert_eq!(tier_weight(None), 1);
    }
}
