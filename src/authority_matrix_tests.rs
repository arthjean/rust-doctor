use crate::completeness::{
    AnalyzerIdentity, AnalyzerReceipt, AnalyzerScope, ScoreVisibility, dimension_coverage,
    dimensions_are_authoritative, score_decision,
};
use crate::config::resolve_config_defaults;
use crate::diagnostics::{
    CheckStatus, CompletenessState, DimensionScores, GateResult, PackageExecution, ReportV1,
    ScanExecution, ScanMode, ScanResult, ScoreLabel, SourceSurface,
};
use crate::discovery::{CargoTargetContext, ProjectInfo, WorkspaceMember};
use std::path::PathBuf;
use std::time::Duration;

const SCORE: u32 = 87;

#[derive(Clone, Copy, Debug)]
enum ScopeCase {
    CompleteRoot,
    Category,
    Diff,
    EmptyProject,
    SinglePackage,
    MultiPackage,
    VirtualWorkspace,
    NestedRoot,
}

impl ScopeCase {
    const ALL: [Self; 8] = [
        Self::CompleteRoot,
        Self::Category,
        Self::Diff,
        Self::EmptyProject,
        Self::SinglePackage,
        Self::MultiPackage,
        Self::VirtualWorkspace,
        Self::NestedRoot,
    ];

    const fn package_count(self) -> usize {
        match self {
            Self::MultiPackage | Self::VirtualWorkspace => 2,
            _ => 1,
        }
    }

    const fn has_files(self) -> bool {
        !matches!(self, Self::EmptyProject)
    }

    const fn category_only(self) -> bool {
        matches!(self, Self::Category)
    }
}

#[derive(Clone, Copy, Debug)]
enum DegradedCase {
    Complete,
    RequiredSkip,
    OptionalSkip,
    FailedExit,
    Timeout,
    Cancellation,
    Panic,
    Truncated,
    Malformed,
    UnsupportedVersion,
}

impl DegradedCase {
    const ALL: [Self; 10] = [
        Self::Complete,
        Self::RequiredSkip,
        Self::OptionalSkip,
        Self::FailedExit,
        Self::Timeout,
        Self::Cancellation,
        Self::Panic,
        Self::Truncated,
        Self::Malformed,
        Self::UnsupportedVersion,
    ];

    const fn status(self) -> Option<CheckStatus> {
        match self {
            Self::Complete => None,
            Self::RequiredSkip | Self::OptionalSkip => Some(CheckStatus::Skipped),
            Self::Timeout => Some(CheckStatus::TimedOut),
            Self::Cancellation => Some(CheckStatus::Cancelled),
            Self::FailedExit
            | Self::Panic
            | Self::Truncated
            | Self::Malformed
            | Self::UnsupportedVersion => Some(CheckStatus::Failed),
        }
    }

    const fn reason(self) -> Option<&'static str> {
        match self {
            Self::Complete => None,
            Self::RequiredSkip | Self::OptionalSkip => Some("cargo-deny is not installed"),
            Self::FailedExit => Some("cargo-deny exited with status 2"),
            Self::Timeout => Some("cargo-deny timed out"),
            Self::Cancellation => Some("cargo-deny was cancelled"),
            Self::Panic => Some("cargo-deny adapter panicked"),
            Self::Truncated => Some("cargo-deny output was truncated at the capture limit"),
            Self::Malformed => Some("cargo-deny emitted malformed JSON"),
            Self::UnsupportedVersion => Some("cargo-deny version is unsupported"),
        }
    }

    const fn blocks_score(self) -> bool {
        !matches!(self, Self::Complete | Self::OptionalSkip)
    }
}

fn core_receipts(package: &str, multi_package: bool, category_only: bool) -> Vec<AnalyzerReceipt> {
    let analyzers: &[AnalyzerIdentity] = if category_only {
        &[AnalyzerIdentity::Msrv]
    } else {
        &[
            AnalyzerIdentity::Clippy,
            AnalyzerIdentity::CustomRules,
            AnalyzerIdentity::Msrv,
        ]
    };
    analyzers
        .iter()
        .cloned()
        .map(|analyzer| AnalyzerReceipt {
            analyzer,
            scope: if multi_package {
                AnalyzerScope::Package
            } else {
                AnalyzerScope::Root
            },
            package_id: Some(package.to_string()),
            required: true,
            status: CheckStatus::Completed,
            reason: None,
        })
        .collect()
}

fn fixture(scope: ScopeCase, degraded: DegradedCase) -> ScanResult {
    let package_count = scope.package_count();
    let package_ids: Vec<_> = (0..package_count)
        .map(|index| format!("fixture-{index} 0.1.0 (path+file:///fixture/{index}:opaque)"))
        .collect();
    let mut receipts: Vec<_> = package_ids
        .iter()
        .flat_map(|package| core_receipts(package, package_count > 1, scope.category_only()))
        .collect();
    if let Some(status) = degraded.status() {
        receipts.push(AnalyzerReceipt {
            analyzer: AnalyzerIdentity::CargoDeny,
            scope: AnalyzerScope::Workspace,
            package_id: None,
            required: !matches!(degraded, DegradedCase::OptionalSkip),
            status,
            reason: degraded.reason().map(str::to_string),
        });
    }

    let planned_files: Vec<_> = if scope.has_files() {
        package_ids
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let prefix = if matches!(scope, ScopeCase::NestedRoot) {
                    "nested/member"
                } else {
                    "member"
                };
                PathBuf::from(format!("/fixture/{prefix}-{index}/src/lib.rs"))
            })
            .collect()
    } else {
        Vec::new()
    };
    let packages = package_ids
        .iter()
        .enumerate()
        .map(|(index, package)| {
            let planned: Vec<_> = planned_files.get(index).cloned().into_iter().collect();
            PackageExecution {
                cargo_package_id: package.clone(),
                package_root: PathBuf::from(format!("/fixture/member-{index}")),
                planned_files: planned.clone(),
                analyzed_files: planned,
                checks: core_receipts(package, package_count > 1, scope.category_only())
                    .iter()
                    .map(AnalyzerReceipt::to_check_state)
                    .collect(),
                elapsed: Duration::from_millis(1),
                score: Some(SCORE),
            }
        })
        .collect();
    let source_file_count = planned_files.len();
    ScanResult {
        diagnostics: Vec::new(),
        score: SCORE,
        score_label: ScoreLabel::Great,
        dimension_scores: DimensionScores {
            security: 90,
            reliability: 88,
            maintainability: 86,
            performance: 92,
            dependencies: 80,
        },
        source_file_count,
        elapsed: Duration::from_millis(10),
        skipped_passes: degraded.reason().map(str::to_string).into_iter().collect(),
        error_count: 0,
        warning_count: 0,
        info_count: 0,
        pass_timings: Vec::new(),
        suppressed_security: Vec::new(),
        planned_files: planned_files.clone(),
        analyzed_files: planned_files,
        compiler_evidence: Vec::new(),
        execution: ScanExecution {
            execution_scope: match scope {
                ScopeCase::Diff => "affected_packages",
                ScopeCase::VirtualWorkspace => "virtual_workspace",
                ScopeCase::NestedRoot => "nested_root",
                _ => "full_packages",
            }
            .to_string(),
            reporting_scope: match scope {
                ScopeCase::Category => "category",
                ScopeCase::Diff => "changed",
                _ => "full",
            }
            .to_string(),
            analyzer_receipts: receipts,
            packages,
            ..ScanExecution::default()
        },
    }
}

fn validate_fixture(result: &ScanResult) -> Result<(), String> {
    if !result.planned_files.is_empty() && result.execution.packages.is_empty() {
        return Err("fixture has planned files without package ownership".to_string());
    }
    for receipt in &result.execution.analyzer_receipts {
        if matches!(receipt.scope, AnalyzerScope::Root | AnalyzerScope::Package)
            && receipt.package_id.is_none()
        {
            return Err(format!(
                "fixture receipt lacks package ownership: {}",
                receipt.analyzer.display_name()
            ));
        }
    }
    for package in &result.execution.packages {
        let has_completion_marker = result.execution.analyzer_receipts.iter().any(|receipt| {
            receipt.package_id.as_deref() == Some(&package.cargo_package_id)
                && receipt.required
                && receipt.status == CheckStatus::Completed
        });
        if !has_completion_marker && !package.planned_files.is_empty() {
            return Err(format!(
                "fixture lacks a completion marker for {}",
                package.cargo_package_id
            ));
        }
    }
    Ok(())
}

#[test]
fn degraded_execution_matrix_has_no_false_healthy_outcome() {
    for scope in ScopeCase::ALL {
        for degraded in DegradedCase::ALL {
            let result = fixture(scope, degraded);
            validate_fixture(&result).expect("matrix fixture must prove its own setup");
            let decision = score_decision(&result);
            let receipts = crate::completeness::effective_receipts(&result);
            let dimensions = dimension_coverage(&receipts, &result.dimension_scores, &[], &[]);
            let empty = matches!(scope, ScopeCase::EmptyProject);
            let category = matches!(scope, ScopeCase::Category);
            let visible = !empty && !category && !degraded.blocks_score();
            let expected_completeness = if degraded.blocks_score() {
                CompletenessState::Incomplete
            } else if matches!(degraded, DegradedCase::OptionalSkip) {
                CompletenessState::Partial
            } else {
                CompletenessState::Complete
            };

            assert_eq!(
                crate::completeness::compute(&result).state,
                expected_completeness,
                "unexpected completeness for {scope:?}/{degraded:?}"
            );
            assert_eq!(
                decision.published_score().is_some(),
                visible,
                "unexpected visibility for {scope:?}/{degraded:?}: {decision:?}"
            );
            assert_eq!(
                decision.visibility == ScoreVisibility::Absent,
                empty,
                "unexpected absence for {scope:?}/{degraded:?}"
            );
            assert_eq!(
                dimensions_are_authoritative(&dimensions),
                !category
                    && !matches!(degraded, DegradedCase::RequiredSkip)
                    && !degraded.blocks_score(),
                "unexpected dimension authority for {scope:?}/{degraded:?}: {dimensions:?}"
            );
            assert_eq!(
                crate::run::check_score_authority(&result, true),
                (!visible).then(|| std::process::ExitCode::from(1)),
                "bare score exit disagrees for {scope:?}/{degraded:?}"
            );
            assert_eq!(
                crate::run::check_score_gate(&result, Some(0)),
                (!visible).then(|| std::process::ExitCode::from(1)),
                "threshold gate disagrees for {scope:?}/{degraded:?}"
            );
            if degraded.blocks_score() && !empty {
                assert!(
                    decision
                        .reasons
                        .iter()
                        .any(|reason| reason.starts_with("required_analysis_")),
                    "missing required-work reason for {scope:?}/{degraded:?}: {decision:?}"
                );
            }
        }
    }
}

#[test]
fn malformed_matrix_fixture_fails_setup_instead_of_counting_as_degraded() {
    let mut result = fixture(ScopeCase::SinglePackage, DegradedCase::Complete);
    result.execution.analyzer_receipts[0].package_id = None;
    assert!(validate_fixture(&result).is_err());

    let mut result = fixture(ScopeCase::SinglePackage, DegradedCase::Complete);
    for receipt in &mut result.execution.analyzer_receipts {
        receipt.status = CheckStatus::Failed;
    }
    assert!(validate_fixture(&result).is_err());
}

fn project() -> ProjectInfo {
    let root = PathBuf::from("/fixture");
    let target = CargoTargetContext {
        name: "fixture".to_string(),
        src_path: root.join("member-0/src/lib.rs"),
        source_surface: SourceSurface::Library,
        is_proc_macro: false,
    };
    let package_id = "fixture-0 0.1.0 (path+file:///fixture/0:opaque)".to_string();
    ProjectInfo {
        root_dir: root.clone(),
        name: "fixture".to_string(),
        version: "0.1.0".to_string(),
        package_id: package_id.clone(),
        targets: vec!["fixture:[Lib]".to_string()],
        cargo_targets: vec![target.clone()],
        edition: "2024".to_string(),
        frameworks: Vec::new(),
        framework_capabilities: Vec::new(),
        is_workspace: false,
        member_count: 1,
        has_build_script: false,
        rust_version: Some("1.97".to_string()),
        is_no_std: false,
        package_metadata: serde_json::json!({}),
        workspace_members: vec![WorkspaceMember {
            name: "fixture".to_string(),
            root_dir: root.join("member-0"),
            package_id: package_id.clone(),
            targets: vec!["fixture:[Lib]".to_string()],
            cargo_targets: vec![target],
            frameworks: Vec::new(),
            framework_capabilities: Vec::new(),
            rust_version: Some("1.97".to_string()),
            edition: "2024".to_string(),
            enabled_features: Vec::new(),
        }],
        default_member_ids: vec![package_id],
        enabled_features: Vec::new(),
        declared_features: Vec::new(),
        analyzed_target: None,
    }
}

#[test]
fn score_bearing_surfaces_project_the_same_canonical_decision() {
    for degraded in [DegradedCase::Complete, DegradedCase::FailedExit] {
        let result = fixture(ScopeCase::SinglePackage, degraded);
        let decision = score_decision(&result);
        let config = resolve_config_defaults(None);
        let report = ReportV1::from_scan_with_context(
            &result,
            &project(),
            &config,
            ScanMode::Full,
            PathBuf::from("/fixture").as_path(),
            GateResult::NotEvaluated,
        );
        assert_eq!(report.summary.score, decision.published_score());
        assert_eq!(report.summary.score_label, decision.published_label());
        assert_eq!(
            report.summary.score_authoritative,
            decision.is_authoritative()
        );
        assert_eq!(report.summary.score_reasons, decision.reasons);
        assert!(
            report
                .projects
                .iter()
                .all(|project| project.score_authoritative == project.score.is_some())
        );

        let json = serde_json::to_value(&report).expect("report serializes");
        assert_eq!(
            json["summary"]["score"].as_u64(),
            decision.published_score().map(u64::from)
        );

        let sarif: serde_json::Value =
            serde_json::from_str(&crate::sarif::render_report_sarif(&report).expect("SARIF"))
                .expect("SARIF parses");
        assert_eq!(
            sarif["runs"][0]["properties"]["rustDoctorScore"].as_u64(),
            decision.published_score().map(u64::from)
        );
        assert_eq!(
            sarif["runs"][0]["properties"]["rustDoctorScoreReasons"],
            serde_json::json!(decision.reasons)
        );

        let plan = crate::plan::format_plan_markdown(&[], &result);
        if let Some(score) = decision.published_score() {
            assert!(plan.contains(&format!("Score: {score}/100")));
        } else {
            assert!(plan.contains("Score: unavailable"));
        }

        #[cfg(feature = "mcp")]
        {
            let groups = crate::mcp::helpers::group_report_diagnostics(&report.diagnostics);
            let narrative = crate::mcp::helpers::format_report_scan(&report, &groups);
            if let Some(score) = decision.published_score() {
                assert!(narrative.contains(&format!("## {score}/100")));
            } else {
                assert!(narrative.contains("## n/a/100"));
            }
        }

        let directory = tempfile::tempdir().expect("handoff directory");
        crate::handoff::execute(
            &report,
            &crate::handoff::HandoffRequest {
                output_dir: Some(directory.path().to_path_buf()),
                target: None,
                remember_target: false,
                reset_target: false,
                interactive: false,
            },
        )
        .expect("handoff succeeds")
        .expect("handoff written");
        let dump: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("diagnostics.json")).expect("handoff dump"),
        )
        .expect("handoff parses");
        assert_eq!(
            dump["score"].as_u64(),
            decision.published_score().map(u64::from)
        );
        assert_eq!(dump["score_reasons"], serde_json::json!(decision.reasons));
    }
}
