use super::by_id;

const BASIS_POINTS: u64 = 10_000;

pub(crate) const SMOOTHING: Smoothing = Smoothing {
    false_positives: 1,
    sites: 2,
};

pub(crate) const UNMEASURED_NOISE_BASIS_POINTS: u16 = SMOOTHING.rate(0, 0);

pub(crate) const WITHHOLD_NOISE_BASIS_POINTS: u16 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Smoothing {
    pub(crate) false_positives: u64,
    pub(crate) sites: u64,
}

impl Smoothing {
    pub(crate) const fn rate(&self, false_positives: u64, reviewed: u64) -> u16 {
        let numerator = (false_positives + self.false_positives) * BASIS_POINTS;
        let denominator = reviewed + self.sites;
        ((2 * numerator + denominator) / (2 * denominator)) as u16
    }
}

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

    pub(crate) const fn noise_basis_points(&self) -> u16 {
        SMOOTHING.rate(self.false_positives, self.reviewed)
    }

    pub(crate) const fn reviewed(&self) -> u64 {
        self.reviewed
    }
}

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
    CorpusMeasurement::new("clippy::undocumented_unsafe_blocks", 3, 15),
    CorpusMeasurement::new("clippy::unreachable", 4, 5),
    CorpusMeasurement::new("clippy::unwrap_used", 5, 5),
    CorpusMeasurement::new("rust_doctor::cargo::unchecked_release_overflow", 1, 3),
    CorpusMeasurement::new("rust_doctor::structure::complex_function", 27, 31),
    CorpusMeasurement::new("rust_doctor::structure::duplicate_function_body", 6, 18),
    CorpusMeasurement::new(
        "rust_doctor::structure::near_duplicate_function_body",
        18,
        29,
    ),
    CorpusMeasurement::new("rust_doctor::structure::oversized_unit", 13, 39),
    CorpusMeasurement::new("rust_doctor::structure::unreferenced_feature", 5, 5),
];

pub(crate) fn corpus_measurement(id: &str) -> Option<&'static CorpusMeasurement> {
    by_id(&CORPUS_NOISE, id, |measurement| measurement.id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::policy::find;

    fn corpus() -> serde_json::Value {
        let corpus = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus.json"),
        )
        .expect("the published corpus should be readable");
        serde_json::from_str(&corpus).expect("the published corpus should parse")
    }

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

        assert_eq!(SMOOTHING.rate(40, 40), 9762);
        assert_eq!(SMOOTHING.rate(1, 1), 6667);
        assert_eq!(SMOOTHING.rate(0, 1), 3333);
        assert_eq!(SMOOTHING.rate(0, 5), 1429);
        assert_eq!(
            corpus_measurement("clippy::ptr_arg")
                .map(CorpusMeasurement::noise_basis_points),
            Some(3333)
        );
        assert_eq!(corpus_measurement("clippy::todo"), None);
    }

    #[test]
    fn no_p0_or_p1_rule_is_withheld() {
        let withheld: Vec<(&str, u16)> = CORPUS_NOISE
            .iter()
            .filter(|measurement| {
                find(measurement.id).is_some_and(|rule| {
                    matches!(rule.tier, crate::policy::RuleTier::P0 | crate::policy::RuleTier::P1)
                })
            })
            .map(|measurement| (measurement.id, measurement.noise_basis_points()))
            .filter(|(_, noise)| *noise > WITHHOLD_NOISE_BASIS_POINTS)
            .collect();
        assert!(
            withheld.is_empty(),
            "these P0 or P1 rules are smoothed above {WITHHOLD_NOISE_BASIS_POINTS} basis points \
             and would stop capping: {withheld:?}"
        );
    }

    #[test]
    fn an_unmeasured_rule_reads_the_prior_itself() {
        assert_eq!(UNMEASURED_NOISE_BASIS_POINTS, 5000);
        assert_eq!(UNMEASURED_NOISE_BASIS_POINTS, SMOOTHING.rate(0, 0));
    }

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
