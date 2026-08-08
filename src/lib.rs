#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::Path;

mod audit;
mod baseline;
mod cargo_health;
mod configuration;
mod delta;
mod execution;
mod git_scope;
mod policy;
pub mod presentation;
pub mod render;
mod report;
mod scan_target;
mod source_kernel;
mod structure;
mod terminal_text;
mod workspace_path;

pub use audit::{
    Audit, AuditCategory, AuditCategoryName, AuditScore, ScoreDimensions, ScoreLabel,
    SeverityCounts, ShareError,
};
pub use delta::{DeltaMatch, DeltaReport, DeltaSummary};
pub use git_scope::{ExecutionScope, ScopeMode, ScopeReport};
pub use policy::{
    BlockingLevel, BlockingLevelSource, CategoryOverride, RuleLevel, RuleLevelSource, RuleOverride,
    RuleTier,
};
pub use report::{
    Diagnostic, DiagnosticSource, DiagnosticSpan, GateReport, GateStatus, InspectReport,
    InspectRequest, PackageReport, PolicyBlockingReport, PolicyReport, PolicyRuleReport,
    ProjectReport, RelatedLocation, ReportError, SCHEMA_VERSION, ScanReport, Severity, Status,
    Summary, ToolchainReport,
};

pub fn inspect(request: InspectRequest) -> InspectReport {
    match InspectionSession::prepare(request) {
        Ok(session) => session.inspect(),
        Err(report) => *report,
    }
}

#[derive(Debug)]
pub struct InspectionSession {
    prepared: execution::PreparedInspection,
    plan: policy::PolicyPlan,
    scope_request: git_scope::ScopeRequest,
    preview_scope: Option<ScopeReport>,
}

impl InspectionSession {
    pub fn prepare(request: InspectRequest) -> Result<Self, Box<InspectReport>> {
        let policy = request.policy().clone();
        Self::prepare_with(request, &policy)
    }

    fn prepare_with(
        request: InspectRequest,
        policy: &policy::PolicyInput,
    ) -> Result<Self, Box<InspectReport>> {
        if let Err(error) = policy.validate() {
            return Err(Box::new(report::policy_failure(
                error,
                policy.failure_blocking(),
            )));
        }
        let scope_request = request.scope().clone();
        if let Err(error) = git_scope::validate(&scope_request) {
            return Err(Box::new(report::scope_failure(
                error,
                policy.failure_blocking(),
            )));
        }
        let prepared = match execution::prepare(&request.path) {
            Ok(prepared) => prepared,
            Err(result) => {
                return Err(Box::new(report::preparation_failure(
                    *result,
                    policy.failure_blocking(),
                )));
            }
        };
        let plan =
            match policy::PolicyPlan::compile_with_configuration(policy, &prepared.configuration) {
                Ok(plan) => plan,
                Err(error) => {
                    return Err(Box::new(report::policy_failure(
                        error,
                        policy.failure_blocking(),
                    )));
                }
            };
        Ok(Self {
            prepared,
            plan,
            scope_request,
            preview_scope: None,
        })
    }

    pub fn workspace_root(&self) -> &Path {
        self.prepared.workspace_root()
    }

    pub fn preview_uncommitted_rust_files(&mut self) -> Option<usize> {
        if self.preview_scope.is_none() {
            self.preview_scope = git_scope::resolve(
                &git_scope::ScopeRequest::Files {
                    base: "HEAD".to_owned(),
                },
                self.prepared.workspace_root(),
            )
            .ok();
        }
        Some(
            self.preview_scope
                .as_ref()?
                .files()?
                .iter()
                .filter(|path| path.ends_with(".rs"))
                .count(),
        )
    }

    pub fn select_uncommitted_files(&mut self) {
        self.scope_request = git_scope::ScopeRequest::Files {
            base: "HEAD".to_owned(),
        };
    }

    pub fn inspect(self) -> InspectReport {
        inspect_prepared(self)
    }
}

#[cfg(test)]
fn inspect_with(request: InspectRequest, policy: &policy::PolicyInput) -> InspectReport {
    match InspectionSession::prepare_with(request, policy) {
        Ok(session) => session.inspect(),
        Err(report) => *report,
    }
}

fn inspect_prepared(session: InspectionSession) -> InspectReport {
    let InspectionSession {
        prepared,
        plan,
        scope_request,
        preview_scope,
    } = session;
    let cached_preview_matches = matches!(
        &scope_request,
        git_scope::ScopeRequest::Files { base } if base == "HEAD"
    );
    let scope = match preview_scope.filter(|_| cached_preview_matches) {
        Some(scope) => scope,
        None => match git_scope::resolve(&scope_request, prepared.workspace_root()) {
            Ok(scope) => scope,
            Err(error) => {
                return report::preparation_failure(prepared.fail(error), plan.blocking());
            }
        },
    };
    if let git_scope::ScopeDetails::Baseline { comparison_base } = scope.details() {
        let snapshot = match baseline::materialize(prepared.workspace_root(), comparison_base) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return report::from_execution_scoped(prepared.fail(error), &plan, scope);
            }
        };
        let execution =
            execution::execute_baseline(prepared, snapshot.workspace(), snapshot.target(), &plan);
        let report = report::from_baseline_execution(execution, &plan, scope);
        return match snapshot.cleanup() {
            Ok(()) => report,
            Err(error) => report::baseline_cleanup_failure(report, error),
        };
    }
    report::from_execution_scoped(execution::execute(prepared, &plan), &plan, scope)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::policy::{PolicyInput, RuleLevel};

    #[test]
    fn invalid_policy_returns_before_execution_or_path_discovery() {
        let request = InspectRequest::new("/path/that/must/not/be/inspected");
        let policy = PolicyInput::default().with_rule("unknown::rule", RuleLevel::Warn);

        let report = inspect_with(request, &policy);

        assert_eq!(report.status, Status::Failed);
        assert!(!report.complete);
        assert!(report.project.is_none());
        assert!(report.scan.command.is_none());
        assert!(report.diagnostics.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].stage, "policy");
        assert_eq!(report.errors[0].code, "unknown-rule");
    }

    #[test]
    fn clippy_off_prunes_execution_and_error_preserves_warning_identity() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kernel-contract/todo");
        let baseline = inspect_with(InspectRequest::new(&fixture), &PolicyInput::default());
        assert_eq!(baseline.status, Status::Complete);
        let baseline_ids: Vec<_> = baseline
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_deref() == Some("clippy::todo"))
            .map(|diagnostic| diagnostic.id.clone())
            .collect();
        assert!(!baseline_ids.is_empty());

        let off_policy = PolicyInput::default().with_rule("clippy::todo", RuleLevel::Off);
        let off = inspect_with(InspectRequest::new(&fixture), &off_policy);
        assert_eq!(off.status, Status::Complete);
        assert!(
            off.diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code.as_deref() != Some("clippy::todo"))
        );
        let off_command = off
            .scan
            .command
            .expect("complete scan should expose its command");
        assert!(
            !off_command
                .iter()
                .any(|argument| argument == "clippy::todo")
        );

        let error_policy = PolicyInput::default().with_rule("clippy::todo", RuleLevel::Error);
        let error = inspect_with(InspectRequest::new(&fixture), &error_policy);
        assert_eq!(error.status, Status::Complete);
        let error_ids: Vec<_> = error
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_deref() == Some("clippy::todo"))
            .map(|diagnostic| {
                assert_eq!(diagnostic.base_severity, Severity::Warning);
                assert_eq!(diagnostic.severity, Severity::Error);
                diagnostic.id.clone()
            })
            .collect();
        assert_eq!(error_ids, baseline_ids);
        assert_eq!(error.gate.status, GateStatus::Failed);
        assert_eq!(error.gate.blocking_diagnostics, Some(error_ids.len()));
        let error_command = error
            .scan
            .command
            .expect("complete scan should expose its command");
        assert!(
            error_command
                .windows(2)
                .any(|pair| pair == ["-W", "clippy::todo"])
        );
        assert!(!error_command.iter().any(|argument| argument == "-D"));
    }
}
