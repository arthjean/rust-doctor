//! Agreement between the two passes, computed rather than written.
//!
//! Until this block existed, the whole assertion on the agreement of the
//! 2026-08-11 double pass was that the prose describing it ran past eighty
//! characters. Every number in that sentence could drift without a test
//! moving, while the rate beside it was recomputed and compared field by
//! field. A coefficient is derived from the pairs on record here, the way the
//! rate is derived from the sites, so the sentence in `sampling` summarizes
//! checked data instead of being the only place the data lives.
//!
//! One table per `(rule, population)`, and both coefficients derived from it,
//! so two figures over the same pairs can never disagree about what those
//! pairs were.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::agreement::{Agreement, AdjudicatedPair};
use super::{Population, Verdict};

/// One whole, in basis points.
const WHOLE: f64 = 10_000.0;

/// Whether the pairs of a row admit a kappa at all.
///
/// An enum rather than an `Option` carrying no reason: "undefined because one
/// pass never varied" is a fact worth naming, and it is exactly the case the
/// 2026-08-11 run hit twice. A pass that judged every site the same way fixes
/// the expected agreement from the other pass alone, so the coefficient stops
/// measuring the two raters and starts measuring the prevalence. Publishing
/// 1.0 there, which is what an unguarded formula tends toward, states perfect
/// agreement on the strength of a rater that made no choice.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KappaStatus {
    Defined,
    UndefinedNoVariance,
}

/// The 2x2 table of one `(rule, population)`, counted over its pairs.
///
/// Published rather than kept as an intermediate, because a coefficient whose
/// table is not visible is a number nobody can recompute by hand, and the four
/// cells are what a reader disagreeing with a coefficient has to look at.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContingencyTable {
    pub(crate) both_false_positive: u64,
    pub(crate) both_true_positive: u64,
    /// The first pass alone called it a false positive.
    pub(crate) first_only_false_positive: u64,
    /// The second pass alone called it a false positive.
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

    /// False positives declared by the first pass, then by the second.
    fn margins(&self) -> (u64, u64) {
        (
            self.both_false_positive + self.first_only_false_positive,
            self.both_false_positive + self.second_only_false_positive,
        )
    }
}

/// Agreement of the two passes over one `(rule, population)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Coefficient {
    /// Gwet's AC1 in basis points, defined wherever the row holds a pair.
    ///
    /// Beside kappa rather than instead of it, which is the standard response
    /// in the annotation literature and not a choice between the two. Kappa
    /// collapses at high prevalence: with most sites judged the same way,
    /// agreement by chance is already high, so the correction eats an
    /// agreement of nine sites in ten and publishes a coefficient that reads
    /// as moderate. AC1 was built against exactly that, its chance term does
    /// not inflate with prevalence, and it stays defined where kappa does not.
    ///
    /// Not an `Option`: a row that holds a pair holds an AC1, so absence is
    /// refused at deserialization rather than checked afterwards. Signed for
    /// the reason kappa is, and this is not hypothetical: the three escalated
    /// sites of 2026-08-11 are two passes that agreed on nothing, which is an
    /// AC1 of minus one whole.
    pub(crate) ac1_basis_points: i64,
    pub(crate) agreed: u64,
    /// Cohen's kappa in basis points, absent exactly when it is undefined.
    ///
    /// Signed, because kappa is: two passes agreeing less often than chance
    /// would predict is a real outcome, and a coefficient stored unsigned turns
    /// it into its own opposite.
    pub(crate) kappa_basis_points: Option<i64>,
    pub(crate) kappa_status: KappaStatus,
    pub(crate) pairs: u64,
    pub(crate) population: Population,
    pub(crate) rule: String,
    pub(crate) table: ContingencyTable,
}

/// One row per `(rule, population)` carrying at least one pair.
///
/// A pair nobody judged is not a row of zeros: a `(rule, population)` with no
/// pair has no agreement to publish, and a zeroed row states one.
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

/// Cohen's kappa over one table, `None` when a pass shows no variance.
///
/// The degenerate margin is the only way out without a value, which is what
/// lets the status name a reason rather than an absence. It is also what makes
/// the division safe: with both margins strictly inside the sample, expected
/// agreement is strictly under one.
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

/// Gwet's AC1 over one table.
///
/// One table for both coefficients, so two figures over the same pairs can
/// never disagree about what those pairs were. What separates them is the
/// chance term alone: kappa multiplies the two margins, which is what makes it
/// collapse when both lean the same way, while AC1 reads the prevalence of one
/// category across both passes and corrects by `2 p (1 - p)`. That term is at
/// most one half, so the denominator never approaches zero and the coefficient
/// is defined on every table that holds a pair, including the no-variance case
/// kappa has to decline.
///
/// A table with no pair yields zero, which no published row can carry: a
/// `(rule, population)` reaches this function only by having been counted at
/// least once.
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

/// A coefficient in signed basis points, rounded half away from zero.
fn to_basis_points(value: f64) -> i64 {
    (value * WHOLE).round() as i64
}

/// Closed defects of the coefficients block, each naming the rule and the field.
///
/// Field by field rather than one comparison of the whole list, because a
/// reader handed "the coefficients disagree" has to diff two blocks by hand to
/// find out which number moved, and the number that moved is the finding.
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
