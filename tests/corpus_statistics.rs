#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! What a published rate and a published agreement are worth.
//!
//! Every proof here runs from the artifact alone: no clone cache, no network,
//! no environment variable. A rate and an agreement are both statistics over a
//! sample, and until this crate existed the artifact published neither the
//! sample's precision nor its own recomputation of the agreement: twenty of the
//! twenty-four healthy rates rest on five sites or fewer, and the only
//! assertion on the agreement of the 2026-08-11 double pass was that the
//! sentence describing it ran past eighty characters.

mod support;

use std::collections::BTreeSet;

use serde_json::Value;
use support::corpus::agreement::{AdjudicatedPair, Agreement, Independence, Pass};
use support::corpus::coefficients::{
    Coefficient, KappaStatus, coefficient_defects, coefficients,
};
use support::corpus::comparison::{RateComparison, comparison_defects};
use support::corpus::interval::{Separation, interval_defects, separation, wilson_95};
use support::corpus::sampling::{PROTOCOL_TARGET, target_floor};
use support::corpus::{
    MINIMUM_REVIEWED_SITES, Population, PrecisionStatus, RulePrecision, SiteContext,
    THRESHOLD_BASIS_POINTS, Verdict, artifact, artifact_path, gate,
};

/// The rules the gate clears on a sample that separates them from nothing.
///
/// Frozen by name rather than by count: a rule leaving this list because its
/// sample was deepened is a measurement, and a rule entering it is a rate
/// published on a sample that cannot carry it. Both deserve to move an
/// assertion rather than a number.
const CLEARED_BLIND: [&str; 5] = [
    "clippy::missing_safety_doc",
    "clippy::ptr_arg",
    "clippy::rc_buffer",
    "clippy::stable_sort_primitive",
    "rust_doctor::cargo::duplicate_major_versions",
];

fn defect_naming(defects: &[String], needle: &str) -> String {
    let named = defects.iter().find(|defect| defect.contains(needle));
    assert!(named.is_some(), "no defect naming {needle}: {defects:?}");
    named.cloned().unwrap_or_default()
}

fn measured(precision: &[RulePrecision]) -> Vec<&RulePrecision> {
    precision
        .iter()
        .filter(|rule| rule.status == PrecisionStatus::Measured)
        .collect()
}

// ---------------------------------------------------------------------------
// US-005: the interval a rate rests on
// ---------------------------------------------------------------------------

/// Every published rate carries the bounds of its own sample.
///
/// Both populations, and both directions of the coupling: a rate with no bounds
/// is a rate nobody can weigh, and bounds with no rate are bounds around
/// nothing.
#[test]
fn every_published_rate_carries_the_interval_it_rests_on() {
    let artifact = artifact();
    for precision in [&artifact.precision, &artifact.agent_population.precision] {
        assert_eq!(
            interval_defects(precision, THRESHOLD_BASIS_POINTS),
            Vec::<String>::new()
        );
        let measured = measured(precision);
        assert!(!measured.is_empty(), "a population with no measured rate");
        for rule in measured {
            let low = rule.interval_low_basis_points.unwrap();
            let high = rule.interval_high_basis_points.unwrap();
            assert!(low <= high, "{} publishes [{low}, {high}]", rule.id);
            assert!(high <= 10_000, "{} publishes {high}", rule.id);
            assert!(rule.separation.is_some(), "{} settles nothing", rule.id);
        }
        for rule in precision
            .iter()
            .filter(|rule| rule.false_positive_rate_basis_points.is_none())
        {
            assert_eq!(rule.interval_low_basis_points, None, "{}", rule.id);
            assert_eq!(rule.interval_high_basis_points, None, "{}", rule.id);
            assert_eq!(rule.separation, None, "{}", rule.id);
        }
    }
}

/// A bound moved by hand fails, and the failure names the rule and the field.
#[test]
fn a_hand_edited_interval_is_refused_naming_the_rule_and_the_field() {
    let artifact = artifact();
    let target = measured(&artifact.precision)[0].clone();

    let mut forged = target.clone();
    forged.interval_low_basis_points = Some(target.interval_low_basis_points.unwrap() + 1);
    let named = defect_naming(
        &interval_defects(&[forged], THRESHOLD_BASIS_POINTS),
        "interval_low_basis_points",
    );
    assert!(named.contains(&target.id), "{named}");

    let mut forged = target.clone();
    forged.interval_high_basis_points = Some(0);
    let named = defect_naming(
        &interval_defects(&[forged], THRESHOLD_BASIS_POINTS),
        "interval_high_basis_points",
    );
    assert!(named.contains(&target.id), "{named}");

    let mut forged = target.clone();
    forged.separation = Some(match target.separation.unwrap() {
        Separation::Above => Separation::Below,
        _ => Separation::Above,
    });
    let named = defect_naming(
        &interval_defects(&[forged], THRESHOLD_BASIS_POINTS),
        "separation",
    );
    assert!(named.contains(&target.id), "{named}");

    let mut forged = target.clone();
    forged.false_positive_rate_basis_points = None;
    let named = defect_naming(
        &interval_defects(&[forged], THRESHOLD_BASIS_POINTS),
        "bounds with no rate",
    );
    assert!(named.contains(&target.id), "{named}");
}

/// One clean site bounds nothing, and the interval says so.
///
/// This is the whole reason the interval is Wilson and not Wald: at x = 0 the
/// Wald interval is the point `[0, 0]`, which states certainty on the strength
/// of a single observation. Three of the five rules the gate clears blind rest
/// on exactly that sample.
#[test]
fn a_single_clean_site_bounds_nothing_and_says_so() {
    let (low, high) = wilson_95(0, 1);
    assert_eq!(low, 0);
    assert!(high > 7_000, "one clean site published [{low}, {high}]");

    let artifact = artifact();
    let single = artifact
        .precision
        .iter()
        .find(|rule| rule.id == "clippy::ptr_arg")
        .expect("clippy::ptr_arg is catalogued");
    assert_eq!(single.reviewed, 1);
    assert_eq!(single.false_positives, Some(0));
    assert_eq!(single.false_positive_rate_basis_points, Some(0));
    assert!(
        single.interval_high_basis_points.unwrap() > 7_000,
        "{single:?}"
    );
    assert_eq!(single.separation, Some(Separation::Indecisive));
}

/// A sample with no denominator bounds nothing either, and is not `[0, 0]`.
#[test]
fn a_sample_with_no_denominator_spans_the_whole_scale() {
    assert_eq!(wilson_95(0, 0), (0, 10_000));
    assert_eq!(wilson_95(2, 1), (0, 10_000));
}

/// The separation is strict on both sides.
///
/// A bound landing exactly on the threshold places the rule at it, not under it
/// and not past it, which is the only reading that keeps `below` a positive
/// statement.
#[test]
fn a_bound_landing_on_the_threshold_settles_nothing() {
    let threshold = THRESHOLD_BASIS_POINTS;
    assert_eq!(separation(threshold, 9_000, threshold), Separation::Indecisive);
    assert_eq!(separation(0, threshold, threshold), Separation::Indecisive);
    assert_eq!(separation(threshold + 1, 9_000, threshold), Separation::Above);
    assert_eq!(separation(0, threshold - 1, threshold), Separation::Below);
    assert_eq!(separation(0, 10_000, threshold), Separation::Indecisive);
}

/// No float reaches the artifact, at any bound.
///
/// Read from the file rather than from the deserialized shape, because what a
/// `u64` field refuses is not what the record is: a bound written `7.9e3` would
/// fail to parse, and a bound written `7935.0` is what a generator emitting
/// floats produces. The record derives `Eq`, and a statistic two machines can
/// disagree on is a record neither can reproduce.
#[test]
fn every_published_bound_is_an_integer_basis_point() {
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(artifact_path()).unwrap()).unwrap();
    let mut bounds = 0;
    for block in ["precision", "agent_population"] {
        let rows = match block {
            "precision" => raw["precision"].as_array().unwrap(),
            _ => raw["agent_population"]["precision"].as_array().unwrap(),
        };
        for row in rows {
            for field in ["interval_low_basis_points", "interval_high_basis_points"] {
                match &row[field] {
                    Value::Null => {}
                    value => {
                        assert!(value.is_u64(), "{field} is not an integer: {value}");
                        let number = value.as_u64().unwrap_or_default();
                        assert!(number <= 10_000, "{field}: {number}");
                        bounds += 1;
                    }
                }
            }
        }
    }
    assert!(bounds > 0, "the artifact publishes no bound at all");

    for row in raw["adjudication"]["agreement"]["coefficients"]
        .as_array()
        .unwrap()
    {
        if let value @ Value::Number(_) = &row["kappa_basis_points"] {
            assert!(value.is_i64(), "kappa_basis_points is not an integer: {value}");
            let number = value.as_i64().unwrap_or_default();
            assert!((-10_000..=10_000).contains(&number), "{number}");
        }
    }
}

/// The published vocabulary of a separation is closed.
#[test]
fn an_unknown_separation_value_fails_deserialization() {
    for value in ["above", "below", "indecisive"] {
        serde_json::from_str::<Separation>(&format!("\"{value}\"")).unwrap();
    }
    assert!(serde_json::from_str::<Separation>("\"unclear\"").is_err());
}

// ---------------------------------------------------------------------------
// US-006: the agreement, recomputed rather than written
// ---------------------------------------------------------------------------

fn pass(judge: &str, verdict: Verdict) -> Pass {
    Pass {
        judge: judge.to_owned(),
        justification: "probe".to_owned(),
        verdict,
    }
}

fn pair(line: u64, rule: &str, first: Verdict, second: Verdict) -> AdjudicatedPair {
    AdjudicatedPair {
        context: SiteContext::Production,
        family: None,
        independence: Independence::SeparateContext,
        line,
        passes: [pass("judge-a", first), pass("judge-b", second)],
        path: "src/lib.rs".to_owned(),
        population: Population::Healthy,
        repository: "probe".to_owned(),
        rule: rule.to_owned(),
    }
}

/// Every maximal run of digits in a string, deduplicated.
///
/// Read off the raw text rather than off a parsed shape, because the point is
/// what a reader of the sentence sees: a number restated in prose is a second
/// copy of a computed figure, and the copy is the one no test compares.
fn numbers(prose: &str) -> BTreeSet<&str> {
    let mut found = BTreeSet::new();
    let mut rest = prose;
    while let Some(start) = rest.find(|character: char| character.is_ascii_digit()) {
        let tail = &rest[start..];
        let end = tail
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(tail.len());
        found.insert(&tail[..end]);
        rest = &tail[end..];
    }
    found
}

/// An agreement record whose coefficients are the ones its pairs support.
fn recorded(pairs: Vec<AdjudicatedPair>) -> Agreement {
    let mut agreement = Agreement {
        coefficients: Vec::new(),
        escalations_open: pairs.iter().filter(|pair| !pair.agrees()).count() as u64,
        pairs,
    };
    agreement.coefficients = coefficients(&agreement);
    agreement
}

/// One row per `(rule, population)` carrying at least one pair, recomputed.
#[test]
fn the_published_coefficients_are_the_coefficients_recomputed_from_the_pairs() {
    let artifact = artifact();
    let agreement = &artifact.adjudication.agreement;
    assert_eq!(coefficient_defects(agreement), Vec::<String>::new());
    assert!(
        !agreement.coefficients.is_empty(),
        "pairs are on record and no coefficient is published"
    );
    for row in &agreement.coefficients {
        assert!(row.pairs > 0, "{row:?}");
        assert_eq!(row.pairs, row.table.pairs(), "{row:?}");
        assert_eq!(row.agreed, row.table.agreed(), "{row:?}");
        assert!(row.agreed <= row.pairs, "{row:?}");
        assert_eq!(
            row.kappa_basis_points.is_some(),
            row.kappa_status == KappaStatus::Defined,
            "{row:?}"
        );
        assert!(
            (-10_000..=10_000).contains(&row.ac1_basis_points),
            "{row:?}"
        );
    }
}

/// A coefficient moved by hand fails, and the failure names the rule and the
/// field that moved.
#[test]
fn a_hand_edited_coefficient_is_refused_naming_the_rule_and_the_field() {
    let mut agreement = recorded(vec![
        pair(1, "clippy::probe", Verdict::FalsePositive, Verdict::FalsePositive),
        pair(2, "clippy::probe", Verdict::TruePositive, Verdict::TruePositive),
        pair(3, "clippy::probe", Verdict::TruePositive, Verdict::FalsePositive),
    ]);
    assert_eq!(coefficient_defects(&agreement), Vec::<String>::new());
    let honest = agreement.coefficients[0].clone();

    let forge = |agreement: &mut Agreement, edit: fn(&mut Coefficient)| -> Vec<String> {
        agreement.coefficients = vec![honest.clone()];
        edit(&mut agreement.coefficients[0]);
        coefficient_defects(agreement)
    };

    for (field, edit) in [
        (
            "ac1_basis_points",
            (|row: &mut Coefficient| row.ac1_basis_points += 1) as fn(&mut Coefficient),
        ),
        ("agreed", |row: &mut Coefficient| row.agreed += 1),
        ("pairs", |row: &mut Coefficient| row.pairs += 1),
        ("kappa_basis_points", |row: &mut Coefficient| {
            row.kappa_basis_points = Some(10_000)
        }),
        ("kappa_status", |row: &mut Coefficient| {
            row.kappa_status = KappaStatus::UndefinedNoVariance
        }),
        ("table", |row: &mut Coefficient| {
            row.table.both_true_positive += 1
        }),
    ] {
        let named = defect_naming(&forge(&mut agreement, edit), field);
        assert!(named.contains("clippy::probe"), "{named}");
    }
}

/// A pass that never varied leaves the coefficient undefined, and says why.
///
/// The unguarded formula tends toward 1.0 there, which states perfect agreement
/// on the strength of a rater that made no choice. This is not a hypothetical:
/// it is the case both Clippy rules of the 2026-08-11 run landed in.
#[test]
fn a_pass_with_no_variance_leaves_the_coefficient_undefined() {
    let agreement = recorded(vec![
        pair(1, "clippy::probe", Verdict::TruePositive, Verdict::TruePositive),
        pair(2, "clippy::probe", Verdict::TruePositive, Verdict::TruePositive),
        pair(3, "clippy::probe", Verdict::TruePositive, Verdict::TruePositive),
    ]);
    let row = &agreement.coefficients[0];
    assert_eq!(row.kappa_status, KappaStatus::UndefinedNoVariance);
    assert_eq!(row.kappa_basis_points, None);
    assert_eq!(row.agreed, 3);
    assert_eq!(row.pairs, 3);

    // The same absence of variance, with the two passes agreeing on nothing.
    let split = recorded(vec![
        pair(1, "clippy::probe", Verdict::TruePositive, Verdict::FalsePositive),
        pair(2, "clippy::probe", Verdict::TruePositive, Verdict::FalsePositive),
    ]);
    assert_eq!(
        split.coefficients[0].kappa_status,
        KappaStatus::UndefinedNoVariance
    );
    assert_eq!(split.coefficients[0].kappa_basis_points, None);
}

/// With variance on both sides the coefficient is a value, and it is signed.
#[test]
fn a_pass_with_variance_publishes_a_signed_coefficient() {
    let agreed = recorded(vec![
        pair(1, "clippy::probe", Verdict::FalsePositive, Verdict::FalsePositive),
        pair(2, "clippy::probe", Verdict::TruePositive, Verdict::TruePositive),
        pair(3, "clippy::probe", Verdict::FalsePositive, Verdict::FalsePositive),
        pair(4, "clippy::probe", Verdict::TruePositive, Verdict::TruePositive),
    ]);
    assert_eq!(agreed.coefficients[0].kappa_status, KappaStatus::Defined);
    assert_eq!(agreed.coefficients[0].kappa_basis_points, Some(10_000));

    // Two passes agreeing less often than chance predicts is a real outcome,
    // and a coefficient stored unsigned turns it into its own opposite.
    let opposed = recorded(vec![
        pair(1, "clippy::probe", Verdict::FalsePositive, Verdict::TruePositive),
        pair(2, "clippy::probe", Verdict::TruePositive, Verdict::FalsePositive),
        pair(3, "clippy::probe", Verdict::FalsePositive, Verdict::TruePositive),
        pair(4, "clippy::probe", Verdict::TruePositive, Verdict::FalsePositive),
    ]);
    assert_eq!(opposed.coefficients[0].kappa_status, KappaStatus::Defined);
    assert!(
        opposed.coefficients[0].kappa_basis_points.unwrap() < 0,
        "{:?}",
        opposed.coefficients[0]
    );
}

/// A `(rule, population)` with no pair gets no row, rather than a row of zeros.
#[test]
fn a_rule_with_no_pair_publishes_no_coefficient_row() {
    assert_eq!(coefficients(&recorded(Vec::new())), Vec::new());

    let agreement = recorded(vec![pair(
        1,
        "clippy::probe",
        Verdict::TruePositive,
        Verdict::TruePositive,
    )]);
    assert_eq!(agreement.coefficients.len(), 1);
    assert_eq!(agreement.coefficients[0].rule, "clippy::probe");

    // A row nothing backs is a published agreement over pairs that do not
    // exist, which is the forgery the recomputation exists to catch.
    let mut forged = agreement;
    let mut orphan = forged.coefficients[0].clone();
    orphan.rule = "clippy::absent".to_owned();
    forged.coefficients.push(orphan);
    let named = defect_naming(&coefficient_defects(&forged), "backed by no pair");
    assert!(named.contains("clippy::absent"), "{named}");

    // And pairs with no row is the other direction of the same coupling.
    let mut stripped = recorded(vec![pair(
        1,
        "clippy::probe",
        Verdict::TruePositive,
        Verdict::TruePositive,
    )]);
    stripped.coefficients.clear();
    let named = defect_naming(&coefficient_defects(&stripped), "no coefficient row");
    assert!(named.contains("clippy::probe"), "{named}");
}

/// The sampling prose states no quantity the coefficients block publishes.
///
/// The whole assertion on the agreement of the 2026-08-11 run used to be that
/// this sentence ran past eighty characters, and it stated two counts and a
/// coefficient the record could not back. Prose that restates a computed number
/// is a second copy of it, and the copy is the one no test compares.
#[test]
fn the_sampling_prose_states_no_agreement_quantity() {
    let artifact = artifact();
    let prose = artifact.adjudication.sampling.to_lowercase();
    for forbidden in ["kappa", "cohen", "gwet", "ac1", "agreement was", "agreed on"] {
        assert!(
            !prose.contains(forbidden),
            "the sampling prose states {forbidden}, which the coefficients block publishes"
        );
    }
    let bytes = prose.as_bytes();
    for window in bytes.windows(3) {
        assert!(
            !(window[0].is_ascii_digit() && window[1] == b'.' && window[2].is_ascii_digit()),
            "the sampling prose carries a decimal fraction, and every published statistic is an integer basis point"
        );
    }

    // Every number the prose is allowed to carry, named. A whitelist rather
    // than a list of forbidden words, because a quantity re-entering this
    // sentence arrives in whatever wording its author picks and only its
    // digits are predictable: "26 of 29" states an agreement as surely as
    // "agreed on 26", and only one of the two is a phrase anyone thought to
    // forbid. Each entry is a sampling quantity or a date, which is what this
    // field is for; an agreement quantity has no member here and cannot
    // acquire one without moving this assertion.
    assert_eq!(
        numbers(&prose),
        BTreeSet::from([
            "0", "08", "09", "11", "20", "2026", "241", "30", "31", "34", "40", "46", "5"
        ]),
        "the sampling prose carries a number this field is not allowed to state"
    );

    // The prose still describes the sampling itself, which is what the three
    // length assertions of `corpus_precision` hold it to.
    assert!(artifact.adjudication.sampling.len() > 80);
    assert!(artifact.adjudication.sampling.contains("stride"));
}

// ---------------------------------------------------------------------------
// US-008: a coefficient that does not collapse at high prevalence
// ---------------------------------------------------------------------------

/// An agreement whose table holds the four given cells, one pair per site.
///
/// Built from counts rather than from a list of verdicts, because what a
/// coefficient reads is the table and what a reader disagreeing with a
/// coefficient recomputes by hand is the table: a test that states the four
/// cells states the input of both figures at once.
fn tabled(
    both_false_positive: u64,
    both_true_positive: u64,
    first_only_false_positive: u64,
    second_only_false_positive: u64,
) -> Agreement {
    let mut pairs = Vec::new();
    let mut line = 0;
    let mut push = |count: u64, first: Verdict, second: Verdict, pairs: &mut Vec<AdjudicatedPair>| {
        for _ in 0..count {
            line += 1;
            pairs.push(pair(line, "clippy::probe", first, second));
        }
    };
    push(
        both_false_positive,
        Verdict::FalsePositive,
        Verdict::FalsePositive,
        &mut pairs,
    );
    push(
        both_true_positive,
        Verdict::TruePositive,
        Verdict::TruePositive,
        &mut pairs,
    );
    push(
        first_only_false_positive,
        Verdict::FalsePositive,
        Verdict::TruePositive,
        &mut pairs,
    );
    push(
        second_only_false_positive,
        Verdict::TruePositive,
        Verdict::FalsePositive,
        &mut pairs,
    );
    recorded(pairs)
}

/// AC1 is published wherever a pair is, kappa only where the passes varied.
///
/// Both behaviors are asserted on the same row, because the point is not that
/// two coefficients exist: it is that the row kappa has to decline is a row a
/// reader still gets an agreement figure for. The artifact's own row is that
/// case today, and the no-variance row on the other side of it is the one an
/// unguarded kappa would have published as perfect agreement.
#[test]
fn ac1_is_published_where_kappa_declines_on_the_same_pairs() {
    let artifact = artifact();
    let published = artifact
        .adjudication
        .agreement
        .coefficients
        .iter()
        .find(|row| row.kappa_status == KappaStatus::UndefinedNoVariance);
    let published = published.expect("the artifact publishes a row with no variance");
    assert_eq!(published.kappa_basis_points, None);
    assert!(published.pairs > 0, "{published:?}");
    // Two passes that agreed on nothing, which is an AC1 of minus one whole:
    // negative is representable, and it is the value the record actually
    // carries rather than a case invented to exercise the sign.
    assert_eq!(published.ac1_basis_points, -10_000, "{published:?}");

    let unanimous = tabled(0, 3, 0, 0);
    let row = &unanimous.coefficients[0];
    assert_eq!(row.kappa_status, KappaStatus::UndefinedNoVariance);
    assert_eq!(row.kappa_basis_points, None);
    assert_eq!(row.ac1_basis_points, 10_000, "{row:?}");
}

/// At high prevalence the same pairs read materially higher under AC1.
///
/// The 2026-08-11 shape: twenty-nine pairs, twenty-six agreed, most sites
/// judged the same way. Kappa reads that as moderate, which is the documented
/// prevalence paradox and the reason AC1 is published beside it. The ordering
/// is what is asserted, never the two values: the claim is that one coefficient
/// does not collapse where the other does, and freezing the figures would make
/// this test a copy of the arithmetic it is checking.
#[test]
fn a_prevalence_skewed_agreement_reads_higher_under_ac1_than_under_kappa() {
    let skewed = tabled(3, 23, 2, 1);
    let row = &skewed.coefficients[0];
    assert_eq!(row.pairs, 29, "{row:?}");
    assert_eq!(row.agreed, 26, "{row:?}");
    assert_eq!(row.kappa_status, KappaStatus::Defined, "{row:?}");
    let kappa = row.kappa_basis_points.unwrap_or_default();
    assert!(
        row.ac1_basis_points > kappa + 1_000,
        "AC1 {} does not read materially above kappa {kappa}",
        row.ac1_basis_points
    );

    // And where prevalence is balanced the two agree about the same pairs,
    // which is what says the gap above is the prevalence and not the formula.
    let balanced = tabled(13, 13, 2, 1);
    let row = &balanced.coefficients[0];
    let kappa = row.kappa_basis_points.unwrap_or_default();
    assert!(
        (row.ac1_basis_points - kappa).abs() < 1_000,
        "AC1 {} and kappa {kappa} disagree on a balanced table",
        row.ac1_basis_points
    );
}

/// A negative AC1 clamped by hand fails, naming the rule and the field.
#[test]
fn a_hand_clamped_negative_ac1_is_refused() {
    let mut opposed = tabled(0, 0, 0, 3);
    assert_eq!(opposed.coefficients[0].ac1_basis_points, -10_000);
    opposed.coefficients[0].ac1_basis_points = 0;
    let named = defect_naming(&coefficient_defects(&opposed), "ac1_basis_points");
    assert!(named.contains("clippy::probe"), "{named}");
}

/// A row that states an agreement and no AC1 is not a row this schema holds.
///
/// The field is not an `Option`, so the refusal is the deserialization itself
/// rather than a check written after it: a row holding a pair holds an AC1, and
/// the only way to publish one without is to invent a shape the reader rejects.
#[test]
fn a_coefficient_row_without_an_ac1_fails_deserialization() {
    let honest = &tabled(0, 2, 1, 0).coefficients[0];
    let mut value = serde_json::to_value(honest).unwrap();
    let object = value.as_object_mut().unwrap();
    assert!(object.remove("ac1_basis_points").is_some());
    let refused = serde_json::from_value::<Coefficient>(value);
    assert!(refused.is_err(), "{refused:?}");
}

// ---------------------------------------------------------------------------
// US-007: what the gate clears blind
// ---------------------------------------------------------------------------

/// The gate names the rules it admits on a sample that separates them from
/// nothing, and that list is not empty today.
#[test]
fn the_gate_names_the_rules_it_clears_blind() {
    let artifact = artifact();
    let recomputed = gate(&artifact.catalog, &artifact.precision, THRESHOLD_BASIS_POINTS);
    assert_eq!(recomputed.indecisive, artifact.gate.indecisive);
    assert_eq!(
        artifact.gate.indecisive,
        CLEARED_BLIND.map(str::to_owned).to_vec()
    );
    assert!(
        !artifact.gate.indecisive.is_empty(),
        "the gate clears every admitted rule on a sample that measures it"
    );

    let indecisive: BTreeSet<&str> = artifact.gate.indecisive.iter().map(String::as_str).collect();
    let noisy: BTreeSet<&str> = artifact
        .gate
        .noisy_on_healthy_code
        .iter()
        .map(String::as_str)
        .collect();
    let admitted: BTreeSet<&str> = artifact.gate.admitted.iter().map(String::as_str).collect();
    assert!(
        indecisive.is_disjoint(&noisy),
        "a rule the gate names noisy is not a rule it clears"
    );
    assert!(indecisive.is_subset(&admitted));

    // Every name on the list carries a measured rate whose interval spans the
    // threshold: the list is what the separations say, never a second opinion.
    for id in &artifact.gate.indecisive {
        let published = artifact.precision.iter().find(|rule| &rule.id == id);
        assert!(
            published.is_some(),
            "{id} is named indecisive and carries no rate"
        );
        if let Some(rule) = published {
            assert_eq!(rule.status, PrecisionStatus::Measured, "{rule:?}");
            assert_ne!(rule.separation, Some(Separation::Below), "{rule:?}");
        }
    }
    for rule in measured(&artifact.precision) {
        if rule.separation == Some(Separation::Below) {
            assert!(!indecisive.contains(rule.id.as_str()), "{rule:?}");
        }
    }
}

/// A rule whose interval clears the threshold is not named indecisive.
///
/// The list is what separates a measurement from an absence of one, so a rate
/// the sample actually places under the threshold has to leave it.
#[test]
fn a_rule_the_sample_places_under_the_threshold_is_not_cleared_blind() {
    let artifact = artifact();
    let mut precision = artifact.precision.clone();
    let target = artifact.gate.indecisive[0].clone();
    let deepened = precision
        .iter_mut()
        .find(|rule| rule.id == target)
        .expect("the named rule carries a rate");

    // A hundred clean sites out of a hundred findings: the same point estimate
    // on a sample that can carry it. Forty could not: at x = 0 the upper bound
    // is roughly z squared over n, so it takes seventy-four clean sites before
    // the interval clears five percent at all.
    deepened.findings = 100;
    deepened.reviewed = 100;
    deepened.false_positives = Some(0);
    deepened.true_positives = Some(100);
    let (low, high) = wilson_95(0, 100);
    deepened.interval_low_basis_points = Some(low);
    deepened.interval_high_basis_points = Some(high);
    deepened.separation = Some(separation(low, high, THRESHOLD_BASIS_POINTS));
    assert_eq!(deepened.separation, Some(Separation::Below));

    let outcome = gate(&artifact.catalog, &precision, THRESHOLD_BASIS_POINTS);
    assert!(!outcome.indecisive.contains(&target), "{:?}", outcome.indecisive);
    assert!(outcome.admitted.contains(&target));
    assert_eq!(outcome.indecisive.len(), artifact.gate.indecisive.len() - 1);
}

/// Naming a rate as blind never withholds it.
///
/// The interval annotates the admitted set the way the noisy list does: it says
/// what a rate is worth, it does not decide whether the rule ships.
#[test]
fn no_published_rate_is_withheld_by_what_its_interval_settles() {
    assert_eq!(MINIMUM_REVIEWED_SITES, 5);
    let artifact = artifact();
    for id in &artifact.gate.indecisive {
        let rule = artifact.precision.iter().find(|rule| &rule.id == id).unwrap();
        assert!(rule.false_positive_rate_basis_points.is_some(), "{rule:?}");
        assert_eq!(rule.status, PrecisionStatus::Measured, "{rule:?}");
    }
    assert_eq!(
        measured(&artifact.precision).len(),
        artifact
            .precision
            .iter()
            .filter(|rule| rule.false_positive_rate_basis_points.is_some())
            .count()
    );
}

// ---------------------------------------------------------------------------
// US-013: no structural rule of the agent population is left unjudged
// ---------------------------------------------------------------------------

/// The structural rules the agent population observes and has never judged.
///
/// Frozen by name, the way `CLEARED_BLIND` is and for the same reason: the
/// four rules that dominate this population's score carry a rate, and these
/// three are the debt that remains. A rule leaving this list is a measurement
/// and a rule entering it is a rate that went away, so both move an assertion
/// rather than a count nobody reads. Between them they account for a hundred
/// and twenty-five findings against the four thousand the measured rules do,
/// which is why the debt is named rather than blocking.
const UNJUDGED_ON_AGENT_CODE: [&str; 3] = [
    "rust_doctor::structure::crate_level_allow",
    "rust_doctor::structure::unreasoned_allow_attribute",
    "rust_doctor::structure::unreferenced_feature",
];

/// Every structural rule this population observes carries a verdict, or is
/// named in the frozen debt.
///
/// The failure names the offending rule, because the whole point of the list
/// is that a reader learns which rule went unjudged without diffing sixty-two
/// rows: a rate on the population the tool exists for is the number its score
/// ranks by, and a rule with no verdict there is ranked on a healthy-code
/// estimate of what it costs somewhere else.
#[test]
fn no_observed_structural_rule_of_the_agent_population_is_left_without_a_verdict() {
    let artifact = artifact();
    let debt: BTreeSet<&str> = UNJUDGED_ON_AGENT_CODE.into_iter().collect();

    let mut judged = 0usize;
    for rule in &artifact.agent_population.precision {
        if !rule.id.starts_with("rust_doctor::structure::") || rule.findings == 0 {
            continue;
        }
        if debt.contains(rule.id.as_str()) {
            assert_eq!(rule.reviewed, 0, "{} is judged and still named as debt", rule.id);
            continue;
        }
        assert!(
            rule.reviewed > 0,
            "{} is observed on agent-written code and judged by nothing",
            rule.id
        );
        judged += 1;
    }
    assert_eq!(judged, 6);

    // The debt is the debt of this population, never of the record: each of
    // the three carries a healthy rate, so what is missing is the second
    // answer and not the first.
    for id in UNJUDGED_ON_AGENT_CODE {
        let healthy = artifact.precision.iter().find(|rule| rule.id == id).unwrap();
        assert_eq!(healthy.status, PrecisionStatus::Measured, "{id}");
    }
}

// ---------------------------------------------------------------------------
// US-014: what the two rates of one rule say next to each other
// ---------------------------------------------------------------------------

/// The rules measured on both populations, in the order the comparison
/// publishes them.
const COMPARED: [&str; 4] = [
    "rust_doctor::structure::complex_function",
    "rust_doctor::structure::duplicate_function_body",
    "rust_doctor::structure::near_duplicate_function_body",
    "rust_doctor::structure::oversized_unit",
];

/// The comparison and the count are recomputed from the two precision lists.
#[test]
fn the_two_rates_of_a_rule_are_published_with_the_distance_between_them() {
    let artifact = artifact();
    let population = &artifact.agent_population;
    assert_eq!(
        comparison_defects(
            &artifact.precision,
            &population.precision,
            population.measured_rules,
            &population.rate_comparison,
        ),
        Vec::<String>::new()
    );

    assert_eq!(population.measured_rules, 7);
    assert_eq!(
        measured(&population.precision).len() as u64,
        population.measured_rules
    );
    let compared: Vec<&str> = population
        .rate_comparison
        .iter()
        .map(|row| row.rule.as_str())
        .collect();
    assert_eq!(compared, COMPARED.to_vec());

    // The gap is the finding, and on all four rules it points the same way:
    // every structural rule measured on both sides is wrong less often on
    // agent-written code than on the healthy corpus the score was calibrated
    // against. Asserted rather than described, because a sign that flips is
    // the one thing this row exists to make visible.
    for row in &population.rate_comparison {
        assert!(row.gap_basis_points < 0, "{}", row.rule);
        assert_eq!(
            row.gap_basis_points,
            i64::from(u32::try_from(row.agent_basis_points).unwrap())
                - i64::from(u32::try_from(row.healthy_basis_points).unwrap()),
            "{}",
            row.rule
        );
    }
}

/// A rule measured on one population alone carries no comparison row.
///
/// An absence is not a rate of nought: subtracting a rate the other population
/// never measured from the one it did would publish a gap the size of the
/// whole rate, pointing at a population that never observed the rule.
#[test]
fn a_rule_measured_on_one_population_alone_is_not_compared() {
    let artifact = artifact();
    let compared: BTreeSet<&str> = artifact
        .agent_population
        .rate_comparison
        .iter()
        .map(|row| row.rule.as_str())
        .collect();
    let one_sided: Vec<&str> = measured(&artifact.agent_population.precision)
        .iter()
        .map(|rule| rule.id.as_str())
        .filter(|id| !compared.contains(id))
        .collect();
    assert_eq!(
        one_sided,
        vec![
            "rust_doctor::cargo::missing_lockfile",
            "rust_doctor::structure::orphan_module_file",
            "rust_doctor::structure::stacked_allow_attribute",
        ]
    );
    for id in one_sided {
        let healthy = artifact.precision.iter().find(|rule| rule.id == id).unwrap();
        assert_ne!(healthy.status, PrecisionStatus::Measured, "{id}");
    }
}

/// A hand-edited comparison is refused, and the failure names the rule and the
/// field.
#[test]
fn a_hand_edited_comparison_is_refused_naming_the_rule_and_the_field() {
    let artifact = artifact();
    let population = &artifact.agent_population;
    let honest = population.rate_comparison.clone();

    for (mutate, needle) in [
        ((|row: &mut RateComparison| row.agent_basis_points += 1) as fn(&mut RateComparison), "publishes an agent rate of"),
        (|row| row.healthy_basis_points += 1, "publishes a healthy rate of"),
        (|row| row.gap_basis_points += 1, "publishes a gap of"),
    ] {
        let mut forged = honest.clone();
        mutate(&mut forged[0]);
        let defects = comparison_defects(
            &artifact.precision,
            &population.precision,
            population.measured_rules,
            &forged,
        );
        let named = defect_naming(&defects, needle);
        assert!(named.contains(COMPARED[0]), "{named}");
    }

    // And the count is the other half of the block, refused the same way.
    let defects = comparison_defects(
        &artifact.precision,
        &population.precision,
        population.measured_rules + 1,
        &honest,
    );
    defect_naming(&defects, "publishes measured_rules");
}

/// A sample that settles nothing is published as such rather than deepened.
///
/// `orphan_module_file` reports one false positive on agent-written code over
/// a production population of twelve, which is every site the population can
/// supply: the stride is the whole list, so there is no deeper sample to draw.
/// The interval that census supports still spans the threshold, and the
/// indecisive verdict is the published one, the rate beside it rather than
/// withheld. Drawing more sites until the bound moves is how a record ends up
/// publishing the sample size that produced the answer its author wanted, and
/// here it is not even available: what the scope has is what it observed.
#[test]
fn a_rate_the_sample_cannot_place_is_published_indecisive_rather_than_deepened() {
    let artifact = artifact();
    let rule = artifact
        .agent_population
        .precision
        .iter()
        .find(|rule| rule.id == "rust_doctor::structure::orphan_module_file")
        .unwrap();

    // Below the protocol target because the population is, which is the one
    // way a scope under this protocol reviews fewer sites than it asks for.
    // Compared against the scope's own target rather than against the constant:
    // the target is a floor, so a scope reviewed exhaustively publishes a
    // target above it, and only the target the plan recorded says what this
    // draw actually asked for.
    let plan = artifact
        .adjudication
        .sampling_plan
        .iter()
        .find(|plan| plan.rule == rule.id && plan.population == Population::Agent)
        .unwrap();
    assert!(rule.reviewed < PROTOCOL_TARGET);
    assert_eq!(plan.target, rule.reviewed);
    assert_eq!(plan.target, target_floor(plan.observed));
    assert_eq!(rule.false_positives, Some(1));
    assert_eq!(rule.false_positive_rate_basis_points, Some(833));
    assert_eq!(rule.separation, Some(Separation::Indecisive));
    let low = rule.interval_low_basis_points.unwrap();
    let high = rule.interval_high_basis_points.unwrap();
    assert!(low <= THRESHOLD_BASIS_POINTS && THRESHOLD_BASIS_POINTS <= high);
    assert_eq!((low, high), wilson_95(1, rule.reviewed));

    // The other three rates of the same draw separate, which is what says the
    // indecision is this rule's population and not the target the protocol
    // takes or the way the four were judged.
    for id in [
        "rust_doctor::structure::duplicate_function_body",
        "rust_doctor::structure::near_duplicate_function_body",
        "rust_doctor::structure::oversized_unit",
    ] {
        let other = artifact
            .agent_population
            .precision
            .iter()
            .find(|rule| rule.id == id)
            .unwrap();
        assert_eq!(other.separation, Some(Separation::Above), "{id}");
    }
}
