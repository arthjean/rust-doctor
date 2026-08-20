//! A report assembled from what an execution came back with.
//!
//! Nothing here asks the execution back for what it was given: the metadata,
//! the toolchain and the scan are read once, in the order the published shape
//! declares them. The three entry points are a planned run, a baseline
//! comparison and a failure that happened before either existed, and [`Origin`]
//! is what tells them apart.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cargo_metadata::Metadata;

use super::normalize::{
    canonical_severity, compare_diagnostics, compare_optional_paths, merge_compiler_messages,
    merge_diagnostic, normalize_cargo_health_candidate, normalize_repo_finding,
    normalize_source_candidate, normalize_structure_finding,
};
use super::sanitize::{HomePaths, home_paths, sanitize_text};
use super::{
    Diagnostic, DiagnosticContext, GateReport, GateStatus, InspectReport, PackageReport,
    PolicyReport, ProjectReport, ReportError, SCHEMA_VERSION, ScanReport, Severity, Status,
    Summary, ToolchainReport,
};
use crate::audit::{self, Audit, SourceFileInventory};
use crate::delta::DeltaReport;
use crate::execution::{BaselineExecution, ExecutionResult, ScanExecution};
use crate::git_scope::ScopeReport;
use crate::internal_error::InternalError;
use crate::policy::{BlockingLevel, PolicyError, PolicyPlan};
use crate::workspace_path;

/// What the run that produced a report got as far as.
///
/// A run either compiled a plan and resolved a scope, or it failed before
/// either existed. The two used to reach `from_origin` as an
/// `Option<&PolicyPlan>`, a `BlockingLevel` and an `Option<ScopeReport>` side
/// by side, three parameters naming eight combinations for the two that can
/// occur: every caller holding a plan passed that plan's own blocking level,
/// and the single caller without a plan was also the one without a scope.
enum Origin<'a> {
    Planned(&'a PolicyPlan, ScopeReport),
    Unplanned(BlockingLevel),
}

#[cfg(test)]
pub(crate) fn from_execution(result: ExecutionResult, plan: &PolicyPlan) -> InspectReport {
    from_origin(result, Origin::Planned(plan, ScopeReport::full()))
}

pub(crate) fn from_execution_scoped(
    result: ExecutionResult,
    plan: &PolicyPlan,
    scope: ScopeReport,
) -> InspectReport {
    from_origin(result, Origin::Planned(plan, scope))
}

pub(crate) fn from_baseline_execution(
    execution: Result<BaselineExecution, Box<ExecutionResult>>,
    plan: &PolicyPlan,
    scope: ScopeReport,
) -> InspectReport {
    match execution {
        Ok(execution) => analyze_baseline_execution(execution, plan, scope).into_report(),
        Err(execution) => force_baseline_gate(from_origin(
            *execution,
            Origin::Planned(plan, scope),
        )),
    }
}

pub(super) struct BaselineAnalysis {
    pub(super) baseline: InspectReport,
    pub(super) current: InspectReport,
    pub(super) baseline_root: Option<PathBuf>,
    pub(super) current_root: Option<PathBuf>,
}

impl BaselineAnalysis {
    pub(super) fn into_report(self) -> InspectReport {
        let Self {
            baseline,
            mut current,
            baseline_root,
            current_root,
        } = self;
        if baseline.status != Status::Complete || current.status != Status::Complete {
            return force_baseline_gate(current);
        }
        let (Some(baseline_root), Some(current_root)) = (baseline_root, current_root) else {
            return baseline_report_failure(current, crate::baseline::scan_incomplete());
        };
        let delta = match crate::delta::compute(
            &baseline.diagnostics,
            &current.diagnostics,
            &baseline_root,
            &current_root,
        ) {
            Ok(delta) => delta,
            Err(error) => return baseline_report_failure(current, error),
        };
        current.gate = evaluate_baseline_gate(
            current.status,
            &current.diagnostics,
            &delta,
            current.gate.blocking,
        );
        let introduced: BTreeSet<_> = delta.introduced.iter().map(String::as_str).collect();
        let scoped: Vec<_> = current
            .diagnostics
            .iter()
            .filter(|diagnostic| introduced.contains(diagnostic.id.as_str()))
            .cloned()
            .collect();
        current.audit = current.audit.rebuild_for_scope(current.status, &scoped);
        current.delta = Some(delta);
        current
    }
}

pub(super) fn analyze_baseline_execution(
    execution: BaselineExecution,
    plan: &PolicyPlan,
    scope: ScopeReport,
) -> BaselineAnalysis {
    let (baseline, current) = execution.into_sides();
    let baseline_root = baseline
        .metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path().to_path_buf());
    let current_root = current
        .metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path().to_path_buf());
    BaselineAnalysis {
        baseline: from_origin(baseline, Origin::Planned(plan, scope.clone())),
        current: from_origin(current, Origin::Planned(plan, scope)),
        baseline_root,
        current_root,
    }
}

/// A baseline comparison that could not be published, whether the snapshot
/// failed to build, the delta failed to compute or the cleanup failed
/// afterwards: the report keeps its shape, drops what it can no longer vouch
/// for, and names the failure.
pub(crate) fn baseline_report_failure(
    mut report: InspectReport,
    error: InternalError,
) -> InspectReport {
    let source_files = report.audit.source_files;
    let production_lines = report.audit.production_lines;
    report.status = Status::Failed;
    report.complete = false;
    report.diagnostics.clear();
    report.delta = None;
    report.errors = vec![ReportError {
        stage: error.stage.to_owned(),
        code: error.code.to_owned(),
        message: error.message,
    }];
    report.summary = Summary::default();
    report.audit = Audit::build(
        source_files,
        production_lines,
        report.status,
        &report.diagnostics,
    );
    force_baseline_gate(report)
}

fn force_baseline_gate(mut report: InspectReport) -> InspectReport {
    report.delta = None;
    report.gate = GateReport {
        blocking: report.gate.blocking,
        status: GateStatus::NotEvaluated,
        blocking_diagnostics: None,
    };
    report
}

fn from_origin(result: ExecutionResult, origin: Origin<'_>) -> InspectReport {
    let (plan, scope, blocking) = match origin {
        Origin::Planned(plan, scope) => (Some(plan), Some(scope), plan.blocking()),
        Origin::Unplanned(blocking) => (None, None, blocking),
    };
    let home = home_paths();
    let status = classify(&result);
    let workspace_root = result
        .metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path());
    let diagnostics = plan.map_or_else(Vec::new, |plan| {
        diagnostics_from_execution(&result, plan, scope.as_ref(), &home)
    });
    let summary = Summary::from_diagnostics(&diagnostics);
    let gate = evaluate_gate(status, &diagnostics, blocking);
    let project = project_report(result.manifest_path.as_deref(), result.metadata.as_ref());
    let scan = scan_report(result.scan.finished());
    let errors = report_errors(&result, workspace_root, &home);
    let source_inventory =
        result
            .metadata
            .as_ref()
            .map_or_else(SourceFileInventory::default, |metadata| {
                audit::source_file_inventory(
                    metadata,
                    result.scan.finished(),
                    result.source_measurement.as_ref(),
                )
            });
    let audit = Audit::build_from_inventory(source_inventory, status, &diagnostics);

    InspectReport {
        schema_version: SCHEMA_VERSION,
        audit,
        status,
        complete: status == Status::Complete,
        policy: plan.map(PolicyReport::from_plan),
        scope,
        project,
        toolchain: {
            // Three versions or none: the shape the result carries is unfolded
            // here, at the boundary whose schema publishes three nullable
            // fields.
            let toolchain = result.toolchain.as_ref();
            let published = |value: &str| sanitize_text(value, workspace_root, &home);
            ToolchainReport {
                rustc: toolchain.map(|toolchain| published(&toolchain.rustc)),
                cargo: toolchain.map(|toolchain| published(&toolchain.cargo)),
                clippy: toolchain.map(|toolchain| published(&toolchain.clippy)),
            }
        },
        scan,
        diagnostics,
        delta: None,
        errors,
        summary,
        gate,
    }
}

/// Every diagnostic the five producers published, merged on identity, ordered
/// once.
///
/// The map stays open across all five. `merge_diagnostic` pairs on the
/// content-derived identity and `merge_optional_context` keeps a field only
/// when every occurrence agrees on it, so merging is commutative and a
/// producer costs one insertion per finding. Closing the map between producers
/// is what used to make three word-for-word `merge_*` functions necessary,
/// each rebuilding the map it had just been handed and re-sorting behind it:
/// the vector was sorted five times per scan, and the guard that skipped them
/// on a failed scan was spelled three more times after the `match` above had
/// already answered it.
fn diagnostics_from_execution(
    result: &ExecutionResult,
    plan: &PolicyPlan,
    scope: Option<&ScopeReport>,
    home: &HomePaths,
) -> Vec<Diagnostic> {
    // A failed scan publishes no finding, and nothing is published relative to
    // a workspace root the run never resolved. Both are answered once, here,
    // rather than by every producer below.
    if classify(result) == Status::Failed {
        return Vec::new();
    }
    let Some(workspace_root) = result
        .metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path())
    else {
        return Vec::new();
    };

    let mut merged = BTreeMap::<String, Diagnostic>::new();
    for candidate in result.cargo_health.iter().flat_map(|scan| &scan.candidates) {
        merge_diagnostic(
            &mut merged,
            normalize_cargo_health_candidate(candidate, workspace_root, home),
        );
    }
    merge_compiler_messages(
        &mut merged,
        result
            .scan
            .finished()
            .map(|scan| scan.messages.as_slice())
            .unwrap_or_default(),
        workspace_root,
        result.metadata.as_ref(),
        home,
        plan,
    );
    for candidate in result.source.iter().flat_map(|scan| &scan.candidates) {
        merge_diagnostic(
            &mut merged,
            normalize_source_candidate(candidate, workspace_root, home),
        );
    }
    for finding in result.structure.iter().flat_map(|scan| &scan.findings) {
        merge_diagnostic(
            &mut merged,
            normalize_structure_finding(finding, workspace_root, home),
        );
    }
    for finding in result.repo.iter().flat_map(|scan| &scan.findings) {
        merge_diagnostic(
            &mut merged,
            normalize_repo_finding(finding, workspace_root, home),
        );
    }

    let mut diagnostics: Vec<_> = merged.into_values().collect();
    apply_policy(&mut diagnostics, plan);
    if let Some(scope) = scope {
        project_diagnostics(&mut diagnostics, scope);
    }
    diagnostics.sort_by(compare_diagnostics);
    diagnostics
}

pub(crate) fn preparation_failure(
    result: ExecutionResult,
    blocking: BlockingLevel,
) -> InspectReport {
    from_origin(result, Origin::Unplanned(blocking))
}

pub(crate) fn policy_failure(error: PolicyError, blocking: BlockingLevel) -> InspectReport {
    immediate_failure(
        ReportError {
            stage: "policy".to_owned(),
            code: error.code.to_owned(),
            message: error.message.to_owned(),
        },
        blocking,
    )
}

pub(crate) fn scope_failure(error: InternalError, blocking: BlockingLevel) -> InspectReport {
    immediate_failure(
        ReportError {
            stage: error.stage.to_owned(),
            code: error.code.to_owned(),
            message: error.message,
        },
        blocking,
    )
}

fn immediate_failure(error: ReportError, blocking: BlockingLevel) -> InspectReport {
    InspectReport {
        schema_version: SCHEMA_VERSION,
        audit: Audit::build(0, 0, Status::Failed, &[]),
        status: Status::Failed,
        complete: false,
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
            exit_code: None,
            build_finished: None,
            noise_lines: None,
        },
        diagnostics: Vec::new(),
        delta: None,
        errors: vec![error],
        summary: Summary::default(),
        gate: GateReport {
            blocking,
            status: GateStatus::NotEvaluated,
            blocking_diagnostics: None,
        },
    }
}

pub(super) fn project_diagnostics(diagnostics: &mut Vec<Diagnostic>, scope: &ScopeReport) {
    diagnostics.retain(|diagnostic| scope.includes(diagnostic.path.as_deref()));
}

fn classify(result: &ExecutionResult) -> Status {
    if result.error.is_some() {
        return Status::Failed;
    }
    if result.is_complete() {
        return Status::Complete;
    }
    if !result.scan.has_outcome() {
        return Status::Failed;
    }
    Status::Incomplete
}

fn project_report(
    manifest_path: Option<&Path>,
    metadata: Option<&Metadata>,
) -> Option<ProjectReport> {
    let metadata = metadata?;
    let workspace_root = metadata.workspace_root.as_std_path();
    let manifest_path = manifest_path
        .and_then(|path| workspace_path::normalize(workspace_root, path))
        .unwrap_or_else(|| "Cargo.toml".to_owned());
    let mut packages: Vec<_> = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| {
            let manifest_path =
                workspace_path::normalize(workspace_root, package.manifest_path.as_std_path());
            let mut targets: Vec<_> = package
                .targets
                .iter()
                .map(|target| target.name.to_string())
                .collect();
            targets.sort();
            PackageReport {
                name: package.name.to_string(),
                manifest_path,
                targets,
            }
        })
        .collect();
    packages.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| compare_optional_paths(&left.manifest_path, &right.manifest_path))
    });

    Some(ProjectReport {
        workspace_root: ".".to_owned(),
        manifest_path,
        packages,
    })
}

fn scan_report(scan: Option<&ScanExecution>) -> ScanReport {
    match scan {
        Some(scan) => ScanReport {
            command: Some(scan.command.clone()),
            exit_code: scan.exit_code,
            build_finished: scan.build_finished,
            noise_lines: Some(scan.noise_lines),
        },
        None => ScanReport {
            command: None,
            exit_code: None,
            build_finished: None,
            noise_lines: None,
        },
    }
}

fn report_errors(
    result: &ExecutionResult,
    workspace_root: Option<&Path>,
    home: &HomePaths,
) -> Vec<ReportError> {
    let mut errors = Vec::new();
    if let Some(error) = result.error.as_ref() {
        errors.push(normalize_error(error, workspace_root, home));
    }
    if let Some(scan) = result.scan.finished() {
        errors.extend(
            scan.errors
                .iter()
                .map(|error| normalize_error(error, workspace_root, home)),
        );
        match scan.exit_code {
            Some(code) if code != 0 || scan.exit_success != Some(true) => {
                errors.push(ReportError {
                    stage: "execution".to_owned(),
                    code: "clippy-exit".to_owned(),
                    message: format!("Clippy exited with status {code}"),
                });
            }
            None => errors.push(ReportError {
                stage: "execution".to_owned(),
                code: "clippy-exit".to_owned(),
                message: "Clippy terminated without an exit code".to_owned(),
            }),
            Some(_) => {}
        }
        match scan.build_finished {
            Some(false) => errors.push(ReportError {
                stage: "execution".to_owned(),
                code: "build-failed".to_owned(),
                message: "Cargo reported build-finished.success: false".to_owned(),
            }),
            None => errors.push(ReportError {
                stage: "execution".to_owned(),
                code: "build-finished-missing".to_owned(),
                message: "Cargo did not emit build-finished".to_owned(),
            }),
            Some(true) => {}
        }
        if scan.malformed_messages > 0 {
            errors.push(ReportError {
                stage: "parsing".to_owned(),
                code: "malformed-message".to_owned(),
                message: "malformed Cargo message".to_owned(),
            });
        }
    }
    // One list of the producers, shared with `ExecutionResult::is_complete`: a
    // pass cannot publish an error here and still let the report call itself
    // authoritative, which is what four hand-written blocks let `cargo_health`
    // do.
    errors.extend(result.producer_errors().map(|error| ReportError {
        stage: error.stage.to_owned(),
        code: error.code.to_owned(),
        message: sanitize_text(error.message, workspace_root, home),
    }));
    errors.sort_by(|left, right| {
        (&left.stage, &left.code, &left.message).cmp(&(&right.stage, &right.code, &right.message))
    });
    errors.dedup_by(|left, right| left == right);
    errors
}

fn normalize_error(
    error: &InternalError,
    workspace_root: Option<&Path>,
    home: &HomePaths,
) -> ReportError {
    ReportError {
        stage: error.stage.to_owned(),
        code: error.code.to_owned(),
        message: sanitize_text(&error.message, workspace_root, home),
    }
}

pub(super) fn apply_policy(diagnostics: &mut [Diagnostic], plan: &PolicyPlan) {
    for diagnostic in diagnostics {
        if let Some(level) = diagnostic
            .code
            .as_deref()
            .and_then(|code| plan.restamp_level(code))
        {
            diagnostic.severity = canonical_severity(level);
        }
    }
}

pub(super) fn evaluate_gate(
    status: Status,
    diagnostics: &[Diagnostic],
    blocking: BlockingLevel,
) -> GateReport {
    if status != Status::Complete {
        return GateReport {
            blocking,
            status: GateStatus::NotEvaluated,
            blocking_diagnostics: None,
        };
    }

    let blocking_diagnostics = diagnostics
        .iter()
        .filter(|diagnostic| DiagnosticContext::weighs(diagnostic) && is_blocking(diagnostic, blocking))
        .count();
    evaluated_gate(blocking, blocking_diagnostics)
}

fn evaluate_baseline_gate(
    status: Status,
    diagnostics: &[Diagnostic],
    delta: &DeltaReport,
    blocking: BlockingLevel,
) -> GateReport {
    if status != Status::Complete {
        return GateReport {
            blocking,
            status: GateStatus::NotEvaluated,
            blocking_diagnostics: None,
        };
    }
    let introduced = delta.introduced.iter().collect::<BTreeSet<_>>();
    let blocking_diagnostics = diagnostics
        .iter()
        .filter(|diagnostic| {
            introduced.contains(&diagnostic.id) && is_blocking(diagnostic, blocking)
        })
        .count();
    evaluated_gate(blocking, blocking_diagnostics)
}

fn is_blocking(diagnostic: &Diagnostic, blocking: BlockingLevel) -> bool {
    match blocking {
        BlockingLevel::None => false,
        BlockingLevel::Error => diagnostic.severity == Severity::Error,
        BlockingLevel::Warning => {
            matches!(diagnostic.severity, Severity::Error | Severity::Warning)
        }
    }
}

fn evaluated_gate(blocking: BlockingLevel, blocking_diagnostics: usize) -> GateReport {
    GateReport {
        blocking,
        status: if blocking_diagnostics == 0 {
            GateStatus::Passed
        } else {
            GateStatus::Failed
        },
        blocking_diagnostics: Some(blocking_diagnostics),
    }
}

