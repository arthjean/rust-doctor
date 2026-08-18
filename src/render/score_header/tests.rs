//! Tests of the score block the linear report draws.
//!
//! They live in their own file for the reason the block itself exists: the
//! crate passes its own `oversized_unit` rule, and a module that carried its
//! renderer and its tests in one file would not.

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
        dimensions: ScoreDimensions {
            security: value,
            reliability: value,
            maintainability: value,
            performance: value,
            dependencies: value,
        },
        worst_tier: None,
        applied_ceiling: None,
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
    }
}

fn rendered(value: u8, label: ScoreLabel, width: usize, color: bool) -> String {
    render_with(&score(value, label, true), true, options(width, color))
}

fn render_with(score: &AuditScore, allow_links: bool, options: TerminalOptions<'_>) -> String {
    let mut output = Vec::new();
    render(&mut output, score, allow_links, options, Cadence::INSTANT).unwrap();
    String::from_utf8(output).unwrap()
}

/// The literals this file formats with must match the columns
/// [`score_block`] budgets for them, since the geometry is computed there
/// and painted here.
#[test]
fn the_padding_this_file_writes_is_the_padding_the_geometry_counts() {
    assert_eq!(display_width(INDENT), score_block::INDENT_COLUMNS);
    assert_eq!(display_width(GAP), score_block::GAP_COLUMNS);
}

/// The width the linear report guarantees carries the block whole. The
/// assertion beside `MIN_WIDTH` proves at compile time that the bar fits; this
/// proves nothing else is cut at that width either, which is what lets the
/// block carry no narrow-terminal branch.
#[test]
fn the_width_the_report_guarantees_carries_the_block_whole() {
    let output = rendered(100, ScoreLabel::Great, super::super::MIN_WIDTH, false);
    assert!(output.contains("100 / 100 Great"), "{output}");
    assert!(
        output.contains(&format!(
            "{} ({})",
            score_block::BRANDING_NAME,
            score_block::BRANDING_URL
        )),
        "{output}"
    );
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

/// The frozen gradient must start from cyan on the left and end on orange
/// on the right, like the final frame of the reference. A null hue shift
/// would reverse the direction.
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
fn a_report_that_cannot_show_links_drops_the_url() {
    let output = render_with(
        &score(100, ScoreLabel::Great, true),
        false,
        options(100, false),
    );
    assert!(!output.contains("https://"));
    assert!(output.contains("Rust Doctor"));
    assert_eq!(output.lines().count(), BLOCK_ROWS);
}

/// A score whose core passes could not be judged names itself rather than
/// publishing a band, and carries the warning style whatever its label would
/// have been.
#[test]
fn a_score_that_is_not_authoritative_names_itself_partial_and_drops_its_band() {
    for label in [
        ScoreLabel::Great,
        ScoreLabel::NeedsWork,
        ScoreLabel::Critical,
    ] {
        let partial = score(100, label, false);
        let plain = render_with(&partial, true, options(100, false));
        assert!(plain.contains("100 / 100 Core partial"), "{plain}");
        assert!(!plain.contains(label.as_str()));

        let colored = render_with(&partial, true, options(100, true));
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

/// The gradient advances one step per character, so a block whose characters
/// were not all one column wide would scroll at a different speed than it
/// measures. Every row it draws has to keep the two counts equal.
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

/// A right column too narrow for the branding cuts inside the URL and keeps the
/// name plain and the URL dimmed. Cutting the rendered line instead lost the
/// dimming altogether, on exactly the terminals the comment claimed to serve.
#[test]
fn a_cut_row_keeps_the_dimming_on_the_piece_it_cut() {
    let output = rendered(100, ScoreLabel::Great, 40, true);
    let branding = output.lines().nth(2).unwrap();
    assert!(
        branding.contains(&format!("{}\u{1b}[{DIM}m", score_block::BRANDING_NAME)),
        "the name should stay plain and the cut URL dim: {branding:?}"
    );
    assert!(
        !branding.contains(score_block::BRANDING_URL),
        "the URL should have been cut at this width: {branding:?}"
    );
}

/// The fill rule and the easing are proved in [`score_block`]; what this
/// file owns is that the bar it paints is exactly `bar_width` cells wide.
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
    render_with(&score(value, label, true), true, animated)
}

const REWIND: &str = "\u{1b}[4A\r";

/// The frame an animated render left on the screen: what follows the last
/// rewind it wrote.
fn last_frame(output: &str) -> &str {
    output.rsplit(REWIND).next().unwrap_or(output)
}

/// An animated output must rewind exactly the height of the block between
/// two frames, otherwise it stacks blocks instead of replacing them.
#[test]
fn an_animated_render_rewinds_exactly_one_block_per_frame() {
    assert_eq!(REWIND, format!("\u{1b}[{BLOCK_ROWS}A\r"));
    let output = animated(60, ScoreLabel::NeedsWork);
    assert_eq!(
        output.matches(REWIND).count() as u32,
        score_block::COUNT_UP_FRAME_COUNT
    );
    // The value and the label are painted separately, so the line is only
    // readable once the sequences are stripped.
    assert!(sanitize(&output).contains("60 / 100 Needs work"));
}

/// A perfect score counts up scrolling the gradient over the whole block, keeps
/// scrolling it for [`RAINBOW_FRAME_COUNT`] frames, then freezes on one last
/// frame whose face is plain and whose bar keeps the gradient.
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

/// The frame the animation freezes on is the frame a non-animated render draws
/// directly. Two paths computing it apart is how they drifted before.
#[test]
fn the_frame_the_animation_freezes_on_is_the_frame_a_static_render_draws() {
    for (value, label) in [(100, ScoreLabel::Great), (60, ScoreLabel::NeedsWork)] {
        let animated = animated(value, label);
        assert_eq!(last_frame(&animated), rendered(value, label, 100, true));
    }
}
