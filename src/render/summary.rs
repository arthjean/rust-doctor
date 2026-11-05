use std::io::Write;

use crate::presentation::{DiagnosticGroup, GroupDiagnostic};

use super::{GroupView, RenderError, Style, TerminalOptions, line, render_group, severity_style};

const SUMMARY_ROWS: usize = 20;

pub(super) fn render_summary<W: Write>(
    writer: &mut W,
    scored: &[&DiagnosticGroup],
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let weighing: Vec<_> = scored.iter().filter(|group| !group.is_unscored()).collect();
    let Some((first, rest)) = weighing.split_first() else {
        return line(writer, "No scored issues found.", options, Style::Success);
    };
    render_group(writer, first, options, GroupView::Top)?;
    let shown = rest.len().min(SUMMARY_ROWS - 1);
    for group in rest.iter().take(shown) {
        render_group_row(writer, group, options)?;
    }
    match rest.len() - shown {
        0 => Ok(()),
        more => line(
            writer,
            &format!("and {more} more rules: --verbose"),
            options,
            Style::Muted,
        ),
    }
}

fn render_group_row<W: Write>(
    writer: &mut W,
    group: &DiagnosticGroup,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let representative = group.representative();
    let note = representative
        .and_then(GroupDiagnostic::site_note)
        .map(|note| format!(" ({note})"))
        .unwrap_or_default();
    let site = representative
        .and_then(GroupDiagnostic::location)
        .map(|location| {
            format!(
                "  {}:{}:{}{note}",
                location.path, location.span.line_start, location.span.column_start
            )
        })
        .unwrap_or_default();
    line(
        writer,
        &format!(
            "{}  {}  {}  {}{site}",
            group.severity,
            group.title,
            super::tally::summary_occurrences(group),
            group.rule_id
        ),
        options,
        severity_style(group.severity),
    )
}
