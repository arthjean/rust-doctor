#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use serde_json::Value;
use support::corpus::coefficients::coefficients;
use support::corpus::agreement::{
    AdjudicatedPair, Agreement, ENROLLED_SCOPES, Independence, PROTOCOL_JUDGE, PROTOCOL_JUDGES, Pass,
    ProtocolScope,
    SCHEMA_VERSION, agreement_defects, escalations_open, protocol_defects,
};
use support::corpus::position::PositionProof;
use support::corpus::sampling::{PROTOCOL_TARGET, SamplingPlan, sampling_defects, stride, target_floor};
use support::corpus::{
    Adjudication, CatalogRule, Observation, Population, PrecisionStatus, Provenance,
    RepositoryOutcome, ReviewedSite, RuleObservation, RuleTrigger, SiteContext, TriggerVerification,
    Verdict, artifact, artifact_path, precision, precision_of,
};

const RULE: &str = "clippy::probe";
const CUTOFF: &str = "2026-08-21";
const EVIDENCE_CUTOFF: &str = "2026-10-14";
const COMPLEX_FUNCTION: &str = "rust_doctor::structure::complex_function";

const ESCALATED: [(&str, &str, u64); 3] = [
    ("ripgrep", "crates/ignore/src/dir.rs", 340),
    ("thiserror", "impl/src/expand.rs", 31),
    ("thiserror", "impl/src/expand.rs", 221),
];

const ESCALATED_OVERSIZED: [(&str, &str, u64); 7] = [
    ("ripgrep", "crates/core/flags/hiargs.rs", 109),
    ("ripgrep", "crates/matcher/src/lib.rs", 1),
    ("ripgrep", "crates/searcher/src/searcher/core.rs", 39),
    ("serde_json", "src/map.rs", 1),
    ("serde_json", "src/value/de.rs", 1),
    ("serde_json", "src/value/mod.rs", 1),
    ("serde_json", "src/value/ser.rs", 1),
];

const ESCALATED_AGENT: [(&str, &str, u64); 2] = [
    (
        "vibesql",
        "crates/vibesql-bench-common/src/tpcc/data.rs",
        70,
    ),
    (
        "vibesql",
        "crates/vibesql-executor/src/select/executor/aggregation/window.rs",
        352,
    ),
];

const ESCALATED_OVERSIZED_AGENT: [(&str, &str, u64); 2] = [
    ("claudes-c-compiler", "src/backend/x86/codegen/intrinsics.rs", 22),
    ("vibesql", "crates/vibesql-ast/src/identifier.rs", 1),
];

const ESCALATED_UNSAFE: [(&str, &str, u64); 5] = [
    ("anyhow", "src/error.rs", 478),
    ("anyhow", "src/error.rs", 823),
    ("anyhow", "src/error.rs", 859),
    ("anyhow", "src/error.rs", 951),
    ("bytes", "src/buf/uninit_slice.rs", 60),
];

fn site(line: u64, verdict: Verdict) -> ReviewedSite {
    ReviewedSite {
        context: SiteContext::Production,
        family: None,
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
        evidence: Vec::new(),
        judge: judge.to_owned(),
        justification: "what the surrounding code established".to_owned(),
        verdict,
    }
}

fn pair(line: u64, first: Pass, second: Pass) -> AdjudicatedPair {
    AdjudicatedPair {
        adjudicated_on: None,
        context: SiteContext::Production,
        family: None,
        independence: Independence::SeparateContext,
        line,
        passes: [first, second],
        path: "src/lib.rs".to_owned(),
        population: Population::Healthy,
        repository: "probe".to_owned(),
        rule: RULE.to_owned(),
    }
}

fn agreeing(line: u64, verdict: Verdict) -> AdjudicatedPair {
    pair(line, pass("judge-a", verdict), pass("judge-b", verdict))
}

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
        evidence_cutoff: EVIDENCE_CUTOFF.to_owned(),
        position_proof: PositionProof {
            date: CUTOFF.to_owned(),
            digest: "probe".to_owned(),
            sites: 0,
            toolchain: "probe".to_owned(),
        },
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
        reasons: Vec::new(),
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

#[test]
fn the_artifact_declares_the_schema_version_the_harness_reads() {
    assert_eq!(artifact().schema_version, SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, 8);
}

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

#[test]
fn the_three_escalations_of_the_double_pass_are_findable_with_both_justifications() {
    let artifact = artifact();
    let pairs = &artifact.adjudication.agreement.pairs;
    for (repository, path, line) in ESCALATED {
        let found = pairs.iter().find(|pair| {
            pair.rule == COMPLEX_FUNCTION
                && pair.repository == repository
                && pair.path == path
                && pair.line == line
        });
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
                site.rule == COMPLEX_FUNCTION
                    && site.repository == repository
                    && site.path == path
                    && site.line == line
            }),
            "escalated site published as a verdict: {repository}/{path}:{line}"
        );
    }
}

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

        assert!(
            !artifact.adjudication.reviewed.iter().any(|site| {
                site.repository == repository && site.path == path && site.line == line
            }),
            "{repository}/{path}:{line} is escalated and published"
        );
    }
}

#[test]
fn the_seven_escalations_of_the_deepened_healthy_scope_are_findable() {
    let artifact = artifact();
    let rule = "rust_doctor::structure::oversized_unit";
    for (repository, path, line) in ESCALATED_OVERSIZED {
        let found = artifact.adjudication.agreement.pairs.iter().find(|pair| {
            pair.rule == rule
                && pair.population == Population::Healthy
                && pair.repository == repository
                && pair.path == path
                && pair.line == line
        });
        assert!(found.is_some(), "escalation absent: {repository}/{path}:{line}");
        let Some(found) = found else { continue };
        assert_eq!(found.context, SiteContext::Production);
        assert_eq!(found.independence, Independence::SeparateContext);
        assert!(!found.agrees());
        assert_eq!(found.verdict(), None);
        for judged in &found.passes {
            assert_eq!(judged.judge, PROTOCOL_JUDGE);
            assert!(!judged.justification.trim().is_empty());
        }

        assert!(
            !artifact.adjudication.reviewed.iter().any(|site| {
                site.rule == rule
                    && site.population == Population::Healthy
                    && site.repository == repository
                    && site.path == path
                    && site.line == line
            }),
            "{repository}/{path}:{line} is escalated and published"
        );
    }

    let split = artifact
        .adjudication
        .agreement
        .pairs
        .iter()
        .filter(|pair| {
            pair.rule == rule && pair.population == Population::Healthy && !pair.agrees()
        })
        .count();
    assert_eq!(split, ESCALATED_OVERSIZED.len());
}

#[test]
fn escalations_open_is_recomputed_from_the_disagreeing_pairs() {
    let artifact = artifact();
    let agreement = &artifact.adjudication.agreement;
    assert_eq!(agreement.escalations_open, escalations_open(agreement));
    assert_eq!(
        agreement.escalations_open,
        (ESCALATED.len()
            + ESCALATED_OVERSIZED.len()
            + ESCALATED_AGENT.len()
            + ESCALATED_OVERSIZED_AGENT.len()
            + ESCALATED_UNSAFE.len()) as u64
    );

    let mut forged = adjudication(vec![site(7, Verdict::TruePositive)], vec![disagreeing(9)]);
    forged.agreement.escalations_open = 0;
    let named = defect_naming(&agreement_defects(&forged), "escalations_open published");
    assert!(named.contains('0'), "{named}");
}

#[test]
fn the_escalations_of_the_core_v6_replay_are_findable_and_unpublished() {
    let artifact = artifact();
    for (repository, path, line) in ESCALATED_OVERSIZED_AGENT {
        let found = artifact
            .adjudication
            .agreement
            .pairs
            .iter()
            .find(|pair| pair.repository == repository && pair.path == path && pair.line == line);
        assert!(found.is_some(), "escalation absent: {repository}/{path}:{line}");
        let Some(found) = found else { continue };
        assert_eq!(found.rule, "rust_doctor::structure::oversized_unit");
        assert_eq!(found.population, Population::Agent);
        assert_eq!(found.adjudicated_on.as_deref(), Some("2026-10-29"));
        assert_eq!(found.verdict(), None);
        for judged in &found.passes {
            assert_eq!(judged.judge, PROTOCOL_JUDGES[1]);
            assert!(!judged.justification.trim().is_empty());
        }
        assert!(
            !artifact.adjudication.reviewed.iter().any(|site| {
                site.repository == repository && site.path == path && site.line == line
            }),
            "{repository}/{path}:{line} is escalated and published"
        );
    }
}

#[test]
fn the_five_escalations_of_the_undocumented_unsafe_scope_are_findable() {
    let artifact = artifact();
    let rule = "clippy::undocumented_unsafe_blocks";
    for (repository, path, line) in ESCALATED_UNSAFE {
        let found = artifact.adjudication.agreement.pairs.iter().find(|pair| {
            pair.rule == rule && pair.repository == repository && pair.path == path && pair.line == line
        });
        assert!(found.is_some(), "escalation absent: {repository}/{path}:{line}");
        let Some(found) = found else { continue };
        assert_eq!(found.population, Population::Healthy);
        assert_eq!(found.independence, Independence::SeparateContext);
        assert!(!found.agrees());
        for judged in &found.passes {
            assert_eq!(judged.judge, PROTOCOL_JUDGES[1]);
        }
        assert!(
            !artifact.adjudication.reviewed.iter().any(|site| {
                site.rule == rule && site.repository == repository && site.path == path && site.line == line
            }),
            "{repository}/{path}:{line} is escalated and published"
        );
    }
}

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

    for rate in artifact().precision {
        if ENROLLED_SCOPES
            .iter()
            .any(|scope| scope.0 == Population::Healthy && scope.1 == rate.id)
        {
            continue;
        }
        assert_eq!(rate.judges, Vec::<String>::new(), "{}", rate.id);
    }
}

#[test]
fn the_reviewed_site_keeps_the_only_provenance_it_can_prove() {
    let artifact = artifact();
    let mut counts: BTreeMap<Provenance, usize> = BTreeMap::new();
    for site in &artifact.adjudication.reviewed {
        *counts.entry(site.provenance).or_default() += 1;
    }
    assert_eq!(counts.get(&Provenance::Agent), Some(&399));
    assert_eq!(counts.get(&Provenance::Unrecorded), Some(&92));
    assert_eq!(counts.get(&Provenance::Human), None);
    assert_eq!(artifact.adjudication.reviewed.len(), 491);
}

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

#[test]
fn only_an_enrolled_scope_is_doubly_judged_and_every_site_of_one_is() {
    let artifact = artifact();
    assert_eq!(artifact.adjudication.protocol_cutoff, CUTOFF);
    assert_eq!(
        artifact.adjudication.adjudicated_after_cutoff,
        ENROLLED_SCOPES
            .iter()
            .map(|(population, rule)| ProtocolScope {
                population: *population,
                rule: (*rule).to_owned(),
            })
            .collect::<Vec<ProtocolScope>>()
    );
    assert!(protocol_defects(&artifact.adjudication).is_empty());

    let enrolled = |population: Population, id: &str| {
        ENROLLED_SCOPES
            .iter()
            .any(|scope| scope.0 == population && scope.1 == id)
    };
    for (population, precision) in [
        (Population::Healthy, &artifact.precision),
        (Population::Agent, &artifact.agent_population.precision),
    ] {
        for rate in precision {
            if enrolled(population, rate.id.as_str()) {
                assert_eq!(rate.doubly_judged, rate.reviewed, "{}", rate.id);
                assert!(!rate.judges.is_empty(), "{}", rate.id);
                assert!(
                    rate.judges.iter().all(|judge| PROTOCOL_JUDGES.contains(&judge.as_str())),
                    "{}",
                    rate.id
                );
            } else {
                assert_eq!(rate.doubly_judged, 0, "{}", rate.id);
            }
        }
    }
}

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

fn plan(observed: u64, target: u64) -> SamplingPlan {
    SamplingPlan {
        carried_over: Vec::new(),
        indices: stride(observed, target),
        observed,
        population: Population::Healthy,
        rule: RULE.to_owned(),
        target,
    }
}

#[test]
fn the_stride_selects_positions_across_the_ordered_population() {
    assert_eq!(stride(10, 4), vec![0, 2, 5, 7]);
    assert_eq!(stride(9, 3), vec![0, 3, 6]);
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

#[test]
fn a_plan_publishes_the_indices_its_own_stride_selects() {
    let honest = adjudication_planned(20, plan(25, 20));
    assert_eq!(sampling_defects(&honest), Vec::<String>::new());

    let mut forged = honest.clone();
    forged.sampling_plan[0].indices = vec![0, 1, 2];
    let named = defect_naming(&sampling_defects(&forged), "the stride selects");
    assert!(named.contains(RULE), "{named}");
}

#[test]
fn a_target_over_the_observed_population_is_named_with_the_rule() {
    let mut forged = adjudication_planned(20, plan(25, 20));
    forged.sampling_plan[0].target = 40;
    forged.sampling_plan[0].indices = stride(25, 40);
    let named = defect_naming(&sampling_defects(&forged), "over a population of 25");
    assert!(named.contains(RULE), "{named}");

    let mut empty = adjudication_planned(20, plan(25, 20));
    empty.sampling_plan[0].target = 0;
    empty.sampling_plan[0].indices = Vec::new();
    defect_naming(&sampling_defects(&empty), "publishes a target of zero");
}

#[test]
fn a_target_under_the_protocol_floor_is_named_with_the_rule() {
    let mut shallow = adjudication_planned(20, plan(25, 20));
    shallow.sampling_plan[0].target = 12;
    shallow.sampling_plan[0].indices = stride(25, 12);
    let named = defect_naming(&sampling_defects(&shallow), "below the floor of");
    assert!(named.contains(RULE), "{named}");
    assert!(named.contains("12"), "{named}");
    assert!(named.contains("20"), "{named}");

    let exhausted = adjudication_planned(9, plan(9, 9));
    assert_eq!(sampling_defects(&exhausted), Vec::<String>::new());
    assert_eq!(target_floor(9), 9);
    assert_eq!(target_floor(25), PROTOCOL_TARGET);
}

#[test]
fn a_plan_with_no_target_reads_at_the_protocol_target() {
    let honest = plan(25, PROTOCOL_TARGET);
    let mut value = serde_json::to_value(&honest).unwrap();
    value.as_object_mut().unwrap().remove("target");
    let read: SamplingPlan = serde_json::from_value(value).unwrap();
    assert_eq!(read.target, PROTOCOL_TARGET);
    assert_eq!(read, honest);
}

#[test]
fn the_sites_a_plan_selected_are_the_sites_the_record_adjudicated() {
    let escalating = {
        let reviewed: Vec<ReviewedSite> = (1..=19).map(|line| site(line, Verdict::TruePositive)).collect();
        let mut pairs: Vec<AdjudicatedPair> =
            (1..=19).map(|line| agreeing(line, Verdict::TruePositive)).collect();
        pairs.push(disagreeing(20));
        let mut record = adjudication(reviewed, pairs);
        record.adjudicated_after_cutoff = enrolled();
        record.sampling_plan = vec![plan(25, 20)];
        record
    };
    assert_eq!(escalating.agreement.escalations_open, 1);
    assert_eq!(escalating.reviewed.len(), 19);
    assert_eq!(sampling_defects(&escalating), Vec::<String>::new());

    let mut short = adjudication_planned(20, plan(25, 20));
    short.reviewed.pop();
    short.agreement.pairs.pop();
    let named = defect_naming(&sampling_defects(&short), "adjudicated 19 sites against the 20");
    assert!(named.contains(RULE), "{named}");
}

#[test]
fn a_plan_accounts_for_its_carry_over_as_well_as_its_stride() {
    let carried = {
        let mut drawn = plan(25, 20);
        drawn.carried_over = vec![4, 9];
        adjudication_planned(22, drawn)
    };
    assert_eq!(sampling_defects(&carried), Vec::<String>::new());

    let mut short = carried.clone();
    short.reviewed.pop();
    short.agreement.pairs.pop();
    let named = defect_naming(&sampling_defects(&short), "adjudicated 21 sites against the 22");
    assert!(named.contains(RULE), "{named}");
}

#[test]
fn a_carried_position_the_stride_also_draws_is_named() {
    let mut forged = adjudication_planned(20, plan(25, 20));
    forged.sampling_plan[0].carried_over = vec![3];
    let named = defect_naming(&sampling_defects(&forged), "which its own stride selects");
    assert!(named.contains(RULE), "{named}");

    let mut unsorted = adjudication_planned(22, plan(25, 20));
    unsorted.sampling_plan[0].carried_over = vec![9, 4];
    defect_naming(
        &sampling_defects(&unsorted),
        "carry-over that is not sorted and unique",
    );

    let mut outside = adjudication_planned(21, plan(25, 20));
    outside.sampling_plan[0].carried_over = vec![25];
    defect_naming(&sampling_defects(&outside), "outside a population of 25");
}

#[test]
fn an_enrolled_scope_with_no_sampling_plan_is_named_with_the_cutoff() {
    let mut record = adjudication_planned(20, plan(25, 20));
    record.sampling_plan.clear();
    let named = defect_naming(&sampling_defects(&record), "with no sampling plan");
    assert!(named.contains(CUTOFF), "{named}");
    assert!(named.contains(RULE), "{named}");
}

#[test]
fn two_plans_for_one_scope_are_named() {
    let mut record = adjudication_planned(20, plan(25, 20));
    record.sampling_plan.push(plan(25, 20));
    defect_naming(&sampling_defects(&record), "publishes two sampling plans");
}

#[test]
fn the_planned_scopes_are_the_enrolled_ones_and_nothing_else() {
    let artifact = artifact();
    let planned: Vec<(Population, &str)> = artifact
        .adjudication
        .sampling_plan
        .iter()
        .map(|plan| (plan.population, plan.rule.as_str()))
        .collect();
    assert_eq!(planned, ENROLLED_SCOPES.to_vec());
    for plan in &artifact.adjudication.sampling_plan {
        assert!(
            plan.target >= target_floor(plan.observed),
            "{} asks for {} below the floor of {}",
            plan.rule,
            plan.target,
            target_floor(plan.observed)
        );
        assert!(plan.target <= plan.observed, "{}", plan.rule);
        assert_eq!(plan.indices, stride(plan.observed, plan.target));
    }
    assert_eq!(
        sampling_defects(&artifact.adjudication),
        Vec::<String>::new()
    );

    let mut unenrolled = adjudication_planned(20, plan(25, 20));
    unenrolled.adjudicated_after_cutoff.clear();
    let named = defect_naming(&sampling_defects(&unenrolled), "is adjudicated before the cutoff");
    assert!(named.contains(RULE), "{named}");
    assert!(named.contains(CUTOFF), "{named}");
}

#[test]
fn a_plan_with_an_unknown_member_fails_deserialization() {
    let honest = plan(25, 20);
    let mut value = serde_json::to_value(&honest).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("method".to_owned(), Value::String("stride".to_owned()));
    let refused = serde_json::from_value::<SamplingPlan>(value);
    assert!(refused.is_err(), "{refused:?}");
}

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
