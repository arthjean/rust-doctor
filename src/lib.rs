#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod cargo_health;
mod execution;
mod policy;
pub mod render;
mod report;
mod source_kernel;

pub use policy::{BlockingLevel, CategoryOverride, RuleLevel, RuleOverride};
pub use report::{
    Diagnostic, DiagnosticSource, DiagnosticSpan, GateReport, GateStatus, InspectReport,
    InspectRequest, PackageReport, ProjectReport, ReportError, ScanReport, Severity, Status,
    Summary, ToolchainReport,
};

pub fn inspect(request: InspectRequest) -> InspectReport {
    let policy = request.policy().clone();
    inspect_with(request, &policy, execution::execute)
}

fn inspect_with(
    request: InspectRequest,
    policy: &policy::PolicyInput,
    execute: impl FnOnce(&std::path::Path, &policy::PolicyPlan) -> execution::ExecutionResult,
) -> InspectReport {
    let plan = match policy::PolicyPlan::compile(policy) {
        Ok(plan) => plan,
        Err(error) => return report::policy_failure(error, policy.blocking()),
    };
    report::from_execution(execute(&request.path, &plan), &plan)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::Path;

    use super::*;
    use crate::policy::{PolicyInput, RuleLevel};

    #[test]
    fn invalid_policy_returns_before_execution_or_path_discovery() {
        let executed = Cell::new(false);
        let request = InspectRequest::new("/path/that/must/not/be/inspected");
        let policy = PolicyInput::default().with_rule("unknown::rule", RuleLevel::Warn);

        let report = inspect_with(request, &policy, |_, _| {
            executed.set(true);
            unreachable!("invalid policy must prevent execution")
        });

        assert!(!executed.get());
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
        let baseline = inspect_with(
            InspectRequest::new(&fixture),
            &PolicyInput::default(),
            execution::execute,
        );
        assert_eq!(baseline.status, Status::Complete);
        let baseline_ids: Vec<_> = baseline
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_deref() == Some("clippy::todo"))
            .map(|diagnostic| diagnostic.id.clone())
            .collect();
        assert!(!baseline_ids.is_empty());

        let off_policy = PolicyInput::default().with_rule("clippy::todo", RuleLevel::Off);
        let off = inspect_with(
            InspectRequest::new(&fixture),
            &off_policy,
            execution::execute,
        );
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
        let error = inspect_with(
            InspectRequest::new(&fixture),
            &error_policy,
            execution::execute,
        );
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
