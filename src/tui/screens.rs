//! The five screens of the interactive report, each one a list of rendered
//! rows.
//!
//! Every function here transposes one Ink component: `score-header.tsx`,
//! `action-menu.tsx`, `report-landing.tsx`, `agent-handoff.tsx`,
//! `handoff-ci-recommendation.tsx`, `ci-setup.tsx`, `diagnostic-list.tsx`,
//! `diagnostic-detail.tsx` and `status-bar.tsx`. Margins are the same rows and
//! paddings the same columns, so a frame lands on the geometry the reference
//! renders.

use std::path::Path;

use rust_doctor::AuditScore;
use rust_doctor::presentation::{CodeFrameLine, GroupLocation, code_frame};

use super::model::{
    ACTION_MENU_ITEM_GAP_ROWS, ACTION_MENU_MARGIN_ROWS, Arrangement, DETAIL_INDENT_COLUMNS, Entry,
    HORIZONTAL_PADDING_COLUMNS, Layout, PERFECT_SCORE, REPORT_SPLIT_MARGIN_COLUMNS,
    REPORT_SPLIT_PADDING_COLUMNS, Row, category_impact, doctor_face, pluralize, score_bar_width,
    score_color, score_right_column_width, severity_variant,
};
use super::text::{Color, Line, Span, Style, wrap_spans};

pub const BRANDING_NAME: &str = "Rust Doctor";
pub const BRANDING_URL: &str = "https://rust-doctor.vercel.app";
pub const GITHUB_ACTIONS_SETUP_URL: &str = "https://github.com/arthjean/rust-doctor#github-actions";

const POINTER: &str = "❯";
const POINTER_SMALL: &str = "›";

/// One entry of an action menu, with the block of description lines the
/// reference only reveals under the selected entry.
pub struct Action {
    pub label: String,
    pub description: Vec<Line>,
}

impl Action {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: Vec::new(),
        }
    }

    pub fn described(label: impl Into<String>, description: Vec<Line>) -> Self {
        Self {
            label: label.into(),
            description,
        }
    }
}

// ---------------------------------------------------------------- score header

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScoreVariant {
    Landing,
    Viewer,
}

/// The two-column score block: face on the left, value, bar and branding on the
/// right. `displayed` is the counted-up value of the current animation frame,
/// `displayed_projection` the head of the projected gain.
pub fn score_header(
    variant: ScoreVariant,
    score: Option<&AuditScore>,
    project_name: &str,
    finding_count: usize,
    width: usize,
    displayed: u8,
    displayed_projection: Option<u8>,
) -> Vec<Line> {
    let Some(score) = score else {
        let inner = width.saturating_sub(HORIZONTAL_PADDING_COLUMNS);
        return vec![
            branding_line().truncate_end(inner),
            Line::text(
                format!("{} · {project_name}", pluralize(finding_count, "finding")),
                Style::DIM,
            )
            .truncate_end(inner),
        ]
        .into_iter()
        .map(|line| line.indented(HORIZONTAL_PADDING_COLUMNS))
        .collect();
    };

    let color = score_color(score.label);
    let label = if score.authoritative {
        score.label.as_str()
    } else {
        "Core partial"
    };
    let summary = Line::blank()
        .with(Span::new(displayed.to_string(), Style::color(color).bold()))
        .with(Span::dim(format!(" / {PERFECT_SCORE} ")))
        .with(Span::new(label, Style::color(color)))
        .with(Span::dim(format!("  ·  {project_name}")));

    let Some(bar_width) = score_bar_width(width) else {
        return vec![
            summary.truncate_end(width),
            branding_line().truncate_end(width),
        ];
    };
    let right_width = score_right_column_width(width);

    let filled = scaled(displayed, bar_width);
    let projected = displayed_projection.map_or(filled, |projection| {
        scaled(projection, bar_width).min(bar_width)
    });
    let gain = projected.saturating_sub(filled);
    let empty = bar_width.saturating_sub(filled).saturating_sub(gain);
    let bar = Line::blank()
        .with(Span::new("█".repeat(filled), Style::color(color)))
        .with(Span::new("▓".repeat(gain), Style::color(color).dim()))
        .with(Span::dim("░".repeat(empty)));

    let [eyes, mouth] = doctor_face(score.label);
    let face = [
        "┌─────┐".to_owned(),
        format!("│ {eyes} │"),
        format!("│ {mouth} │"),
        "└─────┘".to_owned(),
    ];
    let right = [
        summary.truncate_end(right_width),
        bar,
        branding_line().truncate_end(right_width),
        Line::blank(),
    ];

    let mut lines: Vec<Line> = face
        .into_iter()
        .zip(right)
        .map(|(face, right)| {
            Line::of(Span::new(face, Style::color(color)))
                .indented(HORIZONTAL_PADDING_COLUMNS)
                .with(Span::plain("  "))
                .extend(right)
        })
        .collect();

    if variant == ScoreVariant::Landing
        && let Some(projection) = score
            .projected_after_top_three
            .filter(|projection| *projection > score.value)
    {
        let projection_color = score_color(projection_label(projection));
        lines.push(
            Line::blank()
                .with(Span::dim("  Potential score "))
                .with(Span::new(
                    projection.to_string(),
                    Style::color(projection_color),
                ))
                .with(Span::dim(" after priority fixes "))
                .with(Span::new(
                    format!("+{}", projection.saturating_sub(score.value)),
                    Style::color(projection_color),
                ))
                .truncate_end(width),
        );
    }
    lines
}

/// Rounds a score onto the bar, the way the reference does: half a cell up, so
/// a bar never reads empty on a workspace that scored.
fn scaled(value: u8, bar_width: usize) -> usize {
    let perfect = usize::from(PERFECT_SCORE);
    (usize::from(value) * bar_width + perfect / 2) / perfect
}

/// The bands the audit publishes, applied to a projected value the audit does
/// not label itself.
const fn projection_label(value: u8) -> rust_doctor::ScoreLabel {
    if value >= 75 {
        rust_doctor::ScoreLabel::Great
    } else if value >= 50 {
        rust_doctor::ScoreLabel::NeedsWork
    } else {
        rust_doctor::ScoreLabel::Critical
    }
}

fn branding_line() -> Line {
    Line::blank()
        .with(Span::linked(BRANDING_NAME, Style::PLAIN, BRANDING_URL))
        .with(Span::linked(
            format!(" ({BRANDING_URL})"),
            Style::DIM,
            BRANDING_URL,
        ))
}

// ----------------------------------------------------------------- action menu

fn action_menu(actions: &[Action], selected: usize) -> Vec<Line> {
    let mut lines = vec![Line::blank(); ACTION_MENU_MARGIN_ROWS];
    for (index, action) in actions.iter().enumerate() {
        let is_selected = index == selected;
        let style = if is_selected {
            Style::color(Color::Cyan).bold()
        } else {
            Style::PLAIN
        };
        let pointer = if is_selected { POINTER } else { POINTER_SMALL };
        lines.push(Line::text(format!("{pointer} {}", action.label), style));
        if is_selected {
            lines.extend(action.description.iter().cloned());
        }
        if index + 1 < actions.len() {
            lines.extend(vec![Line::blank(); ACTION_MENU_ITEM_GAP_ROWS]);
        }
    }
    lines
}

fn hint(content: &str) -> Vec<Line> {
    let mut lines = vec![Line::blank(); ACTION_MENU_MARGIN_ROWS];
    lines.push(Line::text(content, Style::DIM));
    lines
}

// --------------------------------------------------------------- landing

pub struct LandingNotices {
    pub issue_count: usize,
    pub incomplete: Option<String>,
    pub empty_state: Option<String>,
}

pub fn landing(
    header: Vec<Line>,
    notices: &LandingNotices,
    actions: &[Action],
    selected: usize,
    show_actions: bool,
) -> Vec<Line> {
    let mut lines = header;
    if notices.incomplete.is_some() || notices.issue_count == 0 {
        lines.push(Line::blank());
        if let Some(message) = &notices.incomplete {
            lines.push(Line::text(
                format!("⚠ {message}"),
                Style::color(Color::Yellow),
            ));
        } else {
            lines.push(Line::text(
                format!(
                    "✔ {}",
                    notices
                        .empty_state
                        .as_deref()
                        .unwrap_or("No issues found. Nice work.")
                ),
                Style::color(Color::Green),
            ));
        }
    }
    if !show_actions {
        return lines;
    }
    lines.extend(action_menu(actions, selected));
    lines.extend(hint(if actions.is_empty() {
        "q quit"
    } else {
        "↑/↓ move · enter select · q quit"
    }));
    lines
}

// ----------------------------------------------------------- agent handoff

pub fn agent_handoff(actions: &[Action], selected: usize) -> Vec<Line> {
    let mut lines = vec![
        Line::text("Choose how to continue", Style::BOLD).indented(HORIZONTAL_PADDING_COLUMNS),
    ];
    lines.extend(action_menu(actions, selected));
    lines.extend(hint("↑/↓ move · enter select · esc back · q quit"));
    lines
}

// -------------------------------------------------------------- CI screens

pub fn ci_justification(width: usize) -> Vec<Line> {
    [
        Line::text(
            "Scan every pull request to prevent new Rust issues while you fix the backlog.",
            Style::DIM,
        ),
        Line::text(
            "Baseline scope on a pull request, so only what the change introduces is judged.",
            Style::DIM,
        ),
        Line::of(Span::linked(
            GITHUB_ACTIONS_SETUP_URL,
            Style::color(Color::Cyan),
            GITHUB_ACTIONS_SETUP_URL,
        )),
    ]
    .into_iter()
    .map(|line| {
        line.indented(HORIZONTAL_PADDING_COLUMNS)
            .truncate_end(width)
    })
    .collect()
}

pub fn ci_recommendation(actions: &[Action], selected: usize, width: usize) -> Vec<Line> {
    let mut lines = vec![
        Line::text("Add Rust Doctor to GitHub Actions first", Style::BOLD)
            .indented(HORIZONTAL_PADDING_COLUMNS),
    ];
    lines.extend(ci_justification(width));
    lines.extend(action_menu(actions, selected));
    lines.extend(hint("↑/↓ move · enter select · esc skip · q quit"));
    lines
}

pub struct CiFeedback {
    pub succeeded: bool,
    pub message: String,
}

pub fn ci_setup(
    actions: &[Action],
    selected: usize,
    feedback: Option<&CiFeedback>,
    width: usize,
) -> Vec<Line> {
    let mut lines = vec![
        Line::blank()
            .with(Span::new("?", Style::color(Color::Cyan).bold()))
            .with(Span::new(
                " Add Rust Doctor to GitHub Actions?",
                Style::BOLD,
            )),
    ];
    lines.extend(ci_justification(width));
    lines.extend(action_menu(actions, selected));
    if let Some(feedback) = feedback {
        let style = Style::color(if feedback.succeeded {
            Color::Green
        } else {
            Color::Yellow
        });
        lines.push(Line::text(&feedback.message, style).truncate_end(width));
    }
    lines.extend(hint("↑/↓ move · enter select · esc back · q quit"));
    lines
}

// ------------------------------------------------------------- issue viewer

pub struct ViewerState<'a> {
    pub rows: &'a [Row],
    pub entries: &'a [Entry],
    pub header: Vec<Line>,
    pub selected_entry: usize,
    pub visible_start: usize,
    pub read: &'a dyn Fn(usize) -> bool,
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
            lines.extend(clipped(detail, layout.detail_height));
            lines.extend(status);
            lines
        }
        Arrangement::Split => {
            let mut left = state.header.clone();
            left.push(Line::blank());
            left.extend(list);
            let right = clipped(detail, layout.detail_height);
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

fn clipped(lines: Vec<Line>, height: usize) -> Vec<Line> {
    lines.into_iter().take(height).collect()
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
                Some(row) => {
                    diagnostic_item(row, index == state.selected_entry, (state.read)(*row_index))
                        .truncate_end(width)
                }
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
    let mut lines = vec![
        {
            let mut title = Line::blank()
                .with(Span::new(
                    format!("{} ", variant.icon),
                    Style::color(variant.color),
                ))
                .with(Span::new(&row.title, Style::color(variant.color).bold()));
            if row.site_count > 1 {
                title.push(Span::dim(format!(" ×{}", row.site_count)));
            }
            title.truncate_end(width)
        },
    ];

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
        body.push(copy_feedback_line(feedback));
    }

    lines.extend(
        body.into_iter()
            .map(|line| line.indented(DETAIL_INDENT_COLUMNS)),
    );
    lines
}

fn copy_feedback_line(feedback: CopyFeedback) -> Line {
    match feedback {
        CopyFeedback::Copied => Line::text("✓ Copied issue context", Style::color(Color::Green)),
        CopyFeedback::Failed => Line::text(
            "Couldn't copy issue context. Press Enter to try again.",
            Style::color(Color::Yellow),
        ),
    }
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
    let gutter = frame
        .lines
        .iter()
        .map(|line| line.number.to_string().len())
        .max()
        .unwrap_or(1);
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

// ------------------------------------------------------------------ status bar

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
    let unread = state
        .rows
        .iter()
        .enumerate()
        .filter(|(index, _)| !(state.read)(*index))
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
        ));
    let position_span = Span::dim(format!("  ·  issue {position}/{}", state.rows.len()));

    let key_hints = |line: Line| match state.copy_feedback {
        Some(feedback) if layout.arrangement == Arrangement::Compact => {
            line.extend(compact_copy_hint(feedback))
        }
        _ => line
            .with(Span::dim("↑/↓ move · "))
            .with(Span::new("enter", Style::color(Color::Cyan)))
            .with(Span::dim(" copy context")),
    };

    if layout.arrangement == Arrangement::Compact {
        let mut line = counts.with(position_span).with(Span::dim("  ·  "));
        line = key_hints(line);
        return vec![
            line.with(Span::dim(format!(" · {}", state.exit_hint)))
                .truncate_end(layout.width),
        ];
    }

    let mut second = Line::of(Span::new(
        format!("{} unread  ·  ", pluralize(unread, "issue")),
        if unread > 0 {
            Style::color(Color::Cyan)
        } else {
            Style::DIM
        },
    ));
    second = key_hints(second);
    vec![
        Line::blank(),
        counts.with(position_span).truncate_end(layout.width),
        second
            .with(Span::dim(format!(" · {}", state.exit_hint)))
            .truncate_end(layout.width),
    ]
}

fn compact_copy_hint(feedback: CopyFeedback) -> Line {
    match feedback {
        CopyFeedback::Copied => Line::text("✓ Copied issue context", Style::color(Color::Green)),
        CopyFeedback::Failed => Line::text("Copy failed · enter retry", Style::color(Color::Yellow)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_doctor::{AuditCategoryName, ScoreDimensions, ScoreLabel, Severity};

    fn score(value: u8, label: ScoreLabel) -> AuditScore {
        AuditScore {
            model: "test".to_owned(),
            value,
            label,
            authoritative: true,
            dimensions: ScoreDimensions {
                security: 100,
                reliability: 100,
                maintainability: 100,
                performance: 100,
                dependencies: 100,
            },
            worst_tier: None,
            applied_ceiling: None,
            projected_after_top_three: None,
            projected_rule_ids: Vec::new(),
            withheld_rule_ids: Vec::new(),
        }
    }

    fn visible(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|line| line.render(false, false)).collect()
    }

    #[test]
    fn the_score_block_is_four_rows_of_face_score_bar_and_branding() {
        let rendered = visible(&score_header(
            ScoreVariant::Landing,
            Some(&score(90, ScoreLabel::Great)),
            "delgres-ceramique",
            2,
            60,
            90,
            None,
        ));
        assert_eq!(rendered.len(), 4);
        assert!(rendered[0].starts_with("  ┌─────┐  90 / 100 Great  ·  delgres-ceramique"));
        assert!(rendered[1].starts_with("  │ ◠ ◠ │  █"));
        assert!(rendered[1].ends_with('░'));
        assert!(rendered[2].contains("Rust Doctor (https://rust-doctor.vercel.app)"));
        assert_eq!(rendered[3], "  └─────┘  ");
    }

    #[test]
    fn a_projection_paints_its_gain_between_the_filled_and_the_empty_bar() {
        let mut projected = score(60, ScoreLabel::NeedsWork);
        projected.projected_after_top_three = Some(90);
        let rendered = visible(&score_header(
            ScoreVariant::Landing,
            Some(&projected),
            "workspace",
            4,
            60,
            60,
            Some(90),
        ));
        assert_eq!(rendered.len(), 5);
        assert_eq!(rendered[1].matches('▓').count(), 14);
        assert!(rendered[4].contains("Potential score 90 after priority fixes +30"));
    }

    #[test]
    fn a_terminal_too_narrow_for_the_face_keeps_the_score_and_the_branding() {
        let rendered = visible(&score_header(
            ScoreVariant::Viewer,
            Some(&score(40, ScoreLabel::Critical)),
            "workspace",
            1,
            20,
            40,
            None,
        ));
        assert_eq!(rendered.len(), 2);
        assert!(rendered[0].starts_with("40 / 100 Critic"));
        assert!(!rendered[1].contains('┌'));
    }

    #[test]
    fn the_menu_points_at_the_selection_and_gaps_every_entry() {
        let actions = [
            Action::new("Review 2 issues"),
            Action::described(
                "Add to GitHub Actions (Recommended)",
                vec![Line::text("why", Style::DIM)],
            ),
            Action::new("Hand off to an agent"),
        ];
        assert_eq!(
            visible(&action_menu(&actions, 1)),
            [
                "",
                "› Review 2 issues",
                "",
                "❯ Add to GitHub Actions (Recommended)",
                "why",
                "",
                "› Hand off to an agent",
            ]
        );
    }

    #[test]
    fn the_landing_reports_a_clean_workspace_before_its_menu() {
        let notices = LandingNotices {
            issue_count: 0,
            incomplete: None,
            empty_state: None,
        };
        let rendered = visible(&landing(Vec::new(), &notices, &[], 0, true));
        assert_eq!(
            rendered,
            ["", "✔ No issues found. Nice work.", "", "", "q quit"]
        );
    }

    #[test]
    fn the_status_bar_counts_sites_locates_the_issue_and_tracks_what_is_unread() {
        let rows = [
            sample_row("clippy::a", Severity::Error, 3),
            sample_row("clippy::b", Severity::Warning, 2),
        ];
        let entries = super::super::model::build_entries(&rows);
        let read = |index: usize| index == 0;
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
        let layout = super::super::model::resolve_layout(160, 48, entries.len());
        let rendered = visible(&status_bar(&state, layout));
        assert_eq!(rendered[0], "");
        assert_eq!(rendered[1], "5 findings  3 errors  2 warnings  ·  issue 1/2");
        assert_eq!(
            rendered[2],
            "1 issue unread  ·  ↑/↓ move · enter copy context · esc back · q to quit"
        );
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
