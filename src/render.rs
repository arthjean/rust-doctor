use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use crate::git_scope::ScopeDetails;
use crate::presentation::{DiagnosticGroup, ReportPresentation, code_frame};
use crate::terminal_text::{sanitize, truncate, wrap};
use crate::{GateStatus, InspectReport, Status};

mod score_header;

const DEFAULT_WIDTH: usize = 80;
const MIN_WIDTH: usize = 80;
const DOCS_URL: &str = "https://rust-doctor.vercel.app/docs";
const GITHUB_URL: &str = "https://github.com/arthjean/rust-doctor";

#[derive(Debug)]
pub enum RenderError {
    InvalidReport,
    Json(serde_json::Error),
    Write(io::Error),
}

impl RenderError {
    pub fn is_broken_pipe(&self) -> bool {
        match self {
            Self::InvalidReport => false,
            Self::Json(error) => error.io_error_kind() == Some(io::ErrorKind::BrokenPipe),
            Self::Write(error) => error.kind() == io::ErrorKind::BrokenPipe,
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidReport => formatter.write_str("refusing to render an invalid report"),
            Self::Json(error) => write!(formatter, "could not serialize report: {error}"),
            Self::Write(error) => write!(formatter, "could not write report: {error}"),
        }
    }
}

impl Error for RenderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidReport => None,
            Self::Json(error) => Some(error),
            Self::Write(error) => Some(error),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalOptions<'a> {
    pub workspace_root: &'a Path,
    pub elapsed: Duration,
    pub verbose: bool,
    pub width: usize,
    pub color: bool,
    /// Autorise l'animation du bloc de score. Réservé à un vrai terminal
    /// interactif: toute sortie capturée doit rester déterministe.
    pub animate: bool,
}

impl<'a> TerminalOptions<'a> {
    pub const fn new(workspace_root: &'a Path) -> Self {
        Self {
            workspace_root,
            elapsed: Duration::ZERO,
            verbose: false,
            width: DEFAULT_WIDTH,
            color: false,
            animate: false,
        }
    }

    fn normalized(self) -> Self {
        Self {
            width: self.width.max(MIN_WIDTH),
            ..self
        }
    }
}

pub fn render_json<W: Write>(report: &InspectReport, mut writer: W) -> Result<(), RenderError> {
    if !report.is_valid() {
        return Err(RenderError::InvalidReport);
    }
    serde_json::to_writer(&mut writer, report).map_err(RenderError::Json)?;
    writer.write_all(b"\n").map_err(RenderError::Write)
}

pub fn render_terminal<W: Write>(report: &InspectReport, writer: W) -> Result<(), RenderError> {
    render_terminal_with_options(report, writer, TerminalOptions::new(Path::new(".")))
}

pub fn render_terminal_with_options<W: Write>(
    report: &InspectReport,
    writer: W,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let presentation = ReportPresentation::derive_terminal(report);
    render_terminal_with_presentation(report, &presentation, writer, options)
}

pub fn render_terminal_with_presentation<W: Write>(
    report: &InspectReport,
    presentation: &ReportPresentation,
    mut writer: W,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    if !report.is_valid() {
        return Err(RenderError::InvalidReport);
    }
    let options = options.normalized();
    let issue_count = presentation.issue_count;

    render_scope(&mut writer, report, options)?;
    line(
        &mut writer,
        &format!(
            "Scanned {} files in {:.1}s",
            report.audit.source_files,
            options.elapsed.as_secs_f64()
        ),
        options,
        Style::Accent,
    )?;

    if issue_count == 0 {
        line(&mut writer, "No issues found.", options, Style::Success)?;
    } else if options.verbose {
        for group in &presentation.groups {
            render_group(&mut writer, group, options, false, true)?;
        }
    } else if let Some(group) = presentation.groups.first() {
        render_group(&mut writer, group, options, true, false)?;
    }

    line(
        &mut writer,
        &"─".repeat(options.width.min(48)),
        options,
        Style::Muted,
    )?;
    line(
        &mut writer,
        &format!(
            "All {issue_count} occurrences across {} findings",
            presentation.finding_count
        ),
        options,
        Style::Heading,
    )?;
    render_categories(&mut writer, report, options)?;
    render_legacy_context(&mut writer, report, options)?;

    if !options.verbose && !presentation.groups.is_empty() {
        line(
            &mut writer,
            "Run with --verbose to see every issue.",
            options,
            Style::Muted,
        )?;
    }
    for advisory in &presentation.migration_advisories {
        line(
            &mut writer,
            &format!(
                "Migration advisory: {} appears {} times across {} files.",
                advisory.rule_id, advisory.occurrences, advisory.files
            ),
            options,
            Style::Warning,
        )?;
    }

    render_score(&mut writer, report, options)?;
    if report.status != Status::Failed {
        if let Ok(url) = report.audit.share_url() {
            line(
                &mut writer,
                &format!("Share: {url}"),
                options,
                Style::Accent,
            )?;
        }
        line(
            &mut writer,
            &format!("Docs: {DOCS_URL}"),
            options,
            Style::Muted,
        )?;
        line(
            &mut writer,
            &format!("GitHub: {GITHUB_URL}"),
            options,
            Style::Muted,
        )?;
    }
    Ok(())
}

fn render_scope<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let description = report.scope.as_ref().map_or_else(
        || "Scope: full codebase".to_owned(),
        |scope| match scope.details() {
            ScopeDetails::Full => "Scope: full codebase".to_owned(),
            ScopeDetails::Files {
                comparison_base,
                files,
            } => format!(
                "Scope: changed files ({} selected, base {})",
                files.len(),
                short_revision(comparison_base)
            ),
            ScopeDetails::Baseline { comparison_base } => format!(
                "Scope: baseline comparison (base {})",
                short_revision(comparison_base)
            ),
        },
    );
    line(writer, &description, options, Style::Heading)
}

fn render_group<W: Write>(
    writer: &mut W,
    group: &DiagnosticGroup,
    options: TerminalOptions<'_>,
    top: bool,
    all_locations: bool,
) -> Result<(), RenderError> {
    let heading = if top {
        format!("Top {}: {}", group.severity, group.title)
    } else {
        format!(
            "{}: {} ({} occurrences)",
            capitalize(group.severity.to_string()),
            group.title,
            group.occurrences
        )
    };
    line(writer, &heading, options, severity_style(group.severity))?;
    line(
        writer,
        &format!("Rule ID: {}", group.rule_id),
        options,
        Style::Accent,
    )?;

    let diagnostics: Box<dyn Iterator<Item = _>> = if all_locations {
        Box::new(group.diagnostics.iter())
    } else {
        Box::new(group.diagnostics.iter().take(1))
    };
    for diagnostic in diagnostics {
        line(writer, &diagnostic.message, options, Style::Plain)?;
        if let Some(help) = &diagnostic.help {
            line(writer, &format!("Help: {help}"), options, Style::Muted)?;
        }
        if diagnostic.base_severity != diagnostic.severity {
            line(
                writer,
                &format!(
                    "Policy: base severity {}, effective severity {}",
                    diagnostic.base_severity, diagnostic.severity
                ),
                options,
                Style::Muted,
            )?;
        }
        let Some(location) = diagnostic.location() else {
            continue;
        };
        match code_frame(options.workspace_root, &location) {
            Ok(frame) => {
                line(writer, &frame.location, options, Style::Accent)?;
                for source in frame.lines {
                    let prefix = if source.primary { ">" } else { " " };
                    frame_line(
                        writer,
                        &format!("{prefix} {:>4} | {}", source.number, source.text),
                        options,
                        Style::Plain,
                    )?;
                    if let Some(marker) = source.marker {
                        let spaces = marker.column_start.saturating_sub(1);
                        let carets = marker.column_end.saturating_sub(marker.column_start).max(1);
                        frame_line(
                            writer,
                            &format!("       | {}{}", " ".repeat(spaces), "^".repeat(carets)),
                            options,
                            Style::Warning,
                        )?;
                    }
                }
            }
            Err(unavailable) => {
                if let Some(location) = unavailable.location {
                    line(writer, &location, options, Style::Accent)?;
                }
                line(writer, &unavailable.message, options, Style::Muted)?;
            }
        }
    }
    line(
        writer,
        &format!("Rule: {}", group.rule_url),
        options,
        Style::Muted,
    )
}

fn render_categories<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    for category in &report.audit.categories {
        line(
            writer,
            &format!(
                "{}: {} errors, {} warnings, {} info, {} unknown (occurrences)",
                category.name, category.errors, category.warnings, category.info, category.unknown
            ),
            options,
            Style::Plain,
        )?;
    }
    if report.audit.categories.is_empty() {
        line(writer, "Categories: none", options, Style::Plain)?;
    }
    Ok(())
}

fn render_legacy_context<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
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
        line(
            writer,
            &format!(
                "Configuration: {configuration}; blocking {} ({source})",
                policy.blocking.level
            ),
            options,
            Style::Muted,
        )?;
    }
    if let Some(delta) = &report.delta {
        line(
            writer,
            &format!(
                "Delta: +{} introduced; ={} pre-existing; -{} fixed; {} cross-file matches.",
                delta.summary.introduced,
                delta.summary.pre_existing,
                delta.summary.fixed,
                delta.summary.cross_file_matches
            ),
            options,
            Style::Muted,
        )?;
        for diagnostic in &delta.fixed {
            let path = diagnostic.path.as_deref().unwrap_or("<unknown>");
            let (line_number, column) = diagnostic
                .span
                .as_ref()
                .map_or((0, 0), |span| (span.line_start, span.column_start));
            let code = diagnostic
                .code
                .as_deref()
                .map_or_else(String::new, |code| format!(" [{code}]"));
            line(
                writer,
                &format!(
                    "Fixed: {path}:{line_number}:{column} {}{code} {}",
                    diagnostic.severity, diagnostic.message
                ),
                options,
                Style::Success,
            )?;
        }
    }
    match (report.gate.status, report.gate.blocking_diagnostics) {
        (GateStatus::Passed | GateStatus::Failed, Some(count)) => line(
            writer,
            &format!(
                "Gate {}: blocking {}, {} blocking diagnostic(s)",
                report.gate.status, report.gate.blocking, count
            ),
            options,
            Style::Muted,
        )?,
        _ => line(
            writer,
            &format!("Gate not evaluated: blocking {}", report.gate.blocking),
            options,
            Style::Muted,
        )?,
    }
    if report.status != Status::Complete {
        let heading = match report.status {
            Status::Incomplete => "Scan incomplete",
            Status::Failed => "Scan failed",
            Status::Complete => "Scan",
        };
        for error in &report.errors {
            line(
                writer,
                &format!(
                    "{heading}: {} ({}/{})",
                    error.message, error.stage, error.code
                ),
                options,
                Style::Warning,
            )?;
        }
    }
    if let Some(delta) = &report.delta {
        let introduced = delta.introduced.iter().collect::<BTreeSet<_>>();
        if !introduced.is_empty() || !delta.fixed.is_empty() {
            line(
                writer,
                "Baseline details remain available in the JSON report.",
                options,
                Style::Muted,
            )?;
        }
    }
    Ok(())
}

/// Règles du pire tier présentes dans la portée du score, bornées pour tenir
/// sur une ligne. Un plafond sans cause nommée n'est pas explicable.
fn capping_rule_ids(report: &InspectReport, tier: crate::RuleTier) -> Vec<String> {
    const MAX_NAMED_RULES: usize = 3;
    let scoped: Option<BTreeSet<_>> = report.delta.as_ref().map(|delta| {
        delta
            .introduced
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
    });
    let mut ids: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            scoped
                .as_ref()
                .is_none_or(|scoped| scoped.contains(diagnostic.id.as_str()))
        })
        .filter_map(|diagnostic| diagnostic.code.as_deref())
        .filter(|code| crate::policy::find(code).is_some_and(|definition| definition.tier == tier))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_owned)
        .collect();
    ids.truncate(MAX_NAMED_RULES);
    ids
}

fn render_score<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let Some(score) = &report.audit.score else {
        return line(
            writer,
            "Score unavailable: no Rust files were analyzed.",
            options,
            Style::Warning,
        );
    };
    let label = if score.authoritative {
        score.label.as_str()
    } else {
        "Core partial"
    };
    if let Some((tier, ceiling)) = score.worst_tier.zip(score.applied_ceiling) {
        let blocking = capping_rule_ids(report, tier);
        line(
            writer,
            &format!(
                "Capped at {ceiling}/100 by a {} finding: {}",
                tier.as_str(),
                blocking.join(", ")
            ),
            options,
            Style::Warning,
        )?;
    }
    let drawn = score_header::render(
        writer,
        score.value,
        score.label,
        score.authoritative,
        report.status != Status::Failed,
        options,
    )?;
    if !drawn {
        let filled = usize::from(score.value) * 20 / 100;
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(20 - filled));
        line(
            writer,
            &format!("Rust Doctor score: {}/100 {label} [{bar}]", score.value),
            options,
            if score.authoritative {
                Style::Success
            } else {
                Style::Warning
            },
        )?;
    }
    if !score.authoritative {
        line(
            writer,
            "Score is partial because the scan did not complete or contains unscored findings.",
            options,
            Style::Warning,
        )?;
    }
    if let Some(projected) = score
        .projected_after_top_three
        .filter(|projected| *projected > score.value)
    {
        line(
            writer,
            &format!(
                "Fix the top {} rules to reach a projected {projected}/100: {}",
                score.projected_rule_ids.len(),
                score.projected_rule_ids.join(", ")
            ),
            options,
            Style::Accent,
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Style {
    Plain,
    Heading,
    Accent,
    Success,
    Warning,
    Muted,
}

fn severity_style(severity: crate::Severity) -> Style {
    match severity {
        crate::Severity::Error => Style::Warning,
        crate::Severity::Warning => Style::Warning,
        crate::Severity::Info => Style::Accent,
        crate::Severity::Unknown => Style::Muted,
    }
}

fn line<W: Write>(
    writer: &mut W,
    content: &str,
    options: TerminalOptions<'_>,
    style: Style,
) -> Result<(), RenderError> {
    let sanitized = sanitize(content);
    for bounded in wrap(&sanitized, options.width) {
        write_styled(writer, &bounded, options.color, style)?;
    }
    Ok(())
}

fn frame_line<W: Write>(
    writer: &mut W,
    content: &str,
    options: TerminalOptions<'_>,
    style: Style,
) -> Result<(), RenderError> {
    let sanitized = sanitize(content);
    let bounded = truncate(&sanitized, options.width);
    write_styled(writer, &bounded, options.color, style)
}

fn write_styled<W: Write>(
    writer: &mut W,
    content: &str,
    color: bool,
    style: Style,
) -> Result<(), RenderError> {
    if color && !matches!(style, Style::Plain) {
        let code = match style {
            Style::Heading => "1",
            Style::Accent => "36",
            Style::Success => "32",
            Style::Warning => "33",
            Style::Muted => "2",
            Style::Plain => "0",
        };
        writeln!(writer, "\u{1b}[{code}m{content}\u{1b}[0m").map_err(RenderError::Write)
    } else {
        writeln!(writer, "{content}").map_err(RenderError::Write)
    }
}

fn short_revision(revision: &str) -> &str {
    revision.get(..12).unwrap_or(revision)
}

fn capitalize(mut value: String) -> String {
    if let Some(first) = value.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_text::display_width;
    use crate::{
        Audit, BlockingLevel, DeltaMatch, DeltaReport, DeltaSummary, Diagnostic, DiagnosticSource,
        DiagnosticSpan, GateReport, GateStatus, InspectReport, ScanReport, Severity, Summary,
        ToolchainReport,
    };

    struct ClosedWriter;

    impl Write for ClosedWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn report() -> InspectReport {
        let diagnostics = vec![Diagnostic {
            context: None,
            id: "id".to_owned(),
            source: DiagnosticSource::Clippy,
            code: Some("clippy::todo".to_owned()),
            base_severity: Severity::Warning,
            severity: Severity::Warning,
            category: Some("correctness".to_owned()),
            message: "replace the placeholder".to_owned(),
            help: Some("Implement the intended behavior.".to_owned()),
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
        }];
        let summary = Summary::from_diagnostics(&diagnostics);
        InspectReport {
            schema_version: crate::report::SCHEMA_VERSION,
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
            diagnostics,
            delta: None,
            errors: Vec::new(),
            summary,
            gate: GateReport {
                blocking: BlockingLevel::Error,
                status: GateStatus::Passed,
                blocking_diagnostics: Some(0),
            },
        }
    }

    fn rendered(report: &InspectReport, width: usize, color: bool, verbose: bool) -> String {
        let mut output = Vec::new();
        render_terminal_with_options(
            report,
            &mut output,
            TerminalOptions {
                workspace_root: Path::new("tests/fixtures/kernel-contract/todo"),
                elapsed: Duration::from_millis(1250),
                verbose,
                width,
                color,
                animate: false,
            },
        )
        .unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn json_is_one_document_followed_by_newline() {
        let mut output = Vec::new();
        render_json(&report(), &mut output).unwrap();
        assert_eq!(output.last(), Some(&b'\n'));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output).unwrap()["schema_version"],
            crate::report::SCHEMA_VERSION
        );
    }

    #[test]
    fn invalid_report_is_rejected_before_any_output() {
        let mut report = report();
        report.audit.score.as_mut().unwrap().value = 101;
        let mut json = Vec::new();
        assert!(matches!(
            render_json(&report, &mut json),
            Err(RenderError::InvalidReport)
        ));
        assert!(json.is_empty());

        let mut terminal = Vec::new();
        assert!(matches!(
            render_terminal(&report, &mut terminal),
            Err(RenderError::InvalidReport)
        ));
        assert!(terminal.is_empty());
    }

    #[test]
    fn terminal_sections_follow_the_normative_order() {
        let output = rendered(&report(), 100, false, false);
        let labels = [
            "Scope:",
            "Scanned ",
            "Top warning:",
            "All 1 occurrences across 1 findings",
            "Bugs:",
            "Run with --verbose",
            // Bordure haute du bloc de score: le marqueur stable de la section
            // depuis que la valeur se lit `N / 100 Label`.
            "┌─────┐",
            "Share:",
            "Docs:",
            "GitHub:",
        ];
        let mut previous = 0usize;
        for label in labels {
            let position = output.find(label).expect("section should be rendered");
            assert!(position >= previous, "{label} was out of order\n{output}");
            previous = position;
        }
        assert!(output.contains("GitHub: https://github.com/arthjean/rust-doctor"));
    }

    #[test]
    fn widths_and_color_policy_are_stable() {
        for width in [80, 100, 140] {
            let plain = rendered(&report(), width, false, false);
            assert!(plain.lines().all(|line| display_width(line) <= width));
            assert!(!plain.contains("\u{1b}["));

            let colored = rendered(&report(), width, true, false);
            assert!(colored.contains("\u{1b}["));
            for line in colored.lines() {
                // `sanitize` retire toute séquence d'échappement, y compris le
                // truecolor de la barre d'un score parfait, que la liste de
                // codes en dur d'origine ne couvrait pas.
                let visible = sanitize(line);
                assert!(display_width(&visible) <= width, "{visible}");
            }
        }
    }

    #[test]
    fn wide_dynamic_text_respects_terminal_columns() {
        let mut report = report();
        report.diagnostics[0].message = "界".repeat(80);
        let output = rendered(&report, 80, false, false);
        assert!(output.lines().all(|line| display_width(line) <= 80));
    }

    #[test]
    fn partial_and_missing_scores_suppress_share_and_projection() {
        let mut partial = report();
        partial.audit.score.as_mut().unwrap().authoritative = false;
        partial
            .audit
            .score
            .as_mut()
            .unwrap()
            .projected_after_top_three = None;
        partial
            .audit
            .score
            .as_mut()
            .unwrap()
            .projected_rule_ids
            .clear();
        let output = rendered(&partial, 80, false, false);
        assert!(output.contains("Core partial"));
        assert!(!output.contains("Share:"));
        assert!(!output.contains("projected"));

        partial.audit.source_files = 0;
        partial.audit.score = None;
        let output = rendered(&partial, 80, false, false);
        assert!(output.contains("Score unavailable: no Rust files were analyzed."));
        assert!(!output.contains("Share:"));
    }

    #[test]
    fn projection_is_rendered_only_when_it_raises_the_score() {
        let mut flat = report();
        flat.diagnostics[0].code = Some("clippy::dbg_macro".to_owned());
        flat.diagnostics[0].category = Some("maintainability".to_owned());
        flat.audit = Audit::build(1, Status::Complete, &flat.diagnostics);
        let score = flat.audit.score.as_ref().unwrap();
        assert_eq!(score.projected_after_top_three, Some(score.value));
        assert!(!score.projected_rule_ids.is_empty());
        let output = rendered(&flat, 80, false, false);
        assert!(output.contains("100 / 100 Great"));
        assert!(!output.contains("projected"));

        let mut raising = report();
        raising.diagnostics[0].code = Some("rust_doctor::source::dynamic_shell_command".to_owned());
        raising.diagnostics[0].category = Some("security".to_owned());
        raising.audit = Audit::build(1, Status::Complete, &raising.diagnostics);
        let score = raising.audit.score.as_ref().unwrap();
        assert_eq!(score.value, 40);
        assert_eq!(score.projected_after_top_three, Some(100));
        let output = rendered(&raising, 80, false, false);
        assert!(output.contains("to reach a projected 100/100"));
        assert!(output.contains(
            "Capped at 40/100 by a P0 finding: rust_doctor::source::dynamic_shell_command"
        ));
    }

    #[test]
    fn failed_reports_preserve_the_closed_no_url_terminal_contract() {
        let mut failed = report();
        failed.status = Status::Failed;
        failed.complete = false;
        failed.diagnostics.clear();
        failed.audit = Audit::build(0, Status::Failed, &failed.diagnostics);
        failed.summary = Summary::default();

        let output = rendered(&failed, 80, false, false);

        assert!(!output.contains("https://"));
        assert!(!output.contains("Share:"));
        assert!(!output.contains("Docs:"));
        assert!(!output.contains("GitHub:"));
    }

    #[test]
    fn verbose_lists_all_groups_without_the_cta() {
        let output = rendered(&report(), 100, false, true);
        assert!(output.contains("Warning: Todo (1 occurrences)"));
        assert!(output.contains("Rule ID: clippy::todo"));
        assert!(!output.contains("Run with --verbose"));
    }

    #[test]
    fn broken_pipe_is_typed_and_detectable() {
        let error = render_terminal(&report(), ClosedWriter).unwrap_err();
        assert!(error.is_broken_pipe());
    }

    #[test]
    fn baseline_hides_pre_existing_details_and_keeps_introduced_and_fixed() {
        let mut report = report();
        let mut pre_existing = report.diagnostics[0].clone();
        pre_existing.id = "pre-existing-current".to_owned();
        pre_existing.message = "must stay hidden".to_owned();
        report.diagnostics.push(pre_existing);
        let mut fixed = report.diagnostics[0].clone();
        fixed.id = "fixed-baseline".to_owned();
        fixed.message = "removed debt".to_owned();
        report.delta = Some(DeltaReport {
            fingerprint_version: 1,
            base_diagnostics: 2,
            current_diagnostics: 2,
            introduced: vec!["id".to_owned()],
            pre_existing: vec![DeltaMatch {
                current_id: "pre-existing-current".to_owned(),
                baseline_id: "pre-existing-baseline".to_owned(),
            }],
            fixed: vec![fixed],
            summary: DeltaSummary {
                introduced: 1,
                pre_existing: 1,
                fixed: 1,
                cross_file_matches: 1,
            },
        });
        report.audit = Audit::build(1, Status::Complete, &report.diagnostics[..1]);
        report.summary = Summary::from_diagnostics(&report.diagnostics);

        let output = rendered(&report, 100, false, false);

        assert!(output.contains("replace the placeholder"));
        assert!(output.contains("Fixed: src/lib.rs:2:3 warning [clippy::todo] removed debt"));
        assert!(!output.contains("must stay hidden"));
    }
}
