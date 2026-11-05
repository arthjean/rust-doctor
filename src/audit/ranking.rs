use std::collections::BTreeSet;

use crate::policy::{UNMEASURED_NOISE_BASIS_POINTS, WITHHOLD_NOISE_BASIS_POINTS};

use super::density::capped_score_micro;
use super::{BASIS_POINTS, RuleAggregate, RuleAggregation, worse_tier};

impl RuleAggregate {
    pub(crate) fn expected_repair_value(&self) -> u64 {
        self.contribution()
            .saturating_mul(self.kept_basis_points())
            / BASIS_POINTS
    }

    pub(super) fn kept_basis_points(&self) -> u64 {
        if self.noise.is_some() {
            BASIS_POINTS
        } else {
            BASIS_POINTS.saturating_sub(u64::from(UNMEASURED_NOISE_BASIS_POINTS))
        }
    }

    pub(super) fn is_withheld(&self) -> bool {
        self.noise
            .is_some_and(|noise| noise > WITHHOLD_NOISE_BASIS_POINTS)
    }
}

const PROJECTED_RULES: usize = 3;

impl RuleAggregation {
    pub(crate) fn projection(&self) -> (Vec<&RuleAggregate>, Vec<&RuleAggregate>) {
        let (candidates, mut withheld): (Vec<_>, Vec<_>) = self
            .rules
            .iter()
            .filter(|rule| rule.is_scorable() && rule.contribution() > 0)
            .partition(|rule| !rule.is_withheld());
        withheld.sort_by(|left, right| {
            right
                .contribution()
                .cmp(&left.contribution())
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut projected: Vec<&RuleAggregate> = Vec::with_capacity(PROJECTED_RULES);
        let mut removed = BTreeSet::new();
        while projected.len() < PROJECTED_RULES {
            let reached = capped_score_micro(&self.rules, &removed, self.scale);
            let worst_tier = self
                .rules
                .iter()
                .filter(|rule| !removed.contains(&rule.id))
                .fold(None, |worst, rule| worse_tier(worst, rule.scoring_tier()));
            let best = candidates
                .iter()
                .filter(|rule| !removed.contains(&rule.id))
                .map(|rule| {
                    let mut after = removed.clone();
                    after.insert(rule.id.clone());
                    let gain = capped_score_micro(&self.rules, &after, self.scale)
                        .saturating_sub(reached)
                        .saturating_mul(rule.kept_basis_points())
                        / BASIS_POINTS;
                    let holds_the_ceiling =
                        worst_tier.is_some() && rule.scoring_tier() == worst_tier;
                    (gain, holds_the_ceiling, rule.expected_repair_value(), *rule)
                })
                .max_by(|left, right| {
                    left.0
                        .cmp(&right.0)
                        .then_with(|| left.1.cmp(&right.1))
                        .then_with(|| left.2.cmp(&right.2))
                        .then_with(|| right.3.id.cmp(&left.3.id))
                });
            let Some((_, _, _, rule)) = best else {
                break;
            };
            removed.insert(rule.id.clone());
            projected.push(rule);
        }
        (projected, withheld)
    }
}

