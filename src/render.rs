use std::error::Error;
use std::fmt;
use std::io::{self, Write};

use crate::{GateStatus, InspectReport, Status};

#[derive(Debug)]
pub enum RenderError {
    Json(serde_json::Error),
    Write(io::Error),
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "could not serialize report: {error}"),
            Self::Write(error) => write!(formatter, "could not write report: {error}"),
        }
    }
}

impl Error for RenderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Write(error) => Some(error),
        }
    }
}

pub fn render_json<W: Write>(report: &InspectReport, mut writer: W) -> Result<(), RenderError> {
    serde_json::to_writer(&mut writer, report).map_err(RenderError::Json)?;
    writer.write_all(b"\n").map_err(RenderError::Write)
}

pub fn render_terminal<W: Write>(report: &InspectReport, mut writer: W) -> Result<(), RenderError> {
    if let Some(scope) = &report.scope {
        match scope.files_details() {
            None => writeln!(
                writer,
                "Scope: full; execution workspace; all files selected; base none."
            ),
            Some((comparison_base, files)) => {
                let file_count = files.len();
                let comparison_base = &comparison_base[..12];
                writeln!(
                    writer,
                    "Scope: files; execution workspace; {file_count} selected files; base {comparison_base}."
                )
            }
        }
        .map_err(RenderError::Write)?;
    }

    if let Some(policy) = &report.policy {
        let source = match policy.blocking.source {
            crate::BlockingLevelSource::Default => "default",
            crate::BlockingLevelSource::Config => "config",
            crate::BlockingLevelSource::Request => "request",
        };
        let configuration = policy
            .config_file
            .as_deref()
            .map_or_else(|| "none loaded".to_owned(), |file| format!("{file} loaded"));
        writeln!(
            writer,
            "Configuration: {configuration}; blocking {} ({source})",
            policy.blocking.level,
        )
        .map_err(RenderError::Write)?;
    }

    for diagnostic in &report.diagnostics {
        let path = diagnostic.path.as_deref().unwrap_or("<unknown>");
        let (line, column) = diagnostic
            .span
            .as_ref()
            .map_or((0, 0), |span| (span.line_start, span.column_start));
        match diagnostic.code.as_deref() {
            Some(code) => writeln!(
                writer,
                "{path}:{line}:{column} {} [{code}] {}",
                diagnostic.severity, diagnostic.message
            ),
            None => writeln!(
                writer,
                "{path}:{line}:{column} {} {}",
                diagnostic.severity, diagnostic.message
            ),
        }
        .map_err(RenderError::Write)?;
        if let (Some(category), Some(help)) =
            (diagnostic.category.as_deref(), diagnostic.help.as_deref())
        {
            writeln!(writer, "Help ({category}): {help}").map_err(RenderError::Write)?;
        }
        if diagnostic.base_severity != diagnostic.severity {
            writeln!(
                writer,
                "Policy: base severity {}, effective severity {}",
                diagnostic.base_severity, diagnostic.severity
            )
            .map_err(RenderError::Write)?;
        }
    }

    writeln!(
        writer,
        "{} diagnostic(s): {} error(s), {} warning(s), {} info, {} unknown; status {}",
        report.summary.total,
        report.summary.errors,
        report.summary.warnings,
        report.summary.info,
        report.summary.unknown,
        report.status,
    )
    .map_err(RenderError::Write)?;

    match (report.gate.status, report.gate.blocking_diagnostics) {
        (GateStatus::Passed | GateStatus::Failed, Some(count)) => writeln!(
            writer,
            "Gate {}: blocking {}, {} blocking diagnostic(s)",
            report.gate.status, report.gate.blocking, count
        ),
        (GateStatus::NotEvaluated, None) => writeln!(
            writer,
            "Gate not evaluated: blocking {}",
            report.gate.blocking
        ),
        _ => writeln!(writer, "Gate not evaluated: inconsistent gate state"),
    }
    .map_err(RenderError::Write)?;

    if report.status != Status::Complete {
        let heading = match report.status {
            Status::Incomplete => "Scan incomplete",
            Status::Failed => "Scan failed",
            Status::Complete => "Scan",
        };
        for error in &report.errors {
            writeln!(
                writer,
                "{heading}: {} ({}/{})",
                error.message, error.stage, error.code
            )
            .map_err(RenderError::Write)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BlockingLevel, Diagnostic, DiagnosticSource, DiagnosticSpan, GateReport, InspectReport,
        ScanReport, ScopeReport, Severity, Summary, ToolchainReport,
    };

    struct ClosedWriter {
        writes: usize,
    }

    impl Write for ClosedWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn report() -> InspectReport {
        InspectReport {
            schema_version: 6,
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
            diagnostics: vec![Diagnostic {
                id: "id".to_owned(),
                source: DiagnosticSource::Clippy,
                code: Some("clippy::lint".to_owned()),
                base_severity: Severity::Warning,
                severity: Severity::Warning,
                category: None,
                message: "message".to_owned(),
                help: None,
                package: Some("package".to_owned()),
                target: Some("target".to_owned()),
                path: Some("src/lib.rs".to_owned()),
                span: Some(DiagnosticSpan {
                    line_start: 2,
                    column_start: 3,
                    line_end: 2,
                    column_end: 4,
                }),
                occurrences: 1,
            }],
            errors: Vec::new(),
            summary: Summary {
                errors: 0,
                warnings: 1,
                info: 0,
                unknown: 0,
                total: 1,
            },
            gate: GateReport {
                blocking: BlockingLevel::Error,
                status: GateStatus::Passed,
                blocking_diagnostics: Some(0),
            },
        }
    }

    #[test]
    fn json_is_one_document_followed_by_newline() {
        let mut output = Vec::new();
        render_json(&report(), &mut output).unwrap();

        assert_eq!(output.last(), Some(&b'\n'));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output).unwrap()["schema_version"],
            6
        );
    }

    #[test]
    fn terminal_diagnostic_contains_location_severity_code_and_message() {
        let mut output = Vec::new();
        render_terminal(&report(), &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("src/lib.rs:2:3 warning [clippy::lint] message"));
        assert!(output.contains("status complete"));
        assert!(!output.contains("Help ("));
    }

    #[test]
    fn curated_terminal_diagnostic_is_followed_by_stable_help() {
        let mut report = report();
        report.diagnostics[0].category = Some("correctness".to_owned());
        report.diagnostics[0].help = Some("Replace the placeholder.".to_owned());
        let mut output = Vec::new();

        render_terminal(&report, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains(
            "warning [clippy::lint] message\nHelp (correctness): Replace the placeholder.\n"
        ));
    }

    #[test]
    fn incomplete_terminal_output_explains_each_structured_error() {
        let mut report = report();
        report.status = Status::Incomplete;
        report.complete = false;
        report.gate.status = GateStatus::NotEvaluated;
        report.gate.blocking_diagnostics = None;
        report.errors = vec![crate::ReportError {
            stage: "execution".to_owned(),
            code: "clippy-exit".to_owned(),
            message: "Clippy exited with status 101".to_owned(),
        }];
        let mut output = Vec::new();

        render_terminal(&report, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(
            output
                .contains("Scan incomplete: Clippy exited with status 101 (execution/clippy-exit)")
        );
        assert!(output.contains("Gate not evaluated: blocking error"));
    }

    #[test]
    fn closed_writer_returns_typed_error_without_second_document() {
        let mut writer = ClosedWriter { writes: 0 };
        let error = render_json(&report(), &mut writer).unwrap_err();

        assert!(matches!(error, RenderError::Json(_)));
        assert_eq!(writer.writes, 1);
    }

    #[test]
    fn terminal_renders_one_private_scope_line_for_full_and_files() {
        let mut full = report();
        full.scope = Some(ScopeReport::full());
        let mut output = Vec::new();
        render_terminal(&full, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.matches("Scope:").count(), 1);
        assert!(
            output
                .starts_with("Scope: full; execution workspace; all files selected; base none.\n")
        );

        let mut files = report();
        files.scope = Some(ScopeReport::files_scope(
            "0123456789abcdef0123456789abcdef01234567".to_owned(),
            Vec::new(),
        ));
        let mut output = Vec::new();
        render_terminal(&files, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.matches("Scope:").count(), 1);
        assert!(output.starts_with(
            "Scope: files; execution workspace; 0 selected files; base 0123456789ab.\n"
        ));

        files.scope = Some(ScopeReport::files_scope(
            "0123456789abcdef0123456789abcdef01234567".to_owned(),
            vec!["Cargo.toml".to_owned(), "src/private.rs".to_owned()],
        ));
        let mut output = Vec::new();
        render_terminal(&files, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.matches("Scope:").count(), 1);
        assert!(output.starts_with(
            "Scope: files; execution workspace; 2 selected files; base 0123456789ab.\n"
        ));
        assert!(!output.contains("src/private.rs"));
    }

    #[test]
    fn closed_writer_during_scope_returns_the_historical_typed_error() {
        let mut report = report();
        report.scope = Some(ScopeReport::full());
        let mut writer = ClosedWriter { writes: 0 };

        let error = render_terminal(&report, &mut writer).unwrap_err();

        assert!(matches!(error, RenderError::Write(_)));
        assert_eq!(writer.writes, 1);
    }
}
