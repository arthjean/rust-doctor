#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! The record of a double pass, and what it is coupled to.
//!
//! Every proof here runs from the artifact alone: no clone cache, no network,
//! no environment variable. What a reproduction run has to confirm is where a
//! site lives; what a pair asserts is that two passes produced it and that the
//! published verdict is the one they agreed on, which the file answers by
//! itself.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use serde_json::Value;
use support::corpus::coefficients::coefficients;
use support::corpus::agreement::{
    AdjudicatedPair, Agreement, ENROLLED_RULES, Independence, PROTOCOL_JUDGE, Pass, ProtocolScope,
    SCHEMA_VERSION, agreement_defects, escalations_open, protocol_defects,
};
use support::corpus::sampling::{PROTOCOL_TARGET, SamplingPlan, sampling_defects, stride};
use support::corpus::{
    Adjudication, CatalogRule, Observation, Population, PrecisionStatus, Provenance,
    RepositoryOutcome, ReviewedSite, RuleObservation, RuleTrigger, SiteContext, TriggerVerification,
    Verdict, artifact, artifact_path, precision, precision_of,
};

const RULE: &str = "clippy::probe";
const CUTOFF: &str = "2026-08-21";
const COMPLEX_FUNCTION: &str = "rust_doctor::structure::complex_function";

/// The three sites the two passes of 2026-08-11 disagreed on.
const ESCALATED: [(&str, &str, u64); 3] = [
    ("ripgrep", "crates/ignore/src/dir.rs", 340),
    ("thiserror", "impl/src/expand.rs", 31),
    ("thiserror", "impl/src/expand.rs", 221),
];

/// The site the agent-population passes disagreed on, kept apart from the three
/// above because it is a different rule on a different population: one list
/// covering both would let a queue emptied on one side be refilled by the other
/// and still count.
const ESCALATED_AGENT: [(&str, &str, u64); 1] = [(
    "vibesql",
    "crates/vibesql-executor/src/select/executor/aggregation/window.rs",
    352,
)];

fn site(line: u64, verdict: Verdict) -> ReviewedSite {
    ReviewedSite {
        context: SiteContext::Production,
        justification: "read at the pinned revision with its surrounding code".to_owned(),
        line,
        path: "src/lib.rs".to_owned(),
        population: Population::Healthy,
        provenance: Provenance::Agent,
        repository: "probe".to_owned(),
        rule: RULE.to_owned(),
        verdict,
    }
}

fn pass(judge: &str, verdict: Verdict) -> Pass {
    Pass {
        judge: judge.to_owned(),
        justification: "what the surrounding code established".to_owned(),
        verdict,
    }
}

fn pair(line: u64, first: Pass, second: Pass) -> AdjudicatedPair {
    AdjudicatedPair {
        context: SiteContext::Production,
        independence: Independence::SeparateContext,
        line,
        passes: [first, second],
        path: "src/lib.rs".to_owned(),
        population: Population::Healthy,
        repository: "probe".to_owned(),
        rule: RULE.to_owned(),
    }
}

/// A pair both passes judged the same way.
fn agreeing(line: u64, verdict: Verdict) -> AdjudicatedPair {
    pair(line, pass("judge-a", verdict), pass("judge-b", verdict))
}

/// A pair the two passes split on, which is what an escalation is.
fn disagreeing(line: u64) -> AdjudicatedPair {
    pair(
        line,
        pass("judge-a", Verdict::TruePositive),
        pass("judge-b", Verdict::FalsePositive),
    )
}

fn adjudication(reviewed: Vec<ReviewedSite>, pairs: Vec<AdjudicatedPair>) -> Adjudication {
    let mut agreement = Agreement {
        coefficients: Vec::new(),
        escalations_open: pairs.iter().filter(|pair| !pair.agrees()).count() as u64,
        pairs,
    };
    agreement.coefficients = coefficients(&agreement);
    Adjudication {
        adjudicated_after_cutoff: Vec::new(),
        agreement,
        criterion: "probe".to_owned(),
        protocol_cutoff: CUTOFF.to_owned(),
        provenance: "probe".to_owned(),
        reviewed,
        sampling: "probe".to_owned(),
        sampling_plan: Vec::new(),
        trigger_verification: TriggerVerification {
            confirmed: 0,
            findings: 0,
            method: "probe".to_owned(),
            triggers: vec![RuleTrigger {
                evidence: "probe".to_owned(),
                rule: RULE.to_owned(),
            }],
        },
    }
}

fn enrolled() -> Vec<ProtocolScope> {
    vec![ProtocolScope {
        population: Population::Healthy,
        rule: RULE.to_owned(),
    }]
}

fn catalog_of() -> Vec<CatalogRule> {
    vec![CatalogRule {
        default_level: "warn".to_owned(),
        id: RULE.to_owned(),
        tier: "P2".to_owned(),
    }]
}

fn observations_of(findings: u64) -> Vec<Observation> {
    vec![Observation {
        authoritative: true,
        commit: "0".repeat(40),
        distinct: findings,
        exit_code: 0,
        findings_digest: String::new(),
        name: "probe".to_owned(),
        occurrences: findings,
        outcome: RepositoryOutcome::Processed,
        production_lines: 0,
        rules: vec![RuleObservation {
            distinct: findings,
            id: RULE.to_owned(),
            occurrences: findings,
        }],
        score: None,
        status: "complete".to_owned(),
    }]
}

fn defect_naming(defects: &[String], needle: &str) -> String {
    let named = defects.iter().find(|defect| defect.contains(needle));
    assert!(named.is_some(), "no defect naming {needle}: {defects:?}");
    named.cloned().unwrap_or_default()
}

/// The version is asserted, not merely deserialized.
///
/// Versions 1 through 3 were bumped by hand against no constant at all, so the
/// field cost nothing to move and proved nothing once moved. A schema change
/// that forgets the bump, or a bump with no schema change behind it, now fails
/// here rather than travelling as a number nobody reads.
#[test]
fn the_artifact_declares_the_schema_version_the_harness_reads() {
    assert_eq!(artifact().schema_version, SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, 4);
}

/// Exactly two passes, enforced by the shape.
#[test]
fn a_pair_holds_exactly_two_passes_under_a_closed_schema() {
    let two = serde_json::to_string(&agreeing(1, Verdict::TruePositive)).unwrap();
    serde_json::from_str::<AdjudicatedPair>(&two).unwrap();

    let one = two.replace(
        r#",{"judge":"judge-b","justification":"what the surrounding code established","verdict":"true_positive"}"#,
        "",
    );
    assert!(one != two, "the second pass should have been removed");
    assert!(serde_json::from_str::<AdjudicatedPair>(&one).is_err());

    let three = two.replace(
        r#""verdict":"true_positive"}]"#,
        r#""verdict":"true_positive"},{"judge":"judge-c","justification":"a third reading","verdict":"true_positive"}]"#,
    );
    assert!(serde_json::from_str::<AdjudicatedPair>(&three).is_err());

    let unknown = two.replace(r#""line":1"#, r#""line":1,"escalated":true"#);
    assert!(serde_json::from_str::<AdjudicatedPair>(&unknown).is_err());
}

/// `independence` is a closed vocabulary, so an unrecognized value is a
/// deserialization failure rather than a string nobody reads.
#[test]
fn an_unknown_independence_value_fails_deserialization() {
    let encoded = serde_json::to_string(&agreeing(1, Verdict::TruePositive)).unwrap();
    for value in ["separate_context", "separate_model"] {
        let accepted = encoded.replace(r#""independence":"separate_context""#, &format!(r#""independence":"{value}""#));
        serde_json::from_str::<AdjudicatedPair>(&accepted).unwrap();
    }
    let refused = encoded.replace(
        r#""independence":"separate_context""#,
        r#""independence":"same_session""#,
    );
    assert!(serde_json::from_str::<AdjudicatedPair>(&refused).is_err());
}

/// An agreeing pair has exactly one reviewed site carrying its verdict.
///
/// This is the half of the coupling that is intuitive, and the one a reader
/// checks: a verdict two passes produced has to be published somewhere.
#[test]
fn an_agreeing_pair_with_no_reviewed_site_is_named() {
    let record = adjudication(Vec::new(), vec![agreeing(7, Verdict::TruePositive)]);
    let defects = agreement_defects(&record);
    let named = defect_naming(&defects, "agreeing pair with no reviewed site");
    assert!(named.contains("probe/src/lib.rs:7"), "{named}");
}

#[test]
fn an_agreeing_pair_contradicting_its_reviewed_verdict_is_named() {
    let record = adjudication(
        vec![site(7, Verdict::FalsePositive)],
        vec![agreeing(7, Verdict::TruePositive)],
    );
    let named = defect_naming(&agreement_defects(&record), "agreeing pair judged");
    assert!(named.contains("TruePositive"), "{named}");
    assert!(named.contains("FalsePositive"), "{named}");
    assert!(named.contains("probe/src/lib.rs:7"), "{named}");
}

/// A disagreeing pair has no reviewed site, and that absence is escalation.
///
/// This is the half nobody checks by reflex, and the one that matters: a site
/// present here is a tie an agent broke, which is exactly the information the
/// second pass existed to produce.
#[test]
fn a_disagreeing_pair_carrying_a_published_verdict_is_named() {
    let record = adjudication(vec![site(7, Verdict::TruePositive)], vec![disagreeing(7)]);
    let named = defect_naming(
        &agreement_defects(&record),
        "escalated site carries a published verdict",
    );
    assert!(named.contains("probe/src/lib.rs:7"), "{named}");
}

#[test]
fn two_pairs_of_one_identity_are_refused_naming_it() {
    let record = adjudication(
        vec![site(7, Verdict::TruePositive)],
        vec![agreeing(7, Verdict::TruePositive), agreeing(7, Verdict::TruePositive)],
    );
    let named = defect_naming(&agreement_defects(&record), "two pairs share one identity");
    assert!(named.contains("probe/src/lib.rs:7"), "{named}");
}

/// An artifact with no pair yet is a valid artifact.
///
/// The protocol applies forward, so the record has to be empty and clean
/// before it is anything else: a shape that only validates once it is
/// populated forces the first pair and the schema to land together.
#[test]
fn an_empty_pair_list_carries_no_defect() {
    let record = adjudication(vec![site(7, Verdict::TruePositive)], Vec::new());
    assert_eq!(record.agreement.pairs, Vec::new());
    assert!(agreement_defects(&record).is_empty());
    assert!(protocol_defects(&record).is_empty());
}

#[test]
fn the_published_record_of_the_double_pass_carries_no_defect() {
    let artifact = artifact();
    assert_eq!(agreement_defects(&artifact.adjudication), Vec::<String>::new());
    assert_eq!(protocol_defects(&artifact.adjudication), Vec::<String>::new());
    assert_eq!(sampling_defects(&artifact.adjudication), Vec::<String>::new());
}

/// The three sites of 2026-08-11 are findable, with both verdicts and both
/// justifications.
///
/// They were excluded from `reviewed`, named in one sentence of one document
/// and in no field of the artifact, and open ten days later. Nothing in the
/// repository could tell a reader they existed.
#[test]
fn the_three_escalations_of_the_double_pass_are_findable_with_both_justifications() {
    let artifact = artifact();
    let pairs = &artifact.adjudication.agreement.pairs;
    for (repository, path, line) in ESCALATED {
        let found = pairs
            .iter()
            .find(|pair| pair.repository == repository && pair.path == path && pair.line == line);
        assert!(
            found.is_some(),
            "escalation absent from the record: {repository}/{path}:{line}"
        );
        let Some(found) = found else { continue };
        assert_eq!(found.rule, COMPLEX_FUNCTION);
        assert_eq!(found.population, Population::Healthy);
        assert_eq!(found.context, SiteContext::Production);
        assert!(!found.agrees(), "{repository}/{path}:{line}");
        assert_eq!(found.verdict(), None);
        let verdicts: BTreeSet<Verdict> =
            found.passes.iter().map(|pass| pass.verdict).collect();
        assert_eq!(verdicts.len(), 2, "{repository}/{path}:{line}");
        for judged in &found.passes {
            assert!(!judged.justification.trim().is_empty());
            assert!(!judged.judge.trim().is_empty());
        }
        assert!(
            !artifact.adjudication.reviewed.iter().any(|site| {
                site.repository == repository && site.path == path && site.line == line
            }),
            "escalated site published as a verdict: {repository}/{path}:{line}"
        );
    }
}

/// The escalation of the agent population is findable the same way.
///
/// One site of the twenty drawn for `duplicate_function_body` split on whether
/// a module note about the GROUP BY path covers the map builder beside the
/// validator it was written for. It stays out of `reviewed`, which is why that
/// rate rests on nineteen sites and not twenty, and it is not settled toward
/// the verdict the other nineteen carry.
#[test]
fn the_escalation_of_the_agent_population_is_findable_with_both_justifications() {
    let artifact = artifact();
    for (repository, path, line) in ESCALATED_AGENT {
        let found = artifact
            .adjudication
            .agreement
            .pairs
            .iter()
            .find(|pair| pair.repository == repository && pair.path == path && pair.line == line);
        assert!(found.is_some(), "escalation absent: {repository}/{path}:{line}");
        let Some(found) = found else { continue };
        assert_eq!(found.rule, "rust_doctor::structure::duplicate_function_body");
        assert_eq!(found.population, Population::Agent);
        assert_eq!(found.context, SiteContext::Production);
        assert_eq!(found.independence, Independence::SeparateContext);
        assert!(!found.agrees());
        assert_eq!(found.verdict(), None);
        for judged in &found.passes {
            assert_eq!(judged.judge, PROTOCOL_JUDGE);
            assert!(!judged.justification.trim().is_empty());
        }

        // Escalated means unpublished: a site the passes split on carries no
        // verdict in `reviewed`, which is the whole difference between an open
        // escalation and a settled one.
        assert!(
            !artifact.adjudication.reviewed.iter().any(|site| {
                site.repository == repository && site.path == path && site.line == line
            }),
            "{repository}/{path}:{line} is escalated and published"
        );
    }
}

/// The queue is a number a test recomputes, not a sentence in a document.
#[test]
fn escalations_open_is_recomputed_from_the_disagreeing_pairs() {
    let artifact = artifact();
    let agreement = &artifact.adjudication.agreement;
    assert_eq!(agreement.escalations_open, escalations_open(agreement));
    assert_eq!(
        agreement.escalations_open,
        (ESCALATED.len() + ESCALATED_AGENT.len()) as u64
    );

    let mut forged = adjudication(vec![site(7, Verdict::TruePositive)], vec![disagreeing(9)]);
    forged.agreement.escalations_open = 0;
    let named = defect_naming(&agreement_defects(&forged), "escalations_open published");
    assert!(named.contains('0'), "{named}");
}

/// Escalation is derived from a disagreement, never stored beside it.
///
/// A pair carrying two identical verdicts and a flag saying it is open would
/// let agreement and escalation both hold, and nothing in the file could say
/// which of the two was the lie. The flag does not exist: `deny_unknown_fields`
/// refuses one on the way in, and the count is recomputed on the way out.
#[test]
fn no_field_records_an_escalation() {
    let published = fs::read_to_string(artifact_path()).unwrap();
    assert!(!published.contains("\"escalated\""), "the artifact stores an escalation flag");
    let harness = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/corpus/agreement.rs"),
    )
    .unwrap();
    assert!(
        !harness.contains("escalated:"),
        "the harness declares an escalation field"
    );
}

/// Recording the escalations moves no rate.
///
/// The three sites were already out of the sample; entering `pairs` is what
/// makes their absence legible, and it must not make it arithmetic.
#[test]
fn the_escalation_record_leaves_the_complex_function_rate_untouched() {
    let artifact = artifact();
    let computed = precision(&artifact.catalog, &artifact.observations, &artifact.adjudication);
    let rate = computed
        .iter()
        .find(|rule| rule.id == COMPLEX_FUNCTION)
        .expect("complex_function is catalogued");
    assert_eq!(rate.status, PrecisionStatus::Measured);
    assert_eq!(rate.reviewed, 31);
    assert_eq!(rate.false_positives, Some(27));
    assert_eq!(rate.true_positives, Some(4));
    assert_eq!(rate.false_positive_rate_basis_points, Some(8709));
    assert_eq!(computed, artifact.precision);
}

#[test]
fn a_pair_with_an_empty_judge_is_named() {
    let record = adjudication(
        vec![site(7, Verdict::TruePositive)],
        vec![pair(
            7,
            pass("  ", Verdict::TruePositive),
            pass("judge-b", Verdict::TruePositive),
        )],
    );
    let named = defect_naming(&agreement_defects(&record), "pair with an empty judge");
    assert!(named.contains("probe/src/lib.rs:7"), "{named}");
}

/// Two passes of one model are not two models.
///
/// `separate_model` is the claim that the pair reduced self-preference bias
/// rather than merely variance. A pair naming one judge twice makes that claim
/// against its own evidence.
#[test]
fn a_pair_declaring_separate_model_with_one_judge_is_named() {
    let mut declared = agreeing(7, Verdict::TruePositive);
    declared.independence = Independence::SeparateModel;
    declared.passes[1].judge = declared.passes[0].judge.clone();
    let record = adjudication(vec![site(7, Verdict::TruePositive)], vec![declared]);
    let named = defect_naming(
        &agreement_defects(&record),
        "pair declared separate_model with one judge",
    );
    assert!(named.contains("judge-a"), "{named}");

    let mut apart = agreeing(7, Verdict::TruePositive);
    apart.independence = Independence::SeparateModel;
    let clean = adjudication(vec![site(7, Verdict::TruePositive)], vec![apart]);
    assert!(agreement_defects(&clean).is_empty());
}

/// A rate names the judges behind it, the way it already names the provenances.
///
/// Two passes of one model and two passes of two models produce the same
/// agreement figure and do not carry the same weight. A reader who cannot see
/// which of the two a rate rests on cannot discount it.
#[test]
fn a_published_rate_names_the_judges_behind_it() {
    let reviewed: Vec<ReviewedSite> = (1..=5).map(|line| site(line, Verdict::TruePositive)).collect();
    let pairs = vec![
        agreeing(1, Verdict::TruePositive),
        pair(
            2,
            pass("judge-c", Verdict::TruePositive),
            pass("judge-a", Verdict::TruePositive),
        ),
    ];
    let record = adjudication(reviewed, pairs);
    assert!(agreement_defects(&record).is_empty());
    let computed = precision_of(&catalog_of(), &observations_of(9), &record, Population::Healthy);
    let rate = computed.first().unwrap();
    assert_eq!(rate.status, PrecisionStatus::Measured);
    assert_eq!(rate.judges, vec!["judge-a", "judge-b", "judge-c"]);

    // No healthy rate rests on a pair, so every judge list on that side is
    // empty and says so rather than naming a judge nobody recorded. The agent
    // side is the other case, asserted where the enrolment is.
    for rate in artifact().precision {
        assert_eq!(rate.judges, Vec::<String>::new(), "{}", rate.id);
    }
}

/// `ReviewedSite` gains no field and `Provenance` no variant.
///
/// The hundred and ten verdicts judged before the field existed keep the only
/// truthful value they have. A pair is where the second pass goes, precisely so
/// that recording it costs the site list nothing.
#[test]
fn the_reviewed_site_keeps_the_only_provenance_it_can_prove() {
    let artifact = artifact();
    let mut counts: BTreeMap<Provenance, usize> = BTreeMap::new();
    for site in &artifact.adjudication.reviewed {
        *counts.entry(site.provenance).or_default() += 1;
    }
    assert_eq!(counts.get(&Provenance::Agent), Some(&222));
    assert_eq!(counts.get(&Provenance::Unrecorded), Some(&110));
    assert_eq!(counts.get(&Provenance::Human), None);
    assert_eq!(artifact.adjudication.reviewed.len(), 332);
}

/// A verdict produced under the protocol has a pair behind it, or the suite
/// says which of the two conditions it violated.
#[test]
fn a_site_adjudicated_after_the_cutoff_without_a_pair_is_named_with_the_cutoff() {
    let mut record = adjudication(vec![site(7, Verdict::TruePositive)], Vec::new());
    record.adjudicated_after_cutoff = enrolled();
    let named = defect_naming(&protocol_defects(&record), "with no pair behind it");
    assert!(named.contains(CUTOFF), "{named}");
    assert!(named.contains("probe/src/lib.rs:7"), "{named}");
}

#[test]
fn a_site_adjudicated_after_the_cutoff_on_a_disagreeing_pair_is_named_with_the_cutoff() {
    let mut record = adjudication(vec![site(7, Verdict::TruePositive)], vec![disagreeing(7)]);
    record.adjudicated_after_cutoff = enrolled();
    let named = defect_naming(&protocol_defects(&record), "whose passes disagree");
    assert!(named.contains(CUTOFF), "{named}");
    assert!(named.contains("probe/src/lib.rs:7"), "{named}");
}

#[test]
fn a_sample_adjudicated_under_the_protocol_passes_when_every_site_carries_a_pair() {
    let reviewed: Vec<ReviewedSite> = (1..=3).map(|line| site(line, Verdict::TruePositive)).collect();
    let pairs: Vec<AdjudicatedPair> = (1..=3).map(|line| agreeing(line, Verdict::TruePositive)).collect();
    let mut record = adjudication(reviewed, pairs);
    record.adjudicated_after_cutoff = enrolled();
    assert!(agreement_defects(&record).is_empty());
    assert!(protocol_defects(&record).is_empty());
}

/// Only an enrolled scope is judged under the protocol, and every site of one
/// carries a pair.
///
/// The two directions are one assertion. A rate outside the enrolment resting
/// on a double pass would be a protocol claimed over verdicts nobody enrolled;
/// a rate inside it whose sites are not all backed would be an enrolment
/// claimed over verdicts nobody judged twice. `protocol_defects` refuses the
/// second, and `doubly_judged` counted against the enrolment is what refuses
/// the first.
#[test]
fn only_an_enrolled_scope_is_doubly_judged_and_every_site_of_one_is() {
    let artifact = artifact();
    assert_eq!(artifact.adjudication.protocol_cutoff, CUTOFF);
    assert_eq!(
        artifact.adjudication.adjudicated_after_cutoff,
        ENROLLED_RULES
            .iter()
            .map(|rule| ProtocolScope {
                population: Population::Agent,
                rule: (*rule).to_owned(),
            })
            .collect::<Vec<ProtocolScope>>()
    );
    assert!(protocol_defects(&artifact.adjudication).is_empty());

    for rate in &artifact.precision {
        assert_eq!(rate.doubly_judged, 0, "{}", rate.id);
    }
    for rate in &artifact.agent_population.precision {
        if ENROLLED_RULES.contains(&rate.id.as_str()) {
            assert_eq!(rate.doubly_judged, rate.reviewed, "{}", rate.id);
            assert_eq!(rate.judges, vec![PROTOCOL_JUDGE.to_owned()], "{}", rate.id);
        } else {
            assert_eq!(rate.doubly_judged, 0, "{}", rate.id);
        }
    }
}

/// A rate counts the sites a pair backs, recomputed where the rate is.
#[test]
fn a_rate_counts_the_sites_a_pair_backs() {
    let reviewed: Vec<ReviewedSite> = (1..=5).map(|line| site(line, Verdict::TruePositive)).collect();
    let pairs = vec![
        agreeing(1, Verdict::TruePositive),
        agreeing(3, Verdict::TruePositive),
    ];
    let record = adjudication(reviewed, pairs);
    let computed = precision_of(&catalog_of(), &observations_of(9), &record, Population::Healthy);
    let rate = computed.first().unwrap();
    assert_eq!(rate.reviewed, 5);
    assert_eq!(rate.doubly_judged, 2);

    let none = adjudication(
        (1..=5).map(|line| site(line, Verdict::TruePositive)).collect(),
        Vec::new(),
    );
    let computed = precision_of(&catalog_of(), &observations_of(9), &none, Population::Healthy);
    assert_eq!(computed.first().unwrap().doubly_judged, 0);
}

/// A pair of one population never backs a site of the other.
#[test]
fn a_pair_of_one_population_never_backs_a_site_of_the_other() {
    let mut crossed = agreeing(1, Verdict::TruePositive);
    crossed.population = Population::Agent;
    let record = adjudication(vec![site(1, Verdict::TruePositive)], vec![crossed]);
    let named = defect_naming(&agreement_defects(&record), "agreeing pair with no reviewed site");
    assert!(named.contains("Agent"), "{named}");
    let computed = precision_of(&catalog_of(), &observations_of(9), &record, Population::Healthy);
    assert_eq!(computed.first().unwrap().doubly_judged, 0);
}

#[test]
fn a_cutoff_that_is_not_a_date_is_named() {
    let mut record = adjudication(vec![site(7, Verdict::TruePositive)], Vec::new());
    record.protocol_cutoff = "august 2026".to_owned();
    let named = defect_naming(&protocol_defects(&record), "protocol_cutoff is not a date");
    assert!(named.contains("august 2026"), "{named}");
}

#[test]
fn an_unsorted_or_repeated_enrolment_is_named() {
    let mut record = adjudication(Vec::new(), Vec::new());
    record.adjudicated_after_cutoff = vec![
        ProtocolScope {
            population: Population::Healthy,
            rule: RULE.to_owned(),
        },
        ProtocolScope {
            population: Population::Healthy,
            rule: RULE.to_owned(),
        },
    ];
    defect_naming(
        &protocol_defects(&record),
        "adjudicated_after_cutoff is not sorted and unique",
    );
}

// ---------------------------------------------------------------------------
// US-009: the draw, replayed rather than described
// ---------------------------------------------------------------------------

/// A plan drawing `target` sites out of `observed`, its indices computed.
///
/// Built through `stride` rather than with a literal list, because a helper
/// that states the indices is a helper that can agree with a wrong plan: what
/// the tests below forge is always the published list, never the arithmetic.
fn plan(observed: u64, target: u64) -> SamplingPlan {
    SamplingPlan {
        indices: stride(observed, target),
        observed,
        population: Population::Healthy,
        rule: RULE.to_owned(),
        target,
    }
}

/// The stride spreads the draw over the whole population and never past it.
#[test]
fn the_stride_selects_positions_across_the_ordered_population() {
    assert_eq!(stride(10, 4), vec![0, 2, 5, 7]);
    assert_eq!(stride(9, 3), vec![0, 3, 6]);
    // A target the population cannot supply draws the population, which is what
    // `k = min(target, n)` means and why an exhaustive sample is not a special
    // case anyone has to write down.
    assert_eq!(stride(3, 5), vec![0, 1, 2]);
    assert_eq!(stride(0, 5), Vec::<u64>::new());
    assert_eq!(stride(5, 0), Vec::<u64>::new());

    for (observed, target) in [(400_u64, 40_u64), (41, 40), (7, 1), (1, 1)] {
        let selected = stride(observed, target);
        assert_eq!(selected.len() as u64, target.min(observed));
        assert!(selected.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(selected.iter().all(|index| *index < observed));
    }
}

/// The published indices are the ones the stride selects, not a list beside it.
#[test]
fn a_plan_publishes_the_indices_its_own_stride_selects() {
    let honest = adjudication_planned(3, plan(9, 3));
    assert_eq!(sampling_defects(&honest), Vec::<String>::new());

    let mut forged = honest.clone();
    forged.sampling_plan[0].indices = vec![0, 1, 2];
    let named = defect_naming(&sampling_defects(&forged), "the stride selects");
    assert!(named.contains(RULE), "{named}");
}

/// A target the observed population cannot supply is named with the rule.
#[test]
fn a_target_over_the_observed_population_is_named_with_the_rule() {
    let mut forged = adjudication_planned(3, plan(9, 3));
    forged.sampling_plan[0].target = 40;
    forged.sampling_plan[0].indices = stride(9, 40);
    let named = defect_naming(&sampling_defects(&forged), "over a population of 9");
    assert!(named.contains(RULE), "{named}");

    let mut empty = adjudication_planned(3, plan(9, 3));
    empty.sampling_plan[0].target = 0;
    empty.sampling_plan[0].indices = Vec::new();
    defect_naming(&sampling_defects(&empty), "publishes a target of zero");
}

/// The sites the plan selected are the sites the record adjudicated.
///
/// An open escalation counts: the stride drew it, two passes judged it, and it
/// stays out of `reviewed` until a human settles it. Counting only the
/// published verdicts would report a scope with three escalations as a draw
/// three sites short of what it selected, which is the opposite of the fact.
#[test]
fn the_sites_a_plan_selected_are_the_sites_the_record_adjudicated() {
    let escalating = {
        let reviewed: Vec<ReviewedSite> = (1..=2).map(|line| site(line, Verdict::TruePositive)).collect();
        let mut pairs: Vec<AdjudicatedPair> =
            (1..=2).map(|line| agreeing(line, Verdict::TruePositive)).collect();
        pairs.push(disagreeing(3));
        let mut record = adjudication(reviewed, pairs);
        record.adjudicated_after_cutoff = enrolled();
        record.sampling_plan = vec![plan(9, 3)];
        record
    };
    assert_eq!(escalating.agreement.escalations_open, 1);
    assert_eq!(escalating.reviewed.len(), 2);
    assert_eq!(sampling_defects(&escalating), Vec::<String>::new());

    let mut short = adjudication_planned(3, plan(9, 3));
    short.reviewed.pop();
    short.agreement.pairs.pop();
    let named = defect_naming(&sampling_defects(&short), "adjudicated 2 sites against the 3");
    assert!(named.contains(RULE), "{named}");
}

/// A scope adjudicated under the protocol publishes its draw, or is named.
#[test]
fn an_enrolled_scope_with_no_sampling_plan_is_named_with_the_cutoff() {
    let mut record = adjudication_planned(3, plan(9, 3));
    record.sampling_plan.clear();
    let named = defect_naming(&sampling_defects(&record), "with no sampling plan");
    assert!(named.contains(CUTOFF), "{named}");
    assert!(named.contains(RULE), "{named}");
}

/// Two plans for one scope are two accounts of one draw.
#[test]
fn two_plans_for_one_scope_are_named() {
    let mut record = adjudication_planned(3, plan(9, 3));
    record.sampling_plan.push(plan(9, 3));
    defect_naming(&sampling_defects(&record), "publishes two sampling plans");
}

/// The scopes carrying a plan are the enrolled ones, and a scope adjudicated
/// before the cutoff carries none: the absence is the fact rather than a row of
/// zeros.
#[test]
fn the_planned_scopes_are_the_enrolled_ones_and_nothing_else() {
    let artifact = artifact();
    let planned: Vec<&str> = artifact
        .adjudication
        .sampling_plan
        .iter()
        .map(|plan| plan.rule.as_str())
        .collect();
    assert_eq!(planned, ENROLLED_RULES.to_vec());
    for plan in &artifact.adjudication.sampling_plan {
        assert_eq!(plan.population, Population::Agent);
        assert_eq!(plan.target, PROTOCOL_TARGET);
        assert_eq!(plan.indices, stride(plan.observed, plan.target));
    }
    assert_eq!(
        sampling_defects(&artifact.adjudication),
        Vec::<String>::new()
    );

    let mut unenrolled = adjudication_planned(3, plan(9, 3));
    unenrolled.adjudicated_after_cutoff.clear();
    let named = defect_naming(&sampling_defects(&unenrolled), "is adjudicated before the cutoff");
    assert!(named.contains(RULE), "{named}");
    assert!(named.contains(CUTOFF), "{named}");
}

/// A plan is a member of the record, refused at read time like the rest of it.
#[test]
fn a_plan_with_an_unknown_member_fails_deserialization() {
    let honest = plan(9, 3);
    let mut value = serde_json::to_value(&honest).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("method".to_owned(), Value::String("stride".to_owned()));
    let refused = serde_json::from_value::<SamplingPlan>(value);
    assert!(refused.is_err(), "{refused:?}");
}

/// A record whose enrolled scope carries `sites` agreeing pairs and one plan.
fn adjudication_planned(sites: u64, drawn: SamplingPlan) -> Adjudication {
    let reviewed: Vec<ReviewedSite> = (1..=sites).map(|line| site(line, Verdict::TruePositive)).collect();
    let pairs: Vec<AdjudicatedPair> = (1..=sites)
        .map(|line| agreeing(line, Verdict::TruePositive))
        .collect();
    let mut record = adjudication(reviewed, pairs);
    record.adjudicated_after_cutoff = enrolled();
    record.sampling_plan = vec![drawn];
    record
}
