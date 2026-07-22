// Integration test crates aren't covered by clippy.toml `allow-*-in-tests`
// (clippy #13981); unwrap/expect are fine in test assertions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rust_doctor::diagnostics::{
    Category, Diagnostic, ScanExecution, ScanResult, ScoreLabel, Severity,
};
use std::path::PathBuf;
use std::time::Duration;

#[test]
fn test_diagnostic_json_snapshot() {
    let diag = Diagnostic {
        file_path: PathBuf::from("src/main.rs"),
        rule: "unwrap-in-production".to_string(),
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        message: "Use of .unwrap() in production code".to_string(),
        help: Some("Use ? operator or handle the error explicitly".to_string()),
        line: Some(42),
        column: Some(10),
        fix: None,
    };
    insta::assert_json_snapshot!("diagnostic_full", diag);
}

#[test]
fn test_diagnostic_minimal_json_snapshot() {
    let diag = Diagnostic {
        file_path: PathBuf::from("Cargo.toml"),
        rule: "unused-dependency".to_string(),
        category: Category::Dependencies,
        severity: Severity::Warning,
        message: "Unused dependency: serde".to_string(),
        help: None,
        line: None,
        column: None,
        fix: None,
    };
    insta::assert_json_snapshot!("diagnostic_minimal", diag);
}

#[test]
fn test_scan_result_empty_snapshot() {
    let result = ScanResult {
        diagnostics: vec![],
        score: 100,
        score_label: ScoreLabel::Great,
        dimension_scores: rust_doctor::diagnostics::DimensionScores {
            security: 100,
            reliability: 100,
            maintainability: 100,
            performance: 100,
            dependencies: 100,
        },
        source_file_count: 15,
        elapsed: Duration::from_millis(1234),
        skipped_passes: vec![],
        error_count: 0,
        warning_count: 0,
        info_count: 0,
        pass_timings: vec![],
        suppressed_security: vec![],
        planned_files: vec![],
        analyzed_files: vec![],
        compiler_evidence: vec![],
        execution: ScanExecution::default(),
    };
    insta::assert_json_snapshot!("scan_result_empty", result);
}

#[test]
fn test_scan_result_with_findings_snapshot() {
    let result = ScanResult {
        diagnostics: vec![
            Diagnostic {
                file_path: PathBuf::from("src/lib.rs"),
                rule: "unwrap-in-production".to_string(),
                category: Category::ErrorHandling,
                severity: Severity::Warning,
                message: "Use of .unwrap() in production code".to_string(),
                help: Some("Use ? operator".to_string()),
                line: Some(10),
                column: Some(5),
                fix: None,
            },
            Diagnostic {
                file_path: PathBuf::from("src/main.rs"),
                rule: "hardcoded-secrets".to_string(),
                category: Category::Security,
                severity: Severity::Error,
                message: "Potential hardcoded secret in variable 'api_key'".to_string(),
                help: Some("Use environment variables".to_string()),
                line: Some(3),
                column: Some(9),
                fix: None,
            },
        ],
        score: 72,
        score_label: ScoreLabel::NeedsWork,
        dimension_scores: rust_doctor::diagnostics::DimensionScores {
            security: 90,
            reliability: 100,
            maintainability: 100,
            performance: 100,
            dependencies: 100,
        },
        source_file_count: 8,
        elapsed: Duration::from_millis(5678),
        skipped_passes: vec!["dependencies (cargo-audit)".to_string()],
        error_count: 1,
        warning_count: 1,
        info_count: 0,
        pass_timings: vec![],
        suppressed_security: vec![],
        planned_files: vec![],
        analyzed_files: vec![],
        compiler_evidence: vec![],
        execution: ScanExecution::default(),
    };
    insta::assert_json_snapshot!("scan_result_with_findings", result);
}

#[test]
fn test_severity_variants_snapshot() {
    insta::assert_json_snapshot!("severity_error", Severity::Error);
    insta::assert_json_snapshot!("severity_warning", Severity::Warning);
    insta::assert_json_snapshot!("severity_info", Severity::Info);
}

#[test]
fn test_score_label_variants_snapshot() {
    insta::assert_json_snapshot!("score_label_great", ScoreLabel::Great);
    insta::assert_json_snapshot!("score_label_needs_work", ScoreLabel::NeedsWork);
    insta::assert_json_snapshot!("score_label_critical", ScoreLabel::Critical);
}

#[test]
fn test_category_variants_snapshot() {
    let categories = vec![
        Category::ErrorHandling,
        Category::Performance,
        Category::Security,
        Category::Correctness,
        Category::Architecture,
        Category::Dependencies,
        Category::Async,
        Category::Framework,
        Category::Cargo,
        Category::Style,
    ];
    insta::assert_json_snapshot!("all_categories", categories);
}
