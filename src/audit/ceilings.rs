//! The tier ceilings: what each tier caps a dimension and the score at, and the
//! ceilings a score publishes with the rules that impose them.
//!
//! It lives beside the block rather than inside it so that every file of the
//! module stays under the thousand lines `oversized_unit` reports, and because
//! the ceilings are one question: which tier holds what, and which rules hold
//! that tier.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::policy::RuleTier;

use super::{RuleAggregate, ScoreDimension};

/// One ceiling a tier imposes on a dimension or on the score, and the rules
/// holding that tier there.
///
/// Published so that a capped value names its cause without the reader
/// re-deriving the model: the text used to name every rule of the worst tier
/// the report carried, test-context findings and withheld rules included, none
/// of which cap anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScoreCeiling {
    /// The dimension the ceiling holds, or `overall` for the score itself.
    pub dimension: &'static str,
    pub tier: RuleTier,
    pub ceiling: u8,
    /// The rules whose scored sites hold the tier, by id.
    pub rule_ids: Vec<String>,
}

/// The scope a ceiling published for the score itself names.
pub(crate) const OVERALL: &str = "overall";

/// Every ceiling the rules impose, the dimensions in the order the report
/// publishes them and the score's own last. Empty when no tier caps.
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

/// The ceiling the worst tier among `rules` imposes, with the rules holding it.
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
    /// The tier this rule caps its dimension and the score at, if any.
    ///
    /// A non-scorable rule caps nothing: a tier cannot act without a retained
    /// dimension and severity. A withheld rule caps nothing either, while its
    /// sites are still charged at the discount: under core-v5 one correct
    /// `process::exit`, on a rule the corpus judged wrong on every site it
    /// showed, held reliability at 75 and cost a workspace five points the
    /// report itself said were not worth repairing.
    pub(super) fn scoring_tier(&self) -> Option<RuleTier> {
        if self.is_scorable() && !self.is_withheld() {
            self.tier
        } else {
            None
        }
    }
}

/// Cap imposed on a dimension by the worst tier it contains.
pub(super) const fn tier_dimension_ceiling(tier: RuleTier) -> Option<u8> {
    match tier {
        RuleTier::P0 => Some(20),
        RuleTier::P1 => Some(50),
        RuleTier::P2 => Some(75),
        RuleTier::P3 => None,
    }
}

/// Cap imposed on the overall score by the worst tier across all dimensions.
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

/// The worst tier is the minimum, `P0` being declared first.
pub(super) fn worse_tier(current: Option<RuleTier>, candidate: Option<RuleTier>) -> Option<RuleTier> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.min(candidate)),
        (current, candidate) => current.or(candidate),
    }
}
