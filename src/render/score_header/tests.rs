use super::*;
use crate::terminal_text::{display_width, sanitize};
use crate::{ScoreDimensions, ScoreLabel};
use std::path::Path;

fn score(value: u8, label: ScoreLabel, authoritative: bool) -> AuditScore {
    AuditScore {
        model: "test".to_owned(),
        value,
        label,
        authoritative,
        reasons: if authoritative {
            Vec::new()
        } else {
            vec![crate::ScoreReason::ScanIncomplete]
        },
        dimensions: ScoreDimensions {
            security: value,
            reliability: value,
            maintainability: value,
            performance: value,
            dependencies: value,
        },
        worst_tier: None,
        applied_ceiling: None,
        ceilings: Vec::new(),
        projected_after_top_three: None,
        projected_rule_ids: Vec::new(),
        withheld_rule_ids: Vec::new(),
    }
}

fn options(width: usize, color: bool) -> TerminalOptions<'static> {
    TerminalOptions {
        workspace_root: Path::new("."),
        elapsed: Duration::from_millis(0),
        verbose: false,
        width,
        color,
        animate: false,
        requested_scope: None,
    }
}

fn rendered(value: u8, label: ScoreLabel, width: usize, color: bool) -> String {
    render_with(&score(value, label, true), options(width, color))
}

fn render_with(score: &AuditScore, options: TerminalOptions<'_>) -> String {
    let mut output = Vec::new();
    render(&mut output, score, options, Cadence::INSTANT).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn the_padding_this_file_writes_is_the_padding_the_geometry_counts() {
    assert_eq!(display_width(INDENT), score_block::INDENT_COLUMNS);
    assert_eq!(display_width(GAP), score_block::GAP_COLUMNS);
}

#[test]
fn the_width_the_report_guarantees_carries_the_block_whole() {
    let full = format!(
        "{} ({})",
        score_block::BRANDING_NAME,
        score_block::BRANDING_URL
    );
    let output = rendered(100, ScoreLabel::Great, super::super::MIN_WIDTH, false);
    assert!(output.contains("100 / 100 Great"), "{output}");
    assert!(output.contains(score_block::BRANDING_NAME), "{output}");
    assert!(!output.contains('…') && !output.contains("https://"), "{output}");
    let output = rendered(100, ScoreLabel::Great, 80, false);
    assert!(output.contains(&full), "{output}");
}

#[test]
fn the_block_composes_a_face_a_score_a_bar_and_branding() {
    let output = rendered(100, ScoreLabel::Great, 100, false);
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), BLOCK_ROWS);
    assert!(lines[0].contains("┌─────┐"));
    assert!(lines[0].contains("100 / 100 Great"));
    assert!(lines[1].contains("◠ ◠"));
    assert!(lines[1].contains('█'));
    assert!(lines[2].contains("Rust Doctor (https://rust-doctor.com)"));
    assert!(lines[3].contains("└─────┘"));
}

#[test]
fn the_face_follows_the_label() {
    assert!(rendered(100, ScoreLabel::Great, 100, false).contains("◠ ◠"));
    assert!(rendered(60, ScoreLabel::NeedsWork, 100, false).contains("• •"));
    assert!(rendered(10, ScoreLabel::Critical, 100, false).contains("x x"));
}

#[test]
fn only_a_perfect_colored_score_paints_the_bar_in_truecolor() {
    assert!(rendered(100, ScoreLabel::Great, 100, true).contains("\u{1b}[38;2;"));
    assert!(!rendered(100, ScoreLabel::Great, 100, false).contains('\u{1b}'));
    assert!(!rendered(99, ScoreLabel::Great, 100, true).contains("\u{1b}[38;2;"));
}

#[test]
fn the_frozen_gradient_runs_from_cyan_to_orange() {
    let colors = truecolors(&rendered(100, ScoreLabel::Great, 100, true));
    assert!(
        colors.len() >= 40,
        "the bar should be painted per character"
    );
    let (first_red, _, first_blue) = colors[0];
    let (last_red, _, last_blue) = colors[colors.len() - 1];
    assert!(first_blue > first_red, "the gradient should start cool");
    assert!(last_red > last_blue, "the gradient should end warm");
}

fn truecolors(output: &str) -> Vec<(u8, u8, u8)> {
    output
        .split("\u{1b}[38;2;")
        .skip(1)
        .filter_map(|chunk| {
            let channels: Vec<u8> = chunk
                .split_once('m')?
                .0
                .split(';')
                .filter_map(|value| value.parse().ok())
                .collect();
            match channels[..] {
                [red, green, blue] => Some((red, green, blue)),
                _ => None,
            }
        })
        .collect()
}

#[test]
fn a_score_that_is_not_authoritative_names_itself_partial_and_drops_its_band() {
    for label in [
        ScoreLabel::Great,
        ScoreLabel::NeedsWork,
        ScoreLabel::Critical,
    ] {
        let partial = score(100, label, false);
        let plain = render_with(&partial, options(100, false));
        assert!(plain.contains("100 / 100 Core partial"), "{plain}");
        assert!(!plain.contains(label.as_str()));

        let colored = render_with(&partial, options(100, true));
        assert!(colored.contains("\u{1b}[33m"), "{colored:?}");
        assert!(
            !colored.contains("\u{1b}[38;2;"),
            "a partial score never earns the perfect gradient"
        );
    }
}

#[test]
fn every_line_fits_the_terminal_in_both_color_modes() {
    for width in [super::super::MIN_WIDTH, 40, 80, 100, 140] {
        for color in [false, true] {
            for (value, label) in [
                (100, ScoreLabel::Great),
                (60, ScoreLabel::NeedsWork),
                (10, ScoreLabel::Critical),
            ] {
                let output = rendered(value, label, width, color);
                for line in output.lines() {
                    let visible = sanitize(line);
                    assert!(
                        display_width(&visible) <= width - score_block::RIGHT_EDGE_SAFETY_COLUMNS,
                        "width {width} exceeded by {visible:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn every_row_measures_as_many_columns_as_it_has_characters() {
    for width in [40usize, 80, 100] {
        for label in [
            ScoreLabel::Great,
            ScoreLabel::NeedsWork,
            ScoreLabel::Critical,
        ] {
            for value in [0u8, 1, 37, 99, 100] {
                for line in rendered(value, label, width, false).lines() {
                    assert_eq!(
                        display_width(line),
                        line.chars().count(),
                        "row {line:?} is not one column per character"
                    );
                }
            }
        }
    }
}

#[test]
fn a_cut_row_keeps_the_dimming_on_the_piece_it_cut() {
    let row = Row::of([
        ("name".to_owned(), Ink::Plain),
        (" (a long dimmed tail)".to_owned(), Ink::Dim),
    ])
    .truncated(10);
    let painted = row.painted("32", 0);
    assert!(
        painted.contains(&format!("name\u{1b}[{DIM}m")),
        "the name should stay plain and the cut tail dim: {painted:?}"
    );
    assert!(!painted.contains("tail"), "the tail should have been cut: {painted:?}");
}

#[test]
fn the_bar_is_always_exactly_as_wide_as_the_room_it_was_given() {
    assert_eq!(bar(100, 10), "██████████");
    assert_eq!(bar(0, 10), "░░░░░░░░░░");
    for width in [10usize, 27, 50] {
        for value in [0u8, 1, 37, 99, 100] {
            assert_eq!(display_width(&bar(value, width)), width);
        }
    }
}

fn animated(value: u8, label: ScoreLabel) -> String {
    let mut animated = options(100, true);
    animated.animate = true;
    render_with(&score(value, label, true), animated)
}

const REWIND: &str = "\u{1b}[4A\r";

fn last_frame(output: &str) -> &str {
    output.rsplit(REWIND).next().unwrap_or(output)
}

#[test]
fn an_animated_render_rewinds_exactly_one_block_per_frame() {
    assert_eq!(REWIND, format!("\u{1b}[{BLOCK_ROWS}A\r"));
    let output = animated(60, ScoreLabel::NeedsWork);
    assert_eq!(
        output.matches(REWIND).count() as u32,
        score_block::COUNT_UP_FRAME_COUNT
    );
    assert!(sanitize(&output).contains("60 / 100 Needs work"));
}

#[test]
fn an_animated_perfect_score_scrolls_then_freezes_on_one_last_frame() {
    let output = animated(100, ScoreLabel::Great);
    assert_eq!(
        output.matches(REWIND).count() as u32,
        score_block::COUNT_UP_FRAME_COUNT + RAINBOW_FRAME_COUNT + 1
    );

    let last = last_frame(&output);
    assert!(
        last.contains("\u{1b}[32m┌─────┐"),
        "the frozen frame paints the face with the label style: {last:?}"
    );
    assert!(
        !truecolors(last).is_empty(),
        "the frozen frame keeps the gradient on its bar"
    );
    assert!(sanitize(last).contains("100 / 100 Great"));
}

#[test]
fn the_frame_the_animation_freezes_on_is_the_frame_a_static_render_draws() {
    for (value, label) in [(100, ScoreLabel::Great), (60, ScoreLabel::NeedsWork)] {
        let animated = animated(value, label);
        assert_eq!(last_frame(&animated), rendered(value, label, 100, true));
    }
}

#[test]
fn the_two_reports_size_and_fill_their_bar_from_the_same_model() {
    for width in [
        super::super::MIN_WIDTH,
        score_block::MIN_BLOCK_COLUMNS,
        61,
        80,
        200,
    ] {
        let layout = Layout::new(&score(60, ScoreLabel::NeedsWork, true), width);
        assert_eq!(Some(layout.bar_width), score_block::bar_width(width));
        for value in [0u8, 37, 73, 100] {
            let bar = bar(value, layout.bar_width);
            assert_eq!(
                bar.matches('█').count(),
                score_block::bar_fill(value, layout.bar_width),
                "the bar of {value} at {width} columns"
            );
            assert_eq!(display_width(&bar), layout.bar_width);
        }
    }
}
