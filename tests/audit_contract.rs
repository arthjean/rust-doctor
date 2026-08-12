#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::{Path, PathBuf};

use rust_doctor::{
    CategoryOverride, InspectRequest, RuleLevel, RuleOverride, RuleTier, ScoreLabel, ShareError,
    Status, inspect,
};
use serde_json::Value;

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

#[test]
fn report_v9_exposes_one_canonical_audit_block() {
    let clean = inspect(InspectRequest::new(fixture(
        "tests/fixtures/projects/clean",
    )));
    let value = serde_json::to_value(&clean).unwrap();
    assert_eq!(clean.schema_version, 14);
    assert_eq!(clean.status, Status::Complete);
    assert_eq!(clean.audit.source_files, 1);
    assert!(clean.audit.categories.is_empty());
    let score = clean
        .audit
        .score
        .as_ref()
        .expect("clean score should exist");
    assert_eq!(score.model, "core-v2");
    assert_eq!(score.value, 100);
    assert_eq!(score.label, ScoreLabel::Great);
    assert!(score.authoritative);
    assert_eq!(score.projected_after_top_three, None);
    assert!(score.projected_rule_ids.is_empty());
    assert_eq!(
        clean.audit.share_url().unwrap(),
        "https://rust-doctor.com/share?s=100&f=1"
    );
    let audit_keys: Vec<_> = value["audit"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(audit_keys, ["categories", "score", "source_files"]);
}

#[test]
fn incomplete_and_unscorable_reports_keep_existing_outcome_without_a_share() {
    let report = inspect(InspectRequest::new(fixture(
        "tests/fixtures/projects/compile-error",
    )));
    assert_eq!(report.status, Status::Incomplete);
    assert_eq!(report.exit_code(), 1);
    assert_eq!(report.audit.source_files, 1);
    let score = report
        .audit
        .score
        .as_ref()
        .expect("partial score should exist");
    assert!(!score.authoritative);
    assert_eq!(score.projected_after_top_three, None);
    assert!(score.projected_rule_ids.is_empty());
    assert_eq!(report.audit.share_url(), Err(ShareError::NonAuthoritative));
    assert!(!report.errors.is_empty());
}

#[test]
fn metadata_roots_keep_a_partial_inventory_when_code_producers_are_disabled() {
    let report = [
        "correctness",
        "dependencies",
        "maintainability",
        "performance",
        "reliability",
        "security",
    ]
    .into_iter()
    .fold(
        InspectRequest::new(fixture("tests/fixtures/projects/clean")),
        |request, category| {
            request.with_category_override(CategoryOverride::new(category, RuleLevel::Off))
        },
    );
    let report = inspect(report);

    assert_eq!(report.status, Status::Complete);
    assert_eq!(report.audit.source_files, 1);
    assert_eq!(
        report.audit.score.as_ref().map(|score| score.authoritative),
        Some(false)
    );
    assert!(report.scan.command.is_none());
}

#[test]
fn no_rust_files_never_produce_an_artificial_score() {
    let report = inspect(InspectRequest::new(fixture(
        "tests/fixtures/local-cli-experience/no-rust",
    )));
    assert_eq!(report.audit.source_files, 0);
    assert!(report.audit.score.is_none());
    assert_eq!(report.audit.share_url(), Err(ShareError::ScoreUnavailable));
}

#[test]
fn source_inventory_survives_disabling_the_source_rule_producer() {
    let report = inspect(
        InspectRequest::new(fixture("tests/fixtures/projects/clean"))
            .with_rule_override(RuleOverride::new(
                "rust_doctor::source::disabled_tls_verification",
                RuleLevel::Off,
            ))
            .with_rule_override(RuleOverride::new(
                "rust_doctor::source::dynamic_shell_command",
                RuleLevel::Off,
            )),
    );

    assert_eq!(report.status, Status::Complete);
    assert_eq!(report.audit.source_files, 1);
    assert_eq!(
        report.audit.score.as_ref().map(|score| score.value),
        Some(100)
    );
}

#[test]
fn source_inventory_uses_compiler_inputs_instead_of_directory_contents() {
    let report = inspect(
        InspectRequest::new(fixture(
            "tests/fixtures/local-cli-experience/compiler-inventory",
        ))
        .with_rule_override(RuleOverride::new(
            "rust_doctor::source::disabled_tls_verification",
            RuleLevel::Off,
        ))
        .with_rule_override(RuleOverride::new(
            "rust_doctor::source::dynamic_shell_command",
            RuleLevel::Off,
        )),
    );

    assert_eq!(report.status, Status::Complete);
    assert_eq!(report.audit.source_files, 2);
    assert!(
        fixture("tests/fixtures/local-cli-experience/compiler-inventory/src/orphan.rs").is_file(),
        "the fixture must retain an orphan Rust file that the compiler never analyzes"
    );
}

#[test]
fn twenty_constructions_are_byte_identical_and_private_path_free() {
    let fixture = fixture("tests/fixtures/kernel-contract/todo");
    let mut expected = None;
    for _ in 0..20 {
        let report = inspect(InspectRequest::new(&fixture));
        let bytes = serde_json::to_vec(&report).unwrap();
        if let Some(expected) = expected.as_ref() {
            assert_eq!(&bytes, expected);
        } else {
            expected = Some(bytes);
        }
    }
    let bytes = expected.expect("at least one report should be built");
    let json = String::from_utf8(bytes).unwrap();
    assert!(!json.contains(&fixture.display().to_string()));
    assert!(!json.contains("duration"));
    assert!(!json.contains("timestamp"));
    assert!(!json.contains("elapsed"));
}

#[test]
fn scored_findings_drive_projection_and_exact_share_counts() {
    let report = inspect(InspectRequest::new(fixture(
        "tests/fixtures/kernel-contract/todo",
    )));
    let score = report.audit.score.as_ref().expect("score should exist");
    assert!(score.authoritative);
    assert_eq!(score.projected_rule_ids, ["clippy::todo"]);
    assert_eq!(score.projected_after_top_three, Some(100));
    assert_eq!(report.audit.categories[0].name.to_string(), "Bugs");
    // The default targets compile each unit only once: one occurrence per
    // distinct diagnostic, where `--all-targets` produced two for the same
    // site.
    assert_eq!(report.audit.categories[0].warnings, 3);
    assert_eq!(report.audit.categories[0].occurrences.total, 3);
    assert_eq!(report.audit.categories[0].distinct.total, 3);
    // `clippy::todo` is tier P2: the Reliability dimension is capped at 75 and
    // the overall score takes no global cap.
    assert_eq!(score.dimensions.reliability, 75);
    assert_eq!(score.worst_tier, Some(RuleTier::P2));
    assert_eq!(score.applied_ceiling, None);
    assert_eq!(score.value, 94);
    assert_eq!(
        report.audit.share_url().unwrap(),
        "https://rust-doctor.com/share?s=94&w=3&f=1"
    );
}

#[test]
fn audit_json_has_no_unordered_or_process_observation_fields() {
    let report = inspect(InspectRequest::new(fixture(
        "tests/fixtures/kernel-contract/todo",
    )));
    let audit = serde_json::to_value(&report.audit).unwrap();
    let serialized = serde_json::to_string(&audit).unwrap();
    for forbidden in [
        "duration",
        "elapsed",
        "timestamp",
        "workspace_root",
        "HashMap",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    assert!(matches!(audit, Value::Object(_)));
}
