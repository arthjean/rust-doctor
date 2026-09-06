//! What the report tells the reader to repair first: the projection, the rules it withholds,
//! and the discount both read.
//!
//! It lives beside the block rather than inside it so that every file of the module stays under
//! the thousand lines `oversized_unit` reports, and because it is one question: what a repair is
//! expected to be worth to the reader, which is what a rule costs the score discounted by how
//! often the pinned corpus adjudicated it wrong.

use std::collections::BTreeSet;

use crate::policy::UNMEASURED_NOISE_BASIS_POINTS;

use super::density::capped_score_micro;
use super::{BASIS_POINTS, RuleAggregate, RuleAggregation, worse_tier};

/// The smoothed rate past which a rule is more often wrong than right on
/// healthy code, and is withheld from the projection whatever it would recover.
///
/// It is strictly above the middle so that an unmeasured rule, which sits
/// exactly there, is ranked and not withheld. The threshold used to be an
/// expected repair value rounding to zero millionths of a point, which no
/// rule ever reached: one site of `indexing_slicing` on five million lines
/// still carried a value, and the report recommended fixing `print_stderr`,
/// adjudicated wrong on every site it showed, in the entry point of a binary.
const WITHHOLD_NOISE_BASIS_POINTS: u16 = 5_000;


impl RuleAggregate {
    /// What repairing this rule is expected to be worth, which is what it costs
    /// the score discounted by how often the corpus found it wrong.
    ///
    /// A rule the corpus adjudicated wrong on nearly every site it showed is
    /// expected to be worth little to repair, whatever its volume, and volume
    /// is exactly what the noisiest rules have the most of. Ranking by
    /// contribution alone told the user to fix the rule that fires most, which
    /// is not the same question and, on a rule measured wrong almost
    /// everywhere, is advice to change correct code.
    ///
    /// An unmeasured rule is discounted at `UNMEASURED_NOISE_BASIS_POINTS`, the
    /// middle of the interval, rather than kept whole. No measurement is not
    /// evidence of correctness either, and the corpus has adjudicated 24 of the
    /// 62 catalogued rules: keeping the other 38 undiscounted put every one of
    /// them ahead of every rule the corpus ever confirmed, so the ranking read
    /// as a list of what nobody has checked. It is the same smoothing at no
    /// observations rather than a threshold of its own, so the first site the
    /// corpus adjudicates moves the rule off the middle in whichever direction
    /// it was adjudicated.
    pub(crate) fn expected_repair_value(&self) -> u64 {
        self.contribution()
            .saturating_mul(self.kept_basis_points())
            / BASIS_POINTS
    }

    /// The share of a repair's value the corpus expects to be real, in basis
    /// points: the complement of the smoothed rate, read at the middle for a
    /// rule nobody measured.
    pub(super) fn kept_basis_points(&self) -> u64 {
        let noise = self.noise.unwrap_or(UNMEASURED_NOISE_BASIS_POINTS);
        BASIS_POINTS.saturating_sub(u64::from(noise))
    }

    /// Whether the corpus found this rule wrong more often than right, which
    /// is what keeps it out of the projection.
    pub(super) fn is_withheld(&self) -> bool {
        self.noise
            .is_some_and(|noise| noise > WITHHOLD_NOISE_BASIS_POINTS)
    }
}

/// How many rules the projection names.
const PROJECTED_RULES: usize = 3;

impl RuleAggregation {
    /// The rules worth repairing first, in the order to repair them, and the rules the corpus
    /// found too noisy to name, loudest first.
    ///
    /// The projection is climbed rather than sorted. Each of its three places goes to the rule
    /// whose repair, on top of the ones already named, gives the most points back through the
    /// same ceilings the published value takes, discounted by the rate the corpus adjudicated it
    /// wrong. A static key could not answer that: under a `P1` ceiling every `P3` repair is worth
    /// zero points and the relief of its dimension is the only thing that separates it from the
    /// rule holding the ceiling, so a ranking read from relief alone named three `P3` rules on a
    /// workspace capped at 65 and promised 65. Where no ceiling moves, the marginal gain is the
    /// uncapped relief and the order is the one the static key gave. Where nothing moves at all,
    /// two `P1` rules sharing one ceiling, the rules holding the worst tier are named before any
    /// other: no repair is worth points until they are gone, so the projection starts on them.
    ///
    /// A rule the corpus found wrong more often than right is withheld rather than ranked last,
    /// because naming it would still be telling the reader to go and change correct code. It is
    /// published all the same, so its absence from the projection reads as a measurement and not
    /// as a defect, and the sample it rests on is printed beside it wherever the report names it.
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

