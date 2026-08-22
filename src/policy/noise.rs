//! What a rule costs on healthy public code, as the pinned corpus adjudicated
//! it.
//!
//! It sits beside the catalog rather than inside `RuleDefinition` because it is
//! not a property of the rule: it is a dated observation over ten public
//! repositories, and it changes when the corpus is re-adjudicated while the
//! rule does not. `tests/corpus.json` is where it is established and the tests
//! below hold the two in agreement.
//!
//! It is published at all because a measurement that lives only in a test file
//! cannot change what the tool recommends. The rule that fires the most is not
//! the rule worth fixing first, and until this table existed the report had no
//! way to tell the difference.
//!
//! What the table carries is the sample, not the rate. The rate the ranking
//! reads is derived from it through `SMOOTHING`, so the shipped discount and
//! the pseudo-counts it was computed with can never disagree: there is one
//! place holding the counts the corpus adjudicated, and one formula reading
//! them.

use super::by_id;

const BASIS_POINTS: u64 = 10_000;

/// Pseudo-counts every shipped rate is read through: one imagined false
/// positive over two imagined sites, which is the mean of the Beta(1,1)
/// posterior and therefore the uniform prior over the unit interval.
///
/// The raw share of false positives is the wrong key to rank by, in two
/// directions at once. A rule adjudicated wrong on the single site the corpus
/// ever showed it is not the same claim as one adjudicated wrong on forty, and
/// the raw rate states both as 10000 basis points; a rule the corpus never
/// adjudicated at all has no rate, and reading the absence as zero noise makes
/// measuring a rule an act that can only ever lower its rank. The posterior
/// mean answers both: it pulls a small sample toward the middle in proportion
/// to how small it is, and it lands an unmeasured rule at exactly the middle,
/// which is the same formula at no observations.
///
/// The Wilson lower bound was the other candidate and is the same inversion in
/// the other direction: it is negatively biased everywhere and structurally
/// punishes what nobody has measured. An empirical-Bayes prior fitted to the
/// corpus was rejected as a product decision this table does not take, since
/// the healthy and agent populations pool at rates far enough apart that
/// fitting one means choosing which population the shipped discount mirrors.
pub(crate) const SMOOTHING: Smoothing = Smoothing {
    false_positives: 1,
    sites: 2,
};

/// What the ranking retains for a rule the corpus never adjudicated, which is
/// `SMOOTHING` read at no observations rather than a second number chosen to
/// look like it.
pub(crate) const UNMEASURED_NOISE_BASIS_POINTS: u16 = SMOOTHING.rate(0, 0);

/// The pseudo-counts, and the one place that reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Smoothing {
    pub(crate) false_positives: u64,
    pub(crate) sites: u64,
}

impl Smoothing {
    /// The smoothed rate of a sample, in basis points, rounded half away from
    /// zero.
    ///
    /// Integer arithmetic throughout: the doubled numerator plus the
    /// denominator over twice the denominator is the half-away-from-zero
    /// rounding of a ratio of non-negative integers, and it answers the same
    /// on every platform, which a floating-point round never promises.
    pub(crate) const fn rate(&self, false_positives: u64, reviewed: u64) -> u16 {
        let numerator = (false_positives + self.false_positives) * BASIS_POINTS;
        let denominator = reviewed + self.sites;
        ((2 * numerator + denominator) / (2 * denominator)) as u16
    }
}

/// One rule the pinned corpus adjudicated on healthy code: the sites it looked
/// at, and how many of them the rule was wrong about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CorpusMeasurement {
    id: &'static str,
    false_positives: u64,
    reviewed: u64,
}

impl CorpusMeasurement {
    const fn new(id: &'static str, false_positives: u64, reviewed: u64) -> Self {
        Self {
            id,
            false_positives,
            reviewed,
        }
    }

    /// What the ranking discounts this rule by, derived rather than stored.
    pub(crate) const fn noise_basis_points(&self) -> u16 {
        SMOOTHING.rate(self.false_positives, self.reviewed)
    }

    /// How many sites the rate rests on, which is what separates a measurement
    /// from an anecdote and is published for that reason.
    pub(crate) const fn reviewed(&self) -> u64 {
        self.reviewed
    }
}

/// The sample the pinned corpus adjudicated per rule, sorted by id.
pub(crate) const CORPUS_NOISE: [CorpusMeasurement; 22] = [
    CorpusMeasurement::new("clippy::exit", 5, 5),
    CorpusMeasurement::new("clippy::expect_used", 4, 5),
    CorpusMeasurement::new("clippy::indexing_slicing", 40, 40),
    CorpusMeasurement::new("clippy::mem_forget", 1, 1),
    CorpusMeasurement::new("clippy::missing_safety_doc", 0, 2),
    CorpusMeasurement::new("clippy::panic", 5, 5),
    CorpusMeasurement::new("clippy::panic_in_result_fn", 5, 5),
    CorpusMeasurement::new("clippy::print_stderr", 5, 5),
    CorpusMeasurement::new("clippy::ptr_arg", 0, 1),
    CorpusMeasurement::new("clippy::rc_buffer", 0, 5),
    CorpusMeasurement::new("clippy::stable_sort_primitive", 0, 1),
    CorpusMeasurement::new("clippy::string_slice", 40, 40),
    CorpusMeasurement::new("clippy::too_many_arguments", 1, 1),
    CorpusMeasurement::new("clippy::unreachable", 4, 5),
    CorpusMeasurement::new("clippy::unwrap_used", 5, 5),
    CorpusMeasurement::new("rust_doctor::cargo::duplicate_major_versions", 0, 1),
    CorpusMeasurement::new("rust_doctor::cargo::unchecked_release_overflow", 1, 3),
    CorpusMeasurement::new("rust_doctor::structure::complex_function", 27, 31),
    CorpusMeasurement::new("rust_doctor::structure::duplicate_function_body", 8, 20),
    CorpusMeasurement::new(
        "rust_doctor::structure::near_duplicate_function_body",
        19,
        30,
    ),
    CorpusMeasurement::new("rust_doctor::structure::oversized_unit", 13, 39),
    CorpusMeasurement::new("rust_doctor::structure::unreferenced_feature", 5, 5),
];

/// What the corpus measured for this rule, or `None` when it never adjudicated
/// it.
///
/// An absent measurement is not a rate of zero and not a rate of ten thousand:
/// the ranking reads the absence itself and retains
/// `UNMEASURED_NOISE_BASIS_POINTS`, so measuring a rule can move it either way.
pub(crate) fn corpus_measurement(id: &str) -> Option<&'static CorpusMeasurement> {
    by_id(&CORPUS_NOISE, id, |measurement| measurement.id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::policy::find;

    /// The record, read as the bytes the repository ships.
    fn corpus() -> serde_json::Value {
        let corpus = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus.json"),
        )
        .expect("the published corpus should be readable");
        serde_json::from_str(&corpus).expect("the published corpus should parse")
    }

    /// The sample every measured rule of the record was adjudicated on.
    fn adjudicated(corpus: &serde_json::Value) -> BTreeMap<String, (u64, u64)> {
        corpus["precision"]
            .as_array()
            .expect("precision should be an array")
            .iter()
            .filter(|rule| rule["status"] == "measured")
            .map(|rule| {
                (
                    rule["id"].as_str().unwrap_or_default().to_owned(),
                    (
                        rule["false_positives"].as_u64().unwrap_or_default(),
                        rule["reviewed"].as_u64().unwrap_or_default(),
                    ),
                )
            })
            .collect()
    }

    /// The rate the score ranks by is the rate the corpus adjudicated.
    ///
    /// `CORPUS_NOISE` is a hand-written copy of what `tests/corpus.json`
    /// measures, and a copy that drifts is worse than no copy: the report would
    /// rank on a number nobody measured while every published artifact still
    /// showed the real one. This test is the only thing holding the two
    /// together, since the rate reaches the score without passing through the
    /// report.
    ///
    /// It compares the sample rather than the rate, because the sample is what
    /// the record and the table both hold: the discount is derived from it by
    /// `SMOOTHING` on both sides, so a table copied correctly and read through
    /// a moved constant fails in `the_smoothing_is_the_one_the_record_was_generated_under`
    /// instead of here.
    #[test]
    fn the_noise_the_score_ranks_by_matches_the_adjudicated_rate() {
        let corpus = corpus();
        let published = adjudicated(&corpus);
        let shipped: BTreeMap<String, (u64, u64)> = CORPUS_NOISE
            .iter()
            .map(|measurement| {
                (
                    measurement.id.to_owned(),
                    (measurement.false_positives, measurement.reviewed),
                )
            })
            .collect();

        assert_eq!(
            shipped, published,
            "the shipped noise table and the adjudicated samples disagree"
        );
        assert!(
            CORPUS_NOISE
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id),
            "the table is read by binary search, so it stays sorted and unique"
        );
        assert!(
            CORPUS_NOISE.iter().all(|measurement| find(measurement.id)
                .is_some()
                && measurement.false_positives <= measurement.reviewed
                && measurement.reviewed > 0),
            "every measured rule is catalogued and every sample is a real one"
        );
    }

    /// Every shipped discount is the smoothed rate of its own sample.
    ///
    /// The record publishes the raw share of false positives and keeps
    /// publishing it: it is the measurement, and a measurement is not a
    /// ranking key. What the table adds is the pseudo-counts, and this
    /// recomputes them from the record's counts rather than from the table's
    /// own arithmetic, so a rule whose sample moved without its discount
    /// following fails here naming both.
    #[test]
    fn every_shipped_rate_is_the_smoothed_rate_of_its_sample() {
        let corpus = corpus();
        for (id, (false_positives, reviewed)) in adjudicated(&corpus) {
            let measurement =
                corpus_measurement(&id).expect("every measured rule ships a measurement");
            let expected = SMOOTHING.rate(false_positives, reviewed);
            assert_eq!(
                measurement.noise_basis_points(),
                expected,
                "{id} is adjudicated {false_positives}/{reviewed} and ranks at {} rather than \
                 {expected} basis points",
                measurement.noise_basis_points()
            );
        }

        // The two ends of the resolution the smoothing exists to produce: forty
        // sites all wrong is not the same claim as one site wrong, and the raw
        // rate states both as ten thousand.
        assert_eq!(SMOOTHING.rate(40, 40), 9762);
        assert_eq!(SMOOTHING.rate(1, 1), 6667);
        // And one site right is not proof: the raw rate states it as zero.
        assert_eq!(SMOOTHING.rate(0, 1), 3333);
        assert_eq!(SMOOTHING.rate(0, 5), 1429);
        assert_eq!(
            corpus_measurement("rust_doctor::cargo::duplicate_major_versions")
                .map(CorpusMeasurement::noise_basis_points),
            Some(3333)
        );
        assert_eq!(corpus_measurement("clippy::todo"), None);
    }

    /// An unmeasured rule is ranked at the middle, and the middle is the same
    /// formula at no observations rather than a constant beside it.
    #[test]
    fn an_unmeasured_rule_reads_the_prior_itself() {
        assert_eq!(UNMEASURED_NOISE_BASIS_POINTS, 5000);
        assert_eq!(UNMEASURED_NOISE_BASIS_POINTS, SMOOTHING.rate(0, 0));
    }

    /// The record was generated under the smoothing the binary ships.
    ///
    /// Recorded beside the toolchain and the λ table for the reason both of
    /// those are: the toolchain decides which diagnostics exist, λ decides what
    /// they cost, and the pseudo-counts decide what a measured rate is worth to
    /// the ranking. A constant moved without regenerating the record fails
    /// here, naming both values, in the shape of
    /// `src/audit/tests/lambda_freeze.rs`.
    #[test]
    fn the_smoothing_is_the_one_the_record_was_generated_under() {
        let corpus = corpus();
        let recorded = &corpus["smoothing"];
        let false_positives = recorded["prior_false_positives"].as_u64();
        let sites = recorded["prior_sites"].as_u64();
        assert_eq!(
            (false_positives, sites),
            (Some(SMOOTHING.false_positives), Some(SMOOTHING.sites)),
            "the corpus was generated under a smoothing of {false_positives:?} false positives \
             over {sites:?} sites and the shipped constant is {} over {}, so every shipped rate \
             has to be regenerated with it",
            SMOOTHING.false_positives,
            SMOOTHING.sites
        );
    }
}
