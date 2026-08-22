//! Tests for the report: the pipeline that assembles one, the gate it
//! publishes, and the statuses and errors a degraded scan carries into it.
//!
//! The fixtures below are shared with [`normalization`], which carries the
//! other half: what a producer's finding becomes once it is a diagnostic. They
//! sit in two files so that every file of the module stays under the thousand
//! lines `oversized_unit` reports at, for the reason the rest of the crate
//! holds that bound: the module that declares the shape has to pass the rule
//! the shape publishes.

mod normalization;

use std::collections::BTreeSet;
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

use std::path::Path;

use cargo_metadata::Metadata;
use serde_json::Value;

use super::assembly::{
    analyze_baseline_execution, apply_policy, baseline_report_failure, evaluate_gate,
    project_diagnostics,
};
use super::normalize::*;
use super::sanitize::*;
use super::*;
use crate::cargo_health;
use crate::execution::{
    BaselineExecution, CapturedDiagnostic, CapturedDiagnosticCode, CapturedMessage, CapturedSpan,
    CapturedTarget, ClippyExecution, CompilerMessageData, ExecutionResult, ScanExecution,
};
use crate::internal_error::InternalError;
use crate::policy::PolicyPlan;
use crate::source_kernel;
use crate::workspace_path;

/// The report passes the rule it publishes. `oversized_unit` reports a file at
/// a thousand lines, and this module holds that bound across all six of its
/// files. It was one file of 3226 lines, three times over, and the self-scan
/// that named it froze the defect rather than gating it: a test asserting the
/// crate's largest violation is a test that fails when the violation is fixed.
#[test]
fn the_report_holds_the_size_bound_it_publishes() {
    for own in [
        include_str!("../report.rs"),
        include_str!("assembly.rs"),
        include_str!("normalize.rs"),
        include_str!("sanitize.rs"),
        include_str!("tests.rs"),
        include_str!("tests/normalization.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the report is {lines} lines long, over the {} it reports",
            crate::structure::FILE_LINES
        );
    }
}

fn from_execution(result: ExecutionResult) -> InspectReport {
    super::assembly::from_execution(result, &PolicyPlan::default())
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

fn compiler_message(
    code: Option<&str>,
    level: &str,
    message: &str,
    path: &str,
    line: usize,
) -> CapturedMessage {
    CapturedMessage::Compiler(CompilerMessageData {
        package_id: "opaque-package-id".to_owned(),
        target: CapturedTarget {
            kind: vec!["lib".to_owned()],
            name: "example".to_owned(),
        },
        message: CapturedDiagnostic {
            message: message.to_owned(),
            code: code.map(|code| CapturedDiagnosticCode {
                code: code.to_owned(),
            }),
            level: level.to_owned(),
            spans: vec![CapturedSpan {
                file_name: path.to_owned(),
                line_start: line,
                line_end: line,
                column_start: 2,
                column_end: 4,
                is_primary: true,
            }],
        },
    })
}

fn compiler_message_for_target(target: &str) -> CapturedMessage {
    let mut message =
        compiler_message(Some("clippy::lint"), "warning", "same", "src/lib.rs", 2);
    if let CapturedMessage::Compiler(message) = &mut message {
        message.target.name = target.to_owned();
    }
    message
}

fn clone_compiler_message(message: &CapturedMessage) -> CapturedMessage {
    match message {
        CapturedMessage::Compiler(message) => CapturedMessage::Compiler(CompilerMessageData {
            package_id: message.package_id.clone(),
            target: CapturedTarget {
                kind: vec!["lib".to_owned()],
                name: message.target.name.clone(),
            },
            message: CapturedDiagnostic {
                message: message.message.message.clone(),
                code: message
                    .message
                    .code
                    .as_ref()
                    .map(|code| CapturedDiagnosticCode {
                        code: code.code.clone(),
                    }),
                level: message.message.level.clone(),
                spans: message
                    .message
                    .spans
                    .iter()
                    .map(|span| CapturedSpan {
                        file_name: span.file_name.clone(),
                        line_start: span.line_start,
                        line_end: span.line_end,
                        column_start: span.column_start,
                        column_end: span.column_end,
                        is_primary: span.is_primary,
                    })
                    .collect(),
            },
        }),
        _ => unreachable!(),
    }
}

fn next_permutation(values: &mut [usize]) -> bool {
    let Some(pivot) = (0..values.len().saturating_sub(1))
        .rev()
        .find(|&index| values[index] < values[index + 1])
    else {
        return false;
    };
    let successor = (pivot + 1..values.len())
        .rev()
        .find(|&index| values[pivot] < values[index])
        .unwrap_or(pivot);
    values.swap(pivot, successor);
    values[pivot + 1..].reverse();
    true
}

fn report_with_diagnostics(diagnostics: Vec<Diagnostic>) -> InspectReport {
    let gate = evaluate_gate(Status::Complete, &diagnostics, BlockingLevel::Error);
    InspectReport {
        schema_version: SCHEMA_VERSION,
        audit: Audit::build(1, 100, Status::Complete, &diagnostics),
        status: Status::Complete,
        complete: true,
        policy: None,
        scope: None,
        project: None,
        toolchain: ToolchainReport {
            rustc: None,
            cargo: None,
            clippy: None,
        },
        scan: ScanReport {
            command: None,
            exit_code: Some(0),
            build_finished: Some(true),
            noise_lines: Some(0),
        },
        summary: Summary::from_diagnostics(&diagnostics),
        diagnostics,
        delta: None,
        errors: Vec::new(),
        gate,
    }
}

#[test]
fn baseline_cleanup_failure_overrides_an_otherwise_complete_report() {
    let report = report_with_diagnostics(Vec::new());
    let failed = baseline_report_failure(report, crate::baseline::cleanup_failed());

    assert_eq!(failed.status, Status::Failed);
    assert!(!failed.complete);
    assert!(failed.diagnostics.is_empty());
    assert_eq!(failed.summary, Summary::default());
    assert_eq!(failed.gate.status, GateStatus::NotEvaluated);
    assert_eq!(failed.gate.blocking_diagnostics, None);
    assert_eq!(failed.exit_code(), 2);
    assert_eq!(failed.errors.len(), 1);
    assert_eq!(failed.errors[0].stage, "baseline");
    assert_eq!(failed.errors[0].code, "baseline-cleanup-failed");
}

#[test]
fn restamping_changes_only_effective_severity_summary_and_gate() {
    let workspace = fixture("clean");
    let home = HomePaths::default();
    let messages = vec![compiler_message(
        Some("clippy::todo"),
        "warning",
        "placeholder",
        "src/lib.rs",
        2,
    )];
    let baseline = normalize_diagnostics(&messages, Some(&workspace), None, &home);
    let input = PolicyInput::default().with_rule("clippy::todo", RuleLevel::Error);
    let plan = PolicyPlan::compile(&input).expect("policy should compile");
    let mut effective = baseline.clone();
    apply_policy(&mut effective, &plan);

    assert_eq!(effective[0].id, baseline[0].id);
    assert_eq!(effective[0].base_severity, Severity::Warning);
    assert_eq!(effective[0].severity, Severity::Error);
    assert_eq!(effective[0].message, baseline[0].message);
    assert_eq!(effective[0].path, baseline[0].path);
    assert_eq!(effective[0].span, baseline[0].span);
    assert_eq!(effective[0].occurrences, baseline[0].occurrences);
    assert_eq!(Summary::from_diagnostics(&baseline).warnings, 1);
    assert_eq!(Summary::from_diagnostics(&effective).errors, 1);
    assert_eq!(
        evaluate_gate(Status::Complete, &effective, BlockingLevel::Error),
        GateReport {
            blocking: BlockingLevel::Error,
            status: GateStatus::Failed,
            blocking_diagnostics: Some(1),
        }
    );
}

#[test]
fn files_projection_is_exact_and_runs_after_policy_before_summary_and_gate() {
    let workspace = fixture("clean");
    let home = HomePaths::default();
    let mut diagnostics = normalize_diagnostics(
        &[
            compiler_message(Some("clippy::todo"), "warning", "selected", "src/lib.rs", 2),
            compiler_message(
                Some("clippy::todo"),
                "warning",
                "selected encoded path",
                "src/100%.rs",
                4,
            ),
            compiler_message(
                Some("clippy::todo"),
                "warning",
                "not selected",
                "src/other.rs",
                3,
            ),
        ],
        Some(&workspace),
        None,
        &home,
    );
    let selected_id = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.path.as_deref() == Some("src/lib.rs"))
        .unwrap()
        .id
        .clone();
    let encoded_path_id = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.path.as_deref() == Some("src/100%25.rs"))
        .unwrap()
        .id
        .clone();
    let mut pathless = diagnostics[0].clone();
    pathless.id = "pathless".to_owned();
    pathless.path = None;
    diagnostics.push(pathless);
    let plan = PolicyPlan::compile(
        &PolicyInput::default().with_rule("clippy::todo", RuleLevel::Error),
    )
    .unwrap();
    apply_policy(&mut diagnostics, &plan);
    let scope = ScopeReport::files_scope(
        "0".repeat(40),
        vec![
            "z.rs".to_owned(),
            workspace_path::normalize_changed("src/100%.rs").unwrap(),
            "src/lib.rs".to_owned(),
        ],
    )
    .unwrap();

    project_diagnostics(&mut diagnostics, &scope);
    diagnostics.sort_by(compare_diagnostics);

    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == Severity::Error)
    );
    let selected_ids: BTreeSet<_> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.id.as_str())
        .collect();
    assert_eq!(
        selected_ids,
        BTreeSet::from([selected_id.as_str(), encoded_path_id.as_str()])
    );
    assert_eq!(Summary::from_diagnostics(&diagnostics).errors, 2);
    assert_eq!(
        evaluate_gate(Status::Complete, &diagnostics, BlockingLevel::Error),
        GateReport {
            blocking: BlockingLevel::Error,
            status: GateStatus::Failed,
            blocking_diagnostics: Some(2),
        }
    );

    let empty_scope = ScopeReport::files_scope("0".repeat(40), Vec::new()).unwrap();
    project_diagnostics(&mut diagnostics, &empty_scope);
    assert_eq!(Summary::from_diagnostics(&diagnostics), Summary::default());
    assert_eq!(
        evaluate_gate(Status::Complete, &diagnostics, BlockingLevel::Error),
        GateReport {
            blocking: BlockingLevel::Error,
            status: GateStatus::Passed,
            blocking_diagnostics: Some(0),
        }
    );
}

#[test]
fn gate_counts_deduplicated_diagnostics_and_exit_codes_follow_both_states() {
    let workspace = fixture("clean");
    let home = HomePaths::default();
    let mut diagnostics = normalize_diagnostics(
        &[
            compiler_message(Some("E0001"), "error", "error", "src/lib.rs", 1),
            compiler_message(None, "warning", "warning", "src/lib.rs", 2),
            compiler_message(None, "note", "info", "src/lib.rs", 3),
            compiler_message(None, "future", "unknown", "src/lib.rs", 4),
        ],
        Some(&workspace),
        None,
        &home,
    );
    diagnostics[0].occurrences = 99;

    let none = evaluate_gate(Status::Complete, &diagnostics, BlockingLevel::None);
    let error = evaluate_gate(Status::Complete, &diagnostics, BlockingLevel::Error);
    let warning = evaluate_gate(Status::Complete, &diagnostics, BlockingLevel::Warning);
    let incomplete = evaluate_gate(Status::Incomplete, &diagnostics, BlockingLevel::Warning);
    assert_eq!(none.blocking_diagnostics, Some(0));
    assert_eq!(none.status, GateStatus::Passed);
    assert_eq!(error.blocking_diagnostics, Some(1));
    assert_eq!(error.status, GateStatus::Failed);
    assert_eq!(warning.blocking_diagnostics, Some(2));
    assert_eq!(warning.status, GateStatus::Failed);
    assert_eq!(incomplete.blocking_diagnostics, None);
    assert_eq!(incomplete.status, GateStatus::NotEvaluated);

    let mut report = report_with_diagnostics(diagnostics);
    report.gate = none;
    assert_eq!(report.exit_code(), 0);
    report.gate = error;
    assert_eq!(report.exit_code(), 1);
    report.status = Status::Incomplete;
    report.gate = incomplete.clone();
    assert_eq!(report.exit_code(), 1);
    report.status = Status::Failed;
    assert_eq!(report.exit_code(), 2);
}

fn dependency(
    name: &str,
    source: Option<&str>,
    requirement: &str,
    rename: Option<&str>,
    path: Option<&Path>,
) -> Value {
    serde_json::json!({
        "name": name,
        "source": source,
        "req": requirement,
        "kind": null,
        "rename": rename,
        "optional": false,
        "uses_default_features": true,
        "features": [],
        "target": null,
        "registry": null,
        "path": path.map(|path| path.to_string_lossy().into_owned()),
    })
}

fn cargo_health_metadata(order: &[usize]) -> Metadata {
    let workspace = fixture("clean").canonicalize().unwrap();
    let package_id = format!("path+file://{}#example@0.1.0", workspace.display());
    let dependencies = [
        dependency(
            "serde",
            Some("registry+https://github.com/rust-lang/crates.io-index"),
            "*",
            Some("serde_alias"),
            None,
        ),
        dependency(
            "serde",
            Some("registry+https://github.com/rust-lang/crates.io-index"),
            "*",
            Some("serde_alias"),
            None,
        ),
        dependency(
            "internal_core",
            Some("git+https://credential@git.invalid/private.git?branch=main"),
            "*",
            None,
            None,
        ),
        dependency(
            "pinned_core",
            Some(
                "git+https://credential@git.invalid/pinned.git?rev=0123456789abcdef0123456789abcdef01234567",
            ),
            "*",
            None,
            None,
        ),
        dependency(
            "bounded",
            Some("registry+https://github.com/rust-lang/crates.io-index"),
            "1.*",
            None,
            None,
        ),
    ];
    let dependencies: Vec<_> = order
        .iter()
        .map(|&index| dependencies[index].clone())
        .collect();
    let manifest_path = workspace.join("Cargo.toml");
    let target_directory = workspace.join("target");

    serde_json::from_value(serde_json::json!({
        "packages": [{
            "name": "example",
            "version": "0.1.0",
            "id": package_id,
            "license": null,
            "license_file": null,
            "description": null,
            "source": null,
            "dependencies": dependencies,
            "targets": [],
            "features": {},
            "manifest_path": manifest_path,
            "metadata": null,
            "publish": [],
            "authors": [],
            "categories": [],
            "keywords": [],
            "readme": null,
            "repository": null,
            "homepage": null,
            "documentation": null,
            "edition": "2024",
            "links": null,
            "default_run": null,
            "rust_version": null
        }],
        "workspace_members": [package_id],
        "workspace_default_members": [package_id],
        "resolve": null,
        "workspace_root": workspace,
        "target_directory": target_directory,
        "build_directory": target_directory,
        "metadata": null,
        "version": 1
    }))
    .unwrap()
}

fn scan(
    messages: Vec<CapturedMessage>,
    exit_code: i32,
    success: bool,
    build_finished: bool,
) -> ScanExecution {
    ScanExecution {
        command: vec![
            "cargo".to_owned(),
            "clippy".to_owned(),
            "--workspace".to_owned(),
            "--no-deps".to_owned(),
            "--message-format=json".to_owned(),
        ],
        exit_code: Some(exit_code),
        exit_success: Some(success),
        build_finished: Some(build_finished),
        noise_lines: 0,
        malformed_messages: 0,
        messages,
        errors: Vec::new(),
    }
}

/// The dependency pack now runs inside the execution, like the source
/// kernel. The test constructors reproduce that wiring rather than letting
/// normalization run it again.
fn cargo_health_scan(metadata: &Metadata) -> Option<cargo_health::CargoHealthScan> {
    Some(cargo_health::inspect(metadata, &PolicyPlan::default()))
}

fn complete_analysis_side(
    metadata: Metadata,
    message: &'static str,
    path: &str,
) -> ExecutionResult {
    let manifest_path = metadata
        .workspace_root
        .join("Cargo.toml")
        .into_std_path_buf();
    ExecutionResult {
        manifest_path: Some(manifest_path),
        structure: None,
        cargo_health: cargo_health_scan(&metadata),
        repo: None,
        metadata: Some(metadata),
        toolchain: None,
        scan: ClippyExecution::Finished(scan(
            vec![compiler_message(
                Some("clippy::todo"),
                "warning",
                message,
                path,
                2,
            )],
            0,
            true,
            true,
        )),
        source_measurement: None,
        source: Some(crate::source_kernel::SourceScan {
            candidates: vec![crate::source_kernel::Candidate {
                definition: &crate::policy::SOURCE_DYNAMIC_SHELL,
                message,
                package: Some("example".to_owned()),
                target: Some("example".to_owned()),
                path: path.to_owned(),
                span: crate::source_text::SourceSpan {
                    line_start: 3,
                    column_start: 1,
                    line_end: 3,
                    column_end: 8,
                },
            }],
            errors: Vec::new(),
            counters: crate::source_kernel::AnalysisCounters::default(),
        }),
        error: None,
    }
}

#[test]
fn baseline_analysis_normalizes_every_active_producer_on_both_sides() {
    let metadata = cargo_health_metadata(&[0, 1, 2, 3, 4]);
    let baseline = complete_analysis_side(
        metadata.clone(),
        "baseline producer diagnostic",
        "src/base.rs",
    );
    let current =
        complete_analysis_side(metadata, "current producer diagnostic", "src/current.rs");
    let execution = BaselineExecution::from_complete_sides(baseline, current);
    let analysis = analyze_baseline_execution(
        execution,
        &PolicyPlan::default(),
        ScopeReport::baseline_scope("1".repeat(40)),
    );

    let baseline_codes: BTreeSet<_> = analysis
        .baseline
        .diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic.code.as_deref())
        .collect();
    assert!(baseline_codes.contains("clippy::todo"));
    assert!(baseline_codes.contains("rust_doctor::cargo::unbounded_registry_dependency"));
    assert!(baseline_codes.contains("rust_doctor::source::dynamic_shell_command"));
    let current_codes: BTreeSet<_> = analysis
        .current
        .diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic.code.as_deref())
        .collect();
    assert_eq!(current_codes, baseline_codes);

    let report = analysis.into_report();
    assert_eq!(report.gate.status, GateStatus::Passed);
    assert!(report.delta.is_some());
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_deref() == Some("clippy::todo"))
            .count(),
        1
    );
}

#[test]
fn cargo_health_joins_the_v4_pipeline_without_restamping_compiler_ids() {
    let metadata = cargo_health_metadata(&[0, 1, 2, 3, 4]);
    let workspace = metadata.workspace_root.as_std_path();
    let messages = vec![compiler_message(
        Some("clippy::lint"),
        "warning",
        "compiler warning",
        "src/lib.rs",
        2,
    )];
    let compiler_only =
        normalize_diagnostics(&messages, Some(workspace), None, &HomePaths::default());
    let mixed = normalize_diagnostics(
        &messages,
        Some(workspace),
        Some(&metadata),
        &HomePaths::default(),
    );

    assert_eq!(mixed.len(), 3);
    let compiler = mixed
        .iter()
        .find(|diagnostic| diagnostic.source == DiagnosticSource::Clippy)
        .expect("compiler diagnostic should remain");
    assert_eq!(compiler.id, compiler_only[0].id);
    assert_eq!(compiler.occurrences, compiler_only[0].occurrences);

    let registry = mixed
        .iter()
        .find(|diagnostic| {
            diagnostic.code.as_deref()
                == Some("rust_doctor::cargo::unbounded_registry_dependency")
        })
        .expect("registry finding should exist");
    assert_eq!(registry.source, DiagnosticSource::RustDoctor);
    assert_eq!(registry.severity, Severity::Warning);
    assert_eq!(registry.category.as_deref(), Some("reliability"));
    assert_eq!(
        registry.message,
        "Registry dependency \"serde_alias\" uses an unbounded \"*\" version requirement."
    );
    assert_eq!(
        registry.help.as_deref(),
        Some(
            "Replace the unbounded version requirement with the minimum compatible version intended by the project."
        )
    );
    assert_eq!(registry.package.as_deref(), Some("example"));
    assert_eq!(registry.target, None);
    assert_eq!(registry.path.as_deref(), Some("Cargo.toml"));
    assert_eq!(registry.span, None);
    assert_eq!(registry.occurrences, 2);

    let git = mixed
        .iter()
        .find(|diagnostic| {
            diagnostic.code.as_deref() == Some("rust_doctor::cargo::unpinned_git_dependency")
        })
        .expect("Git finding should exist");
    assert_eq!(git.source.to_string(), "rust-doctor");
    assert_eq!(git.category.as_deref(), Some("security"));
    assert_eq!(
        git.message,
        "Git dependency \"internal_core\" is not pinned to a full commit revision."
    );
    assert!(!format!("{mixed:?}").contains("git.invalid"));

    let report = report_with_diagnostics(mixed);
    assert_eq!(report.schema_version, SCHEMA_VERSION);
    assert_eq!(report.summary.warnings, 3);
    assert_eq!(report.summary.total, 3);
    let mut rendered = Vec::new();
    crate::render::render_json(&report, &mut rendered).unwrap();
    let rendered: Value = serde_json::from_slice(&rendered).unwrap();
    assert_eq!(rendered["diagnostics"][0]["source"], "rust-doctor");
}

#[test]
fn native_warnings_follow_existing_complete_incomplete_and_failed_statuses() {
    let metadata = cargo_health_metadata(&[0, 1, 2, 3, 4]);
    let manifest_path = metadata
        .workspace_root
        .join("Cargo.toml")
        .into_std_path_buf();
    let expected_command = scan(Vec::new(), 0, true, true).command;
    let complete = from_execution(ExecutionResult {
        manifest_path: Some(manifest_path.clone()),
        structure: None,
        cargo_health: cargo_health_scan(&metadata),
        repo: None,
        metadata: Some(metadata.clone()),
        toolchain: None,
        scan: ClippyExecution::Finished(scan(Vec::new(), 0, true, true)),
        source: None,
        source_measurement: None,
        error: None,
    });
    assert_eq!(complete.status, Status::Complete);
    assert!(complete.complete);
    assert_eq!(complete.exit_code(), 0);
    assert_eq!(complete.scan.command, Some(expected_command.clone()));
    assert_eq!(complete.diagnostics.len(), 2);

    let incomplete = from_execution(ExecutionResult {
        manifest_path: Some(manifest_path),
        structure: None,
        cargo_health: cargo_health_scan(&metadata),
        repo: None,
        metadata: Some(metadata),
        toolchain: None,
        scan: ClippyExecution::Finished(scan(
            vec![compiler_message(
                Some("E0001"),
                "error",
                "compiler error",
                "src/lib.rs",
                2,
            )],
            101,
            false,
            false,
        )),
        source: None,
        source_measurement: None,
        error: None,
    });
    assert_eq!(incomplete.status, Status::Incomplete);
    assert!(!incomplete.complete);
    assert_eq!(incomplete.exit_code(), 1);
    assert_eq!(incomplete.scan.command, Some(expected_command));
    assert_eq!(incomplete.diagnostics.len(), 3);
    assert!(
        incomplete
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.source == DiagnosticSource::RustDoctor)
    );

    let failed = from_execution(ExecutionResult {
        manifest_path: None,
        metadata: None,
        structure: None,
        cargo_health: None,
        repo: None,
        toolchain: None,
        scan: ClippyExecution::NotRun,
        source: None,
        source_measurement: None,
        error: Some(InternalError {
            stage: "metadata",
            code: "cargo-metadata",
            message: "metadata unavailable".to_owned(),
        }),
    });
    assert_eq!(failed.status, Status::Failed);
    assert_eq!(failed.exit_code(), 2);
    assert!(failed.diagnostics.is_empty());
}

#[test]
fn source_candidates_share_identity_while_source_errors_only_make_scans_incomplete() {
    let metadata = cargo_health_metadata(&[0, 1, 2, 3, 4]);
    let workspace = metadata.workspace_root.as_std_path();
    let compiler_messages = vec![
        compiler_message(
            Some("clippy::lint"),
            "warning",
            "compiler warning",
            "src/lib.rs",
            2,
        ),
        compiler_message(
            Some("clippy::lint"),
            "warning",
            "compiler warning",
            "src/lib.rs",
            2,
        ),
    ];
    let compiler_only = normalize_diagnostics(
        &compiler_messages,
        Some(workspace),
        Some(&metadata),
        &HomePaths::default(),
    );
    let source = source_kernel::SourceScan {
        candidates: vec![source_kernel::Candidate {
            definition: &crate::policy::SOURCE_DYNAMIC_SHELL,
            message: "A dynamic value is interpolated into a shell command string.",
            package: Some("example".to_owned()),
            target: None,
            path: "src/source.rs".to_owned(),
            span: crate::source_text::SourceSpan {
                line_start: 4,
                column_start: 8,
                line_end: 4,
                column_end: 24,
            },
        }],
        errors: vec![source_kernel::SourceError {
            code: "parse-error",
            message: "Source path \"src/broken.rs\" contains 1 parse errors.".to_owned(),
        }],
        counters: source_kernel::AnalysisCounters::default(),
    };
    let report = from_execution(ExecutionResult {
        manifest_path: Some(workspace.join("Cargo.toml")),
        structure: None,
        cargo_health: cargo_health_scan(&metadata),
        repo: None,
        metadata: Some(metadata),
        toolchain: None,
        scan: ClippyExecution::Finished(scan(compiler_messages, 0, true, true)),
        source: Some(source),
        source_measurement: None,
        error: None,
    });

    assert_eq!(report.status, Status::Incomplete);
    assert_eq!(report.exit_code(), 1);
    let compiler = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.source == DiagnosticSource::Clippy)
        .unwrap();
    let baseline = compiler_only
        .iter()
        .find(|diagnostic| diagnostic.source == DiagnosticSource::Clippy)
        .unwrap();
    assert_eq!(compiler.id, baseline.id);
    assert_eq!(compiler.occurrences, baseline.occurrences);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_deref() == Some("rust_doctor::source::dynamic_shell_command")
            && diagnostic.span.is_some()
    }));
    assert!(report.errors.iter().any(|error| {
        error.stage == "source"
            && error.code == "parse-error"
            && !error.message.contains(env!("CARGO_MANIFEST_DIR"))
    }));
}

#[test]
fn twenty_mixed_permutations_render_identically() {
    let mut expected = None;
    let mut order = [0, 1, 2, 3, 4];
    let mut seen = BTreeSet::new();

    for permutation in 0..20 {
        assert!(seen.insert(order));
        let metadata = cargo_health_metadata(&order);
        let workspace = metadata.workspace_root.as_std_path();
        let messages = if permutation % 2 == 0 {
            vec![
                compiler_message(Some("clippy::lint"), "warning", "beta", "src/b.rs", 3),
                compiler_message(Some("E0001"), "error", "alpha", "src/a.rs", 2),
            ]
        } else {
            vec![
                compiler_message(Some("E0001"), "error", "alpha", "src/a.rs", 2),
                compiler_message(Some("clippy::lint"), "warning", "beta", "src/b.rs", 3),
            ]
        };
        let diagnostics = normalize_diagnostics(
            &messages,
            Some(workspace),
            Some(&metadata),
            &HomePaths::default(),
        );
        let mut rendered = Vec::new();
        crate::render::render_json(&report_with_diagnostics(diagnostics), &mut rendered)
            .unwrap();
        match expected.as_ref() {
            Some(expected) => assert_eq!(&rendered, expected),
            None => expected = Some(rendered),
        }
        if permutation < 19 {
            assert!(next_permutation(&mut order));
        }
    }
    assert_eq!(seen.len(), 20);
}

/// The convention is Cargo's, and the outermost directory that matches decides:
/// a file under `benches/` is bench material whatever it is named, and the
/// `tests.rs` spelling only answers for a file no directory above it claims.
/// Everything else is unmarked, because a file this reader does not recognize
/// has to keep weighing on the score.
#[test]
fn a_conventional_path_names_the_target_it_belongs_to() {
    let context = DiagnosticContext::from_conventional_path;
    assert_eq!(context("src/tests/mod.rs"), Some(DiagnosticContext::Tests));
    assert_eq!(context("tests/regression/case.rs"), Some(DiagnosticContext::Tests));
    assert_eq!(context("src/modules/tests.rs"), Some(DiagnosticContext::Tests));
    assert_eq!(context("benches/tests.rs"), Some(DiagnosticContext::Benchmark));
    assert_eq!(context("examples/demo.rs"), Some(DiagnosticContext::Example));
    assert_eq!(context("src/lib.rs"), None);
    // A directory, not a stem: `tests_util` is a package name, and
    // `src/latest.rs` merely ends in the four letters the file name is read on.
    assert_eq!(context("tests_util/src/lib.rs"), None);
    assert_eq!(context("src/latest.rs"), None);
}
