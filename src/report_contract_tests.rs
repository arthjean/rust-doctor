use crate::config::resolve_config_defaults;
use crate::diagnostics::{
    Category, CompletenessState, Diagnostic, DiagnosticLocation, DimensionScores, ReportV1,
    ScanExecution, ScanMode, ScanResult, ScoreLabel, Severity,
};
use crate::discovery::{ProjectInfo, WorkspaceMember};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn project(root: PathBuf) -> ProjectInfo {
    ProjectInfo {
        root_dir: root.clone(),
        name: "fixture".to_string(),
        version: "0.1.0".to_string(),
        package_id: "fixture 0.1.0 (path+file:///fixture)".to_string(),
        targets: vec!["fixture:[Lib]".to_string()],
        edition: "2024".to_string(),
        frameworks: vec![],
        is_workspace: false,
        member_count: 1,
        has_build_script: false,
        rust_version: Some("1.85".to_string()),
        is_no_std: false,
        package_metadata: serde_json::json!({}),
        workspace_members: vec![WorkspaceMember {
            name: "fixture".to_string(),
            root_dir: root,
            package_id: "fixture 0.1.0 (path+file:///fixture)".to_string(),
            targets: vec!["fixture:[Lib]".to_string()],
            frameworks: vec![],
            rust_version: Some("1.85".to_string()),
        }],
        default_member_ids: vec!["fixture 0.1.0 (path+file:///fixture)".to_string()],
    }
}

fn diagnostic(path: &str, rule: &str, message: &str, line: Option<u32>) -> Diagnostic {
    Diagnostic {
        file_path: PathBuf::from(path),
        rule: rule.to_string(),
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        message: message.to_string(),
        help: Some("Handle the error explicitly".to_string()),
        line,
        column: line.map(|_| 3),
        fix: None,
    }
}

fn scan(diagnostics: Vec<Diagnostic>, source_file_count: usize) -> ScanResult {
    let warning_count = diagnostics.len();
    ScanResult {
        diagnostics,
        score: 90,
        score_label: ScoreLabel::Great,
        dimension_scores: DimensionScores {
            security: 100,
            reliability: 90,
            maintainability: 100,
            performance: 100,
            dependencies: 100,
        },
        source_file_count,
        elapsed: Duration::from_millis(125),
        skipped_passes: vec![],
        error_count: 0,
        warning_count,
        info_count: 0,
        pass_timings: vec![("custom rules".to_string(), Duration::from_millis(10))],
        suppressed_security: vec![],
        planned_files: (0..source_file_count)
            .map(|index| PathBuf::from(format!("src/file-{index}.rs")))
            .collect(),
        analyzed_files: (0..source_file_count)
            .map(|index| PathBuf::from(format!("src/file-{index}.rs")))
            .collect(),
        compiler_evidence: vec![],
        execution: ScanExecution::default(),
    }
}

#[test]
fn canonical_identities_are_sorted_and_path_separator_stable() {
    let config = resolve_config_defaults(None);
    let windows_project = project(PathBuf::from(r"C:\repo"));
    let windows = ReportV1::from_scan(
        &scan(
            vec![diagnostic(
                r"C:\repo\src\main.rs",
                "unwrap-in-production",
                "called unwrap",
                Some(7),
            )],
            1,
        ),
        &windows_project,
        &config,
        ScanMode::Full,
    );
    let posix_project = project(PathBuf::from("C:/repo"));
    let posix = ReportV1::from_scan(
        &scan(
            vec![diagnostic(
                "C:/repo/src/main.rs",
                "unwrap-in-production",
                "called unwrap",
                Some(7),
            )],
            1,
        ),
        &posix_project,
        &config,
        ScanMode::Full,
    );
    assert_eq!(windows.diagnostics[0].site_id, posix.diagnostics[0].site_id);
    assert_eq!(
        windows.diagnostics[0].baseline_key,
        posix.diagnostics[0].baseline_key
    );

    let root = project(PathBuf::from("/repo"));
    let report = ReportV1::from_scan(
        &scan(
            vec![
                diagnostic("/repo/src/z.rs", "panic-in-library", "z", Some(9)),
                diagnostic("/repo/src/a.rs", "unwrap-in-production", "a", Some(1)),
            ],
            2,
        ),
        &root,
        &config,
        ScanMode::Full,
    );
    assert!(
        report
            .diagnostics
            .windows(2)
            .all(|pair| pair[0].site_id <= pair[1].site_id)
    );
}

#[test]
fn compiler_without_span_and_unknown_codes_are_explicit() {
    let config = resolve_config_defaults(None);
    let project = project(PathBuf::from("/repo"));
    let mut compiler = diagnostic("<unknown>", "compiler-error", "failed", None);
    compiler.category = Category::Correctness;
    compiler.severity = Severity::Error;
    let mut external = diagnostic("Cargo.lock", "RUSTSEC-2099-9999", "advisory", None);
    external.category = Category::Security;
    let report = ReportV1::from_scan(
        &scan(vec![compiler, external], 1),
        &project,
        &config,
        ScanMode::Full,
    );
    let compiler = report
        .diagnostics
        .iter()
        .find(|value| value.rule == "compiler-error")
        .unwrap();
    assert!(matches!(compiler.location, DiagnosticLocation::Project));
    let rustsec = report
        .diagnostics
        .iter()
        .find(|value| value.rule == "RUSTSEC-2099-9999")
        .unwrap();
    assert_eq!(rustsec.provider, "rustsec");
    assert!(rustsec.namespace_fallback);
}

#[test]
fn empty_and_partial_scans_never_serialize_as_clean() {
    let config = resolve_config_defaults(None);
    let root = tempfile::tempdir().unwrap();
    let project = project(root.path().to_path_buf());
    let empty = ReportV1::from_scan(&scan(vec![], 0), &project, &config, ScanMode::Full);
    assert_eq!(
        empty.outcome,
        crate::diagnostics::ReportOutcome::NothingToScan
    );
    assert_eq!(empty.score, None);

    let mut partial_scan = scan(vec![], 1);
    partial_scan.skipped_passes = vec!["clippy: timed out".to_string()];
    let partial = ReportV1::from_scan(&partial_scan, &project, &config, ScanMode::Full);
    assert_ne!(partial.outcome, crate::diagnostics::ReportOutcome::Clean);
    assert_eq!(partial.completeness.state, CompletenessState::Incomplete);
}

#[test]
fn checked_schema_and_compatibility_fixtures_cover_report_shape() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schemas/report-v1.schema.json")).unwrap();
    let required: BTreeSet<&str> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    for fixture in [
        include_str!("../tests/fixtures/report-v1/failure.json"),
        include_str!("../tests/fixtures/report-v1/nothing-to-scan.json"),
    ] {
        let value: serde_json::Value = serde_json::from_str(fixture).unwrap();
        let keys: BTreeSet<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(required.is_subset(&keys));
        serde_json::from_value::<ReportV1>(value).unwrap();
    }
}

#[cfg(feature = "mcp")]
#[test]
fn checked_schema_tracks_generated_wire_properties() {
    let checked: serde_json::Value =
        serde_json::from_str(include_str!("../schemas/report-v1.schema.json")).unwrap();
    let mut generated = serde_json::to_value(schemars::schema_for!(ReportV1)).unwrap();
    generated.as_object_mut().unwrap().insert(
        "$id".to_string(),
        serde_json::Value::String(
            "https://rust-doctor.vercel.app/schemas/report-v1.schema.json".to_string(),
        ),
    );
    assert_eq!(checked, generated);
}

#[test]
fn readme_counts_are_asserted_against_the_catalog() {
    let catalog = crate::catalog::built_in_catalog().unwrap();
    let custom = catalog
        .descriptors()
        .iter()
        .filter(|descriptor| descriptor.analyzer_kind == crate::catalog::AnalyzerKind::SynAst)
        .count();
    let clippy = catalog
        .descriptors()
        .iter()
        .filter(|descriptor| descriptor.analyzer_kind == crate::catalog::AnalyzerKind::Clippy)
        .count();
    let readme = include_str!("../README.md");
    assert!(readme.contains(&format!("Custom AST Rules ({custom} rules)")));
    assert!(readme.contains(&format!("Clippy Lints ({clippy} with overrides)")));
}

#[test]
fn report_root_is_not_fabricated() {
    let report = ReportV1::failure(
        Path::new("/repo"),
        ScanMode::Full,
        "discovery",
        "no Cargo.toml".to_string(),
    );
    assert!(!report.requested_root.is_empty());
}
