use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cargo_metadata::Metadata;

use super::normalize::{
    canonical_severity, compare_diagnostics, compare_optional_paths, merge_compiler_messages,
    merge_diagnostic, normalize_cargo_health_candidate, normalize_repo_finding,
    normalize_source_candidate, normalize_structure_finding,
};
use super::exclusion::{self, Exclusion};
use super::sanitize::{HomePaths, home_paths, sanitize_text};
use super::{
    Diagnostic, DiagnosticContext, DiagnosticSpan, GateReport, GateStatus, InspectReport,
    NotEvaluated, PackageReport, PolicyReport, ProjectReport, RelatedLocation, ReportError,
    SCHEMA_VERSION, ScanReport, Severity, Status, Summary, ToolchainReport,
};
use crate::audit::{self, Audit, ScoreReason, SourceFileInventory};
use crate::cargo_health::{self, AdvisoryOutcome};
use crate::delta::{DeltaReport, DiffMap};
use crate::execution::{
    BaseSide, BaselineExecution, ExecutionResult, ScanExecution, SideCache, interrupt,
};
use crate::git_scope::ScopeReport;
use crate::internal_error::InternalError;
use crate::policy::{BlockingLevel, DiffScope, PolicyError, PolicyPlan, Producer};
use crate::workspace_path;

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
    map: &DiffMap,
) -> InspectReport {
    match execution {
        Ok(execution) => analyze_baseline_execution(execution, plan, scope).into_report(map),
        Err(execution) => force_baseline_gate(from_origin(
            *execution,
            Origin::Planned(plan, scope),
        )),
    }
}

pub(super) struct BaselineAnalysis {
    pub(super) baseline: BaseReport,
    pub(super) current: InspectReport,
    pub(super) current_root: Option<PathBuf>,
    store: Option<(SideCache, Vec<(String, String)>)>,
}

pub(super) struct BaseReport {
    pub(super) status: Status,
    pub(super) diagnostics: Vec<Diagnostic>,
    evidence: Option<crate::delta::Evidence>,
    root: Option<PathBuf>,
}

impl BaselineAnalysis {
    pub(super) fn into_report(self, map: &DiffMap) -> InspectReport {
        let Self {
            baseline,
            mut current,
            current_root,
            store,
        } = self;
        if baseline.status != Status::Complete {
            return force_baseline_gate(current);
        }
        let evidence = match (baseline.evidence, &baseline.root) {
            (Some(evidence), _) => Some(evidence),
            (None, Some(root)) => Some(crate::delta::evidence(&baseline.diagnostics, root)),
            (None, None) => None,
        };
        if let (Some((cache, members)), Some(evidence)) = (store, &evidence) {
            cache.store(members, &baseline.diagnostics, evidence);
        }
        if current.status != Status::Complete {
            return force_baseline_gate(current);
        }
        let (Some(evidence), Some(current_root)) = (evidence, current_root) else {
            return baseline_report_failure(current, crate::baseline::scan_incomplete());
        };
        let delta = match crate::delta::compute_with(
            &baseline.diagnostics,
            &evidence,
            &current.diagnostics,
            &current_root,
            map,
        ) {
            Ok(delta) => delta,
            Err(error) => return baseline_report_failure(current, error),
        };
        if let Some(notice) = map.notice() {
            current.errors.push(ReportError {
                stage: notice.stage.to_owned(),
                code: notice.code.to_owned(),
                message: notice.message.clone(),
            });
            current.errors.sort_by(|left, right| {
                (&left.stage, &left.code, &left.message).cmp(&(&right.stage, &right.code, &right.message))
            });
        }
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
    let (baseline, current, cache, outcome) = execution.into_parts();
    let current_root = current
        .metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path().to_path_buf());
    let (baseline, store) = match baseline {
        BaseSide::Ran(baseline) => {
            let root = baseline
                .metadata
                .as_ref()
                .map(|metadata| metadata.workspace_root.as_std_path().to_path_buf());
            let members = baseline.metadata.as_ref().map(crate::execution::side_cache_members);
            let report = from_origin(*baseline, Origin::Planned(plan, scope.clone()));
            let base = BaseReport {
                status: report.status,
                diagnostics: report.diagnostics,
                evidence: None,
                root,
            };
            (base, cache.zip(members))
        }
        BaseSide::Cached(cached) => {
            let base = BaseReport {
                status: Status::Complete,
                diagnostics: cached.diagnostics,
                evidence: Some(cached.evidence),
                root: None,
            };
            (base, None)
        }
    };
    let mut current = from_origin(current, Origin::Planned(plan, scope));
    current.base_side_cache = Some(outcome);
    BaselineAnalysis {
        baseline,
        current,
        current_root,
        store,
    }
}

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

pub(crate) fn interrupted_failure(report: InspectReport) -> InspectReport {
    let clippy = report
        .errors
        .iter()
        .any(|error| error.code == interrupt::CODE && error.stage == "clippy");
    let stage = if clippy { "clippy" } else { "execution" };
    let mut report = baseline_report_failure(
        report,
        InternalError::new(stage, interrupt::CODE, interrupt::MESSAGE),
    );
    report.audit.withhold();
    report
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
    let (diagnostics, exclusion) = plan.map_or_else(
        || (Vec::new(), Exclusion::default()),
        |plan| diagnostics_from_execution(&result, plan, scope.as_ref(), &home),
    );
    let summary = Summary::from_diagnostics(&diagnostics);
    let gate = evaluate_gate(status, &diagnostics, blocking);
    let project = project_report(result.manifest_path.as_deref(), result.metadata.as_ref());
    let scan = scan_report(
        result.scan.finished(),
        &exclusion,
        result.cargo_health.as_ref().map(|health| &health.advisory),
    );
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
    let not_evaluated = not_evaluated(&result, plan);
    let unevaluated = not_evaluated.iter().any(|(_, reason)| {
        !matches!(
            reason,
            NotEvaluated::AdvisoryDbMissing
                | NotEvaluated::CrateAbsent
                | NotEvaluated::CrateVersionUnsupported
        )
    });
    let stage_reasons = errors
        .iter()
        .filter_map(|error| ScoreReason::of_error(&error.code))
        .chain(unevaluated.then_some(ScoreReason::RulesNotEvaluated))
        .collect();
    let members = member_inventories(&result, source_inventory.complete);
    let unlinted = result
        .scan
        .finished()
        .map(|scan| scan.unlinted.iter().cloned().collect())
        .unwrap_or_default();
    let audit = Audit::build_from_inventory(
        source_inventory,
        members,
        &unlinted,
        status,
        stage_reasons,
        &diagnostics,
    );

    InspectReport {
        schema_version: SCHEMA_VERSION,
        audit,
        status,
        complete: status == Status::Complete,
        policy: plan.map(|plan| PolicyReport::from_plan(plan, &not_evaluated)),
        scope,
        project,
        toolchain: {
            let toolchain = result.toolchain.as_ref();
            let published = |value: &str| sanitize_text(value, workspace_root, &home);
            ToolchainReport {
                rust_doctor: env!("CARGO_PKG_VERSION"),
                rustc: toolchain.map(|toolchain| published(&toolchain.rustc)),
                cargo: toolchain.map(|toolchain| published(&toolchain.cargo)),
                clippy: toolchain.map(|toolchain| published(&toolchain.clippy)),
                removed_lint_flags: result
                    .scan
                    .finished()
                    .map(|scan| scan.removed_lint_flags.clone())
                    .unwrap_or_default(),
            }
        },
        scan,
        diagnostics,
        suppressions: exclusion.suppressions,
        delta: None,
        errors,
        summary,
        gate,
        timings: None,
        base_side_cache: None,
    }
}

fn member_inventories(result: &ExecutionResult, complete: bool) -> Vec<(String, SourceFileInventory)> {
    let Some(metadata) = result.metadata.as_ref().filter(|_| result.error.is_none()) else {
        return Vec::new();
    };
    let measured = result
        .source_measurement
        .as_ref()
        .map(|measurement| &measurement.packages);
    let names = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| package.name.to_string())
        .filter(|name| {
            result
                .exclusions
                .packages
                .as_ref()
                .is_none_or(|selected| selected.contains(name))
        })
        .collect::<BTreeSet<_>>();
    names
        .into_iter()
        .map(|name| {
            let own = measured
                .and_then(|packages| packages.get(&name))
                .copied()
                .unwrap_or_default();
            let inventory = SourceFileInventory {
                files: own.files,
                production_lines: own.production_lines,
                complete,
            };
            (name, inventory)
        })
        .collect()
}

fn not_evaluated(
    result: &ExecutionResult,
    plan: Option<&PolicyPlan>,
) -> Vec<(&'static str, NotEvaluated)> {
    let unknown = result
        .scan
        .finished()
        .into_iter()
        .flat_map(|scan| &scan.not_evaluated)
        .map(|id| (*id, NotEvaluated::UnknownToToolchain));
    let package_scoped = plan
        .filter(|_| result.exclusions.packages.is_some() && result.error.is_none())
        .into_iter()
        .flat_map(|plan| plan.active_rules(Producer::Repo))
        .map(|(definition, _)| (definition.id, NotEvaluated::PackageScoped));
    let missing_database = result
        .cargo_health
        .as_ref()
        .filter(|health| health.advisory == AdvisoryOutcome::Missing)
        .and(plan)
        .into_iter()
        .flat_map(|plan| plan.active_rules(Producer::CargoHealth))
        .filter(|(definition, _)| cargo_health::ADVISORY_RULES.contains(definition))
        .map(|(definition, _)| (definition.id, NotEvaluated::AdvisoryDbMissing));
    let ungated = result
        .source
        .iter()
        .flat_map(|source| source.not_evaluated.iter().copied());
    unknown
        .chain(package_scoped)
        .chain(missing_database)
        .chain(ungated)
        .collect()
}

fn diagnostics_from_execution(
    result: &ExecutionResult,
    plan: &PolicyPlan,
    scope: Option<&ScopeReport>,
    home: &HomePaths,
) -> (Vec<Diagnostic>, Exclusion) {
    if classify(result) == Status::Failed {
        return (Vec::new(), Exclusion::default());
    }
    let Some(workspace_root) = result
        .metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path())
    else {
        return (Vec::new(), Exclusion::default());
    };

    let mut merged = BTreeMap::<String, Diagnostic>::new();
    let mut dependency_keys = BTreeMap::<String, String>::new();
    for candidate in result.cargo_health.iter().flat_map(|scan| &scan.candidates) {
        let diagnostic = normalize_cargo_health_candidate(candidate, workspace_root, home);
        if let Some(key) = &candidate.key {
            dependency_keys.insert(diagnostic.id.clone(), key.clone());
        }
        merge_diagnostic(&mut merged, diagnostic);
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
    let exclusion =
        exclusion::apply(&mut diagnostics, plan, &result.exclusions, &dependency_keys);
    if let Some(scope) = scope {
        project_diagnostics(&mut diagnostics, scope, workspace_root);
    }
    diagnostics.sort_by(compare_diagnostics);
    (diagnostics, exclusion)
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
            rust_doctor: env!("CARGO_PKG_VERSION"),
            rustc: None,
            cargo: None,
            clippy: None,
            removed_lint_flags: Vec::new(),
        },
        scan: scan_report(None, &Exclusion::default(), None),
        diagnostics: Vec::new(),
        suppressions: Vec::new(),
        delta: None,
        errors: vec![error],
        summary: Summary::default(),
        gate: GateReport {
            blocking,
            status: GateStatus::NotEvaluated,
            blocking_diagnostics: None,
        },
        timings: None,
        base_side_cache: None,
    }
}

pub(super) fn project_diagnostics(
    diagnostics: &mut Vec<Diagnostic>,
    scope: &ScopeReport,
    workspace_root: &Path,
) {
    let meets = |path: Option<&str>, span: Option<&DiagnosticSpan>| {
        scope.includes_span(path, span.map(|span| (span.line_start, span.line_end)))
    };
    diagnostics.retain_mut(|diagnostic| {
        let reach = diagnostic
            .code
            .as_deref()
            .and_then(crate::policy::find)
            .map_or(DiffScope::Line, |definition| definition.diff_scope);
        if scope.includes_reach(
            diagnostic.path.as_deref(),
            diagnostic
                .span
                .as_ref()
                .map(|span| (span.line_start, span.line_end)),
            reach,
            |path| judged_source(workspace_root, path),
        ) {
            return true;
        }
        let Some(index) = diagnostic
            .related
            .iter()
            .position(|location| meets(Some(&location.path), Some(&location.span)))
        else {
            return false;
        };
        let promoted = diagnostic.related.remove(index);
        let previous = diagnostic
            .path
            .replace(promoted.path)
            .zip(diagnostic.span.replace(promoted.span));
        if let Some((path, span)) = previous {
            diagnostic.related.push(RelatedLocation { path, span });
            diagnostic.related.sort_by(|left, right| {
                (&left.path, left.span.line_start, left.span.column_start)
                    .cmp(&(&right.path, right.span.line_start, right.span.column_start))
            });
        }
        true
    });
}

fn judged_source(workspace_root: &Path, path: &str) -> Option<String> {
    const SOURCE_LIMIT: u64 = 8 * 1024 * 1024;
    let file = workspace_root.join(workspace_path::decode_normalized_relative(path)?);
    let metadata = std::fs::symlink_metadata(&file).ok()?;
    (metadata.is_file() && metadata.len() <= SOURCE_LIMIT)
        .then(|| std::fs::read_to_string(&file).ok())
        .flatten()
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

fn scan_report(
    scan: Option<&ScanExecution>,
    exclusion: &Exclusion,
    advisory: Option<&AdvisoryOutcome>,
) -> ScanReport {
    ScanReport {
        command: scan.map(|scan| scan.command.clone()),
        exit_code: scan.and_then(|scan| scan.exit_code),
        build_finished: scan.and_then(|scan| scan.build_finished),
        noise_lines: scan.map(|scan| scan.noise_lines),
        ignored: exclusion.ignored,
        excluded_generated: exclusion.excluded_generated,
        advisory_db: match advisory {
            Some(AdvisoryOutcome::Consulted(report)) => Some(report.clone()),
            _ => None,
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
                .chain(&scan.notices)
                .map(|error| normalize_error(error, workspace_root, home)),
        );
        if !scan.deadline_exceeded {
            push_exit_errors(&mut errors, scan, workspace_root, home);
        }
        if scan.malformed_messages > 0 {
            errors.push(ReportError {
                stage: "parsing".to_owned(),
                code: "malformed-message".to_owned(),
                message: "malformed Cargo message".to_owned(),
            });
        }
    }
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

fn push_exit_errors(
    errors: &mut Vec<ReportError>,
    scan: &ScanExecution,
    workspace_root: Option<&Path>,
    home: &HomePaths,
) {
    let cause = scan
        .cargo_cause
        .as_deref()
        .map(|cause| format!(". Cargo reported: {}", sanitize_text(cause, workspace_root, home)))
        .unwrap_or_default();
    match scan.exit_code {
        Some(code) if code != 0 || scan.exit_success != Some(true) => {
            errors.push(ReportError {
                stage: "execution".to_owned(),
                code: "clippy-exit".to_owned(),
                message: format!("Clippy exited with status {code}{cause}"),
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
            introduced.contains(&diagnostic.id)
                && DiagnosticContext::weighs(diagnostic)
                && is_blocking(diagnostic, blocking)
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

