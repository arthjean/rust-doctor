use std::io::Write;

use crate::presentation::{DiagnosticGroup, GroupDiagnostic, WITHHELD_HEADING};

use super::{GroupView, RenderError, Style, TerminalOptions, line, render_group, severity_style};

const SUMMARY_ROWS: usize = 20;

#[derive(Default)]
pub(super) struct WithheldHeadings {
    last: Option<(Option<crate::policy::RuleTier>, bool)>,
}

impl WithheldHeadings {
    pub(super) fn before<W: Write>(
        &mut self,
        writer: &mut W,
        group: &DiagnosticGroup,
        options: TerminalOptions<'_>,
    ) -> Result<(), RenderError> {
        let current = (group.tier, group.withheld);
        let opens = group.withheld && self.last != Some(current);
        self.last = Some(current);
        if opens {
            line(writer, WITHHELD_HEADING, options, Style::Heading)?;
        }
        Ok(())
    }
}

pub(super) fn render_summary<W: Write>(
    writer: &mut W,
    scored: &[&DiagnosticGroup],
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let framed = scored.iter().position(|group| group.count.weighs());
    if framed.is_none() {
        line(writer, "No scored issues found.", options, Style::Success)?;
    }
    let mut headings = WithheldHeadings::default();
    let mut rows = 0usize;
    for (index, group) in scored.iter().enumerate() {
        if Some(index) == framed {
            headings.before(writer, group, options)?;
            render_group(writer, group, options, GroupView::Top)?;
            continue;
        }
        if rows == SUMMARY_ROWS - 1 {
            break;
        }
        headings.before(writer, group, options)?;
        render_group_row(writer, group, options)?;
        rows += 1;
    }
    let shown = rows + usize::from(framed.is_some());
    match scored.len().saturating_sub(shown) {
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
        &format!("{}  {}{site}", group.header(), group.rule_id),
        options,
        severity_style(group.severity),
    )
}
