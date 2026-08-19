//! Two-column score block: face on the left, value, bar and branding on the
//! right.
//!
//! Transposition of React Doctor's `render-score-header.ts`: same faces, same
//! OKLCH conversion, same animation cadences. The count is eased over
//! [`score_block::COUNT_UP_FRAME_COUNT`] frames, then a perfect score scrolls
//! the rainbow over [`RAINBOW_FRAME_COUNT`] frames before freezing on a
//! colored bar and a plain face.
//!
//! Everything the interactive report must agree with lives in
//! [`crate::score_block`]: the faces, the label, the fill rule, the geometry
//! and the count-up. What stays here is what only this report does, the OKLCH
//! rainbow and the in-place rewind.
//!
//! Three rules hold it together.
//!
//! One frame builder. A frame is the four rows composed under one [`Palette`],
//! and the palette is the only thing that separates the counting frames from
//! the scrolling ones and from the frame the block freezes on. Three builders
//! used to carry the same loop, two of them identical but for a bolted-in
//! `index == 1`.
//!
//! One row, built from its pieces. A [`Row`] carries the segments it paints
//! differently, so the score line never has to be formatted and split back
//! apart on `/ 100` to find its denominator, nor the branding on `" ("` to find
//! its URL. Recovering structure from a string the same code just built is how
//! a cut line used to silently change which half got dimmed.
//!
//! A [`Row`] is deliberately not the interactive report's `Span`. The two look
//! alike, and the tables they must agree on were moved to [`crate::score_block`]
//! for exactly that reason, but they model different things: a span is an Ink
//! `<Text>` node with a color, a bold and a dim flag, sanitized on
//! construction because it carries text a scanned workspace produced. A row
//! carries four inks, one of which is a per-character truecolor gradient no
//! screen of the interactive report draws, and nothing but constants and one
//! integer. Merging them would widen the abstraction past both of its users.
//!
//! One width, guaranteed upstream. [`crate::render::MIN_WIDTH`] is at least
//! [`score_block::MIN_BLOCK_COLUMNS`], asserted at compile time, so the block
//! always fits and carries no narrow-terminal branch: no optional
//! constructor, no drawn flag threaded back to the caller, no second single
//! line renderer with its own rounding.
//!
//! The animation only exists in a real colored terminal outside CI. Any other
//! output receives the final frame directly, which keeps captured renders
//! deterministic.

use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::thread::sleep;
use std::time::Duration;

use super::{RenderError, TerminalOptions};
use crate::score_block::{self, BLOCK_ROWS, PERFECT_SCORE};
use crate::terminal_text::{display_width, truncate};
use crate::{AuditScore, ScoreLabel};

const INDENT: &str = "  ";
const GAP: &str = "  ";

const RAINBOW_GRADIENT_WIDTH: f64 = 80.0;
const RAINBOW_OKLCH_LIGHTNESS: f64 = 0.638;
const RAINBOW_OKLCH_CHROMA: f64 = 0.129;
const RAINBOW_HUE_SHIFT_PER_FRAME: f64 = 9.0;

/// Frames the gradient scrolls over once a perfect score has finished counting,
/// and the frame the bar then freezes on.
const RAINBOW_FRAME_COUNT: u32 = 16;
const RAINBOW_FRAME_DELAY: Duration = Duration::from_millis(50);

const DIM: &str = "2";
const RESET: &str = "\u{1b}[0m";

/// Clears the end of the line so that a frame shorter than the previous one
/// leaves no residue. The reference does without it; we keep it because the
/// score line grows from `0 / 100` to `100 / 100` during the count.
const CLEAR_TO_END: &str = "\u{1b}[K";

/// The delays of the two animation phases.
///
/// Only the delays are injectable: a test that shortened the frame counts
/// instead would stop proving the rewind arithmetic, which is the one thing
/// the animation can get wrong in a way the reader sees.
#[derive(Debug, Clone, Copy)]
pub(super) struct Cadence {
    count_up: Duration,
    rainbow: Duration,
}

impl Cadence {
    pub(super) const DEFAULT: Self = Self {
        count_up: score_block::COUNT_UP_FRAME_DELAY,
        rainbow: RAINBOW_FRAME_DELAY,
    };

    /// Runs every frame with no wall clock at all, for the tests that assert
    /// the frame sequence.
    #[cfg(test)]
    const INSTANT: Self = Self {
        count_up: Duration::ZERO,
        rainbow: Duration::ZERO,
    };
}

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

/// Colors character by character in truecolor. `offset` shifts the gradient so
/// that it seems to continue from the left edge of the line, and `frame` makes
/// it rotate: that shift is what produces the scrolling.
///
/// The gradient advances one step per character, which is one step per column
/// because every character the block draws is one column wide.
/// `every_row_measures_as_many_columns_as_it_has_characters` is what keeps that
/// true.
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
        let _ = write!(
            colored,
            "\u{1b}[38;2;{red};{green};{blue}m{character}\u{1b}[39m"
        );
    }
    colored
}

fn bar(value: u8, width: usize) -> String {
    let filled = score_block::bar_fill(value, width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// How one piece of the right column is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    /// Follows the style the score's own label carries.
    Label,
    /// Dimmed: the denominator and the URL, like the reference.
    Dim,
    /// Left as it is: the branding name.
    Plain,
    /// Frozen mid-gradient, which only the bar of a finished perfect score is.
    Gradient,
}

/// One row of the right column, as the pieces it paints differently.
#[derive(Debug, Clone)]
struct Row(Vec<(String, Ink)>);

impl Row {
    /// A row from the pieces it is made of, left to right.
    fn of<const N: usize>(pieces: [(String, Ink); N]) -> Self {
        Self(pieces.into())
    }

    /// The row the closing face row sits beside, which carries nothing.
    fn empty() -> Self {
        Self(Vec::new())
    }

    /// Cuts the row to `columns`, piece by piece, so a cut lands inside one
    /// piece and leaves the pieces before it whole. Cutting the rendered line
    /// instead is what used to make a narrow terminal lose the dimming
    /// altogether.
    fn truncated(self, columns: usize) -> Self {
        let mut budget = columns;
        let mut pieces = Vec::with_capacity(self.0.len());
        for (text, ink) in self.0 {
            if budget == 0 {
                break;
            }
            let cut = truncate(&text, budget);
            budget -= display_width(&cut);
            pieces.push((cut, ink));
        }
        Self(pieces)
    }

    /// The row with no sequence at all, for a colorless report and as the
    /// input the scrolling palette paints whole.
    fn plain(&self) -> String {
        self.0.iter().map(|(text, _)| text.as_str()).collect()
    }

    /// The row painted piece by piece. `offset` is where the row starts on the
    /// line, which only the frozen gradient reads, to continue from the left
    /// edge rather than restart at the piece.
    fn painted(&self, code: &str, offset: usize) -> String {
        let mut painted = String::new();
        let mut column = offset;
        for (text, ink) in &self.0 {
            let piece = match ink {
                Ink::Label => paint(text, code),
                Ink::Dim => paint(text, DIM),
                Ink::Plain => text.clone(),
                Ink::Gradient => rainbow(text, RAINBOW_FRAME_COUNT, column),
            };
            painted.push_str(&piece);
            column += display_width(text);
        }
        painted
    }
}

/// Assembles one rendered row from its two already-painted columns. The gap
/// only exists when something follows it, so the closing row leaves no trailing
/// spaces.
fn compose(face: &str, right: &str) -> String {
    let separator = if right.is_empty() { "" } else { GAP };
    format!("{INDENT}{face}{separator}{right}")
}

/// How one frame paints its columns.
#[derive(Debug, Clone, Copy)]
enum Palette<'a> {
    /// The whole line scrolls the gradient, face included: what a perfect score
    /// shows while it counts up.
    Scrolling { frame: u32 },
    /// The label style everywhere, with the bar frozen mid-gradient once a
    /// perfect score has finished scrolling.
    Label { code: &'a str, frozen_bar: bool },
}

impl Palette<'_> {
    /// The ink the bar carries under this palette. The scrolling palette paints
    /// the whole line itself and never reads it.
    const fn bar_ink(self) -> Ink {
        match self {
            Self::Label {
                frozen_bar: true, ..
            } => Ink::Gradient,
            _ => Ink::Label,
        }
    }
}

/// Frozen geometry and contents of the block, computed once per render so that
/// every frame shares exactly the same layout. Only the score line and the bar
/// move between two frames; the faces and the branding are built once.
struct Layout {
    faces: [String; BLOCK_ROWS],
    branding: Row,
    available: usize,
    bar_width: usize,
    label_text: &'static str,
}

impl Layout {
    fn new(score: &AuditScore, width: usize) -> Self {
        let available = score_block::right_column_width(width);
        let branding = Row::of([
            (score_block::BRANDING_NAME.to_owned(), Ink::Plain),
            (format!(" ({})", score_block::BRANDING_URL), Ink::Dim),
        ]);
        Self {
            faces: score_block::face_rows(score.label),
            branding: branding.truncated(available),
            available,
            bar_width: available.min(score_block::BAR_MAX_WIDTH_CHARS),
            label_text: score_block::label_text(score.label, score.authoritative),
        }
    }

    /// The four rows of the right column at one value. The bar needs no cut,
    /// its width being the available room capped, and the closing row carries
    /// nothing at all.
    fn rows(&self, value: u8, bar_ink: Ink) -> [Row; BLOCK_ROWS] {
        [
            Row::of([
                (value.to_string(), Ink::Label),
                (format!(" / {PERFECT_SCORE}"), Ink::Dim),
                (format!(" {}", self.label_text), Ink::Label),
            ])
            .truncated(self.available),
            Row::of([(bar(value, self.bar_width), bar_ink)]),
            self.branding.clone(),
            Row::empty(),
        ]
    }

    /// One frame: the four rows, each terminated so the next frame can rewind
    /// onto it.
    fn frame(&self, value: u8, palette: Palette<'_>) -> String {
        let mut output = String::new();
        for (face, row) in self.faces.iter().zip(self.rows(value, palette.bar_ink())) {
            let composed = match palette {
                Palette::Scrolling { frame } => rainbow(&compose(face, &row.plain()), frame, 0),
                Palette::Label { code, .. } => compose(
                    &paint(face, code),
                    &row.painted(code, score_block::FACE_OFFSET_COLUMNS),
                ),
            };
            let _ = write!(output, "{composed}{CLEAR_TO_END}\n\r");
        }
        output
    }

    /// The frame the block freezes on, whether it was animated or drawn at
    /// once. Both paths take it from here so neither can freeze on a different
    /// one.
    fn final_frame(&self, value: u8, code: &str, is_perfect: bool) -> String {
        self.frame(
            value,
            Palette::Label {
                code,
                frozen_bar: is_perfect,
            },
        )
    }
}

/// Writes the two-column block.
///
/// It always fits: [`super::MIN_WIDTH`] is at least
/// [`score_block::MIN_BLOCK_COLUMNS`], asserted at compile time, and every
/// entry point of the linear report normalizes to it.
pub(super) fn render<W: Write>(
    writer: &mut W,
    score: &AuditScore,
    options: TerminalOptions<'_>,
    cadence: Cadence,
) -> Result<(), RenderError> {
    let layout = Layout::new(score, options.width);
    let code = style_code(score.label, score.authoritative);
    let is_perfect = score.authoritative && score.value == PERFECT_SCORE;

    if !options.color {
        return write_plain(writer, &layout, score.value);
    }
    if options.animate {
        return animate(writer, &layout, score.value, code, is_perfect, cadence);
    }
    write_frame(
        writer,
        &layout.final_frame(score.value, code, is_perfect),
        false,
    )
}

/// A report with no color takes the frozen frame with no sequence at all, one
/// row per line, so a captured render carries only what it says.
fn write_plain<W: Write>(writer: &mut W, layout: &Layout, value: u8) -> Result<(), RenderError> {
    for (face, row) in layout.faces.iter().zip(layout.rows(value, Ink::Label)) {
        writeln!(writer, "{}", compose(face, &row.plain())).map_err(RenderError::Write)?;
    }
    Ok(())
}

/// A frame ends every row with `\n\r`; the next one is written in place by
/// going back up the rows this one wrote.
fn write_frame<W: Write>(writer: &mut W, frame: &str, rewind: bool) -> Result<(), RenderError> {
    if rewind {
        write!(writer, "\u{1b}[{BLOCK_ROWS}A\r").map_err(RenderError::Write)?;
    }
    write!(writer, "{frame}").map_err(RenderError::Write)?;
    writer.flush().map_err(RenderError::Write)
}

fn animate<W: Write>(
    writer: &mut W,
    layout: &Layout,
    value: u8,
    code: &str,
    is_perfect: bool,
    cadence: Cadence,
) -> Result<(), RenderError> {
    for frame in 0..=score_block::COUNT_UP_FRAME_COUNT {
        let counted = score_block::eased(0, value, frame, score_block::COUNT_UP_FRAME_COUNT);
        let palette = if is_perfect {
            Palette::Scrolling { frame }
        } else {
            Palette::Label {
                code,
                frozen_bar: false,
            }
        };
        write_frame(writer, &layout.frame(counted, palette), frame > 0)?;
        if frame < score_block::COUNT_UP_FRAME_COUNT {
            sleep(cadence.count_up);
        }
    }

    if !is_perfect {
        return Ok(());
    }

    for frame in 0..RAINBOW_FRAME_COUNT {
        write_frame(
            writer,
            &layout.frame(value, Palette::Scrolling { frame }),
            true,
        )?;
        sleep(cadence.rainbow);
    }
    write_frame(writer, &layout.final_frame(value, code, true), true)
}

#[cfg(test)]
mod tests;
