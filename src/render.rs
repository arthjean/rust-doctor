use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use crate::git_scope::ScopeDetails;
use crate::presentation::{DiagnosticGroup, GroupDiagnostic, ReportPresentation, code_frame};
use crate::terminal_text::{sanitize, truncate, wrap};
use crate::{GateStatus, InspectReport, Status};

mod score_header;

const DEFAULT_WIDTH: usize = 80;

/// The linear report never renders narrower than this, and every entry point
/// normalizes to it. That is what makes the score block total: at this width
/// the block always fits, so neither it nor this file carries a
/// narrow-terminal fallback with a second rounding of its own.
const MIN_WIDTH: usize = 80;
const _: () = assert!(MIN_WIDTH >= crate::score_block::MIN_BLOCK_COLUMNS);
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
    /// Allows the score block to animate. Reserved for a real interactive
    /// terminal: any captured output must stay deterministic.
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
        render_related(writer, diagnostic, options, all_locations)?;
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

/// Other sites a structural finding spans.
///
/// The list is bounded here and complete in `--json`: a function cloned two
/// hundred times is one finding, and printing two hundred references would bury
/// every other finding under it.
fn render_related<W: Write>(
    writer: &mut W,
    diagnostic: &GroupDiagnostic,
    options: TerminalOptions<'_>,
    all_locations: bool,
) -> Result<(), RenderError> {
    const MAX_RELATED: usize = 3;
    if !all_locations || diagnostic.related.is_empty() {
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

/// Rules of the worst tier present in the score scope, bounded to fit on one
/// line. A cap with no named cause cannot be explained.
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
    score_header::render(
        writer,
        score,
        report.status != Status::Failed,
        options,
        score_header::Cadence::DEFAULT,
    )?;
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
    if let Some(withheld) = withheld_sentence(&score.withheld_rule_ids) {
        line(writer, &withheld, options, Style::Plain)?;
    }
    Ok(())
}

/// Why the loudest rule is missing from what to fix.
///
/// The rule with the most findings is often the one the corpus found most often
/// wrong, so a list that drops it without a word reads as a defect of the tool.
/// Two names carry the point; past that a count does, because the sentence is
/// there to explain an absence, not to enumerate one.
fn withheld_sentence(withheld: &[String]) -> Option<String> {
    let named: Vec<&str> = withheld.iter().take(2).map(String::as_str).collect();
    let subject = match (named.as_slice(), withheld.len()) {
        ([], _) => return None,
        ([only], _) => format!("{only} reports here but is"),
        ([first, second], 2) => format!("{first} and {second} report here but are"),
        ([first, second], total) => format!(
            "{first}, {second} and {} more report here but are",
            total - 2
        ),
        _ => return None,
    };
    Some(format!(
        "{subject} left out: the corpus adjudicated no true positive for them on healthy code."
    ))
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
mod tests;
