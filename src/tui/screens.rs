//! The screens of the interactive report, each one a list of rendered rows.
//!
//! Every function here transposes one Ink component. The two that carry weight
//! have their own file: [`menu`] is the shape the landing, the agent handoff
//! and the two GitHub Actions screens all take, and [`viewer`] is the split
//! review. What stays here is the score block, which both of them show.
//!
//! Margins are the same rows and paddings the same columns as the reference,
//! so a frame lands on the geometry it renders.

use rust_doctor::AuditScore;
use rust_doctor::score_block;

use super::model::{HORIZONTAL_PADDING_COLUMNS, pluralize, score_color};
use super::text::{Line, Span, Style};

mod menu;
mod viewer;

pub use menu::{
    Action, GITHUB_ACTIONS_SETUP_URL, HINT_MENU, HINT_MENU_BACK, HINT_MENU_SKIP, HINT_QUIT,
    LandingNotices, Menu, MenuInput, Notice, ci_justification, ci_recommendation_title,
    ci_setup_title, handoff_title, input, landing_title, menu,
};
pub use viewer::{CopyFeedback, ViewerState, viewer};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScoreVariant {
    Landing,
    Viewer,
}

/// The two-column score block: face on the left, value, bar and branding on the
/// right. `displayed` is the counted-up value of the current animation frame,
/// `displayed_projection` the head of the projected gain.
///
/// `width` is the room the block may occupy, its guard column included. The
/// geometry and the tables come from [`score_block`], shared with the linear
/// report so a score reads the same in both.
pub fn score_header(
    variant: ScoreVariant,
    score: Option<&AuditScore>,
    project_name: &str,
    finding_count: usize,
    width: usize,
    displayed: u8,
    displayed_projection: Option<u8>,
) -> Vec<Line> {
    let box_width = width.saturating_sub(score_block::RIGHT_EDGE_SAFETY_COLUMNS);
    let Some(score) = score else {
        return [
            branding_line(),
            Line::text(
                format!("{} · {project_name}", pluralize(finding_count, "finding")),
                Style::DIM,
            ),
        ]
        .into_iter()
        .map(|line| {
            line.indented(HORIZONTAL_PADDING_COLUMNS)
                .truncate_end(box_width)
        })
        .collect();
    };

    let color = score_color(score.label);
    let summary = Line::blank()
        .with(Span::new(displayed.to_string(), Style::color(color).bold()))
        .with(Span::dim(format!(" / {} ", score_block::PERFECT_SCORE)))
        .with(Span::new(
            score_block::label_text(score.label, score.authoritative),
            Style::color(color),
        ))
        .with(Span::dim(format!("  ·  {project_name}")));

    let Some(right_width) = score_block::right_column_width(width) else {
        return vec![
            summary.truncate_end(box_width),
            branding_line().truncate_end(box_width),
        ];
    };
    let bar_width = right_width.min(score_block::BAR_MAX_WIDTH_CHARS);

    let filled = score_block::bar_fill(displayed, bar_width);
    let projected = displayed_projection.map_or(filled, |projection| {
        score_block::bar_fill(projection, bar_width)
    });
    let gain = projected.saturating_sub(filled);
    let empty = bar_width.saturating_sub(filled).saturating_sub(gain);
    let bar = Line::blank()
        .with(Span::new("█".repeat(filled), Style::color(color)))
        .with(Span::new("▓".repeat(gain), Style::color(color).dim()))
        .with(Span::dim("░".repeat(empty)));

    let right = [
        summary.truncate_end(right_width),
        bar,
        branding_line().truncate_end(right_width),
        Line::blank(),
    ];

    let mut lines: Vec<Line> = score_block::face_rows(score.label)
        .into_iter()
        .zip(right)
        .map(|(face, right)| {
            Line::of(Span::new(face, Style::color(color)))
                .indented(score_block::INDENT_COLUMNS)
                .with(Span::plain(" ".repeat(score_block::GAP_COLUMNS)))
                .extend(right)
        })
        .collect();

    if variant == ScoreVariant::Landing
        && let Some(projection) = score
            .projected_after_top_three
            .filter(|projection| *projection > score.value)
    {
        let projection_color = score_color(score_block::label_for(projection));
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
                .truncate_end(box_width),
        );
    }
    lines
}

fn branding_line() -> Line {
    Line::blank()
        .with(Span::linked(
            score_block::BRANDING_NAME,
            Style::PLAIN,
            score_block::BRANDING_URL,
        ))
        .with(Span::linked(
            format!(" ({})", score_block::BRANDING_URL),
            Style::DIM,
            score_block::BRANDING_URL,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_doctor::terminal_text::display_width;
    use rust_doctor::{ScoreDimensions, ScoreLabel};

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
        assert!(rendered[2].contains("Rust Doctor (https://rust-doctor.com)"));
        assert_eq!(rendered[3], "  └─────┘  ");
    }

    /// The block keeps the guard column [`score_block`] budgets for, at every
    /// width it accepts.
    #[test]
    fn no_row_of_the_block_reaches_the_last_column_it_was_given() {
        for width in [22usize, 40, 61, 80, 120, 200] {
            for label in [ScoreLabel::Great, ScoreLabel::Critical] {
                let mut projected = score(60, label);
                projected.projected_after_top_three = Some(90);
                for rendered in visible(&score_header(
                    ScoreVariant::Landing,
                    Some(&projected),
                    "workspace",
                    4,
                    width,
                    60,
                    Some(90),
                )) {
                    assert!(
                        display_width(&rendered) <= width - score_block::RIGHT_EDGE_SAFETY_COLUMNS,
                        "width {width} exceeded by {rendered:?}"
                    );
                }
            }
        }
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
        let bar_width = score_block::bar_width(60).unwrap();
        assert_eq!(
            rendered[1].matches('▓').count(),
            score_block::bar_fill(90, bar_width) - score_block::bar_fill(60, bar_width)
        );
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
}
