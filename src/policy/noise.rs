//! What a rule costs on healthy public code, as the pinned corpus adjudicated
//! it.
//!
//! It sits beside the catalog rather than inside `RuleDefinition` because it is
//! not a property of the rule: it is a dated observation over ten public
//! repositories, and it changes when the corpus is re-adjudicated while the
//! rule does not. `tests/corpus.json` is where it is established and the test
//! below holds the two in agreement.
//!
//! It is published at all because a measurement that lives only in a test file
//! cannot change what the tool recommends. The rule that fires the most is not
//! the rule worth fixing first, and until this table existed the report had no
//! way to tell the difference.

use super::by_id;

/// Adjudicated false-positive rate of a rule on the pinned corpus, in basis
/// points, sorted by id.
pub(crate) const CORPUS_NOISE: [(&str, u16); 24] = [
    ("clippy::exit", 10000),
    ("clippy::expect_used", 8000),
    ("clippy::indexing_slicing", 10000),
    ("clippy::mem_forget", 10000),
    ("clippy::missing_safety_doc", 0),
    ("clippy::panic", 10000),
    ("clippy::panic_in_result_fn", 10000),
    ("clippy::print_stderr", 10000),
    ("clippy::ptr_arg", 0),
    ("clippy::rc_buffer", 0),
    ("clippy::stable_sort_primitive", 0),
    ("clippy::string_slice", 10000),
    ("clippy::too_many_arguments", 10000),
    ("clippy::unreachable", 8000),
    ("clippy::unwrap_used", 10000),
    ("rust_doctor::cargo::duplicate_major_versions", 0),
    ("rust_doctor::cargo::unchecked_release_overflow", 3333),
    ("rust_doctor::structure::complex_function", 8709),
    ("rust_doctor::structure::crate_level_allow", 10000),
    ("rust_doctor::structure::duplicate_function_body", 4000),
    ("rust_doctor::structure::near_duplicate_function_body", 8000),
    ("rust_doctor::structure::oversized_unit", 8000),
    ("rust_doctor::structure::unreasoned_allow_attribute", 10000),
    ("rust_doctor::structure::unreferenced_feature", 10000),
];

/// What the corpus measured for this rule, or `None` when it never adjudicated
/// it. An unmeasured rule carries no discount: absence of a measurement is not
/// evidence of noise.
pub(crate) fn corpus_noise(id: &str) -> Option<u16> {
    by_id(&CORPUS_NOISE, id, |(rule, _)| *rule).map(|(_, rate)| *rate)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::policy::find;

    /// The rate the score ranks by is the rate the corpus adjudicated.
    ///
    /// `CORPUS_NOISE` is a hand-written copy of what `tests/corpus.json`
    /// measures, and a copy that drifts is worse than no copy: the report would
    /// rank on a number nobody measured while every published artifact still
    /// showed the real one. This test is the only thing holding the two
    /// together, since the rate reaches the score without passing through the
    /// report.
    #[test]
    fn the_noise_the_score_ranks_by_matches_the_adjudicated_rate() {
        let corpus = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus.json"),
        )
        .expect("the published corpus should be readable");
        let corpus: serde_json::Value =
            serde_json::from_str(&corpus).expect("the published corpus should parse");

        let published: BTreeMap<&str, u64> = corpus["precision"]
            .as_array()
            .expect("precision should be an array")
            .iter()
            .filter(|rule| rule["status"] == "measured")
            .map(|rule| {
                (
                    rule["id"].as_str().unwrap_or_default(),
                    rule["false_positive_rate_basis_points"]
                        .as_u64()
                        .unwrap_or_default(),
                )
            })
            .collect();
        let shipped: BTreeMap<&str, u64> = CORPUS_NOISE
            .iter()
            .map(|(id, rate)| (*id, u64::from(*rate)))
            .collect();

        assert_eq!(
            shipped, published,
            "the shipped noise table and the adjudicated rates disagree"
        );
        assert!(
            CORPUS_NOISE.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "the table is read by binary search, so it stays sorted and unique"
        );
        assert!(
            CORPUS_NOISE
                .iter()
                .all(|(id, rate)| find(id).is_some() && *rate <= 10_000),
            "every measured rule is catalogued and every rate is a basis point"
        );
        assert_eq!(corpus_noise("clippy::exit"), Some(10000));
        assert_eq!(corpus_noise("clippy::todo"), None);
    }
}
