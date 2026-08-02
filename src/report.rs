use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};

use cargo_metadata::Metadata;
use serde::Serialize;
use serde_json::Value;

use crate::audit::{self, Audit, SourceFileInventory};
use crate::cargo_health;
use crate::delta::DeltaReport;
use crate::execution::{
    BaselineExecution, CapturedDiagnostic, CapturedMessage, CapturedSpan, CompilerMessageData,
    ExecutionResult, InternalError, ScanExecution,
};
use crate::git_scope::{ScopeReport, ScopeRequest};
use crate::policy::{
    self, BlockingLevel, BlockingLevelSource, CategoryOverride, PolicyError, PolicyInput,
    PolicyPlan, RuleLevel, RuleLevelSource, RuleOverride,
};
use crate::source_kernel;
use crate::workspace_path;

pub const SCHEMA_VERSION: u8 = 8;

#[derive(Debug, Clone)]
pub struct InspectRequest {
    pub path: PathBuf,
    policy: PolicyInput,
    scope: ScopeRequest,
}

impl InspectRequest {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            policy: PolicyInput::default(),
            scope: ScopeRequest::Full,
        }
    }

    pub fn with_rule_override(mut self, rule_override: RuleOverride) -> Self {
        self.policy.push_rule(rule_override);
        self
    }

    pub fn with_category_override(mut self, category_override: CategoryOverride) -> Self {
        self.policy.push_category(category_override);
        self
    }

    pub fn with_blocking(mut self, blocking: BlockingLevel) -> Self {
        self.policy = self.policy.with_blocking(blocking);
        self
    }

    pub fn with_files_scope(mut self, base: impl Into<String>) -> Self {
        self.scope = ScopeRequest::Files { base: base.into() };
        self
    }

    pub fn with_baseline_scope(mut self, base: impl Into<String>) -> Self {
        self.scope = ScopeRequest::Baseline { base: base.into() };
        self
    }

    pub(crate) const fn policy(&self) -> &PolicyInput {
        &self.policy
    }

    pub(crate) const fn scope(&self) -> &ScopeRequest {
        &self.scope
    }
}

impl Default for InspectRequest {
    fn default() -> Self {
        Self::new(".")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectReport {
    pub schema_version: u8,
    pub audit: Audit,
    pub status: Status,
    pub complete: bool,
    pub policy: Option<PolicyReport>,
    pub scope: Option<ScopeReport>,
    pub project: Option<ProjectReport>,
    pub toolchain: ToolchainReport,
    pub scan: ScanReport,
    pub diagnostics: Vec<Diagnostic>,
    pub delta: Option<DeltaReport>,
    pub errors: Vec<ReportError>,
    pub summary: Summary,
    pub gate: GateReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyReport {
    pub config_file: Option<String>,
    pub blocking: PolicyBlockingReport,
    pub rules: Vec<PolicyRuleReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PolicyBlockingReport {
    pub level: BlockingLevel,
    pub source: BlockingLevelSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyRuleReport {
    pub id: String,
    pub category: String,
    pub level: RuleLevel,
    pub source: RuleLevelSource,
}

impl PolicyReport {
    fn from_plan(plan: &PolicyPlan) -> Self {
        Self {
            config_file: plan.config_file().map(str::to_owned),
            blocking: PolicyBlockingReport {
                level: plan.blocking(),
                source: plan.blocking_source(),
            },
            rules: plan
                .effective_rules()
                .map(|(definition, level, source)| PolicyRuleReport {
                    id: definition.id.to_owned(),
                    category: definition.category.to_owned(),
                    level,
                    source,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Complete,
    Incomplete,
    Failed,
}

impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectReport {
    pub workspace_root: String,
    pub manifest_path: String,
    pub packages: Vec<PackageReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageReport {
    pub name: String,
    pub manifest_path: Option<String>,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolchainReport {
    pub rustc: Option<String>,
    pub cargo: Option<String>,
    pub clippy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanReport {
    pub command: Option<Vec<String>>,
    pub exit_code: Option<i32>,
    pub build_finished: Option<bool>,
    pub noise_lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub id: String,
    pub source: DiagnosticSource,
    pub code: Option<String>,
    pub base_severity: Severity,
    pub severity: Severity,
    pub category: Option<String>,
    pub message: String,
    pub help: Option<String>,
    pub package: Option<String>,
    pub target: Option<String>,
    pub path: Option<String>,
    pub span: Option<DiagnosticSpan>,
    pub occurrences: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSource {
    Rustc,
    Clippy,
    #[serde(rename = "rust-doctor")]
    RustDoctor,
}

impl DiagnosticSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Rustc => "rustc",
            Self::Clippy => "clippy",
            Self::RustDoctor => "rust-doctor",
        }
    }
}

impl fmt::Display for DiagnosticSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
    Unknown,
}

impl Severity {
    pub(crate) const fn rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Info => 2,
            Self::Unknown => 3,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticSpan {
    pub line_start: usize,
    pub column_start: usize,
    pub line_end: usize,
    pub column_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportError {
    pub stage: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub unknown: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GateStatus {
    Passed,
    Failed,
    NotEvaluated,
}

impl GateStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotEvaluated => "not-evaluated",
        }
    }
}

impl fmt::Display for GateStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateReport {
    pub blocking: BlockingLevel,
    pub status: GateStatus,
    pub blocking_diagnostics: Option<usize>,
}

impl InspectReport {
    pub const fn exit_code(&self) -> u8 {
        match (self.status, self.gate.status) {
            (Status::Complete, GateStatus::Passed) => 0,
            (Status::Complete, GateStatus::Failed | GateStatus::NotEvaluated)
            | (Status::Incomplete, _) => 1,
            (Status::Failed, _) => 2,
        }
    }
}

#[derive(Debug, Default)]
struct HomePaths {
    lexical: Option<String>,
    canonical: Option<String>,
}

impl HomePaths {
    fn from_path(path: Option<PathBuf>) -> Self {
        let canonical = path
            .as_ref()
            .and_then(|path| path.canonicalize().ok())
            .map(|path| path.to_string_lossy().into_owned());
        Self {
            lexical: path.map(|path| path.to_string_lossy().into_owned()),
            canonical,
        }
    }
}

#[cfg(test)]
pub(crate) fn from_execution(result: ExecutionResult, plan: &PolicyPlan) -> InspectReport {
    from_execution_with_plan(
        result,
        Some(plan),
        plan.blocking(),
        Some(ScopeReport::full()),
    )
}

pub(crate) fn from_execution_scoped(
    result: ExecutionResult,
    plan: &PolicyPlan,
    scope: ScopeReport,
) -> InspectReport {
    from_execution_with_plan(result, Some(plan), plan.blocking(), Some(scope))
}

pub(crate) fn from_baseline_execution(
    execution: Result<BaselineExecution, Box<ExecutionResult>>,
    plan: &PolicyPlan,
    scope: ScopeReport,
) -> InspectReport {
    match execution {
        Ok(execution) => analyze_baseline_execution(execution, plan, scope).into_report(),
        Err(execution) => {
            let report =
                from_execution_with_plan(*execution, Some(plan), plan.blocking(), Some(scope));
            force_baseline_gate(report)
        }
    }
}

struct BaselineAnalysis {
    baseline: InspectReport,
    current: InspectReport,
    baseline_root: Option<PathBuf>,
    current_root: Option<PathBuf>,
}

impl BaselineAnalysis {
    fn into_report(self) -> InspectReport {
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
        current.delta = Some(delta);
        current
    }
}

fn analyze_baseline_execution(
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
        baseline: from_execution_with_plan(
            baseline,
            Some(plan),
            plan.blocking(),
            Some(scope.clone()),
        ),
        current: from_execution_with_plan(current, Some(plan), plan.blocking(), Some(scope)),
        baseline_root,
        current_root,
    }
}

pub(crate) fn baseline_cleanup_failure(
    report: InspectReport,
    error: InternalError,
) -> InspectReport {
    baseline_report_failure(report, error)
}

fn baseline_report_failure(mut report: InspectReport, error: InternalError) -> InspectReport {
    let source_files = report.audit.source_files;
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
    report.audit = Audit::build(source_files, report.status, &report.diagnostics);
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

fn from_execution_with_plan(
    result: ExecutionResult,
    plan: Option<&PolicyPlan>,
    blocking: BlockingLevel,
    scope: Option<ScopeReport>,
) -> InspectReport {
    let home = home_paths();
    let status = classify(&result);
    let workspace_root = result
        .metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path());
    let diagnostics = plan.map_or_else(Vec::new, |plan| {
        diagnostics_from_execution(&result, plan, scope.as_ref(), &home)
    });
    let summary = summarize(&diagnostics);
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
                    result.source.as_ref(),
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
        toolchain: ToolchainReport {
            rustc: result
                .toolchain
                .rustc
                .as_deref()
                .map(|value| sanitize_text(value, workspace_root, &home)),
            cargo: result
                .toolchain
                .cargo
                .as_deref()
                .map(|value| sanitize_text(value, workspace_root, &home)),
            clippy: result
                .toolchain
                .clippy
                .as_deref()
                .map(|value| sanitize_text(value, workspace_root, &home)),
        },
        scan,
        diagnostics,
        delta: None,
        errors,
        summary,
        gate,
    }
}

fn diagnostics_from_execution(
    result: &ExecutionResult,
    plan: &PolicyPlan,
    scope: Option<&ScopeReport>,
    home: &HomePaths,
) -> Vec<Diagnostic> {
    let status = classify(result);
    let workspace_root = result
        .metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path());
    let mut diagnostics = match status {
        Status::Failed => Vec::new(),
        Status::Complete | Status::Incomplete => normalize_diagnostics_with_plan(
            result
                .scan
                .finished()
                .map(|scan| scan.messages.as_slice())
                .unwrap_or_default(),
            workspace_root,
            result.metadata.as_ref(),
            home,
            plan,
        ),
    };
    if status != Status::Failed
        && let Some(source) = result.source.as_ref()
    {
        merge_source_candidates(&mut diagnostics, source, workspace_root, home);
    }
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
    from_execution_with_plan(result, None, blocking, None)
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
        audit: Audit::build(0, Status::Failed, &[]),
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

fn project_diagnostics(diagnostics: &mut Vec<Diagnostic>, scope: &ScopeReport) {
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
    if let Some(source) = result.source.as_ref() {
        errors.extend(source.errors.iter().map(|error| ReportError {
            stage: "source".to_owned(),
            code: error.code.to_owned(),
            message: sanitize_text(&error.message, workspace_root, home),
        }));
    }
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

fn normalize_diagnostics_with_plan(
    messages: &[CapturedMessage],
    workspace_root: Option<&Path>,
    metadata: Option<&Metadata>,
    home: &HomePaths,
    plan: &PolicyPlan,
) -> Vec<Diagnostic> {
    let Some(workspace_root) = workspace_root else {
        return Vec::new();
    };
    let mut diagnostics = BTreeMap::<String, Diagnostic>::new();

    if let Some(metadata) = metadata {
        for candidate in cargo_health::inspect(metadata, plan).candidates {
            let diagnostic = normalize_cargo_health_candidate(candidate, workspace_root, home);
            merge_diagnostic(&mut diagnostics, diagnostic);
        }
    }

    for message in messages {
        let message = match message {
            CapturedMessage::Compiler(message) => message,
            CapturedMessage::Known(message) => {
                let _ = message;
                continue;
            }
            CapturedMessage::Unknown(value) => {
                let _ = value;
                continue;
            }
        };
        if message
            .message
            .code
            .as_ref()
            .is_some_and(|code| policy::find(&code.code).is_some() && !plan.is_active(&code.code))
        {
            continue;
        }
        let diagnostic = normalize_diagnostic(message, workspace_root, metadata, home);
        merge_diagnostic(&mut diagnostics, diagnostic);
    }

    let mut diagnostics: Vec<_> = diagnostics.into_values().collect();
    diagnostics.sort_by(compare_diagnostics);
    diagnostics
}

#[cfg(test)]
fn normalize_diagnostics(
    messages: &[CapturedMessage],
    workspace_root: Option<&Path>,
    metadata: Option<&Metadata>,
    home: &HomePaths,
) -> Vec<Diagnostic> {
    normalize_diagnostics_with_plan(
        messages,
        workspace_root,
        metadata,
        home,
        &PolicyPlan::default(),
    )
}

fn normalize_cargo_health_candidate(
    candidate: cargo_health::Candidate,
    workspace_root: &Path,
    home: &HomePaths,
) -> Diagnostic {
    let definition = candidate.definition;
    let source = DiagnosticSource::RustDoctor;
    let code = Some(definition.id.to_owned());
    let message = sanitize_text(&candidate.message, Some(workspace_root), home);
    let path = candidate
        .manifest_path
        .as_deref()
        .and_then(|path| workspace_path::normalize_relative(Path::new(path)));
    let id = fingerprint(
        source,
        code.as_deref(),
        path.as_deref(),
        None,
        canonical_severity(definition.default_level),
        &message,
    );

    Diagnostic {
        id,
        source,
        code,
        base_severity: canonical_severity(definition.default_level),
        severity: canonical_severity(definition.default_level),
        category: Some(definition.category.to_owned()),
        message,
        help: Some(definition.help.to_owned()),
        package: Some(normalize_text(&candidate.package)),
        target: None,
        path,
        span: None,
        occurrences: 1,
    }
}

fn merge_source_candidates(
    diagnostics: &mut Vec<Diagnostic>,
    source_scan: &source_kernel::SourceScan,
    workspace_root: Option<&Path>,
    home: &HomePaths,
) {
    let mut merged: BTreeMap<_, _> = diagnostics
        .drain(..)
        .map(|diagnostic| (diagnostic.id.clone(), diagnostic))
        .collect();
    for candidate in &source_scan.candidates {
        let diagnostic = normalize_source_candidate(candidate, workspace_root, home);
        merge_diagnostic(&mut merged, diagnostic);
    }
    diagnostics.extend(merged.into_values());
    diagnostics.sort_by(compare_diagnostics);
}

fn normalize_source_candidate(
    candidate: &source_kernel::Candidate,
    workspace_root: Option<&Path>,
    home: &HomePaths,
) -> Diagnostic {
    let definition = candidate.definition;
    let source = DiagnosticSource::RustDoctor;
    let code = Some(definition.id.to_owned());
    let path = workspace_path::normalize_relative(Path::new(&candidate.path));
    let span = Some(DiagnosticSpan {
        line_start: candidate.span.line_start,
        column_start: candidate.span.column_start,
        line_end: candidate.span.line_end,
        column_end: candidate.span.column_end,
    });
    let message = sanitize_text(candidate.message, workspace_root, home);
    let id = fingerprint(
        source,
        code.as_deref(),
        path.as_deref(),
        span.as_ref(),
        canonical_severity(definition.default_level),
        &message,
    );
    Diagnostic {
        id,
        source,
        code,
        base_severity: canonical_severity(definition.default_level),
        severity: canonical_severity(definition.default_level),
        category: Some(definition.category.to_owned()),
        message,
        help: Some(definition.help.to_owned()),
        package: candidate.package.as_deref().map(normalize_text),
        target: candidate.target.as_deref().map(normalize_text),
        path,
        span,
        occurrences: 1,
    }
}

fn merge_diagnostic(diagnostics: &mut BTreeMap<String, Diagnostic>, diagnostic: Diagnostic) {
    match diagnostics.get_mut(&diagnostic.id) {
        Some(existing) => {
            existing.occurrences += 1;
            merge_optional_context(&mut existing.package, diagnostic.package);
            merge_optional_context(&mut existing.target, diagnostic.target);
        }
        None => {
            diagnostics.insert(diagnostic.id.clone(), diagnostic);
        }
    }
}

fn normalize_diagnostic(
    captured: &CompilerMessageData,
    workspace_root: &Path,
    metadata: Option<&Metadata>,
    home: &HomePaths,
) -> Diagnostic {
    let rule = captured
        .message
        .code
        .as_ref()
        .and_then(|code| policy::find(&code.code));
    let code = captured
        .message
        .code
        .as_ref()
        .map(|code| normalize_text(&code.code));
    let source = match code.as_deref() {
        Some(code) if code.starts_with("clippy::") => DiagnosticSource::Clippy,
        _ => DiagnosticSource::Rustc,
    };
    let severity = severity(&captured.message.level);
    let message = sanitize_text(&captured.message.message, Some(workspace_root), home);
    let (path, span) = select_primary_span(&captured.message, workspace_root);
    let package = metadata.and_then(|metadata| {
        metadata
            .packages
            .iter()
            .find(|package| package.id.repr == captured.package_id)
            .map(|package| package.name.to_string())
    });
    let target = Some(normalize_text(&captured.target.name));
    let id = fingerprint(
        source,
        code.as_deref(),
        path.as_deref(),
        span.as_ref(),
        severity,
        &message,
    );

    Diagnostic {
        id,
        source,
        code,
        base_severity: severity,
        severity,
        category: rule.map(|rule| rule.category.to_owned()),
        message,
        help: rule.map(|rule| rule.help.to_owned()),
        package,
        target,
        path,
        span,
        occurrences: 1,
    }
}

fn merge_optional_context(existing: &mut Option<String>, incoming: Option<String>) {
    if existing.as_ref() != incoming.as_ref() {
        *existing = None;
    }
}

fn severity(level: &str) -> Severity {
    match level {
        "error" | "failure-note" | "error: internal compiler error" => Severity::Error,
        "warning" => Severity::Warning,
        "note" | "help" => Severity::Info,
        _ => Severity::Unknown,
    }
}

fn canonical_severity(level: RuleLevel) -> Severity {
    match level {
        RuleLevel::Warn => Severity::Warning,
        RuleLevel::Error => Severity::Error,
        RuleLevel::Off => Severity::Unknown,
    }
}

fn apply_policy(diagnostics: &mut [Diagnostic], plan: &PolicyPlan) {
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

fn evaluate_gate(
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
        .filter(|diagnostic| is_blocking(diagnostic, blocking))
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

fn select_primary_span(
    diagnostic: &CapturedDiagnostic,
    workspace_root: &Path,
) -> (Option<String>, Option<DiagnosticSpan>) {
    let mut spans: Vec<_> = diagnostic
        .spans
        .iter()
        .filter(|span| span.is_primary)
        .map(|span| normalized_span(span, workspace_root))
        .collect();
    spans.sort_by(compare_spans);
    spans.into_iter().next().unwrap_or((None, None))
}

fn normalized_span(
    span: &CapturedSpan,
    workspace_root: &Path,
) -> (Option<String>, Option<DiagnosticSpan>) {
    (
        workspace_path::normalize(workspace_root, Path::new(&span.file_name)),
        Some(DiagnosticSpan {
            line_start: span.line_start,
            column_start: span.column_start,
            line_end: span.line_end,
            column_end: span.column_end,
        }),
    )
}

fn compare_spans(
    left: &(Option<String>, Option<DiagnosticSpan>),
    right: &(Option<String>, Option<DiagnosticSpan>),
) -> Ordering {
    compare_optional_paths(&left.0, &right.0)
        .then_with(|| span_coordinates(left.1.as_ref()).cmp(&span_coordinates(right.1.as_ref())))
}

fn compare_diagnostics(left: &Diagnostic, right: &Diagnostic) -> Ordering {
    compare_optional_paths(&left.path, &right.path)
        .then_with(|| span_start(left.span.as_ref()).cmp(&span_start(right.span.as_ref())))
        .then_with(|| left.severity.rank().cmp(&right.severity.rank()))
        .then_with(|| left.code.cmp(&right.code))
        .then_with(|| left.id.cmp(&right.id))
}

fn compare_optional_paths(left: &Option<String>, right: &Option<String>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn span_start(span: Option<&DiagnosticSpan>) -> (usize, usize) {
    span.map_or((usize::MAX, usize::MAX), |span| {
        (span.line_start, span.column_start)
    })
}

fn span_coordinates(span: Option<&DiagnosticSpan>) -> (usize, usize, usize, usize) {
    span.map_or((usize::MAX, usize::MAX, usize::MAX, usize::MAX), |span| {
        (
            span.line_start,
            span.column_start,
            span.line_end,
            span.column_end,
        )
    })
}

fn fingerprint(
    source: DiagnosticSource,
    code: Option<&str>,
    path: Option<&str>,
    span: Option<&DiagnosticSpan>,
    base_severity: Severity,
    message: &str,
) -> String {
    let span = span.map_or_else(
        || "null".to_owned(),
        |span| {
            format!(
                concat!(
                    "{{\"line_start\":{},\"column_start\":{},",
                    "\"line_end\":{},\"column_end\":{}}}"
                ),
                span.line_start, span.column_start, span.line_end, span.column_end
            )
        },
    );
    let tuple = format!(
        "[{},{},{},{},{},{}]",
        json_string(source.as_str()),
        json_optional_string(code),
        json_optional_string(path),
        span,
        json_string(base_severity.as_str()),
        json_string(message),
    );
    blake3::hash(tuple.as_bytes()).to_hex().to_string()
}

fn json_optional_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

fn json_string(value: &str) -> String {
    Value::String(value.to_owned()).to_string()
}

fn summarize(diagnostics: &[Diagnostic]) -> Summary {
    let mut summary = Summary::default();
    for diagnostic in diagnostics {
        match diagnostic.severity {
            Severity::Error => summary.errors += 1,
            Severity::Warning => summary.warnings += 1,
            Severity::Info => summary.info += 1,
            Severity::Unknown => summary.unknown += 1,
        }
    }
    summary.total = diagnostics.len();
    summary
}

fn sanitize_text(value: &str, workspace_root: Option<&Path>, home: &HomePaths) -> String {
    let mut value = normalize_text(value);
    if let Some(workspace_root) = workspace_root.and_then(Path::to_str)
        && !workspace_root.is_empty()
    {
        value = value.replace(workspace_root, ".");
    }
    let mut home_forms: Vec<_> = [home.lexical.as_deref(), home.canonical.as_deref()]
        .into_iter()
        .flatten()
        .filter(|path| !path.is_empty())
        .collect();
    home_forms.sort_by_key(|path| std::cmp::Reverse(path.len()));
    home_forms.dedup();
    for home in home_forms {
        value = value.replace(home, "<home>");
    }
    value
}

fn normalize_text(value: &str) -> String {
    let line_endings = value.replace("\r\n", "\n").replace('\r', "\n");
    let without_ansi = strip_ansi(&line_endings);
    without_ansi
        .split('\n')
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_ansi(value: &str) -> String {
    let mut characters = value.chars().peekable();
    let mut output = String::with_capacity(value.len());
    while let Some(character) = characters.next() {
        match character {
            '\u{001b}' => consume_escape(&mut characters),
            '\u{009b}' => consume_csi(&mut characters),
            '\u{009d}' => consume_control_string(&mut characters, true),
            '\u{0090}' | '\u{0098}' | '\u{009e}' | '\u{009f}' => {
                consume_control_string(&mut characters, false);
            }
            '\n' | '\t' => output.push(character),
            character if character.is_control() => {}
            _ => output.push(character),
        }
    }
    output
}

fn consume_escape(characters: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    let Some(introducer) = characters.next() else {
        return;
    };
    match introducer {
        '[' => consume_csi(characters),
        ']' => consume_control_string(characters, true),
        'P' | 'X' | '^' | '_' => consume_control_string(characters, false),
        '\u{20}'..='\u{2f}' => {
            while characters
                .next_if(|character| ('\u{20}'..='\u{2f}').contains(character))
                .is_some()
            {}
            let _ = characters.next_if(|character| ('\u{30}'..='\u{7e}').contains(character));
        }
        _ => {}
    }
}

fn consume_csi(characters: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for character in characters.by_ref() {
        if matches!(character, '\u{0018}' | '\u{001a}')
            || ('\u{40}'..='\u{7e}').contains(&character)
        {
            break;
        }
    }
}

fn consume_control_string(
    characters: &mut std::iter::Peekable<std::str::Chars<'_>>,
    bell_terminates: bool,
) {
    while let Some(character) = characters.next() {
        if matches!(character, '\u{0018}' | '\u{001a}') {
            break;
        }
        if character == '\u{009c}' || (bell_terminates && character == '\u{0007}') {
            break;
        }
        if character == '\u{001b}' && characters.next_if(|character| *character == '\\').is_some() {
            break;
        }
    }
}

fn home_paths() -> HomePaths {
    HomePaths::from_path(env::var_os("HOME").map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    #[cfg(unix)]
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    use super::*;
    use crate::execution::{CapturedDiagnosticCode, CapturedTarget, ToolchainProvenance};

    fn from_execution(result: ExecutionResult) -> InspectReport {
        super::from_execution(result, &PolicyPlan::default())
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
            audit: Audit::build(1, Status::Complete, &diagnostics),
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
            summary: summarize(&diagnostics),
            diagnostics,
            delta: None,
            errors: Vec::new(),
            gate,
        }
    }

    #[test]
    fn baseline_cleanup_failure_overrides_an_otherwise_complete_report() {
        let report = report_with_diagnostics(Vec::new());
        let failed = baseline_cleanup_failure(report, crate::baseline::cleanup_failed());

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
        assert_eq!(summarize(&baseline).warnings, 1);
        assert_eq!(summarize(&effective).errors, 1);
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
        );

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
        assert_eq!(summarize(&diagnostics).errors, 2);
        assert_eq!(
            evaluate_gate(Status::Complete, &diagnostics, BlockingLevel::Error),
            GateReport {
                blocking: BlockingLevel::Error,
                status: GateStatus::Failed,
                blocking_diagnostics: Some(2),
            }
        );

        let empty_scope = ScopeReport::files_scope("0".repeat(40), Vec::new());
        project_diagnostics(&mut diagnostics, &empty_scope);
        assert_eq!(summarize(&diagnostics), Summary::default());
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
                "--all-targets".to_owned(),
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
            metadata: Some(metadata),
            toolchain: ToolchainProvenance::default(),
            scan: Some(scan(
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
            ))
            .into(),
            source: Some(crate::source_kernel::SourceScan {
                candidates: vec![crate::source_kernel::Candidate {
                    definition: crate::policy::SOURCE_DYNAMIC_SHELL,
                    message,
                    package: Some("example".to_owned()),
                    target: Some("example".to_owned()),
                    path: path.to_owned(),
                    span: crate::source_kernel::SourceSpan {
                        line_start: 3,
                        column_start: 1,
                        line_end: 3,
                        column_end: 8,
                    },
                }],
                errors: Vec::new(),
                counters: crate::source_kernel::SourceCounters::default(),
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
        assert_eq!(report.schema_version, 8);
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
            metadata: Some(metadata.clone()),
            toolchain: ToolchainProvenance::default(),
            scan: Some(scan(Vec::new(), 0, true, true)).into(),
            source: None,
            error: None,
        });
        assert_eq!(complete.status, Status::Complete);
        assert!(complete.complete);
        assert_eq!(complete.exit_code(), 0);
        assert_eq!(complete.scan.command, Some(expected_command.clone()));
        assert_eq!(complete.diagnostics.len(), 2);

        let incomplete = from_execution(ExecutionResult {
            manifest_path: Some(manifest_path),
            metadata: Some(metadata),
            toolchain: ToolchainProvenance::default(),
            scan: Some(scan(
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
            ))
            .into(),
            source: None,
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
            toolchain: ToolchainProvenance::default(),
            scan: None.into(),
            source: None,
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
                definition: crate::policy::SOURCE_DYNAMIC_SHELL,
                message: "A dynamic value is interpolated into a shell command string.",
                package: Some("example".to_owned()),
                target: None,
                path: "src/source.rs".to_owned(),
                span: source_kernel::SourceSpan {
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
            counters: source_kernel::SourceCounters::default(),
        };
        let report = from_execution(ExecutionResult {
            manifest_path: Some(workspace.join("Cargo.toml")),
            metadata: Some(metadata),
            toolchain: ToolchainProvenance::default(),
            scan: Some(scan(compiler_messages, 0, true, true)).into(),
            source: Some(source),
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

    #[test]
    fn normalizes_text_paths_severity_and_deduplicates() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let source = workspace.join("src/lib.rs");
        let raw_message = format!(
            "\u{1b}[31mmessage {} /home/person  \r\nnext\t\r",
            workspace.display()
        );
        let home = HomePaths {
            lexical: Some("/home/person".to_owned()),
            canonical: None,
        };
        let messages = vec![
            compiler_message(
                Some("clippy::needless_return"),
                "warning",
                &raw_message,
                source.to_str().unwrap(),
                3,
            ),
            compiler_message(
                Some("clippy::needless_return"),
                "warning",
                &raw_message,
                source.to_str().unwrap(),
                3,
            ),
        ];
        let diagnostics = normalize_diagnostics(&messages, Some(&workspace), None, &home);

        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.source, DiagnosticSource::Clippy);
        assert_eq!(diagnostic.severity, Severity::Warning);
        assert_eq!(diagnostic.category, None);
        assert_eq!(diagnostic.message, "message . <home>\nnext\n");
        assert_eq!(diagnostic.help, None);
        assert_eq!(diagnostic.path.as_deref(), Some("src/lib.rs"));
        assert_eq!(diagnostic.occurrences, 2);
        assert_eq!(diagnostic.id.len(), 64);
        let tuple = serde_json::to_vec(&(
            diagnostic.source.as_str(),
            diagnostic.code.as_deref(),
            diagnostic.path.as_deref(),
            diagnostic.span.as_ref(),
            diagnostic.severity.as_str(),
            diagnostic.message.as_str(),
        ))
        .unwrap();
        assert_eq!(diagnostic.id, blake3::hash(&tuple).to_hex().to_string());
    }

    #[test]
    fn external_paths_are_null_and_future_severity_is_unknown() {
        let messages = vec![compiler_message(
            None,
            "future-level",
            "future",
            "/outside/project/lib.rs",
            1,
        )];
        let workspace = fixture("clean").canonicalize().unwrap();
        let diagnostics =
            normalize_diagnostics(&messages, Some(&workspace), None, &HomePaths::default());

        assert_eq!(diagnostics[0].path, None);
        assert_eq!(diagnostics[0].severity, Severity::Unknown);
        assert_eq!(diagnostics[0].category, None);
        assert_eq!(diagnostics[0].help, None);
        assert!(diagnostics[0].span.is_some());
    }

    #[test]
    fn exact_curated_codes_gain_metadata_without_restamping_severity() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let messages = [
            compiler_message(
                Some("clippy::todo"),
                "error",
                "toolchain-owned message",
                "src/lib.rs",
                2,
            ),
            compiler_message(
                Some("clippy::todo_suffix"),
                "warning",
                "similar code",
                "src/lib.rs",
                3,
            ),
        ];
        let diagnostics =
            normalize_diagnostics(&messages, Some(&workspace), None, &HomePaths::default());

        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(diagnostics[0].category.as_deref(), Some("correctness"));
        assert_eq!(
            diagnostics[0].help.as_deref(),
            Some(
                "Replace todo! with the intended implementation or remove the reachable placeholder."
            )
        );
        assert_eq!(diagnostics[1].category, None);
        assert_eq!(diagnostics[1].help, None);
    }

    #[test]
    fn code_normalization_cannot_turn_a_non_exact_code_into_a_curated_match() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let diagnostics = normalize_diagnostics(
            &[compiler_message(
                Some("\u{1b}[31mclippy::todo\u{1b}[0m"),
                "warning",
                "similar code",
                "src/lib.rs",
                2,
            )],
            Some(&workspace),
            None,
            &HomePaths::default(),
        );

        assert_eq!(diagnostics[0].code.as_deref(), Some("clippy::todo"));
        assert_eq!(diagnostics[0].category, None);
        assert_eq!(diagnostics[0].help, None);
    }

    #[test]
    fn missing_code_or_primary_span_does_not_invent_structured_fields() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let mut without_span =
            compiler_message(Some("clippy::todo"), "warning", "todo", "src/lib.rs", 2);
        if let CapturedMessage::Compiler(message) = &mut without_span {
            message.message.spans.clear();
        }
        let diagnostics = normalize_diagnostics(
            &[
                compiler_message(None, "warning", "todo", "src/lib.rs", 1),
                without_span,
            ],
            Some(&workspace),
            None,
            &HomePaths::default(),
        );

        assert_eq!(diagnostics[0].code, None);
        assert_eq!(diagnostics[0].category, None);
        assert_eq!(diagnostics[0].help, None);
        assert!(diagnostics[1].path.is_none());
        assert!(diagnostics[1].span.is_none());
        assert_eq!(diagnostics[1].category.as_deref(), Some("correctness"));
    }

    #[test]
    fn editorial_metadata_is_not_part_of_the_v1_fingerprint_tuple() {
        let identity = (
            DiagnosticSource::Clippy,
            Some("clippy::todo"),
            Some("src/lib.rs"),
            Some(DiagnosticSpan {
                line_start: 2,
                column_start: 3,
                line_end: 2,
                column_end: 10,
            }),
            Severity::Warning,
            "toolchain-owned message",
        );
        let first = fingerprint(
            identity.0,
            identity.1,
            identity.2,
            identity.3.as_ref(),
            identity.4,
            identity.5,
        );
        let second = fingerprint(
            identity.0,
            identity.1,
            identity.2,
            identity.3.as_ref(),
            identity.4,
            identity.5,
        );
        let reports = [
            ("correctness", "first editorial help", first),
            ("maintainability", "second editorial help", second),
        ];

        assert_ne!(reports[0].0, reports[1].0);
        assert_ne!(reports[0].1, reports[1].1);
        assert_eq!(reports[0].2, reports[1].2);
    }

    #[test]
    fn multiple_primary_spans_use_the_documented_canonical_order() {
        let mut message = match compiler_message(None, "error", "boom", "src/z.rs", 8) {
            CapturedMessage::Compiler(message) => message,
            _ => unreachable!(),
        };
        message.message.spans.push(CapturedSpan {
            file_name: "src/a.rs".to_owned(),
            line_start: 2,
            line_end: 2,
            column_start: 1,
            column_end: 3,
            is_primary: true,
        });
        let workspace = fixture("clean").canonicalize().unwrap();
        let diagnostics = normalize_diagnostics(
            &[CapturedMessage::Compiler(message)],
            Some(&workspace),
            None,
            &HomePaths::default(),
        );

        assert_eq!(diagnostics[0].path.as_deref(), Some("src/a.rs"));
        assert_eq!(
            diagnostics[0].span.as_ref().map(|span| span.line_start),
            Some(2)
        );
    }

    #[test]
    fn duplicate_context_conflicts_are_arrival_order_independent() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let home = HomePaths::default();
        let first = normalize_diagnostics(
            &[
                compiler_message_for_target("target-a"),
                compiler_message_for_target("target-b"),
            ],
            Some(&workspace),
            None,
            &home,
        );
        let reversed = normalize_diagnostics(
            &[
                compiler_message_for_target("target-b"),
                compiler_message_for_target("target-a"),
            ],
            Some(&workspace),
            None,
            &home,
        );

        assert_eq!(first, reversed);
        assert_eq!(first[0].target, None);
        assert_eq!(first[0].occurrences, 2);
    }

    #[test]
    fn malformed_messages_make_a_started_scan_incomplete() {
        let result = ExecutionResult {
            manifest_path: None,
            metadata: None,
            toolchain: ToolchainProvenance::default(),
            scan: Some(ScanExecution {
                command: vec!["cargo".to_owned(), "clippy".to_owned()],
                exit_code: Some(0),
                exit_success: Some(true),
                build_finished: Some(true),
                noise_lines: 0,
                malformed_messages: 1,
                messages: Vec::new(),
                errors: Vec::new(),
            })
            .into(),
            source: None,
            error: None,
        };
        let report = from_execution(result);

        assert_eq!(report.status, Status::Incomplete);
        assert_eq!(
            report
                .errors
                .iter()
                .map(|error| (
                    error.stage.as_str(),
                    error.code.as_str(),
                    error.message.as_str()
                ))
                .collect::<Vec<_>>(),
            [("parsing", "malformed-message", "malformed Cargo message")]
        );
    }

    #[test]
    fn incomplete_scan_reports_each_distinct_normative_cause_once() {
        let duplicate = InternalError {
            stage: "execution",
            code: "build-failed",
            message: "Cargo reported build-finished.success: false".to_owned(),
        };
        let result = ExecutionResult {
            manifest_path: None,
            metadata: None,
            toolchain: ToolchainProvenance::default(),
            scan: Some(ScanExecution {
                command: vec!["cargo".to_owned(), "clippy".to_owned()],
                exit_code: Some(101),
                exit_success: Some(false),
                build_finished: Some(false),
                noise_lines: 0,
                malformed_messages: 2,
                messages: Vec::new(),
                errors: vec![duplicate],
            })
            .into(),
            source: None,
            error: None,
        };
        let report = from_execution(result);
        let errors: Vec<_> = report
            .errors
            .iter()
            .map(|error| {
                (
                    error.stage.as_str(),
                    error.code.as_str(),
                    error.message.as_str(),
                )
            })
            .collect();

        assert_eq!(report.status, Status::Incomplete);
        assert_eq!(
            errors,
            [
                (
                    "execution",
                    "build-failed",
                    "Cargo reported build-finished.success: false"
                ),
                ("execution", "clippy-exit", "Clippy exited with status 101"),
                ("parsing", "malformed-message", "malformed Cargo message"),
            ]
        );
    }

    #[test]
    fn missing_exit_and_build_finished_have_explicit_causes() {
        let result = ExecutionResult {
            manifest_path: None,
            metadata: None,
            toolchain: ToolchainProvenance::default(),
            scan: Some(ScanExecution {
                command: vec!["cargo".to_owned(), "clippy".to_owned()],
                exit_code: None,
                exit_success: None,
                build_finished: None,
                noise_lines: 0,
                malformed_messages: 0,
                messages: Vec::new(),
                errors: Vec::new(),
            })
            .into(),
            source: None,
            error: None,
        };
        let report = from_execution(result);

        assert_eq!(report.status, Status::Incomplete);
        assert_eq!(report.errors.len(), 2);
        assert!(report.errors.iter().any(|error| {
            error.code == "clippy-exit" && error.message == "Clippy terminated without an exit code"
        }));
        assert!(report.errors.iter().any(|error| {
            error.code == "build-finished-missing"
                && error.message == "Cargo did not emit build-finished"
        }));
    }

    #[test]
    fn twenty_message_permutations_render_identically() {
        let base = [
            compiler_message(Some("E0001"), "error", "alpha", "src/e.rs", 7),
            compiler_message(Some("clippy::lint"), "warning", "beta", "src/b.rs", 3),
            compiler_message(None, "help", "gamma", "src/c.rs", 4),
            compiler_message(None, "future", "delta", "/external/d.rs", 1),
            compiler_message(None, "note", "epsilon", "src/a.rs", 2),
        ];
        let workspace = fixture("clean").canonicalize().unwrap();
        let mut expected = None;
        let mut order = [0, 1, 2, 3, 4];
        let mut seen = BTreeSet::new();

        for permutation in 0..20 {
            assert!(seen.insert(order));
            let messages: Vec<_> = order
                .iter()
                .map(|&index| clone_compiler_message(&base[index]))
                .collect();
            let diagnostics =
                normalize_diagnostics(&messages, Some(&workspace), None, &HomePaths::default());
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

    #[test]
    fn sanitizes_workspace_and_both_home_forms_from_errors() {
        let home = HomePaths {
            lexical: Some("/linked/home".to_owned()),
            canonical: Some("/real/home".to_owned()),
        };
        let sanitized = sanitize_text(
            "\u{1b}[31m/work/project failed in /linked/home and /real/home \r\n",
            Some(Path::new("/work/project")),
            &home,
        );

        assert_eq!(sanitized, ". failed in <home> and <home>\n");
    }

    #[test]
    fn lexical_home_is_redacted_when_canonicalization_fails() {
        let home =
            HomePaths::from_path(Some(PathBuf::from("/definitely/missing/rust-doctor-home")));

        assert!(home.canonical.is_none());
        assert_eq!(
            sanitize_text(
                "failed in /definitely/missing/rust-doctor-home/project",
                None,
                &home
            ),
            "failed in <home>/project"
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_lexical_home_display_is_redacted() {
        let path = PathBuf::from(OsString::from_vec(
            b"/definitely/missing/rust-doctor-\xff-home".to_vec(),
        ));
        let displayed = path.to_string_lossy().into_owned();
        let home = HomePaths::from_path(Some(path));

        assert_eq!(home.lexical.as_deref(), Some(displayed.as_str()));
        assert_eq!(
            sanitize_text(&format!("failed in {displayed}/project"), None, &home),
            "failed in <home>/project"
        );
    }

    #[test]
    fn strips_ecma_48_control_sequences_and_payloads() {
        let value = concat!(
            "a\u{001b}[31mb\u{001b}[0mc",
            "\u{001b}]osc\u{0007}d",
            "\u{001b}Pdcs\u{001b}\\e",
            "\u{001b}Xsos\u{001b}\\f",
            "\u{001b}^pm\u{001b}\\g",
            "\u{001b}_apc\u{001b}\\h",
            "\u{001b}(Bi",
            "\u{009b}31mj",
            "\u{009d}osc\u{009c}k",
        );

        assert_eq!(normalize_text(value), "abcdefghijk");
    }

    #[test]
    fn ecma_48_can_and_sub_cancel_sequences_without_consuming_following_text() {
        let value = concat!(
            "a\u{001b}[31\u{0018}b",
            "\u{001b}Pdiscarded\u{001a}c",
            "\u{009b}32\u{0018}d",
            "\u{009d}discarded\u{001a}e",
        );

        assert_eq!(normalize_text(value), "abcde");
    }

    #[test]
    fn control_characters_in_internal_paths_are_encoded_before_rendering() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let diagnostics = normalize_diagnostics(
            &[compiler_message(
                Some("clippy::lint"),
                "warning",
                "message",
                "src/100%\u{001b}[31mline\n.rs",
                1,
            )],
            Some(&workspace),
            None,
            &HomePaths::default(),
        );

        assert_eq!(
            diagnostics[0].path.as_deref(),
            Some("src/100%25%1B[31mline%0A.rs")
        );
        let report = report_with_diagnostics(diagnostics);
        let mut terminal = Vec::new();
        crate::render::render_terminal(&report, &mut terminal).unwrap();
        let terminal = String::from_utf8(terminal).unwrap();
        assert!(!terminal.contains('\u{001b}'));
        assert!(terminal.contains("src/100%25%1B[31mline%0A.rs:1:2"));

        let mut json = Vec::new();
        crate::render::render_json(&report, &mut json).unwrap();
        let json: Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            json["diagnostics"][0]["path"],
            "src/100%25%1B[31mline%0A.rs"
        );
    }
}
