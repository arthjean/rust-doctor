use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use crate::presentation::{
    COMPILER_NOTES_HEADING, DiagnosticGroup, GroupDiagnostic, ReportPresentation, code_frame,
};
use crate::terminal_text::{sanitize, truncate, wrap};
use crate::{GateStatus, InspectReport, ScopeMode, Status};

pub mod markdown;
mod narrowing;
mod scope;
mod score;
mod score_header;
mod summary;
mod tally;

use narrowing::render_narrowing;
use scope::{render_not_evaluated, render_scanned, render_scope};
use score::render_score;
use summary::render_summary;
use tally::render_categories;

const DEFAULT_WIDTH: usize = 80;

const MIN_WIDTH: usize = 40;
const _: () = assert!(MIN_WIDTH >= crate::score_block::MIN_BLOCK_COLUMNS);
const FRAME_GUTTER_COLUMNS: usize = 4;
const DOCS_URL: &str = "https://rust-doctor.com/docs";
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
    pub animate: bool,
    pub requested_scope: Option<ScopeMode>,
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
            requested_scope: None,
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
    let mut document = serde_json::to_vec(report).map_err(RenderError::Json)?;
    document.push(b'\n');
    writer
        .write_all(&document)
        .map_err(|error| RenderError::Json(serde_json::Error::io(error)))
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
    let writer = &mut writer;

    if report.status == Status::Failed {
        return render_failure(writer, report, options);
    }

    render_scope(writer, report, options)?;
    render_scanned(writer, report, options)?;
    render_findings(writer, report, presentation, options)?;
    render_totals(writer, report, presentation, options)?;
    render_categories(writer, report, options)?;
    render_configuration(writer, report, options)?;
    render_advisory_database(writer, report, options)?;
    render_not_evaluated(writer, report, options)?;
    render_narrowing(writer, report, options)?;
    render_delta(writer, report, options)?;
    render_gate(writer, report, options)?;
    render_scan_errors(writer, report, options)?;
    render_advisories(writer, presentation, options)?;
    render_score(writer, report, presentation, options)?;
    render_links(writer, report, options)
}

fn render_failure<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    render_scope(writer, report, options)?;
    render_scan_errors(writer, report, options)
}

fn render_findings<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    presentation: &ReportPresentation,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    if presentation.issue_count == 0 {
        return match crate::presentation::incomplete_message(report) {
            Some(message) => line(writer, &message, options, Style::Warning),
            None => line(writer, "No issues found.", options, Style::Success),
        };
    }
    let (notes, scored): (Vec<_>, Vec<_>) = presentation
        .groups
        .iter()
        .partition(|group| group.is_compiler_note());
    if !options.verbose {
        return render_summary(writer, &scored, options);
    }
    let mut headings = summary::WithheldHeadings::default();
    for group in scored {
        headings.before(writer, group, options)?;
        if group.is_unscored() {
            render_unscored_group(writer, group, options)?;
        } else {
            render_group(writer, group, options, GroupView::Full)?;
        }
    }
    if !notes.is_empty() {
        line(writer, COMPILER_NOTES_HEADING, options, Style::Heading)?;
    }
    for group in notes {
        line(
            writer,
            &format!(
                "{} ({} occurrences, {})",
                group.title, group.occurrences, group.rule_id
            ),
            options,
            Style::Muted,
        )?;
    }
    Ok(())
}

fn render_totals<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    presentation: &ReportPresentation,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    line(
        writer,
        &"─".repeat(options.width.min(48)),
        options,
        Style::Muted,
    )?;
    let notes = match presentation.note_count {
        0 => String::new(),
        count => format!(", {count} of them compiler notes (not scored)"),
    };
    let production = report.audit.production_totals();
    let scored = |count: usize, noun: &str| {
        format!("{count} scored {noun}{}", if count == 1 { "" } else { "s" })
    };
    let errors = match production.errors {
        0 => String::new(),
        count => format!("{}, ", scored(count, "error")),
    };
    line(
        writer,
        &format!(
            "All {} occurrences across {} findings{notes}; {errors}{}, {} info",
            presentation.issue_count,
            presentation.finding_count,
            scored(production.warnings, "warning"),
            production.info
        ),
        options,
        Style::Heading,
    )
}

fn render_advisories<W: Write>(
    writer: &mut W,
    presentation: &ReportPresentation,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    if !options.verbose && !presentation.groups.is_empty() {
        line(
            writer,
            "Run with --verbose to see every issue.",
            options,
            Style::Muted,
        )?;
    }
    for advisory in &presentation.migration_advisories {
        line(writer, &advisory.sentence(&advisory.rule_id), options, Style::Warning)?;
    }
    Ok(())
}

fn render_links<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    if let Ok(url) = report.audit.share_url() {
        link_line(writer, &format!("Share: {url}"), options, Style::Accent)?;
    }
    line(writer, &format!("Docs: {DOCS_URL}"), options, Style::Muted)?;
    line(
        writer,
        &format!("GitHub: {GITHUB_URL}"),
        options,
        Style::Muted,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupView {
    Top,
    Full,
}

impl GroupView {
    const fn location_limit(self) -> usize {
        match self {
            Self::Top => 1,
            Self::Full => usize::MAX,
        }
    }
}

fn render_group<W: Write>(
    writer: &mut W,
    group: &DiagnosticGroup,
    options: TerminalOptions<'_>,
    view: GroupView,
) -> Result<(), RenderError> {
    line(writer, &group.header(), options, severity_style(group.severity))?;
    line(
        writer,
        &format!("Rule ID: {}", group.rule_id),
        options,
        Style::Accent,
    )?;
    if let Some(help) = group.resolved_help() {
        line(writer, &format!("Help: {help}"), options, Style::Muted)?;
    }
    if let Some(causes) = group.causes.line() {
        line(writer, &causes, options, Style::Plain)?;
    }

    for diagnostic in group.diagnostics.iter().take(view.location_limit()) {
        line(writer, &diagnostic.message, options, Style::Plain)?;
        let (frame, suggestion) = framed_suggestion(diagnostic, options);
        if let Some(suggestion) = suggestion {
            line(writer, &suggestion, options, Style::Accent)?;
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
        render_related(writer, diagnostic, options, view)?;
        if let Some(frame) = frame {
            render_code_frame(writer, frame, diagnostic.site_note(), options)?;
        }
    }
    link_line(
        writer,
        &format!("Rule: {}", group.rule_url),
        options,
        Style::Muted,
    )
}

fn render_unscored_group<W: Write>(
    writer: &mut W,
    group: &DiagnosticGroup,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    line(
        writer,
        &format!("{}  {}", group.header(), group.rule_id),
        options,
        Style::Muted,
    )
}

type Frame = Result<crate::presentation::CodeFrame, crate::presentation::CodeFrameUnavailable>;

fn framed_suggestion(
    diagnostic: &GroupDiagnostic,
    options: TerminalOptions<'_>,
) -> (Option<Frame>, Option<String>) {
    use crate::presentation::{SuggestionPlacement, replaced_text};

    let location = diagnostic.location();
    let finding_frame = || {
        location
            .as_ref()
            .map(|location| code_frame(options.workspace_root, location))
    };
    let Some(suggestion) = &diagnostic.suggestion else {
        return (finding_frame(), None);
    };
    match SuggestionPlacement::of(diagnostic.span.as_ref(), suggestion) {
        SuggestionPlacement::Whole => (finding_frame(), Some(replacement_line(suggestion, None))),
        SuggestionPlacement::Elsewhere => (finding_frame(), None),
        SuggestionPlacement::Part(replaced) => {
            let Some(location) = location.as_ref() else {
                return (None, None);
            };
            let part = crate::presentation::GroupLocation {
                path: location.path.clone(),
                span: replaced.clone(),
            };
            match code_frame(options.workspace_root, &part) {
                Ok(mut frame) => match replaced_text(&frame, &replaced) {
                    Some(original) => {
                        frame.location = format!(
                            "{}:{}:{}",
                            location.path,
                            location.span.line_start.max(1),
                            location.span.column_start.max(1)
                        );
                        let line = replacement_line(suggestion, Some(&original));
                        (Some(Ok(frame)), Some(line))
                    }
                    None => (finding_frame(), None),
                },
                Err(_) => (finding_frame(), None),
            }
        }
    }
}

fn replacement_line(suggestion: &crate::Suggestion, original: Option<&str>) -> String {
    let replacement: String = suggestion
        .replacement
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    let subject = original.map_or_else(
        || "Replace with".to_owned(),
        |original| format!("Replace `{original}` with"),
    );
    match suggestion.applicability {
        crate::Applicability::MachineApplicable => format!("{subject}: {replacement}"),
        other => format!("{subject}: {replacement} ({})", other.as_str()),
    }
}

fn render_code_frame<W: Write>(
    writer: &mut W,
    frame: Frame,
    note: Option<String>,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let note = note.map(|note| format!(" ({note})")).unwrap_or_default();
    let frame = match frame {
        Ok(frame) => frame,
        Err(unavailable) => {
            if let Some(location) = unavailable.location {
                line(writer, &format!("{location}{note}"), options, Style::Accent)?;
            }
            return line(writer, &unavailable.message, options, Style::Muted);
        }
    };
    line(writer, &format!("{}{note}", frame.location), options, Style::Accent)?;
    let gutter = frame.gutter_width().max(FRAME_GUTTER_COLUMNS);
    let indent = " ".repeat(gutter.saturating_add(3));
    for source in frame.lines {
        let prefix = if source.primary { ">" } else { " " };
        frame_line(
            writer,
            &format!("{prefix} {:>gutter$} | {}", source.number, source.text),
            options,
            Style::Plain,
        )?;
        if let Some(marker) = source.marker {
            let spaces = marker.column_start.saturating_sub(1);
            let carets = marker.column_end.saturating_sub(marker.column_start).max(1);
            frame_line(
                writer,
                &format!("{indent}| {}{}", " ".repeat(spaces), "^".repeat(carets)),
                options,
                Style::Warning,
            )?;
        }
    }
    Ok(())
}

fn render_related<W: Write>(
    writer: &mut W,
    diagnostic: &GroupDiagnostic,
    options: TerminalOptions<'_>,
    view: GroupView,
) -> Result<(), RenderError> {
    const MAX_RELATED: usize = 3;
    if view != GroupView::Full || diagnostic.related.is_empty() {
        return Ok(());
    }
    for location in diagnostic.related.iter().take(MAX_RELATED) {
        line(
            writer,
            &format!(
                "Also at: {}:{}:{}",
                location.path, location.span.line_start, location.span.column_start
            ),
            options,
            Style::Accent,
        )?;
    }
    let remaining = diagnostic.related.len().saturating_sub(MAX_RELATED);
    if remaining > 0 {
        line(
            writer,
            &format!("and {remaining} more locations"),
            options,
            Style::Muted,
        )?;
    }
    Ok(())
}

fn render_configuration<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let Some(policy) = &report.policy else {
        return Ok(());
    };
    let source = match policy.blocking.source {
        crate::BlockingLevelSource::Default => "default",
        crate::BlockingLevelSource::Config => "config",
        crate::BlockingLevelSource::Request => "request",
    };
    let configuration = match policy.config_file.as_deref() {
        Some(file) => format!(
            "Configuration: {file} loaded; blocking {} ({source})",
            policy.blocking.level
        ),
        None => "Configuration: no rust-doctor.toml; defaults apply.".to_owned(),
    };
    line(writer, &configuration, options, Style::Muted)
}

const ADVISORY_DB_MISSING: &str = "Known-vulnerability check skipped: no RustSec database under ~/.cargo. Run `cargo deny fetch` or `cargo audit` once, or pass --advisory-db <path>.";

const ADVISORY_CHECK_OFF: &str =
    "Known-vulnerability check skipped: --advisory-db /dev/null turns it off.";

fn render_advisory_database<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    use crate::score_block::UncheckedAdvisories;

    let skipped = match report.unchecked_advisories() {
        Some(UncheckedAdvisories::NoDatabase) => Some(ADVISORY_DB_MISSING),
        Some(UncheckedAdvisories::TurnedOff) => Some(ADVISORY_CHECK_OFF),
        None => None,
    };
    if let Some(skipped) = skipped {
        return line(writer, skipped, options, Style::Muted);
    }
    let Some(database) = &report.scan.advisory_db else {
        return Ok(());
    };
    match database.skipped {
        0 => {}
        1 => line(writer, "1 advisory file could not be read.", options, Style::Muted)?,
        skipped => line(
            writer,
            &format!("{skipped} advisory files could not be read."),
            options,
            Style::Muted,
        )?,
    }
    let age = database.age_days(std::time::SystemTime::now());
    match age {
        Some(days) if days > crate::cargo_health::STALE_AFTER_DAYS => line(
            writer,
            &format!("The advisory database is {days} days old."),
            options,
            Style::Warning,
        ),
        _ => Ok(()),
    }
}

fn render_delta<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let Some(delta) = &report.delta else {
        return Ok(());
    };
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
    if delta.introduced.is_empty() && delta.fixed.is_empty() {
        return Ok(());
    }
    line(
        writer,
        "Baseline details remain available in the JSON report.",
        options,
        Style::Muted,
    )
}

fn render_gate<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    line(writer, &gate_sentence(&report.gate), options, Style::Muted)
}

fn gate_sentence(gate: &crate::GateReport) -> String {
    use crate::BlockingLevel;
    let level = gate.blocking;
    let which = |count: usize| match level {
        BlockingLevel::Warning => format!(
            "{count} finding{} at warning level or above",
            if count == 1 { "" } else { "s" }
        ),
        _ => format!(
            "{count} {level}-level finding{}",
            if count == 1 { "" } else { "s" }
        ),
    };
    let outcome = match (gate.status, gate.blocking_diagnostics, level) {
        (GateStatus::Passed, _, BlockingLevel::None) => "passed, nothing blocks.".to_owned(),
        (GateStatus::Passed, Some(_), BlockingLevel::Warning) => {
            "passed, no finding at warning level or above.".to_owned()
        }
        (GateStatus::Passed, Some(_), _) => format!("passed, no {level}-level finding."),
        (GateStatus::Failed, Some(count), _) => format!("failed, {}.", which(count)),
        _ => "not evaluated.".to_owned(),
    };
    format!("Exit gate (--blocking {level}): {outcome}")
}

fn render_scan_errors<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let heading = match report.status {
        Status::Complete => return Ok(()),
        Status::Incomplete => "Scan incomplete",
        Status::Failed => "Scan failed",
    };
    for error in &report.errors {
        let located = match &report.scan.advisory_check {
            crate::AdvisoryCheck::Unreadable {
                database,
                skipped,
                files,
            } if error.code == crate::cargo_health::UNREADABLE => Some(
                crate::cargo_health::unreadable_sentence(Some(database), *skipped, *files),
            ),
            _ => None,
        };
        line(
            writer,
            &format!(
                "{heading}: {} ({}/{})",
                located.as_deref().unwrap_or(&error.message),
                error.stage,
                error.code
            ),
            options,
            Style::Warning,
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
    for bounded in wrap(&sanitize(content), options.width) {
        write_styled(writer, &bounded, options.color, style)?;
    }
    Ok(())
}

fn link_line<W: Write>(
    writer: &mut W,
    content: &str,
    options: TerminalOptions<'_>,
    style: Style,
) -> Result<(), RenderError> {
    write_styled(writer, &sanitize(content), options.color, style)
}

fn frame_line<W: Write>(
    writer: &mut W,
    content: &str,
    options: TerminalOptions<'_>,
    style: Style,
) -> Result<(), RenderError> {
    let bounded = truncate(&sanitize(content), options.width);
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

#[cfg(test)]
mod tests;
