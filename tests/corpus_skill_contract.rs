#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use support::corpus::agreement::{AdjudicatedPair, Citation, Independence, Pass, ProtocolScope};
use support::corpus::coefficients::{Coefficient, ContingencyTable};
use support::corpus::sampling::{SamplingPlan, stride};
use support::corpus::{Population, Provenance, ReviewedSite, SiteContext, Verdict, artifact};

const NOT_FIELDS: [&str; 6] = [
    "i",
    "k",
    "n",
    "duplicate_function_body",
    "every_reviewed_structural_site_is_production_context",
    "the_published_observations_reproduce_the_pinned_corpus_run",
];

fn skill_text() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".claude/skills/corpus-adjudicate/SKILL.md");
    fs::read_to_string(&path).expect("the adjudication skill should be readable")
}

fn record() -> Value {
    let published = artifact();
    let mut cited = published.adjudication.agreement.pairs[0].clone();
    cited.adjudicated_on = Some(published.adjudication.evidence_cutoff.clone());
    for pass in &mut cited.passes {
        pass.evidence = vec![Citation {
            end_line: cited.line,
            path: cited.path.clone(),
            quote: "probe".to_owned(),
            start_line: cited.line,
        }];
    }
    let mut document = serde_json::to_value(published).unwrap();
    document["adjudication"]["agreement"]["pairs"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(cited).unwrap());
    let exemplar = SamplingPlan {
        carried_over: vec![4],
        indices: stride(9, 3),
        observed: 9,
        population: Population::Healthy,
        rule: "clippy::probe".to_owned(),
        target: 3,
    };
    document["adjudication"]["sampling_plan"] = json!([serde_json::to_value(exemplar).unwrap()]);
    document
}

fn keys(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Object(members) => {
            for (key, member) in members {
                found.insert(key.clone());
                keys(member, found);
            }
        }
        Value::Array(members) => {
            for member in members {
                keys(member, found);
            }
        }
        _ => {}
    }
}

fn vocabulary() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut push = |value: Value| {
        if let Value::String(word) = value {
            found.insert(word);
        }
    };
    for population in [Population::Agent, Population::Healthy] {
        push(serde_json::to_value(population).unwrap());
    }
    for provenance in [Provenance::Agent, Provenance::Human, Provenance::Unrecorded] {
        push(serde_json::to_value(provenance).unwrap());
    }
    for independence in [Independence::SeparateContext, Independence::SeparateModel] {
        push(serde_json::to_value(independence).unwrap());
    }
    for verdict in [Verdict::FalsePositive, Verdict::TruePositive] {
        push(serde_json::to_value(verdict).unwrap());
    }
    for context in [
        SiteContext::Benchmark,
        SiteContext::BuildScript,
        SiteContext::Example,
        SiteContext::Production,
        SiteContext::Tests,
    ] {
        push(serde_json::to_value(context).unwrap());
    }
    found
}

fn prose(text: &str) -> String {
    let mut kept = String::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if !inside {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    kept
}

fn json_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            match current.take() {
                Some(block) => blocks.push(block),
                None => {
                    if trimmed.trim_end() == "```json" {
                        current = Some(String::new());
                    }
                }
            }
            continue;
        }
        if let Some(block) = current.as_mut() {
            block.push_str(line);
            block.push('\n');
        }
    }
    blocks
}

fn identifiers(text: &str) -> Vec<String> {
    prose(text)
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| {
            !span.is_empty()
                && span.split('.').all(|segment| {
                    segment
                        .chars()
                        .next()
                        .is_some_and(|first| first.is_ascii_lowercase())
                        && segment
                            .chars()
                            .all(|character| character.is_ascii_lowercase()
                                || character.is_ascii_digit()
                                || character == '_')
                })
        })
        .map(str::to_owned)
        .collect()
}

fn resolves(document: &Value, path: &str) -> bool {
    let mut current = document;
    for segment in path.split('.') {
        if let Value::Array(members) = current {
            match members.first() {
                Some(first) => current = first,
                None => return false,
            }
        }
        match current.get(segment) {
            Some(next) => current = next,
            None => return false,
        }
    }
    true
}

fn naming_defects(text: &str) -> Vec<String> {
    let document = record();
    let mut published = BTreeSet::new();
    keys(&document, &mut published);
    let words = vocabulary();
    let exceptions: BTreeSet<&str> = NOT_FIELDS.into_iter().collect();

    let mut defects = Vec::new();
    for identifier in identifiers(text) {
        if exceptions.contains(identifier.as_str()) || words.contains(&identifier) {
            continue;
        }
        if identifier.contains('.') {
            if !resolves(&document, &identifier) {
                defects.push(format!("{identifier} resolves against nothing in the record"));
            }
        } else if !published.contains(&identifier) {
            defects.push(format!("{identifier} is no member of any shape of the record"));
        }
    }
    defects
}

fn example_defects(text: &str) -> Vec<String> {
    let mut defects = Vec::new();
    for block in json_blocks(text) {
        let Ok(value) = serde_json::from_str::<Value>(&block) else {
            defects.push(format!("a JSON example does not parse: {block}"));
            continue;
        };
        let accepted = serde_json::from_value::<ReviewedSite>(value.clone()).is_ok()
            || serde_json::from_value::<AdjudicatedPair>(value.clone()).is_ok()
            || serde_json::from_value::<Pass>(value.clone()).is_ok()
            || serde_json::from_value::<SamplingPlan>(value.clone()).is_ok()
            || serde_json::from_value::<ProtocolScope>(value.clone()).is_ok()
            || serde_json::from_value::<Coefficient>(value.clone()).is_ok()
            || serde_json::from_value::<ContingencyTable>(value).is_ok();
        if !accepted {
            defects.push(format!("a JSON example matches no shape of the record: {block}"));
        }
    }
    defects
}

#[test]
fn every_field_the_skill_names_exists_in_the_record() {
    let text = skill_text();
    assert!(identifiers(&text).len() >= 30, "{:?}", identifiers(&text));
    assert_eq!(naming_defects(&text), Vec::<String>::new());
}

#[test]
fn every_json_example_the_skill_prints_is_a_shape_the_record_holds() {
    let text = skill_text();
    assert!(json_blocks(&text).len() >= 3, "the skill prints no example");
    assert_eq!(example_defects(&text), Vec::<String>::new());
}

#[test]
fn a_renamed_field_fails_the_contract() {
    let text = skill_text();
    let renamed_path = text.replace("adjudication.sampling_plan", "adjudication.sampling_draw");
    assert_ne!(renamed_path, text);
    let named = naming_defects(&renamed_path);
    assert!(
        named.iter().any(|defect| defect.contains("sampling_draw")),
        "{named:?}"
    );

    let renamed_member = text.replace("`doubly_judged`", "`doubly_judged_sites`");
    assert_ne!(renamed_member, text);
    let named = naming_defects(&renamed_member);
    assert!(
        named
            .iter()
            .any(|defect| defect.contains("doubly_judged_sites")),
        "{named:?}"
    );

    let renamed_key = text.replace("\"indices\"", "\"positions\"");
    assert_ne!(renamed_key, text);
    assert!(!example_defects(&renamed_key).is_empty());

    let dropped = text.replace("\"population\": \"healthy\",\n", "");
    assert_ne!(dropped, text);
    assert!(!example_defects(&dropped).is_empty());
}

#[test]
fn the_skill_documents_the_pair_shape_the_judge_and_the_independence_values() {
    let text = skill_text();
    for named in [
        "adjudication.agreement.pairs",
        "adjudication.sampling_plan",
        "adjudication.adjudicated_after_cutoff",
        "adjudication.protocol_cutoff",
        "separate_context",
        "separate_model",
        "judge",
    ] {
        assert!(text.contains(named), "the skill never names {named}");
    }
}

#[test]
fn the_skill_asks_for_the_coefficients_to_recompute_rather_than_for_a_figure() {
    let text = skill_text();
    assert!(
        !text.to_lowercase().contains("kappa"),
        "the skill still asks for a kappa figure"
    );
    assert!(text.contains("cargo test --test corpus_statistics"));
    assert!(text.contains("recomputes\n`adjudication.agreement.coefficients` clean"));
}

#[test]
fn the_skill_leaves_an_escalation_to_a_human() {
    let text = skill_text();
    assert!(text.contains("**An escalation is never resolved by an\nagent**"));
    assert!(text.contains("stays out of `adjudication.reviewed` until a human verdict"));
}
