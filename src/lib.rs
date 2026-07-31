#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod cargo_health;
mod execution;
pub mod render;
mod report;
mod rules;

pub use report::{
    Diagnostic, DiagnosticSource, DiagnosticSpan, InspectReport, InspectRequest, PackageReport,
    ProjectReport, ReportError, ScanReport, Severity, Status, Summary, ToolchainReport,
};

pub fn inspect(request: InspectRequest) -> InspectReport {
    report::from_execution(execution::execute(&request.path))
}
