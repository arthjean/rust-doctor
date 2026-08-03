//! Bloc de score à deux colonnes: visage à gauche, valeur, barre et branding à
//! droite.
//!
//! Transposition de `render-score-header.ts` de React Doctor: mêmes visages,
//! même conversion OKLCH, mêmes cadences d'animation. Le compte à rebours est
//! easé sur 40 frames, puis un score parfait fait défiler l'arc-en-ciel sur 16
//! frames avant de se figer sur une barre colorée et un visage uni.
//!
//! L'animation n'existe que dans un vrai terminal coloré hors CI. Toute autre
//! sortie reçoit directement la frame finale, ce qui garde les rendus capturés
//! déterministes.

use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::thread::sleep;
use std::time::Duration;

use super::{RenderError, Style, TerminalOptions, write_styled};
use crate::ScoreLabel;
use crate::terminal_text::{display_width, truncate};

const FACE_TOP: &str = "┌─────┐";
const FACE_BOTTOM: &str = "└─────┘";
const INDENT: &str = "  ";
const GAP: &str = "  ";

/// Largeur de barre de la référence, rabotée par la place disponible.
const BAR_MAX_WIDTH: usize = 50;

/// En dessous de cette largeur de barre le bloc ne vaut plus rien: l'appelant
/// retombe sur la ligne unique historique.
const BAR_MIN_WIDTH: usize = 10;

/// Colonne laissée libre à droite, comme la référence.
const RIGHT_EDGE_SAFETY: usize = 1;

const BRANDING_NAME: &str = "Rust Doctor";
const BRANDING_URL: &str = "(https://rust-doctor.vercel.app)";

const RAINBOW_GRADIENT_WIDTH: f64 = 80.0;
const RAINBOW_OKLCH_LIGHTNESS: f64 = 0.638;
const RAINBOW_OKLCH_CHROMA: f64 = 0.129;
const RAINBOW_HUE_SHIFT_PER_FRAME: f64 = 9.0;

const ANIMATION_FRAME_COUNT: u32 = 40;
const ANIMATION_FRAME_DELAY: Duration = Duration::from_millis(50);
const RAINBOW_FRAME_COUNT: u32 = 16;
const RAINBOW_FRAME_DELAY: Duration = Duration::from_millis(50);

const PERFECT_SCORE: u8 = 100;

const DIM: &str = "2";
const RESET: &str = "\u{1b}[0m";

/// Efface la fin de ligne pour qu'une frame plus courte que la précédente ne
/// laisse pas de résidu. La référence s'en passe; on le garde parce que la
/// ligne de score s'allonge de `0 / 100` à `100 / 100` pendant le compte.
const CLEAR_TO_END: &str = "\u{1b}[K";

fn face(label: ScoreLabel) -> [&'static str; 2] {
    match label {
        ScoreLabel::Great => ["◠ ◠", " ▽ "],
        ScoreLabel::NeedsWork => ["• •", " ─ "],
        ScoreLabel::Critical => ["x x", " ▽ "],
    }
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

/// Portage direct de la conversion OKLCH de la référence. Les coefficients sont
/// ceux de la matrice OKLab vers sRGB linéaire.
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

/// Colorise caractère par caractère en truecolor. `offset` décale le dégradé
/// pour qu'il paraisse continuer depuis le bord gauche de la ligne, et `frame`
/// le fait tourner: c'est ce décalage qui produit le défilement.
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

fn ease_out_cubic(progress: f64) -> f64 {
    1.0 - (1.0 - progress).powi(3)
}

/// Valeur affichée à une frame du compte: départ à zéro, arrivée exacte sur la
/// note, jamais de recul.
fn counted(value: u8, frame: u32) -> u8 {
    let progress = ease_out_cubic(f64::from(frame) / f64::from(ANIMATION_FRAME_COUNT));
    let counted = (f64::from(value) * progress).round();
    if counted <= 0.0 {
        0
    } else if counted >= f64::from(PERFECT_SCORE) {
        PERFECT_SCORE
    } else {
        counted as u8
    }
}

fn bar(value: u8, width: usize) -> String {
    let filled = (usize::from(value) * width)
        .div_ceil(usize::from(PERFECT_SCORE))
        .min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// Géométrie et contenus figés du bloc, calculés une fois par rendu pour que
/// toutes les frames partagent exactement la même mise en page.
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
        let offset = INDENT.len() + display_width(FACE_TOP) + GAP.len();
        // Une colonne de garde à droite: une ligne qui remplit exactement le
        // terminal provoque un retour à la ligne implicite sur certains
        // émulateurs, ce qui désynchroniserait le rembobinage de l'animation.
        let available = width.checked_sub(offset + RIGHT_EDGE_SAFETY)?;
        if available < BAR_MIN_WIDTH {
            return None;
        }
        let [eyes, mouth] = face(label);
        Some(Self {
            faces: [
                FACE_TOP.to_owned(),
                format!("│ {eyes} │"),
                format!("│ {mouth} │"),
                FACE_BOTTOM.to_owned(),
            ],
            offset,
            available,
            bar_width: available.min(BAR_MAX_WIDTH),
            label_text: if authoritative {
                label.as_str()
            } else {
                "Core partial"
            },
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
            truncate(&format!("{BRANDING_NAME} {BRANDING_URL}"), self.available)
        } else {
            truncate(BRANDING_NAME, self.available)
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

    /// Frame intégralement arc-en-ciel: visage compris. C'est l'état pendant
    /// l'animation d'un score parfait.
    fn rainbow_frame(&self, value: u8, frame: u32) -> String {
        let mut output = String::new();
        for (index, right) in self.raw_right(value).iter().enumerate() {
            let line = self.compose(index, right);
            let _ = write!(output, "{}{CLEAR_TO_END}\n\r", rainbow(&line, frame, 0));
        }
        output
    }

    /// Frame finale d'un score parfait: visage uni, barre arc-en-ciel figée.
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

    /// Frame colorée d'un score imparfait: tout suit le style du label, sauf
    /// `/ 100` et l'URL qui restent estompés.
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

    /// Colorise une ligne de la colonne de droite. La ligne de score sépare la
    /// valeur du dénominateur, et le branding sépare le nom de l'URL, comme la
    /// référence.
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
            2 => match right.split_once(BRANDING_URL) {
                Some((name, _)) => {
                    format!("{}{}", name, paint(BRANDING_URL, DIM))
                }
                None => right.to_owned(),
            },
            _ => paint(right, code),
        }
    }
}

/// Écrit le bloc à deux colonnes. Retourne `false` quand le terminal est trop
/// étroit pour le porter, auquel cas l'appelant garde la ligne unique.
///
/// `allow_links` suit la règle des lignes Share, Docs et GitHub: un rapport en
/// échec ne rend aucune URL, donc la ligne de branding perd la sienne.
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

/// Une frame se termine par `\n\r`; on la réécrit en place en remontant de ses
/// quatre lignes.
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
    for frame in 0..=ANIMATION_FRAME_COUNT {
        let counted = counted(value, frame);
        let painted = if is_perfect {
            layout.rainbow_frame(counted, frame)
        } else {
            layout.styled_frame(counted, code)
        };
        write_frame(writer, &painted, frame > 0)?;
        if frame < ANIMATION_FRAME_COUNT {
            sleep(ANIMATION_FRAME_DELAY);
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
    use crate::terminal_text::sanitize;
    use std::path::Path;

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
        assert!(lines[2].contains("Rust Doctor (https://rust-doctor.vercel.app)"));
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

    /// Le dégradé figé doit partir du cyan à gauche et finir sur l'orange à
    /// droite, comme la frame finale de la référence. Un décalage de teinte nul
    /// inverserait le sens.
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
                            display_width(&visible) <= width - RIGHT_EDGE_SAFETY,
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

    #[test]
    fn the_bar_fills_proportionally_and_never_overflows() {
        assert_eq!(bar(100, 10), "██████████");
        assert_eq!(bar(0, 10), "░░░░░░░░░░");
        assert_eq!(display_width(&bar(37, 10)), 10);
        assert!(bar(1, 10).starts_with('█'));
    }

    /// Le compte doit démarrer à zéro, arriver pile sur la valeur et ne jamais
    /// reculer: c'est tout ce que l'easing garantit.
    #[test]
    fn the_eased_count_up_starts_at_zero_and_lands_on_the_score() {
        for value in [1u8, 42, 99, 100] {
            let counted: Vec<u8> = (0..=ANIMATION_FRAME_COUNT)
                .map(|frame| counted(value, frame))
                .collect();
            assert_eq!(counted[0], 0);
            assert_eq!(counted[counted.len() - 1], value);
            assert!(counted.windows(2).all(|pair| pair[0] <= pair[1]));
        }
    }

    /// Une sortie animée doit rembobiner exactement la hauteur du bloc entre
    /// deux frames, sinon elle empile les blocs au lieu de les remplacer.
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
        assert_eq!(rewinds as u32, ANIMATION_FRAME_COUNT);
        // La valeur et le label sont peints séparément, donc la ligne n'est
        // lisible qu'une fois les séquences retirées.
        assert!(sanitize(&output).contains("60 / 100 Needs work"));
    }
}
