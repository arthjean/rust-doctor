//! The λ table, frozen against the corpus measurement it was calibrated on.
//!
//! λ is what the score costs a finding, so a λ moved without a new measurement republishes every
//! rate of `tests/corpus.json` under a model that never produced them. This module recomputes
//! each of the eighteen recorded scores from the densities the record carries and compares
//! against the value the binary published, so the two can only be changed together.
//!
//! λ_reliability and λ_maintainability are calibrated on the healthy population alone. The agent
//! population is scanned with every Clippy rule off, since its build scripts and procedural
//! macros are untrusted, and those two dimensions are where the Clippy rules concentrate: a
//! calibration reading both populations would be fitting the loosest λ of the model to a
//! population that was never given the chance to fire the rules it bounds.

use super::*;
use crate::audit::density::lambda;

/// The corpus record, read as the bytes the repository ships rather than through the integration
/// harness, which cannot reach a private function of this module.
const CORPUS: &str = include_str!("../../../tests/corpus.json");

/// λ in millionths, matching the integer the corpus records.
fn lambda_micro(dimension: ScoreDimension) -> u64 {
    (lambda(dimension) * 1_000_000.0).round() as u64
}

/// Name the report publishes a dimension under, recovered from the report shape itself.
///
/// Scoring one dimension at one and every other at zero and reading back the field that moved is
/// the same mapping `ScoreDimensions::from_fn` and `value_for` are the two halves of, so a
/// renamed field renames the key this test looks the corpus up by instead of silently comparing
/// against nothing.
fn published_name(dimension: ScoreDimension) -> String {
    let dimensions = ScoreDimensions::from_fn(|candidate| u8::from(candidate == dimension));
    let value = serde_json::to_value(dimensions).unwrap_or(Value::Null);
    value
        .as_object()
        .and_then(|fields| {
            fields
                .iter()
                .find(|(_, score)| score.as_u64() == Some(1))
                .map(|(name, _)| name.clone())
        })
        .unwrap_or_default()
}

fn tier_of(name: Option<&str>) -> Option<RuleTier> {
    let name = name?;
    RuleTier::ALL.into_iter().find(|tier| tier.as_str() == name)
}

fn corpus() -> Value {
    serde_json::from_str(CORPUS).unwrap_or(Value::Null)
}

/// Every observation of the two populations, healthy first.
fn observations(corpus: &Value) -> Vec<&Value> {
    let mut all: Vec<&Value> = corpus["observations"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .collect();
    all.extend(
        corpus["agent_population"]["observations"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default(),
    );
    all
}

/// The corpus records the λ table it was measured under, and it is the shipped one.
///
/// A λ changed without regenerating the corpus fails here naming the dimension that moved, which
/// is the whole point of recording the table beside the toolchain: both decide what the recorded
/// numbers mean, one by choosing which diagnostics exist and the other by choosing what they
/// cost.
#[test]
fn the_corpus_records_the_lambda_table_it_was_measured_under() {
    let corpus = corpus();
    let recorded: Vec<&Value> = corpus["lambdas"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .collect();
    assert_eq!(
        recorded.len(),
        ScoreDimension::ALL.len(),
        "the corpus records {} lambdas for {} dimensions",
        recorded.len(),
        ScoreDimension::ALL.len()
    );

    for dimension in ScoreDimension::ALL {
        let name = published_name(dimension);
        let entry = recorded
            .iter()
            .find(|entry| entry["name"].as_str() == Some(name.as_str()));
        let measured = entry.and_then(|entry| entry["lambda_micro"].as_u64());
        assert_eq!(
            measured,
            Some(lambda_micro(dimension)),
            "lambda_{name} moved: the corpus was measured at {measured:?} millionths and the \
             shipped table is at {}, so tests/corpus.json has to be regenerated with it",
            lambda_micro(dimension)
        );
    }
}

/// No λ of the shipped table is zero, so no density takes a dimension to infinity.
#[test]
fn no_recorded_lambda_is_zero() {
    for dimension in ScoreDimension::ALL {
        assert!(
            lambda_micro(dimension) > 0,
            "lambda_{} is zero",
            published_name(dimension)
        );
    }
}

/// Every corpus score recomputes from the densities the record carries.
///
/// The record holds the density, not the score arithmetic, so this walks the same path
/// `Audit::build` walks: the curve per dimension, the tier ceiling of that dimension, the
/// weighted mean, the overall ceiling, then the label. A λ changed, a weight changed or a
/// ceiling changed all land here, on eighteen real measurements rather than on a fixture.
#[test]
fn every_corpus_score_recomputes_from_its_recorded_densities() {
    let corpus = corpus();
    let mut recomputed = 0usize;
    for observation in observations(&corpus) {
        let name = observation["name"].as_str().unwrap_or_default();
        let score = &observation["score"];
        if score.is_null() {
            continue;
        }
        assert_eq!(
            score["model"].as_str(),
            Some(SCORE_MODEL),
            "{name} was measured under a model this binary does not ship"
        );

        let recorded: Vec<&Value> = score["dimensions"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .collect();
        let mut worst = None;
        let dimensions = ScoreDimensions::from_fn(|dimension| {
            let key = published_name(dimension);
            let row = recorded
                .iter()
                .find(|row| row["name"].as_str() == Some(key.as_str()));
            let density = row
                .and_then(|row| row["density_micro"].as_u64())
                .unwrap_or_default() as f64
                / 1_000_000.0;
            let tier = tier_of(row.and_then(|row| row["worst_tier"].as_str()));
            worst = worse_tier(worst, tier);
            let value = capped(
                density::dimension_score(density, dimension),
                tier.and_then(tier_dimension_ceiling),
            );
            assert_eq!(
                u64::from(value),
                row.and_then(|row| row["score"].as_u64()).unwrap_or_default(),
                "{name}: the recorded density of {key} does not explain the dimension the scan \
                 published"
            );
            value
        });

        let overall = capped(
            weighted_score(dimensions),
            tier_of(score["worst_tier"].as_str()).and_then(tier_overall_ceiling),
        );
        assert_eq!(
            u64::from(overall),
            score["value"].as_u64().unwrap_or_default(),
            "{name}: the recorded densities do not explain the score the scan published"
        );
        assert_eq!(
            worst.map(RuleTier::as_str),
            score["worst_tier"].as_str(),
            "{name}: the worst tier of the dimensions is not the one the score was capped by"
        );
        assert_eq!(
            score_label(overall).as_str(),
            score["label"].as_str().unwrap_or_default(),
            "{name}: the recorded label is not the one the recomputed score carries"
        );
        recomputed += 1;
    }

    assert_eq!(
        recomputed, 18,
        "the corpus scored {recomputed} repositories, and the measurement pins eighteen"
    );
}
