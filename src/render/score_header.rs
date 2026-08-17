//! Two-column score block: face on the left, value, bar and branding on the
//! right.
//!
//! Transposition of React Doctor's `render-score-header.ts`: same faces, same
//! OKLCH conversion, same animation cadences. The count is eased over
//! [`score_block::COUNT_UP_FRAME_COUNT`] frames, then a perfect score scrolls
//! the rainbow over 16 frames before freezing on a coloured bar and a plain
//! face.
//!
//! Everything the interactive report must agree with lives in
//! [`crate::score_block`]: the faces, the label, the fill rule, the geometry
//! and the count-up. What stays here is what only this report does, the OKLCH
//! rainbow and the in-place rewind.
//!
//! The animation only exists in a real coloured terminal outside CI. Any other
//! output receives the final frame directly, which keeps captured renders
//! deterministic.

use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::thread::sleep;
use std::time::Duration;

use super::{RenderError, Style, TerminalOptions, write_styled};
use crate::ScoreLabel;
use crate::score_block::{self, PERFECT_SCORE};
use crate::terminal_text::truncate;

const INDENT: &str = "  ";
const GAP: &str = "  ";

const RAINBOW_GRADIENT_WIDTH: f64 = 80.0;
const RAINBOW_OKLCH_LIGHTNESS: f64 = 0.638;
const RAINBOW_OKLCH_CHROMA: f64 = 0.129;
const RAINBOW_HUE_SHIFT_PER_FRAME: f64 = 9.0;

const RAINBOW_FRAME_COUNT: u32 = 16;
const RAINBOW_FRAME_DELAY: Duration = Duration::from_millis(50);

const DIM: &str = "2";
const RESET: &str = "\u{1b}[0m";

/// Clears the end of the line so that a frame shorter than the previous one
/// leaves no residue. The reference does without it; we keep it because the
/// score line grows from `0 / 100` to `100 / 100` during the count.
const CLEAR_TO_END: &str = "\u{1b}[K";

const fn style_code(label: ScoreLabel, authoritative: bool) -> &'static str {
    if !authoritative {
        return "33";
    }
    match label {
        ScoreLabel::Great => "32",
        ScoreLabel::NeedsWork => "33",
        ScoreLabel::Critical => "31",
    }
}

const fn style_for(label: ScoreLabel, authoritative: bool) -> Style {
    if !authoritative {
        return Style::Warning;
    }
    match label {
        ScoreLabel::Great => Style::Success,
        ScoreLabel::NeedsWork => Style::Warning,
        ScoreLabel::Critical => Style::Muted,
    }
}

fn paint(content: &str, code: &str) -> String {
    format!("\u{1b}[{code}m{content}{RESET}")
}

fn encode_srgb(value: f64) -> f64 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn clamp_channel(value: f64) -> u8 {
    let scaled = (value * 255.0).round();
    if scaled <= 0.0 {
        0
    } else if scaled >= 255.0 {
        255
    } else {
        scaled as u8
    }
}

/// Direct port of the reference's OKLCH conversion. The coefficients are those
/// of the OKLab to linear sRGB matrix.
fn oklch_to_rgb(lightness: f64, chroma: f64, hue: f64) -> (u8, u8, u8) {
    let radians = hue.to_radians();
    let lab_a = chroma * radians.cos();
    let lab_b = chroma * radians.sin();
    let long = (lightness + 0.396_337_777_4 * lab_a + 0.215_803_757_3 * lab_b).powi(3);
    let medium = (lightness - 0.105_561_345_8 * lab_a - 0.063_854_172_8 * lab_b).powi(3);
    let short = (lightness - 0.089_484_177_5 * lab_a - 1.291_485_548 * lab_b).powi(3);

    (
        clamp_channel(encode_srgb(
            4.076_741_662_1 * long - 3.307_711_591_3 * medium + 0.230_969_929_2 * short,
        )),
        clamp_channel(encode_srgb(
            -1.268_438_004_6 * long + 2.609_757_401_1 * medium - 0.341_319_396_5 * short,
        )),
        clamp_channel(encode_srgb(
            -0.004_196_086_3 * long - 0.703_418_614_7 * medium + 1.707_614_701 * short,
        )),
    )
}

/// Colours character by character in truecolor. `offset` shifts the gradient so
/// that it seems to continue from the left edge of the line, and `frame` makes
/// it rotate: that shift is what produces the scrolling.
fn rainbow(content: &str, frame: u32, offset: usize) -> String {
    let mut colored = String::with_capacity(content.len() * 20);
    for (index, character) in content.chars().enumerate() {
        if character == ' ' {
            colored.push(character);
            continue;
        }
        let position = (index + offset) as f64;
        let hue = (position / RAINBOW_GRADIENT_WIDTH)
            .mul_add(360.0, f64::from(frame) * RAINBOW_HUE_SHIFT_PER_FRAME)
            % 360.0;
        let (red, green, blue) = oklch_to_rgb(RAINBOW_OKLCH_LIGHTNESS, RAINBOW_OKLCH_CHROMA, hue);
        let _ = write!(colored, "\u{1b}[38;2;{red};{green};{blue}m{character}\u{1b}[39m");
    }
    colored
}

fn bar(value: u8, width: usize) -> String {
    let filled = score_block::bar_fill(value, width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// Frozen geometry and contents of the block, computed once per render so that
/// every frame shares exactly the same layout.
struct Layout {
    faces: [String; 4],
    offset: usize,
    available: usize,
    bar_width: usize,
    label_text: &'static str,
    allow_links: bool,
}

impl Layout {
    fn new(
        label: ScoreLabel,
        authoritative: bool,
        allow_links: bool,
        width: usize,
    ) -> Option<Self> {
        let available = score_block::right_column_width(width)?;
        Some(Self {
            faces: score_block::face_rows(label),
            offset: score_block::FACE_OFFSET_COLUMNS,
            available,
            bar_width: available.min(score_block::BAR_MAX_WIDTH_CHARS),
            label_text: score_block::label_text(label, authoritative),
            allow_links,
        })
    }

    fn raw_score(&self, value: u8) -> String {
        truncate(
            &format!("{value} / {PERFECT_SCORE} {}", self.label_text),
            self.available,
        )
    }

    fn raw_branding(&self) -> String {
        if self.allow_links {
            truncate(
                &format!(
                    "{} ({})",
                    score_block::BRANDING_NAME,
                    score_block::BRANDING_URL
                ),
                self.available,
            )
        } else {
            truncate(score_block::BRANDING_NAME, self.available)
        }
    }

    fn raw_right(&self, value: u8) -> [String; 4] {
        [
            self.raw_score(value),
            bar(value, self.bar_width),
            self.raw_branding(),
            String::new(),
        ]
    }

    fn compose(&self, index: usize, right: &str) -> String {
        let separator = if right.is_empty() { "" } else { GAP };
        format!("{INDENT}{}{separator}{right}", self.faces[index])
    }

    /// Fully rainbow frame, face included. This is the state during the
    /// animation of a perfect score.
    fn rainbow_frame(&self, value: u8, frame: u32) -> String {
        let mut output = String::new();
        for (index, right) in self.raw_right(value).iter().enumerate() {
            let line = self.compose(index, right);
            let _ = write!(output, "{}{CLEAR_TO_END}\n\r", rainbow(&line, frame, 0));
        }
        output
    }

    /// Final frame of a perfect score: plain face, frozen rainbow bar.
    fn perfect_frame(&self, value: u8, code: &str, frame: u32) -> String {
        let mut output = String::new();
        for (index, right) in self.raw_right(value).iter().enumerate() {
            let painted_face = paint(&self.faces[index], code);
            let separator = if right.is_empty() { "" } else { GAP };
            let painted_right = if index == 1 {
                rainbow(right, frame, self.offset)
            } else {
                self.paint_right(index, right, code)
            };
            let _ = write!(
                output,
                "{INDENT}{painted_face}{separator}{painted_right}{CLEAR_TO_END}\n\r"
            );
        }
        output
    }

    /// Coloured frame of an imperfect score: everything follows the label
    /// style, except `/ 100` and the URL which stay dimmed.
    fn styled_frame(&self, value: u8, code: &str) -> String {
        let mut output = String::new();
        for (index, right) in self.raw_right(value).iter().enumerate() {
            let painted_face = paint(&self.faces[index], code);
            let separator = if right.is_empty() { "" } else { GAP };
            let painted_right = self.paint_right(index, right, code);
            let _ = write!(
                output,
                "{INDENT}{painted_face}{separator}{painted_right}{CLEAR_TO_END}\n\r"
            );
        }
        output
    }

    /// Colours a line of the right column. The score line separates the value
    /// from the denominator, and the branding separates the name from the URL,
    /// like the reference.
    fn paint_right(&self, index: usize, right: &str, code: &str) -> String {
        if right.is_empty() {
            return String::new();
        }
        match index {
            0 => {
                let denominator = format!("/ {PERFECT_SCORE}");
                match right.split_once(&denominator) {
                    Some((value, label)) => format!(
                        "{}{}{}",
                        paint(value.trim_end(), code),
                        paint(&format!(" {denominator}"), DIM),
                        paint(label, code)
                    ),
                    None => paint(right, code),
                }
            }
            // The name stays plain and the URL dims, wherever a narrow terminal
            // cut the line.
            2 => match right.split_once(" (") {
                Some((name, url)) => format!("{name}{}", paint(&format!(" ({url}"), DIM)),
                None => right.to_owned(),
            },
            _ => paint(right, code),
        }
    }
}

/// Writes the two-column block. Returns `false` when the terminal is too narrow
/// to carry it, in which case the caller keeps the single line.
///
/// `allow_links` follows the rule of the Share, Docs and GitHub lines: a failed
/// report renders no URL, so the branding line loses its own.
pub(super) fn render<W: Write>(
    writer: &mut W,
    value: u8,
    label: ScoreLabel,
    authoritative: bool,
    allow_links: bool,
    options: TerminalOptions<'_>,
) -> Result<bool, RenderError> {
    let Some(layout) = Layout::new(label, authoritative, allow_links, options.width) else {
        return Ok(false);
    };
    let code = style_code(label, authoritative);
    let is_perfect = authoritative && value == PERFECT_SCORE;

    if options.animate && options.color {
        animate(writer, &layout, value, code, is_perfect)?;
        return Ok(true);
    }

    if !options.color {
        let style = style_for(label, authoritative);
        for (index, right) in layout.raw_right(value).iter().enumerate() {
            write_styled(writer, &layout.compose(index, right), false, style)?;
        }
        return Ok(true);
    }

    let frame = if is_perfect {
        layout.perfect_frame(value, code, RAINBOW_FRAME_COUNT)
    } else {
        layout.styled_frame(value, code)
    };
    write_frame(writer, &frame, false)
}

/// A frame ends with `\n\r`; it is rewritten in place by going back up its four
/// lines.
fn write_frame<W: Write>(writer: &mut W, frame: &str, rewind: bool) -> Result<bool, RenderError> {
    let cursor = if rewind { "\u{1b}[4A\r" } else { "" };
    write!(writer, "{cursor}{frame}").map_err(RenderError::Write)?;
    writer.flush().map_err(RenderError::Write)?;
    Ok(true)
}

fn animate<W: Write>(
    writer: &mut W,
    layout: &Layout,
    value: u8,
    code: &str,
    is_perfect: bool,
) -> Result<(), RenderError> {
    for frame in 0..=score_block::COUNT_UP_FRAME_COUNT {
        let counted = score_block::eased(0, value, frame, score_block::COUNT_UP_FRAME_COUNT);
        let painted = if is_perfect {
            layout.rainbow_frame(counted, frame)
        } else {
            layout.styled_frame(counted, code)
        };
        write_frame(writer, &painted, frame > 0)?;
        if frame < score_block::COUNT_UP_FRAME_COUNT {
            sleep(score_block::COUNT_UP_FRAME_DELAY);
        }
    }

    if !is_perfect {
        return Ok(());
    }

    for frame in 0..RAINBOW_FRAME_COUNT {
        write_frame(writer, &layout.rainbow_frame(value, frame), true)?;
        sleep(RAINBOW_FRAME_DELAY);
    }
    write_frame(
        writer,
        &layout.perfect_frame(value, code, RAINBOW_FRAME_COUNT),
        true,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_text::{display_width, sanitize};
    use std::path::Path;

    /// The literals this file formats with must match the columns
    /// [`score_block`] budgets for them, since the geometry is computed there
    /// and painted here.
    #[test]
    fn the_padding_this_file_writes_is_the_padding_the_geometry_counts() {
        assert_eq!(display_width(INDENT), score_block::INDENT_COLUMNS);
        assert_eq!(display_width(GAP), score_block::GAP_COLUMNS);
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
        let mut output = Vec::new();
        let drawn = render(&mut output, value, label, true, true, options(width, color)).unwrap();
        assert!(drawn, "the block should fit at width {width}");
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn the_block_composes_a_face_a_score_a_bar_and_branding() {
        let output = rendered(100, ScoreLabel::Great, 100, false);
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 4);
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
        let output = rendered(100, ScoreLabel::Great, 100, true);
        let colors: Vec<(u8, u8, u8)> = output
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
            .collect();
        assert!(colors.len() >= 40, "the bar should be painted per character");
        let (first_red, _, first_blue) = colors[0];
        let (last_red, _, last_blue) = colors[colors.len() - 1];
        assert!(first_blue > first_red, "the gradient should start cool");
        assert!(last_red > last_blue, "the gradient should end warm");
    }

    #[test]
    fn a_report_that_cannot_show_links_drops_the_url() {
        let mut output = Vec::new();
        render(
            &mut output,
            100,
            ScoreLabel::Great,
            true,
            false,
            options(100, false),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(!output.contains("https://"));
        assert!(output.contains("Rust Doctor"));
        assert_eq!(output.lines().count(), 4);
    }

    #[test]
    fn every_line_fits_the_terminal_in_both_color_modes() {
        for width in [40, 80, 100, 140] {
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
                            display_width(&visible)
                                <= width - score_block::RIGHT_EDGE_SAFETY_COLUMNS,
                            "width {width} exceeded by {visible:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_terminal_too_narrow_declines_the_block() {
        let mut output = Vec::new();
        let drawn = render(
            &mut output,
            100,
            ScoreLabel::Great,
            true,
            true,
            options(16, false),
        )
        .unwrap();
        assert!(!drawn);
        assert!(output.is_empty());
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

    /// An animated output must rewind exactly the height of the block between
    /// two frames, otherwise it stacks blocks instead of replacing them.
    #[test]
    fn an_animated_render_rewinds_exactly_one_block_per_frame() {
        let mut output = Vec::new();
        let mut animated = options(100, true);
        animated.animate = true;
        render(
            &mut output,
            60,
            ScoreLabel::NeedsWork,
            true,
            true,
            animated,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        let rewinds = output.matches("\u{1b}[4A").count();
        assert_eq!(rewinds as u32, score_block::COUNT_UP_FRAME_COUNT);
        // The value and the label are painted separately, so the line is only
        // readable once the sequences are stripped.
        assert!(sanitize(&output).contains("60 / 100 Needs work"));
    }
}
