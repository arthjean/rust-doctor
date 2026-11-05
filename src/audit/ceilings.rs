use std::collections::BTreeSet;

use serde::Serialize;

use crate::policy::RuleTier;

use super::{RuleAggregate, ScoreDimension};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScoreCeiling {
    pub dimension: &'static str,
    pub tier: RuleTier,
    pub ceiling: u8,
    pub rule_ids: Vec<String>,
}

pub(crate) const OVERALL: &str = "overall";

pub(super) fn ceilings(rules: &[RuleAggregate]) -> Vec<ScoreCeiling> {
    let mut published: Vec<ScoreCeiling> = ScoreDimension::ALL
        .into_iter()
        .filter_map(|dimension| {
            let holding = rules
                .iter()
                .filter(|rule| rule.dimension == Some(dimension));
            imposed(holding, tier_dimension_ceiling, dimension.as_str())
        })
        .collect();
    published.extend(imposed(rules.iter(), tier_overall_ceiling, OVERALL));
    published
}

fn imposed<'a>(
    rules: impl Iterator<Item = &'a RuleAggregate> + Clone,
    ceiling_of: fn(RuleTier) -> Option<u8>,
    dimension: &'static str,
) -> Option<ScoreCeiling> {
    let tier = rules
        .clone()
        .fold(None, |worst, rule| worse_tier(worst, rule.scoring_tier()))?;
    let ceiling = ceiling_of(tier)?;
    let rule_ids: BTreeSet<&str> = rules
        .filter(|rule| rule.scoring_tier() == Some(tier))
        .map(|rule| rule.id.as_str())
        .collect();
    Some(ScoreCeiling {
        dimension,
        tier,
        ceiling,
        rule_ids: rule_ids.into_iter().map(str::to_owned).collect(),
    })
}

impl RuleAggregate {
    pub(super) fn scoring_tier(&self) -> Option<RuleTier> {
        if self.is_scorable() && !self.is_withheld() {
            self.tier
        } else {
            None
        }
    }
}

pub(super) const fn tier_dimension_ceiling(tier: RuleTier) -> Option<u8> {
    match tier {
        RuleTier::P0 => Some(20),
        RuleTier::P1 => Some(50),
        RuleTier::P2 => Some(75),
        RuleTier::P3 => None,
    }
}

pub(super) const fn tier_overall_ceiling(tier: RuleTier) -> Option<u8> {
    match tier {
        RuleTier::P0 => Some(40),
        RuleTier::P1 => Some(65),
        RuleTier::P2 | RuleTier::P3 => None,
    }
}

pub(super) const fn capped(value: u8, ceiling: Option<u8>) -> u8 {
    match ceiling {
        Some(ceiling) if ceiling < value => ceiling,
        _ => value,
    }
}

pub(super) fn worse_tier(current: Option<RuleTier>, candidate: Option<RuleTier>) -> Option<RuleTier> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.min(candidate)),
        (current, candidate) => current.or(candidate),
    }
}
