use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::agreement::{Agreement, AdjudicatedPair};
use super::{Population, Verdict};

const WHOLE: f64 = 10_000.0;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KappaStatus {
    Defined,
    UndefinedNoVariance,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContingencyTable {
    pub(crate) both_false_positive: u64,
    pub(crate) both_true_positive: u64,
    pub(crate) first_only_false_positive: u64,
    pub(crate) second_only_false_positive: u64,
}

impl ContingencyTable {
    fn count(&mut self, pair: &AdjudicatedPair) {
        match (pair.passes[0].verdict, pair.passes[1].verdict) {
            (Verdict::FalsePositive, Verdict::FalsePositive) => self.both_false_positive += 1,
            (Verdict::TruePositive, Verdict::TruePositive) => self.both_true_positive += 1,
            (Verdict::FalsePositive, Verdict::TruePositive) => self.first_only_false_positive += 1,
            (Verdict::TruePositive, Verdict::FalsePositive) => self.second_only_false_positive += 1,
        }
    }

    pub(crate) fn pairs(&self) -> u64 {
        self.both_false_positive
            + self.both_true_positive
            + self.first_only_false_positive
            + self.second_only_false_positive
    }

    pub(crate) fn agreed(&self) -> u64 {
        self.both_false_positive + self.both_true_positive
    }

    fn margins(&self) -> (u64, u64) {
        (
            self.both_false_positive + self.first_only_false_positive,
            self.both_false_positive + self.second_only_false_positive,
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Coefficient {
    pub(crate) ac1_basis_points: i64,
    pub(crate) agreed: u64,
    pub(crate) kappa_basis_points: Option<i64>,
    pub(crate) kappa_status: KappaStatus,
    pub(crate) pairs: u64,
    pub(crate) population: Population,
    pub(crate) rule: String,
    pub(crate) table: ContingencyTable,
}

pub(crate) fn coefficients(agreement: &Agreement) -> Vec<Coefficient> {
    let mut tables: BTreeMap<(&str, Population), ContingencyTable> = BTreeMap::new();
    for pair in &agreement.pairs {
        tables
            .entry((pair.rule.as_str(), pair.population))
            .or_default()
            .count(pair);
    }

    tables
        .into_iter()
        .map(|((rule, population), table)| {
            let kappa = kappa(&table);
            Coefficient {
                ac1_basis_points: to_basis_points(ac1(&table)),
                agreed: table.agreed(),
                kappa_basis_points: kappa.map(to_basis_points),
                kappa_status: match kappa {
                    Some(_) => KappaStatus::Defined,
                    None => KappaStatus::UndefinedNoVariance,
                },
                pairs: table.pairs(),
                population,
                rule: rule.to_owned(),
                table,
            }
        })
        .collect()
}

fn kappa(table: &ContingencyTable) -> Option<f64> {
    let pairs = table.pairs();
    let (first, second) = table.margins();
    if pairs == 0 || first == 0 || first == pairs || second == 0 || second == pairs {
        return None;
    }

    let n = pairs as f64;
    let observed = table.agreed() as f64 / n;
    let first = first as f64 / n;
    let second = second as f64 / n;
    let expected = first * second + (1.0 - first) * (1.0 - second);
    Some((observed - expected) / (1.0 - expected))
}

fn ac1(table: &ContingencyTable) -> f64 {
    let pairs = table.pairs();
    if pairs == 0 {
        return 0.0;
    }

    let n = pairs as f64;
    let (first, second) = table.margins();
    let observed = table.agreed() as f64 / n;
    let prevalence = (first as f64 / n + second as f64 / n) / 2.0;
    let expected = 2.0 * prevalence * (1.0 - prevalence);
    (observed - expected) / (1.0 - expected)
}

fn to_basis_points(value: f64) -> i64 {
    (value * WHOLE).round() as i64
}

pub(crate) fn coefficient_defects(agreement: &Agreement) -> Vec<String> {
    let computed = coefficients(agreement);
    let mut defects = Vec::new();

    let published: BTreeMap<(&str, Population), &Coefficient> = agreement
        .coefficients
        .iter()
        .map(|row| ((row.rule.as_str(), row.population), row))
        .collect();
    if published.len() != agreement.coefficients.len() {
        defects.push("two coefficient rows share one rule and population".to_owned());
    }

    for row in &computed {
        let label = format!("{} on {:?}", row.rule, row.population);
        let Some(stored) = published.get(&(row.rule.as_str(), row.population)) else {
            defects.push(format!("{label} carries pairs and no coefficient row"));
            continue;
        };
        let mut compare = |field: &str, stored: String, computed: String| {
            if stored != computed {
                defects.push(format!(
                    "{label} publishes {field} {stored} against {computed} recomputed"
                ));
            }
        };
        compare(
            "ac1_basis_points",
            stored.ac1_basis_points.to_string(),
            row.ac1_basis_points.to_string(),
        );
        compare("agreed", stored.agreed.to_string(), row.agreed.to_string());
        compare("pairs", stored.pairs.to_string(), row.pairs.to_string());
        compare(
            "kappa_basis_points",
            format!("{:?}", stored.kappa_basis_points),
            format!("{:?}", row.kappa_basis_points),
        );
        compare(
            "kappa_status",
            format!("{:?}", stored.kappa_status),
            format!("{:?}", row.kappa_status),
        );
        compare(
            "table",
            format!("{:?}", stored.table),
            format!("{:?}", row.table),
        );
    }

    let expected: BTreeMap<(&str, Population), &Coefficient> = computed
        .iter()
        .map(|row| ((row.rule.as_str(), row.population), row))
        .collect();
    for row in &agreement.coefficients {
        if !expected.contains_key(&(row.rule.as_str(), row.population)) {
            defects.push(format!(
                "{} on {:?} publishes a coefficient row backed by no pair",
                row.rule, row.population
            ));
        }
    }

    defects
}
