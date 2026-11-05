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

const RAINBOW_FRAME_COUNT: u32 = 16;
const RAINBOW_FRAME_DELAY: Duration = Duration::from_millis(50);

const DIM: &str = "2";
const RESET: &str = "\u{1b}[0m";

const CLEAR_TO_END: &str = "\u{1b}[K";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    Label,
    Dim,
    Plain,
    Gradient,
}

#[derive(Debug, Clone)]
struct Row(Vec<(String, Ink)>);

impl Row {
    fn of<const N: usize>(pieces: [(String, Ink); N]) -> Self {
        Self(pieces.into())
    }

    fn empty() -> Self {
        Self(Vec::new())
    }

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

    fn plain(&self) -> String {
        self.0.iter().map(|(text, _)| text.as_str()).collect()
    }

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

fn compose(face: &str, right: &str) -> String {
    let separator = if right.is_empty() { "" } else { GAP };
    format!("{INDENT}{face}{separator}{right}")
}

#[derive(Debug, Clone, Copy)]
enum Palette<'a> {
    Scrolling { frame: u32 },
    Label { code: &'a str, frozen_bar: bool },
}

impl Palette<'_> {
    const fn bar_ink(self) -> Ink {
        match self {
            Self::Label {
                frozen_bar: true, ..
            } => Ink::Gradient,
            _ => Ink::Label,
        }
    }
}

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
        let url = format!(" ({})", score_block::BRANDING_URL);
        let fits = score_block::BRANDING_NAME.chars().count() + url.chars().count() <= available;
        let branding = if fits {
            Row::of([
                (score_block::BRANDING_NAME.to_owned(), Ink::Plain),
                (url, Ink::Dim),
            ])
        } else {
            Row::of([(score_block::BRANDING_NAME.to_owned(), Ink::Plain)])
        };
        Self {
            faces: score_block::face_rows(score.label),
            branding: branding.truncated(available),
            available,
            bar_width: score_block::bar_width(width).unwrap_or(available),
            label_text: score_block::label_text(score.label, score.authoritative),
        }
    }

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

fn write_plain<W: Write>(writer: &mut W, layout: &Layout, value: u8) -> Result<(), RenderError> {
    for (face, row) in layout.faces.iter().zip(layout.rows(value, Ink::Label)) {
        writeln!(writer, "{}", compose(face, &row.plain())).map_err(RenderError::Write)?;
    }
    Ok(())
}

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
