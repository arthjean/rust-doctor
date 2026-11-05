use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{Adjudication, Population};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SamplingPlan {
    #[serde(default)]
    pub(crate) carried_over: Vec<u64>,
    pub(crate) indices: Vec<u64>,
    pub(crate) observed: u64,
    pub(crate) population: Population,
    pub(crate) rule: String,
    #[serde(default = "protocol_target")]
    pub(crate) target: u64,
}

pub(crate) const PROTOCOL_TARGET: u64 = 20;

fn protocol_target() -> u64 {
    PROTOCOL_TARGET
}

pub(crate) fn target_floor(observed: u64) -> u64 {
    PROTOCOL_TARGET.min(observed)
}

pub(crate) fn stride(observed: u64, target: u64) -> Vec<u64> {
    let count = target.min(observed);
    if count == 0 {
        return Vec::new();
    }
    (0..count).map(|index| index * observed / count).collect()
}

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
        let floor = target_floor(plan.observed);
        if plan.target < floor {
            defects.push(format!(
                "{label} publishes a target of {} below the floor of {floor}",
                plan.target
            ));
        }
        let expected = stride(plan.observed, plan.target);
        if plan.indices != expected {
            defects.push(format!(
                "{label} publishes indices {:?} against {expected:?} the stride selects",
                plan.indices
            ));
        }
        defects.extend(carry_defects(plan, &label));
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
        let judged = adjudicated
            .get(&(scope.rule.as_str(), scope.population))
            .copied()
            .unwrap_or_default();
        let selected = (plan.indices.len() + plan.carried_over.len()) as u64;
        if judged != selected {
            defects.push(format!(
                "{label} adjudicated {judged} sites against the {selected} its plan accounts for"
            ));
        }
    }

    defects
}

fn carry_defects(plan: &SamplingPlan, label: &str) -> Vec<String> {
    let mut defects = Vec::new();
    if plan
        .carried_over
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        defects.push(format!(
            "{label} publishes a carry-over that is not sorted and unique: {:?}",
            plan.carried_over
        ));
    }
    let drawn: BTreeSet<u64> = plan.indices.iter().copied().collect();
    let shared: Vec<u64> = plan
        .carried_over
        .iter()
        .copied()
        .filter(|position| drawn.contains(position))
        .collect();
    if !shared.is_empty() {
        defects.push(format!(
            "{label} carries over {shared:?}, which its own stride selects"
        ));
    }
    let outside: Vec<u64> = plan
        .carried_over
        .iter()
        .copied()
        .filter(|position| *position >= plan.observed)
        .collect();
    if !outside.is_empty() {
        defects.push(format!(
            "{label} carries over {outside:?}, outside a population of {}",
            plan.observed
        ));
    }
    defects
}
