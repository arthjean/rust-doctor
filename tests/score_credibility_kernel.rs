#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! End-to-end proofs of the `core-v2` model: tier capping, occurrence steps,
//! reconciled counts and schema contract.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use rust_doctor::{
    InspectRequest, RuleTier, SCHEMA_VERSION, ScoreLabel, Status, Summary, inspect, render,
};
use serde_json::Value;

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn adversarial() -> PathBuf {
    fixture("tests/fixtures/score-credibility/adversarial")
}

fn json_report(path: &Path) -> Value {
    let report = inspect(InspectRequest::new(path));
    serde_json::to_value(&report).expect("a valid report should serialize")
}

/// US-064: a single `P0` finding caps the score, whatever the average.
#[test]
fn the_adversarial_fixture_cannot_reach_the_top_label() {
    let report = inspect(InspectRequest::new(adversarial()));
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    let score = report.audit.score.as_ref().expect("score should exist");

    assert!(
        score.value <= 40,
        "score {} should stay capped",
        score.value
    );
    assert_ne!(score.label, ScoreLabel::Great);
    assert_eq!(score.label, ScoreLabel::Critical);
    assert!(score.authoritative);
    assert_eq!(score.worst_tier, Some(RuleTier::P0));
    assert_eq!(score.applied_ceiling, Some(40));
    assert_eq!(score.model, "core-v2");

    // The dimension of the P0 finding is capped, so is the P1 one, and the
    // dimension carrying only a P3 keeps its additive score.
    assert_eq!(score.dimensions.security, 20);
    assert_eq!(score.dimensions.reliability, 50);
    assert!(score.dimensions.maintainability > 75);

    let terminal = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--yes")
        .arg(adversarial())
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .expect("rust-doctor should start");
    let terminal = String::from_utf8(terminal.stdout).expect("terminal output should be UTF-8");
    assert!(
        terminal.contains("Capped at 40/100 by a P0 finding"),
        "{terminal}"
    );
    assert!(
        terminal.contains("rust_doctor::source::dynamic_shell_command"),
        "{terminal}"
    );
    assert!(terminal.contains("40 / 100 Critical"), "{terminal}");
}

/// US-063: the tier is published per rule and stays distinct from the effective
/// level.
#[test]
fn every_published_rule_carries_a_closed_tier() {
    let report = json_report(&adversarial());
    let rules = report["policy"]["rules"].as_array().expect("policy rules");
    assert_eq!(rules.len(), 49);

    let mut blocking = Vec::new();
    for rule in rules {
        let tier = rule["tier"]
            .as_str()
            .expect("each rule should carry a tier");
        assert!(
            ["P0", "P1", "P2", "P3"].contains(&tier),
            "unexpected tier {tier}"
        );
        assert_eq!(rule["level"], "warn", "{}", rule["id"]);
        if tier == "P0" {
            blocking.push(rule["id"].as_str().unwrap().to_owned());
        }
    }
    assert_eq!(
        blocking,
        [
            "rust_doctor::source::disabled_tls_verification",
            "rust_doctor::source::dynamic_shell_command",
        ]
    );

    // The published severities do not move with the tier: every catalog rule
    // stays at `warn` by default, hence at effective `warning`.
    for diagnostic in report["diagnostics"].as_array().unwrap() {
        assert_eq!(diagnostic["base_severity"], "warning", "{diagnostic}");
        assert_eq!(diagnostic["severity"], "warning", "{diagnostic}");
    }
}

/// US-065: the penalty of every rule recomputes from the report alone.
#[test]
fn the_score_is_reproducible_from_the_published_report() {
    let report = json_report(&adversarial());

    let tiers: BTreeMap<_, _> = report["policy"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| {
            (
                rule["id"].as_str().unwrap().to_owned(),
                rule["tier"].as_str().unwrap().to_owned(),
            )
        })
        .collect();

    let mut penalties: BTreeMap<&str, (u64, Option<&str>)> = BTreeMap::new();
    let mut occurrences: BTreeMap<&str, usize> = BTreeMap::new();
    let mut severities: BTreeMap<&str, &str> = BTreeMap::new();
    let mut dimensions: BTreeMap<&str, &str> = BTreeMap::new();
    for diagnostic in report["diagnostics"].as_array().unwrap() {
        let Some(code) = diagnostic["code"].as_str() else {
            continue;
        };
        let Some(dimension) = dimension_of(diagnostic["category"].as_str()) else {
            continue;
        };
        *occurrences.entry(code).or_default() +=
            usize::try_from(diagnostic["occurrences"].as_u64().unwrap()).unwrap();
        let severity = diagnostic["severity"].as_str().unwrap();
        let current = severities.entry(code).or_insert(severity);
        if severity_quarters(severity) > severity_quarters(current) {
            *current = severity;
        }
        dimensions.insert(code, dimension);
    }
    for (code, severity) in severities {
        let penalty = severity_quarters(severity) * occurrence_multiplier(occurrences[code]);
        let dimension = dimensions[code];
        let entry = penalties.entry(dimension).or_insert((0, None));
        entry.0 += penalty;
        // The worst tier of a dimension is the smallest published name, `P0`
        // first. A rule outside the catalog carries none and caps nothing.
        if let Some(tier) = tiers.get(code).map(String::as_str)
            && entry.1.is_none_or(|current| tier < current)
        {
            entry.1 = Some(tier);
        }
    }

    const WEIGHTS: [(&str, u64); 5] = [
        ("security", 4),
        ("reliability", 3),
        ("maintainability", 2),
        ("performance", 2),
        ("dependencies", 2),
    ];
    let published = &report["audit"]["score"]["dimensions"];
    for (dimension, _) in WEIGHTS {
        let (penalty, tier) = penalties.get(dimension).copied().unwrap_or((0, None));
        let additive = ((400 - penalty + 2) / 4).min(100);
        let expected = tier
            .and_then(dimension_ceiling)
            .map_or(additive, |ceiling| additive.min(ceiling));
        assert_eq!(
            published[dimension].as_u64().unwrap(),
            expected,
            "dimension {dimension}"
        );
    }

    let numerator: u64 = WEIGHTS
        .into_iter()
        .map(|(dimension, weight)| published[dimension].as_u64().unwrap() * weight)
        .sum();
    let weighted = ((numerator + 6) / 13).min(100);
    let worst = penalties.values().filter_map(|(_, tier)| *tier).min();
    let expected = worst
        .and_then(overall_ceiling)
        .map_or(weighted, |ceiling| weighted.min(ceiling));
    assert_eq!(
        report["audit"]["score"]["value"].as_u64().unwrap(),
        expected
    );
    assert_eq!(
        report["audit"]["score"]["worst_tier"].as_str(),
        worst,
        "the published worst tier should match the recomputed one"
    );
}

fn dimension_of(category: Option<&str>) -> Option<&'static str> {
    match category? {
        "security" => Some("security"),
        "correctness" | "reliability" => Some("reliability"),
        "maintainability" => Some("maintainability"),
        "performance" => Some("performance"),
        "cargo" | "dependencies" => Some("dependencies"),
        _ => None,
    }
}

fn severity_quarters(severity: &str) -> u64 {
    match severity {
        "error" => 6,
        "warning" => 3,
        "info" => 1,
        _ => 0,
    }
}

fn occurrence_multiplier(occurrences: usize) -> u64 {
    match occurrences {
        0 | 1 => 1,
        2..=5 => 2,
        6..=20 => 3,
        _ => 4,
    }
}

fn dimension_ceiling(tier: &str) -> Option<u64> {
    match tier {
        "P0" => Some(20),
        "P1" => Some(50),
        "P2" => Some(75),
        _ => None,
    }
}

fn overall_ceiling(tier: &str) -> Option<u64> {
    match tier {
        "P0" => Some(40),
        "P1" => Some(65),
        _ => None,
    }
}

/// US-066: both quantities are named and agree across the surfaces.
#[test]
fn both_magnitudes_are_named_and_reconciled() {
    let report = json_report(&adversarial());
    let summary = &report["summary"];
    let categories = report["audit"]["categories"].as_array().unwrap();

    for magnitude in ["distinct", "occurrences"] {
        let totals: u64 = categories
            .iter()
            .map(|category| category[magnitude]["total"].as_u64().unwrap())
            .sum();
        assert_eq!(
            totals,
            summary[magnitude]["total"].as_u64().unwrap(),
            "{magnitude}"
        );
        for severity in ["errors", "warnings", "info", "unknown"] {
            let totals: u64 = categories
                .iter()
                .map(|category| category[magnitude][severity].as_u64().unwrap())
                .sum();
            assert_eq!(
                totals,
                summary[magnitude][severity].as_u64().unwrap(),
                "{magnitude} {severity}"
            );
        }
    }

    // The historical flat fields stay the alias of the quantity they already
    // published: distinct diagnostics for `summary`, occurrences for the
    // categories.
    assert_eq!(summary["total"], summary["distinct"]["total"]);
    for category in categories {
        assert_eq!(category["warnings"], category["occurrences"]["warnings"]);
    }

    // Occurrences always cover the distinct ones, and every published
    // diagnostic carries at least one.
    //
    // Multiplicity itself, a diagnostic reported twice that counts as one
    // distinct and two occurrences, is proven in unit tests on two identical
    // compiler messages, in
    // `report::tests::normalizes_text_paths_severity_and_deduplicates`. There
    // it is independent of the compilation scope, whereas here it would depend
    // on what Cargo chooses to compile: under the default targets a library is
    // no longer linted twice, and the invariant would stop being observable
    // without ceasing to be true.
    assert!(
        summary["occurrences"]["total"].as_u64() >= summary["distinct"]["total"].as_u64()
    );
    for diagnostic in report["diagnostics"].as_array().unwrap() {
        assert!(diagnostic["occurrences"].as_u64().unwrap() >= 1);
    }
}

/// US-066: a report whose counts diverge is not publishable.
#[test]
fn diverging_counts_fail_to_serialize() {
    let mut report = inspect(InspectRequest::new(adversarial()));
    assert!(report.is_valid());

    report.summary = Summary::default();
    assert!(!report.is_valid());
    assert!(serde_json::to_vec(&report).is_err());
    let mut rendered = Vec::new();
    assert!(render::render_json(&report, &mut rendered).is_err());
    assert!(rendered.is_empty());
}

/// US-067: the published model must stay consistent with the published value.
#[test]
fn an_inconsistent_model_is_rejected_before_publication() {
    let mut report = inspect(InspectRequest::new(adversarial()));
    assert_eq!(report.schema_version, SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, 13);

    let score = report.audit.score.as_mut().unwrap();
    score.applied_ceiling = None;
    assert!(!report.audit.is_valid());
    assert!(!report.is_valid());
    assert!(serde_json::to_vec(&report).is_err());

    let score = report.audit.score.as_mut().unwrap();
    score.applied_ceiling = Some(40);
    score.model = "core-v1".to_owned();
    assert!(!report.audit.is_valid());
    assert!(serde_json::to_vec(&report).is_err());
}

/// US-067: no historical field of the score contract disappeared or changed
/// type between `core-v1` and `core-v2`.
#[test]
fn the_core_v2_oracle_preserves_every_historical_field() {
    let previous: Value = serde_json::from_str(include_str!(
        "fixtures/local-cli-experience/audit-core-v1.json"
    ))
    .expect("the frozen migration oracle should stay readable");
    let current: Value = serde_json::from_str(include_str!(
        "fixtures/local-cli-experience/audit-core-v2.json"
    ))
    .expect("the core-v2 oracle should be valid");

    assert_eq!(previous["model"], "core-v1");
    assert_eq!(current["model"], "core-v2");
    for section in previous.as_object().unwrap().keys() {
        assert!(
            current.get(section).is_some(),
            "section {section} disappeared from the contract"
        );
    }
    assert_eq!(previous["score_boundaries"], current["score_boundaries"]);
    assert_eq!(previous["rounding_cases"], current["rounding_cases"]);
    assert_eq!(previous["share_cases"], current["share_cases"]);
    assert_eq!(previous["category_mappings"], current["category_mappings"]);

    let historical_score = &previous["score_cases"][1]["expected_audit"]["score"];
    let current_score = &current["score_cases"][1]["expected_audit"]["score"];
    for (field, value) in historical_score.as_object().unwrap() {
        let published = current_score.get(field);
        assert!(published.is_some(), "score field {field} disappeared");
        let published = published.unwrap();
        assert_eq!(
            std::mem::discriminant(value),
            std::mem::discriminant(published),
            "score field {field} changed type"
        );
    }
    for added in ["worst_tier", "applied_ceiling"] {
        assert!(
            current_score.get(added).is_some(),
            "{added} is undocumented"
        );
        assert!(historical_score.get(added).is_none());
    }

    let historical_category = &previous["score_cases"][0]["expected_audit"]["categories"][0];
    let current_category = &current["score_cases"][0]["expected_audit"]["categories"][0];
    for (field, value) in historical_category.as_object().unwrap() {
        let published = current_category.get(field);
        assert!(published.is_some(), "category field {field} disappeared");
        let published = published.unwrap();
        assert_eq!(value, published, "category field {field} changed");
    }
}
