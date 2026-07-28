use crate::config::resolve_config_defaults;
use crate::diagnostics::{
    Category, CompletenessState, Diagnostic, DiagnosticLocation, DiagnosticOwnership,
    DimensionAuthority, DimensionScores, GateResult, ReportOutcome, ReportV1, ScanExecution,
    ScanMode, ScanResult, ScoreLabel, Severity, SourceSurface,
};
use crate::discovery::{CargoTargetContext, ProjectInfo, WorkspaceMember};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn project(root: PathBuf) -> ProjectInfo {
    let cargo_targets = vec![
        CargoTargetContext {
            name: "fixture".to_string(),
            src_path: root.join("src/lib.rs"),
            source_surface: SourceSurface::Library,
            is_proc_macro: false,
        },
        CargoTargetContext {
            name: "fixture-cli".to_string(),
            src_path: root.join("crates/cli/src/main.rs"),
            source_surface: SourceSurface::Binary,
            is_proc_macro: false,
        },
    ];
    ProjectInfo {
        root_dir: root.clone(),
        name: "fixture".to_string(),
        version: "0.1.0".to_string(),
        package_id: "fixture 0.1.0 (path+file:///fixture)".to_string(),
        targets: vec!["fixture:[Lib]".to_string()],
        cargo_targets: cargo_targets.clone(),
        edition: "2024".to_string(),
        frameworks: vec![],
        framework_capabilities: vec![],
        is_workspace: false,
        member_count: 1,
        has_build_script: false,
        rust_version: Some("1.97".to_string()),
        is_no_std: false,
        package_metadata: serde_json::json!({}),
        workspace_members: vec![WorkspaceMember {
            name: "fixture".to_string(),
            root_dir: root,
            package_id: "fixture 0.1.0 (path+file:///fixture)".to_string(),
            targets: vec!["fixture:[Lib]".to_string()],
            cargo_targets,
            frameworks: vec![],
            framework_capabilities: vec![],
            rust_version: Some("1.97".to_string()),
            edition: "2024".to_string(),
            enabled_features: Vec::new(),
        }],
        default_member_ids: vec!["fixture 0.1.0 (path+file:///fixture)".to_string()],
        enabled_features: Vec::new(),
        declared_features: Vec::new(),
        analyzed_target: None,
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
        pass_timings: (source_file_count > 0)
            .then(|| ("custom rules".to_string(), Duration::from_millis(10)))
            .into_iter()
            .collect(),
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
fn canonical_identities_are_unique_and_path_separator_stable() {
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
    assert_ne!(report.diagnostics[0].site_id, report.diagnostics[1].site_id);
}

#[test]
fn fingerprints_survive_checkout_root_and_unrelated_line_shifts() {
    let config = resolve_config_defaults(None);
    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    for root in [first_root.path(), second_root.path()] {
        std::fs::create_dir(root.join("src")).unwrap();
    }
    std::fs::write(
        first_root.path().join("src/lib.rs"),
        "pub fn value() { panic!(\"same\"); }\n",
    )
    .unwrap();
    std::fs::write(
        second_root.path().join("src/lib.rs"),
        "// unrelated insertion\npub fn value() { panic!(\"same\"); }\n",
    )
    .unwrap();

    let first = ReportV1::from_scan(
        &scan(
            vec![diagnostic(
                &first_root.path().join("src/lib.rs").to_string_lossy(),
                "panic-in-library",
                "panic in library code",
                Some(1),
            )],
            1,
        ),
        &project(first_root.path().to_path_buf()),
        &config,
        ScanMode::Full,
    );
    let second = ReportV1::from_scan(
        &scan(
            vec![diagnostic(
                &second_root.path().join("src/lib.rs").to_string_lossy(),
                "panic-in-library",
                "panic in library code",
                Some(2),
            )],
            1,
        ),
        &project(second_root.path().to_path_buf()),
        &config,
        ScanMode::Full,
    );

    assert_eq!(first.diagnostics[0].site_id, second.diagnostics[0].site_id);
    assert_eq!(
        first.diagnostics[0].baseline_key,
        second.diagnostics[0].baseline_key
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
    assert_eq!(compiler.ownership, DiagnosticOwnership::Unowned);
    let rustsec = report
        .diagnostics
        .iter()
        .find(|value| value.rule == "RUSTSEC-2099-9999")
        .unwrap();
    assert_eq!(rustsec.provider, "rustsec");
    assert!(rustsec.namespace_fallback);
    assert_eq!(rustsec.ownership, DiagnosticOwnership::Workspace);

    let mut unknown_rustc = diagnostic(
        "/repo/src/lib.rs",
        "future_rustc_lint",
        "future lint",
        Some(4),
    );
    unknown_rustc.category = Category::Correctness;
    let mut compiler_scan = scan(vec![unknown_rustc.clone()], 1);
    compiler_scan
        .compiler_evidence
        .push(crate::diagnostics::CompilerDiagnosticEvidence {
            provenance: crate::catalog::AdapterProvenance::Rustc,
            rule: unknown_rustc.rule.clone(),
            message: unknown_rustc.message.clone(),
            file_path: unknown_rustc.file_path.clone(),
            line: unknown_rustc.line,
            column: unknown_rustc.column,
            original_level: "warning".to_string(),
            primary_span: None,
            related_locations: Vec::new(),
            macro_expansion: None,
            fixes: Vec::new(),
        });
    let report = ReportV1::from_scan(&compiler_scan, &project, &config, ScanMode::Full);
    assert_eq!(report.diagnostics[0].provider, "rustc");
    assert_eq!(report.diagnostics[0].source_surface, SourceSurface::Library);
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
    let validator = jsonschema::validator_for(&schema).unwrap();
    for fixture in [
        include_str!("../tests/fixtures/report-v1/failure.json"),
        include_str!("../tests/fixtures/report-v1/nothing-to-scan.json"),
    ] {
        let value: serde_json::Value = serde_json::from_str(fixture).unwrap();
        let errors: Vec<_> = validator
            .iter_errors(&value)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "schema errors: {errors:?}");
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
    // Adding an optional field to Report V1 means editing the checked schema in
    // the same change; this assertion is what keeps the two from drifting.
    assert_eq!(checked, generated);
}

#[test]
fn report_root_is_not_fabricated() {
    let report = ReportV1::failure_with_causes(
        Path::new("/repo"),
        ScanMode::Full,
        "discovery",
        "failed under /repo",
        &["could not read /repo/Cargo.toml".to_string()],
    );
    assert!(!report.requested_root.is_empty());
    assert!(report.resolved_root.is_none());
    assert!(report.report_constructed);
    assert_eq!(report.gate_result, GateResult::NotEvaluated);
    let error = report.error.unwrap();
    assert_eq!(error.message, "failed under <requested-root>");
    assert_eq!(error.causes, ["could not read <requested-root>/Cargo.toml"]);
}

#[test]
fn ownership_and_source_surface_do_not_leak_across_roots() {
    let config = resolve_config_defaults(None);
    let project = project(PathBuf::from("/repo"));
    let report = ReportV1::from_scan(
        &scan(
            vec![
                diagnostic(
                    "/outside/src/lib.rs",
                    "unwrap-in-production",
                    "outside",
                    Some(1),
                ),
                diagnostic(
                    "/repo/crates/cli/src/main.rs",
                    "unwrap-in-production",
                    "nested binary",
                    Some(2),
                ),
            ],
            2,
        ),
        &project,
        &config,
        ScanMode::Full,
    );
    let outside = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message == "outside")
        .unwrap();
    assert_eq!(outside.ownership, DiagnosticOwnership::Unowned);
    let nested = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message == "nested binary")
        .unwrap();
    assert_eq!(nested.source_surface, SourceSurface::Binary);
}

// ---------------------------------------------------------------------------
// Trust contract: score model identity, dimension coverage, and authority
// ---------------------------------------------------------------------------

#[test]
fn every_report_identifies_its_score_model() {
    let config = resolve_config_defaults(None);
    let report = ReportV1::from_scan(
        &scan(vec![], 1),
        &project(PathBuf::from("/repo")),
        &config,
        ScanMode::Full,
    );
    assert_eq!(report.score_model_version, "2.1");

    let failed = ReportV1::failure(Path::new("/repo"), ScanMode::Full, "scan", "boom");
    assert_eq!(failed.score_model_version, "2.1");
    assert!(failed.dimensions.is_empty());
    assert!(!failed.summary.score_authoritative);
}

#[test]
fn a_dimension_without_a_completed_core_analyzer_has_no_score() {
    let config = resolve_config_defaults(None);
    // The default fixture only runs custom rules, so nothing speaks for the
    // Dependencies dimension.
    let report = ReportV1::from_scan(
        &scan(vec![], 1),
        &project(PathBuf::from("/repo")),
        &config,
        ScanMode::Full,
    );
    let dependencies = report
        .dimensions
        .iter()
        .find(|dimension| dimension.dimension == "dependencies")
        .expect("every dimension is represented");
    assert_eq!(dependencies.authority, DimensionAuthority::Unobserved);
    assert_eq!(dependencies.score, None);
    assert!(!dependencies.reasons.is_empty());
    assert!(
        !report.summary.score_authoritative,
        "an unobserved dimension must remove score authority"
    );

    let reliability = report
        .dimensions
        .iter()
        .find(|dimension| dimension.dimension == "reliability")
        .expect("every dimension is represented");
    assert_ne!(reliability.authority, DimensionAuthority::Unobserved);
    assert!(reliability.score.is_some());
}

#[test]
fn a_scan_with_every_core_analyzer_keeps_dimension_authority() {
    let config = resolve_config_defaults(None);
    let mut result = scan(vec![], 1);
    result
        .pass_timings
        .push(("clippy".to_string(), Duration::from_millis(5)));
    result
        .pass_timings
        .push(("msrv".to_string(), Duration::from_millis(1)));
    let report = ReportV1::from_scan(
        &result,
        &project(PathBuf::from("/repo")),
        &config,
        ScanMode::Full,
    );
    assert!(
        report
            .dimensions
            .iter()
            .all(|dimension| dimension.score.is_some()),
        "{:?}",
        report.dimensions
    );
    assert!(report.summary.score_authoritative);
}

#[test]
fn a_failed_required_analyzer_marks_its_dimension_failed() {
    let config = resolve_config_defaults(None);
    let mut result = scan(vec![], 1);
    result
        .skipped_passes
        .push("custom rules: panicked while analyzing".to_string());
    let report = ReportV1::from_scan(
        &result,
        &project(PathBuf::from("/repo")),
        &config,
        ScanMode::Full,
    );
    let reliability = report
        .dimensions
        .iter()
        .find(|dimension| dimension.dimension == "reliability")
        .expect("every dimension is represented");
    assert_eq!(reliability.authority, DimensionAuthority::Failed);
    assert_eq!(reliability.score, None);
    assert!(!report.summary.score_authoritative);
}

#[test]
fn duplicate_identities_contribute_to_the_score_once() {
    let repeated = vec![
        diagnostic(
            "src/lib.rs",
            "unwrap-in-production",
            "called unwrap",
            Some(7),
        ),
        diagnostic(
            "src/lib.rs",
            "unwrap-in-production",
            "called unwrap",
            Some(7),
        ),
        diagnostic(
            "src/lib.rs",
            "unwrap-in-production",
            "called unwrap",
            Some(7),
        ),
    ];
    let single = vec![repeated[0].clone()];
    let mut deduped = repeated;
    crate::scan::dedup_diagnostics(&mut deduped);
    assert_eq!(deduped.len(), 1);
    assert_eq!(
        crate::output::calculate_score(&deduped)
            .expect("embedded score model")
            .0,
        crate::output::calculate_score(&single)
            .expect("embedded score model")
            .0
    );
}

#[test]
fn a_scan_with_nothing_to_analyze_emits_no_synthetic_score() {
    let config = resolve_config_defaults(None);
    let report = ReportV1::from_scan(
        &scan(vec![], 0),
        &project(PathBuf::from("/repo")),
        &config,
        ScanMode::Full,
    );
    assert_eq!(report.outcome, ReportOutcome::NothingToScan);
    assert_eq!(report.summary.score, None);
    assert_eq!(report.score, None);
    assert!(!report.summary.score_authoritative);
}

#[test]
fn every_diagnostic_carries_independent_decision_metadata() {
    let config = resolve_config_defaults(None);
    let report = ReportV1::from_scan(
        &scan(
            vec![diagnostic(
                "src/lib.rs",
                "unwrap-in-production",
                "called unwrap",
                Some(7),
            )],
            1,
        ),
        &project(PathBuf::from("/repo")),
        &config,
        ScanMode::Full,
    );
    let found = &report.diagnostics[0];
    assert_eq!(found.priority.as_deref(), Some("p2"));
    assert_eq!(found.trust_tier, "calibrated-heuristic");
    assert_eq!(found.aggregation_policy, "bounded-occurrence");
    assert_eq!(
        found.root_cause_key.as_deref(),
        Some("rule:unwrap-in-production")
    );
    assert!(!found.evidence_summary.is_empty());
    assert!(!found.limitations.is_empty());
    assert!(!found.suppressed);
    // Severity, confidence, priority, trust tier, category, and score
    // eligibility stay independent fields: none is derived at report time.
    assert_eq!(found.severity, Severity::Warning);
    assert_eq!(found.confidence, "medium");
    assert_eq!(found.category, Category::ErrorHandling);
    assert_eq!(
        found.score_eligible,
        found.score_impact == crate::diagnostics::ScoreImpact::Scored
    );
}

#[test]
fn an_unmapped_rule_receives_no_fabricated_decision_metadata() {
    let config = resolve_config_defaults(None);
    let report = ReportV1::from_scan(
        &scan(
            vec![diagnostic(
                "src/lib.rs",
                "clippy::a_future_lint",
                "unknown lint fired",
                Some(3),
            )],
            1,
        ),
        &project(PathBuf::from("/repo")),
        &config,
        ScanMode::Full,
    );
    let found = &report.diagnostics[0];
    assert!(found.namespace_fallback);
    assert_eq!(found.priority, None);
    assert_eq!(found.root_cause_key, None);
    assert_eq!(found.fix_recipe, None);
    assert!(!found.score_eligible);
    assert_eq!(
        found.score_impact,
        crate::diagnostics::ScoreImpact::Ineligible
    );
    // Unranked never means discarded.
    assert_eq!(report.diagnostics.len(), 1);
    assert!(report.root_causes.is_empty());
}

#[test]
fn one_root_cause_owns_priority_while_occurrences_stay_inspectable() {
    let config = resolve_config_defaults(None);
    let diagnostics: Vec<_> = (1..=6)
        .map(|line| {
            diagnostic(
                "src/lib.rs",
                "unwrap-in-production",
                &format!("called unwrap at {line}"),
                Some(line),
            )
        })
        .collect();
    let report = ReportV1::from_scan(
        &scan(diagnostics, 1),
        &project(PathBuf::from("/repo")),
        &config,
        ScanMode::Full,
    );
    assert_eq!(report.root_causes.len(), 1);
    let group = &report.root_causes[0];
    assert_eq!(group.key, "rule:unwrap-in-production");
    assert_eq!(group.priority.as_deref(), Some("p2"));
    assert_eq!(group.occurrences, 6);
    assert_eq!(group.site_ids.len(), 6);
    assert_eq!(report.diagnostics.len(), 6);
}

#[test]
fn report_diagnostics_use_the_canonical_order() {
    let config = resolve_config_defaults(None);
    let mut low = diagnostic("a.rs", "string-from-literal", "allocated", Some(1));
    low.category = Category::Performance;
    let mut high = diagnostic("z.rs", "hardcoded-secrets", "secret", Some(900));
    high.category = Category::Security;
    high.severity = Severity::Error;
    let report = ReportV1::from_scan(
        &scan(vec![low, high], 2),
        &project(PathBuf::from("/repo")),
        &config,
        ScanMode::Full,
    );
    // P0 first even though its path sorts last: priority is not path order.
    assert_eq!(report.diagnostics[0].rule, "hardcoded-secrets");
    assert_eq!(report.diagnostics[0].priority.as_deref(), Some("p0"));
    assert_eq!(report.root_causes[0].key, "rule:hardcoded-secrets");
}
