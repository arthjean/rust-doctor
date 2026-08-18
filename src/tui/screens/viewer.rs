//! The split review: the rule list on the left, the detail on the right, and
//! the status bar under both.
//!
//! Transposition of `diagnostic-list.tsx`, `diagnostic-detail.tsx` and
//! `status-bar.tsx`. The arrangement comes from the [`Layout`] the model
//! resolved, so the same terminal geometry produces the same split, stacked or
//! compact screen the reference produces.

use std::collections::BTreeSet;
use std::path::Path;

use rust_doctor::presentation::{CodeFrameLine, GroupLocation, code_frame};

use crate::tui::model::{
    Arrangement, DETAIL_INDENT_COLUMNS, Entry, Layout, REPORT_SPLIT_MARGIN_COLUMNS,
    REPORT_SPLIT_PADDING_COLUMNS, Row, category_impact, pluralize, severity_variant,
};
use crate::tui::text::{Color, Line, Span, Style, wrap_spans};

pub struct ViewerState<'a> {
    pub rows: &'a [Row],
    pub entries: &'a [Entry],
    pub header: Vec<Line>,
    pub selected_entry: usize,
    pub visible_start: usize,
    /// Indices into `rows` the reader has already landed on.
    pub read: &'a BTreeSet<usize>,
    pub workspace_root: &'a Path,
    pub copy_feedback: Option<CopyFeedback>,
    pub exit_hint: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CopyFeedback {
    Copied,
    Failed,
}

pub fn viewer(state: &ViewerState<'_>, layout: Layout) -> Vec<Line> {
    let selected_row = state
        .entries
        .get(state.selected_entry)
        .and_then(Entry::row_index)
        .and_then(|index| state.rows.get(index));

    let list_width = match layout.arrangement {
        Arrangement::Split => layout.list_column_width,
        _ => layout.width,
    };
    let list = list_column(state, list_width, layout.list_height);
    let detail_width = match layout.arrangement {
        Arrangement::Split => layout
            .detail_column_width
            .saturating_sub(REPORT_SPLIT_MARGIN_COLUMNS + REPORT_SPLIT_PADDING_COLUMNS),
        _ => layout.width,
    };
    let detail = detail_pane(state, selected_row, detail_width);
    let status = status_bar(state, layout);

    match layout.arrangement {
        Arrangement::Compact => {
            let mut lines = if layout.shows_viewer_score_header {
                state.header.clone()
            } else {
                Vec::new()
            };
            lines.extend(list);
            lines.extend(status);
            lines
        }
        Arrangement::Stacked => {
            let mut lines = state.header.clone();
            lines.push(Line::blank());
            lines.extend(list);
            lines.push(Line::text("─".repeat(layout.width), Style::DIM));
            lines.extend(detail.into_iter().take(layout.detail_height));
            lines.extend(status);
            lines
        }
        Arrangement::Split => {
            let mut left = state.header.clone();
            left.push(Line::blank());
            left.extend(list);
            let right: Vec<Line> = detail.into_iter().take(layout.detail_height).collect();
            let mut lines = Vec::with_capacity(layout.detail_height + status.len());
            for row in 0..layout.detail_height {
                let left_line = left.get(row).cloned().unwrap_or_else(Line::blank);
                let right_line = right.get(row).cloned().unwrap_or_else(Line::blank);
                lines.push(
                    left_line
                        .truncate_end(layout.list_column_width)
                        .padded_to(layout.list_column_width)
                        .with(Span::plain(" ".repeat(REPORT_SPLIT_MARGIN_COLUMNS)))
                        .with(Span::new("│", Style::color(Color::Gray)))
                        .with(Span::plain(" ".repeat(REPORT_SPLIT_PADDING_COLUMNS)))
                        .extend(right_line),
                );
            }
            lines.extend(status);
            lines
        }
    }
}

fn list_column(state: &ViewerState<'_>, width: usize, height: usize) -> Vec<Line> {
    let mut lines = Vec::with_capacity(height);
    for offset in 0..height {
        let index = state.visible_start + offset;
        let Some(entry) = state.entries.get(index) else {
            lines.push(Line::blank());
            continue;
        };
        lines.push(match entry {
            Entry::Header(category) => {
                Line::text(category.as_str(), Style::BOLD).truncate_end(width)
            }
            Entry::Item { row_index } => match state.rows.get(*row_index) {
                Some(row) => diagnostic_item(
                    row,
                    index == state.selected_entry,
                    state.read.contains(row_index),
                )
                .truncate_end(width),
                None => Line::blank(),
            },
        });
    }
    lines
}

fn diagnostic_item(row: &Row, is_selected: bool, is_read: bool) -> Line {
    let variant = severity_variant(row.severity);
    let highlight = is_selected || !is_read;
    let marker = if is_selected {
        "› "
    } else if is_read {
        "  "
    } else {
        "• "
    };
    let marker_color = if is_selected {
        Color::Cyan
    } else if highlight {
        variant.color
    } else {
        Color::Default
    };
    let dim = is_read && !is_selected;
    let severity_style = Style {
        color: if highlight {
            variant.color
        } else {
            Color::Default
        },
        bold: false,
        dim,
    };
    let mut line = Line::blank()
        .with(Span::new(
            marker,
            Style {
                color: marker_color,
                bold: false,
                dim,
            },
        ))
        .with(Span::new(format!("{} ", variant.icon), severity_style))
        .with(Span::new(
            &row.title,
            Style {
                bold: is_selected,
                ..severity_style
            },
        ));
    if row.site_count > 1 {
        line.push(Span::dim(format!(" ×{}", row.site_count)));
    }
    line
}

fn detail_pane(state: &ViewerState<'_>, row: Option<&Row>, width: usize) -> Vec<Line> {
    let Some(row) = row else {
        return Vec::new();
    };
    let variant = severity_variant(row.severity);
    let mut title = Line::blank()
        .with(Span::new(
            format!("{} ", variant.icon),
            Style::color(variant.color),
        ))
        .with(Span::new(&row.title, Style::color(variant.color).bold()));
    if row.site_count > 1 {
        title.push(Span::dim(format!(" ×{}", row.site_count)));
    }
    let mut lines = vec![title.truncate_end(width)];

    let body_width = width.saturating_sub(DETAIL_INDENT_COLUMNS);
    let mut body = vec![
        Line::text(
            format!(
                "{} · {} · {}",
                row.category.as_str(),
                variant.label,
                row.location
            ),
            Style::DIM,
        )
        .truncate_end(body_width),
    ];

    if let Some(impact) = category_impact(row.category) {
        body.push(Line::blank());
        body.extend(labelled("Impact ", impact, body_width));
    }
    body.push(Line::blank());
    body.extend(labelled("Why ", &row.message, body_width));

    if let Some(frame) = row.frame.as_ref() {
        let frame_lines = code_frame_lines(state.workspace_root, frame, body_width);
        if !frame_lines.is_empty() {
            body.push(Line::blank());
            body.extend(frame_lines);
        }
    }
    if let Some(help) = &row.help {
        body.push(Line::blank());
        body.extend(labelled("Fix ", help, body_width));
    }
    if !row.rule_url.is_empty() {
        body.push(Line::blank());
        body.push(
            Line::of(Span::linked(
                format!("Rule guide: {}", row.rule_url),
                Style::color(Color::Blue),
                &row.rule_url,
            ))
            .truncate_end(body_width),
        );
    }
    if let Some(feedback) = state.copy_feedback {
        body.push(Line::blank());
        body.push(match feedback {
            CopyFeedback::Copied => {
                Line::text("✓ Copied issue context", Style::color(Color::Green))
            }
            CopyFeedback::Failed => Line::text(
                "Couldn't copy issue context. Press Enter to try again.",
                Style::color(Color::Yellow),
            ),
        });
    }

    lines.extend(
        body.into_iter()
            .map(|line| line.indented(DETAIL_INDENT_COLUMNS)),
    );
    lines
}

fn labelled(label: &str, body: &str, width: usize) -> Vec<Line> {
    wrap_spans(
        &[
            Span::new(label, Style::color(Color::Cyan)),
            Span::plain(body),
        ],
        width,
    )
}

/// The source excerpt around the framed site. A file the workspace no longer
/// exposes simply contributes no lines: the detail keeps its other sections
/// rather than reporting a read error the reader cannot act on.
fn code_frame_lines(workspace_root: &Path, location: &GroupLocation, width: usize) -> Vec<Line> {
    let Ok(frame) = code_frame(workspace_root, location) else {
        return Vec::new();
    };
    let gutter = frame.gutter_width();
    let mut lines = Vec::with_capacity(frame.lines.len());
    for source in &frame.lines {
        lines.push(source_line(source, gutter).truncate_end(width));
        let Some(marker) = source.marker else {
            continue;
        };
        let spaces = marker.column_start.saturating_sub(1);
        let carets = marker
            .column_end
            .saturating_sub(marker.column_start)
            .max(1);
        lines.push(
            Line::blank()
                .with(Span::dim(format!("  {:gutter$} │ ", "", gutter = gutter)))
                .with(Span::new(
                    format!("{}{}", " ".repeat(spaces), "^".repeat(carets)),
                    Style::color(Color::Red),
                ))
                .truncate_end(width),
        );
    }
    lines
}

fn source_line(source: &CodeFrameLine, gutter: usize) -> Line {
    let pointer = if source.primary { "> " } else { "  " };
    Line::blank()
        .with(Span::new(
            format!("{pointer}{:>gutter$} │ ", source.number, gutter = gutter),
            if source.primary {
                Style::color(Color::Yellow)
            } else {
                Style::DIM
            },
        ))
        .with(Span::plain(&source.text))
}

fn status_bar(state: &ViewerState<'_>, layout: Layout) -> Vec<Line> {
    let mut total = 0usize;
    let mut errors = 0usize;
    let mut warnings = 0usize;
    for row in state.rows {
        total = total.saturating_add(row.site_count);
        if row.is_error() {
            errors = errors.saturating_add(row.site_count);
        } else {
            warnings = warnings.saturating_add(row.site_count);
        }
    }
    let position = state
        .entries
        .iter()
        .take(state.selected_entry + 1)
        .filter(|entry| entry.row_index().is_some())
        .count();

    let counts = Line::blank()
        .with(Span::new(pluralize(total, "finding"), Style::BOLD))
        .with(Span::dim("  "))
        .with(Span::new(
            pluralize(errors, "error"),
            Style::color(Color::Red),
        ))
        .with(Span::dim("  "))
        .with(Span::new(
            pluralize(warnings, "warning"),
            Style::color(Color::Yellow),
        ))
        .with(Span::dim(format!(
            "  ·  issue {position}/{}",
            state.rows.len()
        )));
    let exit = Span::dim(format!(" · {}", state.exit_hint));
    let compact = layout.arrangement == Arrangement::Compact;

    // A compact terminal has one row for all of it, so a copy result takes the
    // place of the key hints instead of a line of its own.
    let keys = match state.copy_feedback.filter(|_| compact) {
        Some(CopyFeedback::Copied) => {
            Line::text("✓ Copied issue context", Style::color(Color::Green))
        }
        Some(CopyFeedback::Failed) => {
            Line::text("Copy failed · enter retry", Style::color(Color::Yellow))
        }
        None => Line::blank()
            .with(Span::dim("↑/↓ move · "))
            .with(Span::new("enter", Style::color(Color::Cyan)))
            .with(Span::dim(" copy context")),
    };

    if compact {
        return vec![
            counts
                .with(Span::dim("  ·  "))
                .extend(keys)
                .with(exit)
                .truncate_end(layout.width),
        ];
    }

    let unread = (0..state.rows.len())
        .filter(|index| !state.read.contains(index))
        .count();
    vec![
        Line::blank(),
        counts.truncate_end(layout.width),
        Line::of(Span::new(
            format!("{} unread  ·  ", pluralize(unread, "issue")),
            if unread > 0 {
                Style::color(Color::Cyan)
            } else {
                Style::DIM
            },
        ))
        .extend(keys)
        .with(exit)
        .truncate_end(layout.width),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::model::{build_entries, resolve_layout};
    use rust_doctor::{AuditCategoryName, Severity};

    fn visible(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|line| line.render(false, false)).collect()
    }

    #[test]
    fn the_status_bar_counts_sites_locates_the_issue_and_tracks_what_is_unread() {
        let rows = [
            sample_row("clippy::a", Severity::Error, 3),
            sample_row("clippy::b", Severity::Warning, 2),
        ];
        let entries = build_entries(&rows);
        let read = BTreeSet::from([0usize]);
        let state = ViewerState {
            rows: &rows,
            entries: &entries,
            header: Vec::new(),
            selected_entry: 1,
            visible_start: 0,
            read: &read,
            workspace_root: Path::new("."),
            copy_feedback: None,
            exit_hint: "esc back · q to quit",
        };
        let layout = resolve_layout(160, 48, entries.len());
        let rendered = visible(&status_bar(&state, layout));
        assert_eq!(rendered[0], "");
        assert_eq!(rendered[1], "5 findings  3 errors  2 warnings  ·  issue 1/2");
        assert_eq!(
            rendered[2],
            "1 issue unread  ·  ↑/↓ move · enter copy context · esc back · q to quit"
        );
    }

    /// A compact terminal folds all of it onto one row, and a copy result takes
    /// the place of the key hints rather than adding a line.
    #[test]
    fn a_compact_status_bar_is_one_row_and_a_copy_result_replaces_the_hints() {
        let rows = [sample_row("clippy::a", Severity::Warning, 1)];
        let entries = build_entries(&rows);
        let read = BTreeSet::new();
        let layout = resolve_layout(100, 20, entries.len());
        assert_eq!(layout.arrangement, Arrangement::Compact);
        let mut state = ViewerState {
            rows: &rows,
            entries: &entries,
            header: Vec::new(),
            selected_entry: 1,
            visible_start: 0,
            read: &read,
            workspace_root: Path::new("."),
            copy_feedback: None,
            exit_hint: "esc back · q to quit",
        };
        let hints = visible(&status_bar(&state, layout));
        assert_eq!(hints.len(), 1);
        assert!(hints[0].contains("enter copy context"));

        state.copy_feedback = Some(CopyFeedback::Copied);
        let copied = visible(&status_bar(&state, layout));
        assert_eq!(copied.len(), 1);
        assert!(copied[0].contains("✓ Copied issue context"));
        assert!(!copied[0].contains("enter copy context"));
    }

    fn sample_row(rule_id: &str, severity: Severity, site_count: usize) -> Row {
        Row {
            rule_id: rule_id.to_owned(),
            title: rule_id.to_owned(),
            category: AuditCategoryName::Bugs,
            severity,
            site_count,
            location: "src/lib.rs:1".to_owned(),
            message: "message".to_owned(),
            help: None,
            rule_url: String::new(),
            frame: None,
            sites: Vec::new(),
        }
    }
}
