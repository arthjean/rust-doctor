//! The sampling procedure, as data rather than as a sentence.
//!
//! `adjudication.sampling` states in prose that sites are drawn by a stride
//! over the ordered population. A sentence is not replayable: nothing compares
//! it to what was actually reviewed, so a sample drawn by hand, by convenience
//! or from the head of the list reads exactly the same as one drawn by the
//! procedure the prose claims. A rate is only worth what its sample is worth,
//! and the sample is the one input of the whole record no coefficient can
//! check.
//!
//! So a scope adjudicated under the protocol publishes the three numbers the
//! draw is a function of: the population it observed, the target it asked for,
//! and the positions the stride landed on. The positions are recomputed here
//! from the first two, which is what turns the procedure into something a
//! reader disagrees with by rerunning it.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{Adjudication, Population};

/// The draw of one `(rule, population)` adjudicated after the cutoff.
///
/// Published per scope rather than once for the whole record, because the
/// target is a decision taken rule by rule: a rule firing three times on the
/// corpus is drawn exhaustively, and one firing four hundred times is drawn to
/// the sample the reviewer can actually judge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SamplingPlan {
    /// Positions the stride landed on, into the canonically ordered population.
    ///
    /// Stored rather than derived at read time, so that a plan whose arithmetic
    /// was replayed by a different implementation, or by a hand, is a diff
    /// against a recorded list instead of a claim nobody wrote down.
    pub(crate) indices: Vec<u64>,
    /// Sites the scan produced for this scope, ordered by repository, path and
    /// line. The denominator of the draw, never the denominator of the rate.
    pub(crate) observed: u64,
    pub(crate) population: Population,
    pub(crate) rule: String,
    /// Sites asked for. Never above `observed`: a target the population cannot
    /// supply is a plan that did not describe the draw that happened.
    pub(crate) target: u64,
}

/// Sites drawn per rule under the protocol.
///
/// Twenty rather than the five the pre-protocol samples took, for the reason
/// `sampling` states about the healthy draw of the same rules: five sites
/// resolve steps of twenty points and cannot place a rule against a threshold
/// of five percent, so a five-site rate on this population would be published
/// indecisive by construction rather than by measurement.
pub(crate) const PROTOCOL_TARGET: u64 = 20;

/// The positions a stride of `target` sites lands on over `observed` sites.
///
/// `k = min(target, observed)` and position `i` is `floor(i * observed / k)`,
/// which is the only arithmetic in the procedure and the reason it is written
/// once here. It spreads the draw over the whole ordered population instead of
/// taking its head, which matters because the order is by repository first: the
/// head of the population is one repository, and a sample from one repository
/// measures that repository rather than the corpus.
///
/// Integer arithmetic throughout, in the order stated, for the reason every
/// other figure of this record is an integer: two machines that round
/// differently must not draw different samples.
pub(crate) fn stride(observed: u64, target: u64) -> Vec<u64> {
    let count = target.min(observed);
    if count == 0 {
        return Vec::new();
    }
    (0..count).map(|index| index * observed / count).collect()
}

/// Closed defects of the sampling plans, each naming the rule.
///
/// The coupling to `adjudicated_after_cutoff` is checked in both directions.
/// A scope adjudicated under the protocol with no plan is a sample nobody can
/// replay, which is the whole defect this block exists against; and a plan for
/// a scope adjudicated before the cutoff is a procedure claimed over verdicts
/// it never governed. Absence is the fact a pre-cutoff scope publishes, so the
/// second direction is a defect rather than a row of zeros.
pub(crate) fn sampling_defects(adjudication: &Adjudication) -> Vec<String> {
    let mut defects = Vec::new();

    let enrolled: BTreeSet<(&str, Population)> = adjudication
        .adjudicated_after_cutoff
        .iter()
        .map(|scope| (scope.rule.as_str(), scope.population))
        .collect();

    let mut planned: BTreeMap<(&str, Population), &SamplingPlan> = BTreeMap::new();
    for plan in &adjudication.sampling_plan {
        let label = format!("{} on {:?}", plan.rule, plan.population);
        if planned
            .insert((plan.rule.as_str(), plan.population), plan)
            .is_some()
        {
            defects.push(format!("{label} publishes two sampling plans"));
        }
        if !enrolled.contains(&(plan.rule.as_str(), plan.population)) {
            defects.push(format!(
                "{label} carries a sampling plan and is adjudicated before the cutoff {}",
                adjudication.protocol_cutoff
            ));
        }
        if plan.target == 0 {
            defects.push(format!("{label} publishes a target of zero"));
        }
        if plan.target > plan.observed {
            defects.push(format!(
                "{label} publishes a target of {} over a population of {}",
                plan.target, plan.observed
            ));
        }
        let expected = stride(plan.observed, plan.target);
        if plan.indices != expected {
            defects.push(format!(
                "{label} publishes indices {:?} against {expected:?} the stride selects",
                plan.indices
            ));
        }
    }

    let mut adjudicated: BTreeMap<(&str, Population), u64> = BTreeMap::new();
    for site in &adjudication.reviewed {
        *adjudicated
            .entry((site.rule.as_str(), site.population))
            .or_default() += 1;
    }
    for pair in &adjudication.agreement.pairs {
        if !pair.agrees() {
            *adjudicated
                .entry((pair.rule.as_str(), pair.population))
                .or_default() += 1;
        }
    }

    for scope in &adjudication.adjudicated_after_cutoff {
        let label = format!("{} on {:?}", scope.rule, scope.population);
        let Some(plan) = planned.get(&(scope.rule.as_str(), scope.population)) else {
            defects.push(format!(
                "{label} is adjudicated after the cutoff {} with no sampling plan",
                adjudication.protocol_cutoff
            ));
            continue;
        };
        // Sites judged, not sites published: a pair whose passes disagreed was
        // drawn by the plan and stays out of `reviewed` until a human settles
        // it, so counting only the published verdicts would report every open
        // escalation as a site the stride selected and nobody looked at.
        let judged = adjudicated
            .get(&(scope.rule.as_str(), scope.population))
            .copied()
            .unwrap_or_default();
        let selected = plan.indices.len() as u64;
        if judged != selected {
            defects.push(format!(
                "{label} adjudicated {judged} sites against the {selected} its stride selects"
            ));
        }
    }

    defects
}
