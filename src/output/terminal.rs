use crate::cli::ScanCategory;
use crate::diagnostics::{
    CanonicalDiagnostic, Category, CheckStatus, CompletenessState, DiagnosticLocation,
    ReportOutcome, ReportV1, ScoreImpact, Severity,
};
use owo_colors::{OwoColorize, Stream};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::{IsTerminal, Write as _};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::score::{calculate_score_for_canonical, score_label};

const SCORE_BAR_WIDTH: usize = 50;
const OUTPUT_MEASURE_WIDTH: usize = 60;
const CODE_FRAME_LINES_ABOVE: u32 = 1;
const CODE_FRAME_LINES_BELOW: u32 = 1;
const CODE_FRAME_MAX_LINE_LENGTH: usize = 200;
const CODE_FRAME_CLUSTER_REACH: u32 = 3;
const CODE_FRAME_MAX_SPAN: u32 = 20;
const BRAND_URL: &str = "https://rust-doctor.vercel.app";
const DOCS_URL: &str = "https://rust-doctor.vercel.app/docs";
const GITHUB_URL: &str = "https://github.com/arthjean/rust-doctor";
const VERBOSE_COMMAND: &str = "npx rust-doctor@latest --verbose";

const DISPLAY_CATEGORIES: [&str; 5] = [
    "Security",
    "Bugs",
    "Performance",
    "Dependencies",
    "Maintainability",
];

const WELCOME_TYPEWRITER_DELAY: Duration = Duration::from_millis(16);
const WELCOME_INTER_LINE_DELAY: Duration = Duration::from_millis(250);
const WELCOME_HOLD_DELAY: Duration = Duration::from_secs(1);
const ONBOARDING_SECTION_DELAY: Duration = Duration::from_millis(850);
const CATEGORY_COUNTUP_MAX_STEPS: usize = 24;
const CATEGORY_COUNTUP_FRAME_DELAY: Duration = Duration::from_millis(70);
const CATEGORY_COUNTUP_SETTLE_HOLD: Duration = Duration::from_secs(1);
const SCORE_HEADER_ANIMATION_FRAME_COUNT: u32 = 40;
const SCORE_HEADER_ANIMATION_FRAME_DELAY: Duration = Duration::from_millis(50);
const PERFECT_SCORE_RAINBOW_FRAME_COUNT: u32 = 16;
const PERFECT_SCORE_RAINBOW_FRAME_DELAY: Duration = Duration::from_millis(50);
const SCORE_PROJECTION_FRAME_COUNT: u32 = 16;
const SCORE_PROJECTION_FRAME_DELAY: Duration = Duration::from_millis(35);
const SCORE_PROJECTION_BAR_ROWS_ABOVE_CURSOR: usize = 5;
const RAINBOW_HUE_SHIFT_PER_FRAME: f64 = 9.0;
const RAINBOW_GRADIENT_WIDTH: f64 = 80.0;
const RAINBOW_OKLCH_LIGHTNESS: f64 = 0.638;
const RAINBOW_OKLCH_CHROMA: f64 = 0.129;

/// Render the temporary welcome scene on an interactive terminal, or the
/// static branded header used by pipes, CI, verbose output, and agent shells.
#[allow(
    clippy::redundant_pub_crate,
    reason = "the private terminal module exposes this only through a crate-private re-export"
)]
pub(crate) fn render_welcome(animate: bool, returning_user: bool) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    if !animate {
        writeln!(stdout, "Rust Doctor v{}", env!("CARGO_PKG_VERSION"))?;
        writeln!(stdout)?;
        return stdout.flush();
    }

    let speed_multiplier = if returning_user { 2 } else { 1 };
    let face = ["┌─────┐", "│ ◠ ◠ │", "│  ▽  │", "└─────┘"].map(stdout_success);
    writeln!(stdout)?;
    for line in &face {
        writeln!(stdout, "  {line}")?;
    }
    write!(stdout, "\x1b[3A")?;
    stdout.flush()?;

    let available = welcome_text_width();
    let greeting = clamp_text("Welcome to Rust Doctor", available);
    type_welcome_line(
        &mut stdout,
        &format!("  {}  ", face[1]),
        &greeting,
        stdout_bold,
        WELCOME_TYPEWRITER_DELAY / speed_multiplier,
    )?;
    if super::animation_cancelled() {
        write!(stdout, "\x1b[3A\r\x1b[0J")?;
        return stdout.flush();
    }
    super::animation_sleep(WELCOME_INTER_LINE_DELAY / speed_multiplier);
    if super::animation_cancelled() {
        write!(stdout, "\x1b[3A\r\x1b[0J")?;
        return stdout.flush();
    }
    write!(stdout, "\x1b[1B")?;
    let tagline = clamp_text(
        "I diagnose your Rust code for bugs, security & performance.",
        available,
    );
    type_welcome_line(
        &mut stdout,
        &format!("  {}  ", face[2]),
        &tagline,
        stdout_dim,
        WELCOME_TYPEWRITER_DELAY / speed_multiplier,
    )?;
    if super::animation_cancelled() {
        write!(stdout, "\x1b[3A\r\x1b[0J")?;
        return stdout.flush();
    }
    super::animation_sleep(WELCOME_HOLD_DELAY / speed_multiplier);
    write!(stdout, "\x1b[3A\r\x1b[0J")?;
    stdout.flush()
}

fn type_welcome_line(
    stdout: &mut impl std::io::Write,
    prefix: &str,
    text: &str,
    style: fn(&str) -> String,
    delay: Duration,
) -> std::io::Result<()> {
    let characters: Vec<_> = text.chars().collect();
    for length in 1..=characters.len() {
        if super::animation_cancelled() {
            break;
        }
        let fragment: String = characters[..length].iter().collect();
        write!(stdout, "\r{prefix}{}\x1b[K", style(&fragment))?;
        stdout.flush()?;
        super::animation_sleep(delay);
    }
    Ok(())
}

fn welcome_text_width() -> usize {
    const PREFIX_WIDTH: usize = 11;
    crate::run::stdout_columns().map_or(usize::MAX, |columns| {
        columns.saturating_sub(PREFIX_WIDTH + 1)
    })
}

fn clamp_text(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let mut result = take_width(text, max_width - 1).trim_end().to_string();
    result.push('…');
    result
}

/// Render full scan results to stdout/stderr.
pub fn render_terminal(
    result: &ReportV1,
    pass_timings: &[(String, std::time::Duration)],
    verbose: bool,
    show_warnings: bool,
) {
    render_terminal_for_categories(
        result,
        pass_timings,
        verbose,
        show_warnings,
        &[],
        true,
        false,
        false,
        false,
        false,
    );
}

/// Render full scan results using the React Doctor terminal contract.
///
/// Finding detail remains on stderr and the score/footer remain on stdout,
/// preserving Rust Doctor's piping contract.
#[allow(
    clippy::fn_params_excessive_bools,
    clippy::redundant_pub_crate,
    clippy::too_many_arguments,
    reason = "the existing renderer seam receives resolved CLI policy without changing its crate-private visibility"
)]
pub(crate) fn render_terminal_for_categories(
    result: &ReportV1,
    pass_timings: &[(String, std::time::Duration)],
    verbose: bool,
    show_warnings: bool,
    selected_categories: &[ScanCategory],
    show_default_share: bool,
    render_static_scan_summary: bool,
    show_agent_guidance: bool,
    animate_post_scan: bool,
    animate_stderr_details: bool,
) {
    let rendered = build_terminal_output(
        result,
        RenderOptions {
            verbose,
            show_warnings,
            selected_categories,
            show_default_share,
            render_static_scan_summary,
            show_agent_guidance,
        },
    );

    if animate_post_scan {
        let mut stderr = std::io::stderr().lock();
        let mut stdout = std::io::stdout().lock();
        let _ = write_animated_terminal_output(
            &rendered,
            &mut stderr,
            &mut stdout,
            animate_stderr_details,
            !verbose,
            &mut super::animation_sleep,
        );
    } else {
        if !rendered.stdout.is_empty() {
            print!("{}", rendered.stdout);
            let _ = std::io::stdout().flush();
        }
        if !rendered.stderr.is_empty() {
            eprint!("{}", rendered.stderr);
            let _ = std::io::stderr().flush();
        }
    }
    if verbose && !pass_timings.is_empty() {
        print_pass_timings(pass_timings);
    }
}

#[derive(Clone, Copy)]
struct RenderOptions<'a> {
    verbose: bool,
    show_warnings: bool,
    selected_categories: &'a [ScanCategory],
    show_default_share: bool,
    render_static_scan_summary: bool,
    show_agent_guidance: bool,
}

#[derive(Default)]
struct TerminalOutput {
    stderr: String,
    stdout: String,
    animation: TerminalAnimation,
}

#[derive(Default)]
struct TerminalAnimation {
    has_findings: bool,
    pace_sections: bool,
    stderr_reveal_start: Option<usize>,
    top_error_block_starts: Vec<usize>,
    category: Option<CategoryAnimation>,
    summary_start: usize,
    score: Option<ScoreAnimation>,
    footer_start: Option<usize>,
}

struct CategoryAnimation {
    range: Range<usize>,
    tallies: Vec<CategoryTally>,
}

struct ScoreAnimation {
    range: Range<usize>,
    score: u32,
    label: crate::diagnostics::ScoreLabel,
    potential_score: Option<u32>,
    projection_ready_offset: Option<usize>,
    bar_width: usize,
}

#[derive(Default)]
struct DiagnosticsAnimation {
    top_error_block_starts: Vec<usize>,
    category: Option<CategoryAnimation>,
}

#[expect(
    clippy::too_many_lines,
    reason = "the terminal document and its animation offsets must be assembled in one order-preserving pass"
)]
fn build_terminal_output(result: &ReportV1, options: RenderOptions<'_>) -> TerminalOutput {
    let mut output = TerminalOutput::default();
    output.animation.pace_sections = result.projects.len() <= 1;
    render_operational_context(result, &mut output.stderr);

    if options.render_static_scan_summary && result.source_file_count > 0 {
        let _ = writeln!(
            output.stderr,
            "{} Scanned {} in {:.1}s",
            stderr_success("✔"),
            file_count_label(result.source_file_count),
            result.elapsed,
        );
    }

    let demoted_diagnostic_count = result
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            !diagnostic
                .visible_on
                .iter()
                .any(|surface| surface == "terminal")
        })
        .count();
    let mut diagnostics: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic
                .visible_on
                .iter()
                .any(|surface| surface == "terminal")
                && (options.show_warnings || diagnostic.severity != Severity::Warning)
        })
        .collect();
    sort_terminal_diagnostics(&mut diagnostics);
    let root_causes = crate::ordering::root_cause_groups(diagnostics.iter().copied());
    let groups = diagnostic_groups(&diagnostics);
    output.animation.summary_start = output.stdout.len();
    output.animation.score = render_summary(
        &mut output.stdout,
        result,
        &root_causes,
        options.selected_categories,
        options.verbose,
    );
    output.animation.has_findings = !diagnostics.is_empty();
    let stderr_reveal_start = output.stderr.len();

    if diagnostics.is_empty() {
        if result.source_file_count == 0 {
            let message = match result.reporting_scope.as_str() {
                "staged" => "No staged source files found.",
                "changed" | "files" | "lines" => "No changed source files found.",
                _ => "No Rust source files found.",
            };
            let _ = writeln!(output.stderr, "\n  {}", stderr_warn(message));
        } else if matches!(
            result.outcome,
            ReportOutcome::Partial | ReportOutcome::Failed
        ) {
            let incomplete_checks = result.completeness.skipped_checks
                + result.completeness.failed_checks
                + result.completeness.timed_out_checks
                + result.completeness.cancelled_checks;
            let noun = if incomplete_checks == 1 {
                "check"
            } else {
                "checks"
            };
            let _ = writeln!(
                output.stderr,
                "\n  {}",
                stderr_warn(&format!(
                    "No issues detected, but {} failed: results are incomplete.",
                    incomplete_checks_phrase_with_fallback(result, incomplete_checks, noun)
                ))
            );
        } else if result.projects.len() <= 1 {
            let message = if !options.selected_categories.is_empty() {
                format!(
                    "No issues found in category {}!",
                    selected_category_names(options.selected_categories).join(", ")
                )
            } else if demoted_diagnostic_count > 0 {
                format!(
                    "No issues found! ({demoted_diagnostic_count} demoted from the terminal surface: see config.surfaces.)"
                )
            } else {
                "No issues found!".to_string()
            };
            let _ = writeln!(output.stdout, "  {}\n", stdout_success(&message));
        }
    } else {
        output.stderr.push('\n');
        let animation = render_diagnostics(
            &mut output.stderr,
            &diagnostics,
            &groups,
            &root_causes,
            result,
            options.verbose,
            result.resolved_root.as_deref(),
            options.show_agent_guidance,
            should_render_hyperlinks(options.show_agent_guidance),
        );
        output.animation.top_error_block_starts = animation.top_error_block_starts;
        output.animation.category = animation.category;
    }
    if output.stderr.len() > stderr_reveal_start {
        output.animation.stderr_reveal_start = Some(stderr_reveal_start);
    }

    if options.show_agent_guidance && !diagnostics.is_empty() {
        render_agent_guidance(&mut output.stdout);
    }

    if options.verbose && !result.audit.suppressed_security.is_empty() {
        let _ = writeln!(
            output.stderr,
            "  Security audit: {} finding(s) suppressed by project configuration",
            result.audit.suppressed_security.len()
        );
        for diagnostic in &result.audit.suppressed_security {
            let _ = writeln!(
                output.stderr,
                "    {}: {}",
                diagnostic.rule, diagnostic.message
            );
        }
        output.stderr.push('\n');
    }

    render_project_summary(
        &mut output.stdout,
        result,
        options.show_warnings,
        options.verbose,
    );
    if options.verbose && (!diagnostics.is_empty() || result.projects.len() > 1) {
        output.animation.footer_start = Some(output.stdout.len());
        render_footer(&mut output.stdout, result, options.show_default_share);
    }
    output
}

fn write_animated_terminal_output(
    rendered: &TerminalOutput,
    stderr: &mut impl std::io::Write,
    stdout: &mut impl std::io::Write,
    animate_stderr_details: bool,
    animate_stdout_details: bool,
    sleep: &mut impl FnMut(Duration),
) -> std::io::Result<()> {
    write_animated_stdout(rendered, stdout, animate_stdout_details, sleep)?;
    if super::animation_cancelled() {
        return Ok(());
    }
    write_animated_stderr(rendered, stderr, animate_stderr_details, sleep)
}

fn write_animated_stderr(
    rendered: &TerminalOutput,
    writer: &mut impl std::io::Write,
    animate_details: bool,
    sleep: &mut impl FnMut(Duration),
) -> std::io::Result<()> {
    if super::animation_cancelled() {
        return Ok(());
    }
    let animation = &rendered.animation;
    let bytes = rendered.stderr.as_bytes();
    if !animate_details {
        writer.write_all(bytes)?;
        return writer.flush();
    }
    let Some(reveal_start) = animation.stderr_reveal_start else {
        writer.write_all(bytes)?;
        return writer.flush();
    };

    writer.write_all(&bytes[..reveal_start])?;
    if animation.pace_sections {
        writer.flush()?;
        sleep(ONBOARDING_SECTION_DELAY);
        if super::animation_cancelled() {
            return Ok(());
        }
    }

    let mut cursor = reveal_start;
    for &block_start in &animation.top_error_block_starts {
        writer.write_all(&bytes[cursor..block_start])?;
        if animation.pace_sections {
            writer.flush()?;
            sleep(ONBOARDING_SECTION_DELAY);
            if super::animation_cancelled() {
                return Ok(());
            }
        }
        cursor = block_start;
    }

    if let Some(category) = &animation.category {
        writer.write_all(&bytes[cursor..category.range.start])?;
        write_category_countup(writer, &category.tallies, sleep)?;
        cursor = category.range.end;
    }
    writer.write_all(&bytes[cursor..])?;
    writer.flush()
}

fn write_animated_stdout(
    rendered: &TerminalOutput,
    writer: &mut impl std::io::Write,
    animate_details: bool,
    sleep: &mut impl FnMut(Duration),
) -> std::io::Result<()> {
    if super::animation_cancelled() {
        return Ok(());
    }
    let animation = &rendered.animation;
    let bytes = rendered.stdout.as_bytes();
    let summary_start = animation.summary_start.min(rendered.stdout.len());

    if summary_start > 0 {
        if animate_details && !animation.has_findings && animation.pace_sections {
            sleep(ONBOARDING_SECTION_DELAY);
            if super::animation_cancelled() {
                return Ok(());
            }
        }
        writer.write_all(&bytes[..summary_start])?;
        writer.flush()?;
    }
    if animate_details && summary_start < rendered.stdout.len() && animation.pace_sections {
        sleep(ONBOARDING_SECTION_DELAY);
        if super::animation_cancelled() {
            return Ok(());
        }
    }

    let mut cursor = summary_start;
    if let Some(score) = &animation.score {
        writer.write_all(&bytes[cursor..score.range.start])?;
        write_score_countup(writer, score, animate_details, sleep)?;
        cursor = score.range.end;
        if let Some(projection_offset) = score.projection_ready_offset {
            writer.write_all(&bytes[cursor..projection_offset])?;
            if animate_details {
                write_score_projection(writer, score, sleep)?;
            }
            cursor = projection_offset;
        }
    }

    let footer_start = animation
        .footer_start
        .unwrap_or(rendered.stdout.len())
        .min(rendered.stdout.len());
    writer.write_all(&bytes[cursor..footer_start])?;
    if footer_start < rendered.stdout.len() {
        if animate_details && animation.pace_sections {
            writer.flush()?;
            sleep(ONBOARDING_SECTION_DELAY);
            if super::animation_cancelled() {
                return Ok(());
            }
        }
        writer.write_all(&bytes[footer_start..])?;
    }
    writer.flush()
}

fn write_category_countup(
    writer: &mut impl std::io::Write,
    tallies: &[CategoryTally],
    sleep: &mut impl FnMut(Duration),
) -> std::io::Result<()> {
    let total = category_unit_count(tallies);
    let units_per_step = total.div_ceil(CATEGORY_COUNTUP_MAX_STEPS).max(1);
    for revealed in (0..total).step_by(units_per_step) {
        if super::animation_cancelled() {
            return Ok(());
        }
        if revealed > 0 {
            write!(writer, "\x1b[{}A", tallies.len())?;
        }
        write!(
            writer,
            "\r{}",
            category_tally_text(tallies, revealed, "\n\r")
        )?;
        writer.flush()?;
        sleep(CATEGORY_COUNTUP_FRAME_DELAY);
    }
    if super::animation_cancelled() {
        return Ok(());
    }
    if total > 0 {
        write!(writer, "\x1b[{}A", tallies.len())?;
    }
    write!(writer, "\r{}", category_tally_text(tallies, total, "\n\r"))?;
    writer.flush()?;
    sleep(CATEGORY_COUNTUP_SETTLE_HOLD);
    Ok(())
}

fn write_score_countup(
    writer: &mut impl std::io::Write,
    animation: &ScoreAnimation,
    animate_projection: bool,
    sleep: &mut impl FnMut(Duration),
) -> std::io::Result<()> {
    if animation.score == 100 {
        writer.write_all(
            rainbow_score_header_frame(animation.score, 0, animation.label, 0, animation.bar_width)
                .as_bytes(),
        )?;
        writer.write_all(b"\n")?;
    } else {
        writer.write_all(
            score_header(
                animation.score,
                0,
                animation.label,
                None,
                animation.bar_width,
                0,
            )
            .as_bytes(),
        )?;
    }
    writer.write_all(b"\x1b[5A")?;

    if animation.score == 100 {
        write_perfect_score_countup(writer, animation, sleep)?;
    } else {
        write_regular_score_countup(writer, animation, animate_projection, sleep)?;
    }
    writer.write_all(b"\x1b[3B")?;
    writer.flush()
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the eased score is clamped by the final score in the 0..=100 domain"
)]
fn animated_score(score: u32, frame: u32, frame_count: u32) -> u32 {
    let progress = ease_out_cubic(f64::from(frame) / f64::from(frame_count));
    (f64::from(score) * progress).round() as u32
}

fn write_regular_score_countup(
    writer: &mut impl std::io::Write,
    animation: &ScoreAnimation,
    animate_projection: bool,
    sleep: &mut impl FnMut(Duration),
) -> std::io::Result<()> {
    let face = score_face_lines(animation.score);
    for frame in 0..=SCORE_HEADER_ANIMATION_FRAME_COUNT {
        if super::animation_cancelled() {
            return Ok(());
        }
        if frame > 0 {
            writer.write_all(b"\x1b[2A")?;
        }
        let display_score =
            animated_score(animation.score, frame, SCORE_HEADER_ANIMATION_FRAME_COUNT);
        let score_line = score_line(display_score, animation.score, animation.label);
        let potential_score = (frame == SCORE_HEADER_ANIMATION_FRAME_COUNT && !animate_projection)
            .then_some(animation.potential_score)
            .flatten()
            .map(f64::from);
        let bar = score_bar_for_values(
            f64::from(display_score),
            animation.score,
            potential_score,
            animation.bar_width,
        );
        write!(
            writer,
            "\r{}\n\r{}\n",
            score_header_line(&stdout_score(&face[0], animation.score), &score_line),
            score_header_line(&stdout_score(&face[1], animation.score), &bar)
        )?;
        writer.flush()?;
        if frame < SCORE_HEADER_ANIMATION_FRAME_COUNT {
            sleep(SCORE_HEADER_ANIMATION_FRAME_DELAY);
        }
    }
    Ok(())
}

fn write_perfect_score_countup(
    writer: &mut impl std::io::Write,
    animation: &ScoreAnimation,
    sleep: &mut impl FnMut(Duration),
) -> std::io::Result<()> {
    for frame in 0..=SCORE_HEADER_ANIMATION_FRAME_COUNT {
        if super::animation_cancelled() {
            return Ok(());
        }
        if frame > 0 {
            writer.write_all(b"\x1b[4A\r")?;
        } else {
            writer.write_all(b"\r")?;
        }
        let display_score =
            animated_score(animation.score, frame, SCORE_HEADER_ANIMATION_FRAME_COUNT);
        writer.write_all(
            rainbow_score_header_frame(
                animation.score,
                display_score,
                animation.label,
                frame,
                animation.bar_width,
            )
            .as_bytes(),
        )?;
        writer.flush()?;
        if frame < SCORE_HEADER_ANIMATION_FRAME_COUNT {
            sleep(SCORE_HEADER_ANIMATION_FRAME_DELAY);
        }
    }
    for frame in 0..PERFECT_SCORE_RAINBOW_FRAME_COUNT {
        if super::animation_cancelled() {
            return Ok(());
        }
        writer.write_all(b"\x1b[4A\r")?;
        writer.write_all(
            rainbow_score_header_frame(
                animation.score,
                animation.score,
                animation.label,
                frame,
                animation.bar_width,
            )
            .as_bytes(),
        )?;
        writer.flush()?;
        sleep(PERFECT_SCORE_RAINBOW_FRAME_DELAY);
    }
    writer.write_all(b"\x1b[4A\r")?;
    let final_header = score_header(
        animation.score,
        animation.score,
        animation.label,
        None,
        animation.bar_width,
        PERFECT_SCORE_RAINBOW_FRAME_COUNT,
    );
    writer.write_all(
        final_header
            .strip_suffix('\n')
            .unwrap_or(&final_header)
            .as_bytes(),
    )?;
    writer.write_all(b"\x1b[2A")?;
    Ok(())
}

fn write_score_projection(
    writer: &mut impl std::io::Write,
    animation: &ScoreAnimation,
    sleep: &mut impl FnMut(Duration),
) -> std::io::Result<()> {
    let Some(potential_score) = animation.potential_score else {
        return Ok(());
    };
    if animation.score == 100 || potential_score <= animation.score {
        return Ok(());
    }
    let face = score_face_lines(animation.score);
    for frame in 1..=SCORE_PROJECTION_FRAME_COUNT {
        if super::animation_cancelled() {
            return Ok(());
        }
        let progress = ease_out_cubic(f64::from(frame) / f64::from(SCORE_PROJECTION_FRAME_COUNT));
        let displayed_potential = f64::from(potential_score - animation.score)
            .mul_add(progress, f64::from(animation.score));
        let bar = score_bar_for_values(
            f64::from(animation.score),
            animation.score,
            Some(displayed_potential),
            animation.bar_width,
        );
        write!(
            writer,
            "\x1b[{SCORE_PROJECTION_BAR_ROWS_ABOVE_CURSOR}A\r{}\x1b[{SCORE_PROJECTION_BAR_ROWS_ABOVE_CURSOR}B\r",
            score_header_line(&stdout_score(&face[1], animation.score), &bar)
        )?;
        writer.flush()?;
        if frame < SCORE_PROJECTION_FRAME_COUNT {
            sleep(SCORE_PROJECTION_FRAME_DELAY);
        }
    }
    Ok(())
}

fn ease_out_cubic(progress: f64) -> f64 {
    1.0 - (1.0 - progress).powi(3)
}

fn render_operational_context(result: &ReportV1, output: &mut String) {
    if let Some(baseline) = &result.baseline {
        if baseline.baseline_degraded {
            let _ = writeln!(
                output,
                "Baseline degraded to files scope: {}",
                baseline
                    .degraded_reason
                    .as_deref()
                    .unwrap_or("unknown reason")
            );
        } else {
            let _ = writeln!(
                output,
                "Baseline {}: {} introduced, {} fixed, {} cross-file match(es)",
                baseline.base_commit,
                baseline.new_count,
                baseline.fixed_count,
                baseline.cross_file_match_count
            );
        }
    }
}

struct DiagnosticGroup<'a> {
    rule: &'a str,
    severity: Severity,
    category: &'a Category,
    title: &'a str,
    diagnostics: Vec<&'a CanonicalDiagnostic>,
}

fn diagnostic_groups<'a>(diagnostics: &[&'a CanonicalDiagnostic]) -> Vec<DiagnosticGroup<'a>> {
    let mut groups: Vec<DiagnosticGroup<'a>> = Vec::new();
    for &diagnostic in diagnostics {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.rule == diagnostic.rule && group.severity == diagnostic.severity)
        {
            group.diagnostics.push(diagnostic);
            continue;
        }
        groups.push(DiagnosticGroup {
            rule: &diagnostic.rule,
            severity: diagnostic.severity,
            category: &diagnostic.category,
            title: if diagnostic.title.is_empty() {
                &diagnostic.rule
            } else {
                &diagnostic.title
            },
            diagnostics: vec![diagnostic],
        });
    }
    groups
}

/// Order the terminal through the canonical comparator.
///
/// The terminal has no private ranking: it renders exactly the order JSON,
/// SARIF, MCP, plans, and handoffs use, so the product gives one answer about
/// what to fix first (US-015 AC-1).
fn sort_terminal_diagnostics(diagnostics: &mut [&CanonicalDiagnostic]) {
    let impact = crate::ordering::RootCauseImpact::measure(diagnostics.iter().copied());
    crate::ordering::sort_refs(diagnostics, &impact);
}

fn render_agent_guidance(output: &mut String) {
    const LINES: [&str; 13] = [
        "Treat Rust Doctor diagnostics as starting hypotheses. Read the relevant code before confirming or suppressing each finding.",
        "For each group, decide true positive, false positive, or needs-human-review, then assign high/medium/low confidence.",
        "Do not suppress a finding without evidence from the file in question. Confidence requires code context.",
        "Understand the root cause before editing. Fix the underlying code instead of changing rust-doctor config or suppressing rules unless explicitly asked.",
        "Investigate deeply where relevant: race conditions, security-sensitive flows, state propagation, multi-file refactors, and downstream dependency chains.",
        "Ignore pure style preferences, theoretical issues without real impact, missing features, and unrelated pre-existing code.",
        "Start with high-confidence fixes that preserve behavior. Leave low-confidence or product-dependent changes as notes.",
        "Run `npx rust-doctor@latest --verbose --scope changed` before and after changes, plus relevant tests after each focused batch.",
        "When available, spawn subagents or isolated worktrees for independent rule families, then review and merge only the best safe fixes.",
        "Split unrelated, broad, or behavior-changing work into separate PRs or branches instead of one large cleanup.",
        "When one rule spans dozens of files, fix a representative sample first, confirm the recipe holds, and get the code owner's sign-off before changing the rest.",
        "For confirmed issues that cannot be fixed now, create GitHub issues with the rule, file and line, confidence, impact, and proposed fix.",
        "If a fix needs an API, UX, or architecture decision, stop and ask before editing.",
    ];
    let _ = writeln!(output, "\n{}", stdout_bold("Agent guidance"));
    for line in LINES {
        let _ = writeln!(output, "{}", stdout_dim(&format!("  - {line}")));
    }
    output.push('\n');
}

#[expect(
    clippy::too_many_arguments,
    reason = "the renderer receives precomputed groups plus the existing terminal policy flags"
)]
fn render_diagnostics(
    output: &mut String,
    diagnostics: &[&CanonicalDiagnostic],
    groups: &[DiagnosticGroup<'_>],
    root_causes: &[crate::diagnostics::RootCauseGroup],
    result: &ReportV1,
    verbose: bool,
    root: Option<&str>,
    agent_environment: bool,
    hyperlinks: bool,
) -> DiagnosticsAnimation {
    let mut animation = DiagnosticsAnimation::default();

    if verbose {
        for group in groups {
            render_diagnostic_group(output, group, true, root, agent_environment, hyperlinks);
        }
        let _ = writeln!(output, "\n{}\n", stderr_dim(&section_divider()));
    } else {
        render_migration_advisory(output, root_causes);
        render_fix_first(output, root_causes);
        render_incomplete_evidence(output, result);
    }

    let scored_issues = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.score_impact == ScoreImpact::Scored)
        .count();
    let audit_observations = diagnostics
        .iter()
        .filter(|diagnostic| is_audit_observation(diagnostic))
        .count();
    let advisory_findings = diagnostics
        .len()
        .saturating_sub(scored_issues + audit_observations);
    if verbose {
        let _ = writeln!(
            output,
            "  {}\n  {}\n  {}\n",
            stderr_bold(&count_label(scored_issues, "scored issue")),
            stderr_bold(&count_label(advisory_findings, "advisory finding")),
            stderr_bold(&count_label(audit_observations, "audit observation")),
        );
        let actionable_and_advisory: Vec<_> = diagnostics
            .iter()
            .copied()
            .filter(|diagnostic| !is_audit_observation(diagnostic))
            .collect();
        let tallies = category_tallies(&actionable_and_advisory);
        let category_start = output.len();
        render_category_tallies(output, &tallies);
        animation.category = Some(CategoryAnimation {
            range: category_start..output.len(),
            tallies,
        });
    } else if !diagnostics.is_empty() {
        let _ = writeln!(
            output,
            "  {} errors · {} warnings · {} info",
            result.summary.error_count, result.summary.warning_count, result.summary.info_count,
        );
        let _ = writeln!(
            output,
            "  {} · {} · {}",
            stderr_bold(&format!("{scored_issues} scored")),
            stderr_bold(&format!("{advisory_findings} advisory")),
            stderr_bold(&format!("{audit_observations} audit")),
        );
        let _ = writeln!(
            output,
            "  {} {} {}",
            stderr_dim("Run"),
            stderr_info(&stderr_bold(VERBOSE_COMMAND)),
            stderr_dim("for details")
        );
    }

    if verbose {
        render_migration_advisory(output, root_causes);
        output.push('\n');
    }
    animation
}

fn render_fix_first(output: &mut String, root_causes: &[crate::diagnostics::RootCauseGroup]) {
    let actionable: Vec<_> = root_causes
        .iter()
        .filter(|group| group.score_impact == ScoreImpact::Scored)
        .take(crate::ordering::PROTECTED_ROOT_CAUSE_GROUPS)
        .collect();
    if actionable.is_empty() {
        return;
    }
    let _ = writeln!(output, "  {}", stderr_bold("Fix first"));
    for group in actionable {
        let priority = group.priority.as_deref().unwrap_or("unranked");
        let contribution = group.current_penalty.map_or_else(
            || "no score impact".to_string(),
            |value| {
                let dimension = group.score_dimension.as_deref().unwrap_or("score");
                format!("-{value:.2} {dimension} pts")
            },
        );
        let summary = format!(
            "{} [{priority}] · {} sites · {} files · {contribution}",
            group.title, group.occurrences, group.file_count
        );
        let _ = writeln!(
            output,
            "  {}",
            clip_with_ellipsis(&summary, OUTPUT_MEASURE_WIDTH.saturating_sub(2))
        );
        let action = group
            .remediation_title
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("Inspect one representative site and fix the shared cause.");
        let _ = writeln!(
            output,
            "    Next: {}",
            clip_with_ellipsis(action, OUTPUT_MEASURE_WIDTH.saturating_sub(10))
        );
    }
}

fn render_incomplete_evidence(output: &mut String, result: &ReportV1) {
    let mut reasons = Vec::new();
    for check in result.projects.iter().flat_map(|project| &project.checks) {
        if check.status == CheckStatus::Completed {
            continue;
        }
        let reason = format!(
            "{}: {}",
            check.name,
            check
                .reason
                .as_deref()
                .unwrap_or("analysis did not complete")
        );
        if !reasons.contains(&reason) {
            reasons.push(reason);
        }
    }
    if reasons.is_empty() {
        return;
    }
    let _ = writeln!(output, "  {}", stderr_bold("Evidence incomplete"));
    for reason in reasons.iter().take(3) {
        let _ = writeln!(
            output,
            "    {}",
            clip_with_ellipsis(reason, OUTPUT_MEASURE_WIDTH.saturating_sub(4))
        );
    }
    if reasons.len() > 3 {
        let _ = writeln!(output, "    +{} more", reasons.len() - 3);
    }
}

fn render_diagnostic_group(
    output: &mut String,
    group: &DiagnosticGroup<'_>,
    render_every_site: bool,
    root: Option<&str>,
    agent_environment: bool,
    hyperlinks: bool,
) {
    let icon = match group.severity {
        Severity::Error => stderr_error("✖"),
        Severity::Warning => stderr_warn("⚠"),
        Severity::Info => stderr_info("ℹ"),
    };
    let headline = format!("{}: {}", display_category(group.category), group.title);
    let colored_headline = colorize_stderr_by_severity(&headline, group.severity);
    let badge = if group.diagnostics.len() > 1 {
        stderr_dim(&format!(" ×{}", group.diagnostics.len()))
    } else {
        String::new()
    };
    // An advisory rule runs by default but contributes exactly zero. Marking it
    // here keeps its presence from reading as a score penalty (US-007 AC-6).
    let advisory = if group
        .diagnostics
        .iter()
        .all(|value| is_audit_observation(value))
    {
        stderr_dim(" [audit observation · no score or CI impact]")
    } else if group.diagnostics.iter().all(|value| value.advisory) {
        stderr_dim(" [advisory · no score impact]")
    } else {
        String::new()
    };
    let _ = writeln!(output, "  {icon} {colored_headline}{badge}{advisory}");

    let representative = representative_diagnostic(&group.diagnostics);
    if render_every_site && !agent_environment && !representative.url.is_empty() {
        let _ = writeln!(
            output,
            "    {}",
            stderr_info(&format!("Learn more: {}", representative.url))
        );
    }

    for line in wrap_text(&representative.message, OUTPUT_MEASURE_WIDTH) {
        let _ = writeln!(output, "    {line}");
    }
    if let Some(help) = representative
        .help
        .as_deref()
        .filter(|help| !help.is_empty())
    {
        for line in wrap_text(&format!("→ {help}"), OUTPUT_MEASURE_WIDTH) {
            let _ = writeln!(output, "{}", stderr_dim(&format!("    {line}")));
        }
    }
    if let Some(shared_count) = shared_fix_count(&group.diagnostics) {
        let _ = writeln!(
            output,
            "{}",
            stderr_dim(&format!(
                "    ↳ One fix clears all {shared_count} findings."
            ))
        );
    }
    if render_every_site && agent_environment && !representative.url.is_empty() {
        let directive = format!(
            "Curl with no cache & follow the canonical fix and false positive check recipe before fixing: {}",
            representative.url
        );
        for line in wrap_text(&directive, OUTPUT_MEASURE_WIDTH) {
            let _ = writeln!(output, "{}", stderr_dim(&format!("    {line}")));
        }
    }

    let sites = if render_every_site {
        group.diagnostics.clone()
    } else {
        vec![representative]
    };
    for cluster in cluster_diagnostics(&sites) {
        render_cluster(
            output,
            &cluster,
            root,
            group.severity == Severity::Error,
            hyperlinks,
        );
    }
    output.push('\n');
}

fn is_audit_observation(diagnostic: &CanonicalDiagnostic) -> bool {
    diagnostic.trust_tier == "audit-only" || diagnostic.aggregation_policy == "audit-only"
}

fn representative_diagnostic<'a>(
    diagnostics: &[&'a CanonicalDiagnostic],
) -> &'a CanonicalDiagnostic {
    diagnostics
        .iter()
        .copied()
        .find(|diagnostic| matches!(diagnostic.location, DiagnosticLocation::Source { .. }))
        .unwrap_or(diagnostics[0])
}

fn shared_fix_count(diagnostics: &[&CanonicalDiagnostic]) -> Option<usize> {
    let mut group_ids = HashSet::new();
    for diagnostic in diagnostics {
        for fix in &diagnostic.fixes {
            if let Some(group_id) = &fix.group_id {
                group_ids.insert(group_id.as_str());
            }
        }
    }
    (diagnostics.len() > 1 && group_ids.len() == 1).then_some(diagnostics.len())
}

#[derive(Clone)]
struct DiagnosticSite {
    start_line: u32,
    end_line: u32,
    column: u32,
}

struct DiagnosticCluster<'a> {
    sites: Vec<DiagnosticSite>,
    path: &'a str,
    start_line: u32,
    end_line: u32,
}

fn cluster_diagnostics<'a>(diagnostics: &[&'a CanonicalDiagnostic]) -> Vec<DiagnosticCluster<'a>> {
    let mut sites_by_path: Vec<(&str, Vec<DiagnosticSite>)> = Vec::new();
    for &diagnostic in diagnostics {
        let DiagnosticLocation::Source { path, range } = &diagnostic.location else {
            sites_by_path.push((
                "<project>",
                vec![DiagnosticSite {
                    start_line: 0,
                    end_line: 0,
                    column: 0,
                }],
            ));
            continue;
        };
        let site = DiagnosticSite {
            start_line: range.start.line,
            end_line: range.end.line.max(range.start.line),
            column: range.start.column,
        };
        if let Some((_, sites)) = sites_by_path
            .iter_mut()
            .find(|(candidate, _)| *candidate == path)
        {
            sites.push(site);
        } else {
            sites_by_path.push((path, vec![site]));
        }
    }

    let mut clusters = Vec::new();
    for (path, mut sites) in sites_by_path {
        sites.sort_by_key(|site| (site.start_line, site.column));
        let mut current: Vec<DiagnosticSite> = Vec::new();
        for site in sites {
            let breaks = current.last().is_some_and(|previous| {
                site.start_line.saturating_sub(previous.end_line) > CODE_FRAME_CLUSTER_REACH
                    || site.end_line.saturating_sub(current[0].start_line) > CODE_FRAME_MAX_SPAN
            });
            if breaks {
                push_cluster(&mut clusters, path, std::mem::take(&mut current));
            }
            current.push(site);
        }
        push_cluster(&mut clusters, path, current);
    }
    clusters
}

fn push_cluster<'a>(
    clusters: &mut Vec<DiagnosticCluster<'a>>,
    path: &'a str,
    sites: Vec<DiagnosticSite>,
) {
    let Some(first) = sites.first() else {
        return;
    };
    let start_line = first.start_line;
    let end_line = sites
        .iter()
        .map(|site| site.end_line)
        .max()
        .unwrap_or(start_line);
    clusters.push(DiagnosticCluster {
        sites,
        path,
        start_line,
        end_line,
    });
}

fn render_cluster(
    output: &mut String,
    cluster: &DiagnosticCluster<'_>,
    root: Option<&str>,
    render_code_frame: bool,
    hyperlinks: bool,
) {
    output.push('\n');
    let location = if cluster.start_line == 0 {
        cluster.path.to_string()
    } else if cluster.end_line > cluster.start_line {
        format!(
            "{}:{}-{}",
            cluster.path, cluster.start_line, cluster.end_line
        )
    } else {
        format!("{}:{}", cluster.path, cluster.start_line)
    };
    let location = format_location(&location, cluster.path, root, hyperlinks);
    let _ = writeln!(output, "{}", stderr_dim(&format!("    {location}")));

    if !render_code_frame || cluster.start_line == 0 {
        return;
    }
    let Some(root) = root else {
        return;
    };
    let Some(frame) = build_code_frame(root, cluster) else {
        return;
    };
    for line in frame {
        let _ = writeln!(output, "    {line}");
    }
}

fn format_location(location: &str, relative: &str, root: Option<&str>, hyperlinks: bool) -> String {
    if !hyperlinks {
        return location.to_string();
    }
    let Some(source) = root.and_then(|root| safe_source_path(Path::new(root), relative)) else {
        return location.to_string();
    };
    format!(
        "\x1b]8;;{}\x1b\\{location}\x1b]8;;\x1b\\",
        path_to_file_url(&source)
    )
}

fn path_to_file_url(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let mut url = String::from("file://");
    if !normalized.starts_with('/') {
        url.push('/');
    }
    for byte in normalized.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'.' | b'_' | b'~') {
            url.push(char::from(byte));
        } else {
            let _ = write!(url, "%{byte:02X}");
        }
    }
    url
}

fn should_render_hyperlinks(agent_environment: bool) -> bool {
    if agent_environment {
        return false;
    }
    if let Some(forced) = std::env::var_os("FORCE_HYPERLINK")
        && !forced.is_empty()
    {
        let forced = forced.to_string_lossy();
        return forced != "0" && !forced.eq_ignore_ascii_case("false");
    }
    if !std::io::stdout().is_terminal()
        || std::env::var_os("TERM").is_some_and(|value| value == "dumb")
        || is_ci_environment()
    {
        return false;
    }
    if std::env::var_os("WT_SESSION").is_some()
        || std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var_os("TERM").is_some_and(|value| value == "xterm-kitty")
        || std::env::var("VTE_VERSION")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .is_some_and(|version| version >= 5_000)
    {
        return true;
    }
    std::env::var("TERM_PROGRAM").is_ok_and(|program| {
        matches!(
            program.as_str(),
            "iTerm.app" | "WezTerm" | "vscode" | "Hyper" | "ghostty" | "Tabby" | "rio"
        )
    })
}

fn is_ci_environment() -> bool {
    ["GITHUB_ACTIONS", "GITLAB_CI", "CIRCLECI"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
        || std::env::var("CI")
            .is_ok_and(|value| !matches!(value.to_ascii_lowercase().as_str(), "" | "0" | "false"))
}

struct FrameLine {
    plain: String,
    styled: String,
}

fn build_code_frame(root: &str, cluster: &DiagnosticCluster<'_>) -> Option<Vec<String>> {
    let source_path = safe_source_path(Path::new(root), cluster.path)?;
    let source = std::fs::read_to_string(source_path).ok()?;
    let source_lines: Vec<_> = source.lines().map(neutralize_source_line).collect();
    if source_lines.is_empty() {
        return None;
    }
    let first = cluster.start_line.max(1) as usize;
    let last = (cluster.end_line.max(cluster.start_line) as usize).min(source_lines.len());
    if first > source_lines.len()
        || source_lines[first - 1..last]
            .iter()
            .any(|line| line.chars().count() > CODE_FRAME_MAX_LINE_LENGTH)
    {
        return None;
    }

    let context_start = cluster
        .start_line
        .saturating_sub(CODE_FRAME_LINES_ABOVE)
        .max(1);
    let context_end = cluster
        .end_line
        .saturating_add(CODE_FRAME_LINES_BELOW)
        .min(source_lines.len() as u32);
    let line_number_width = context_end.to_string().len();
    let mut lines = Vec::new();

    for line_number in context_start..=context_end {
        let is_selected = line_number >= cluster.start_line
            && line_number <= cluster.end_line.max(cluster.start_line);
        let marker = if is_selected { ">" } else { " " };
        let prefix_width = line_number_width + 5;
        let available_source_width = OUTPUT_MEASURE_WIDTH.saturating_sub(prefix_width);
        let raw_source = source_lines
            .get(line_number.saturating_sub(1) as usize)
            .map_or("", String::as_str);
        let clipped_source = clip_with_ellipsis(raw_source, available_source_width);
        let plain = format!("{marker} {line_number:>line_number_width$} | {clipped_source}");
        let styled_marker = if is_selected {
            stderr_error(marker)
        } else {
            marker.to_string()
        };
        let styled = format!(
            "{styled_marker} {} {} {}",
            stderr_dim(&format!("{line_number:>line_number_width$}")),
            stderr_dim("|"),
            highlight_rust_source(&clipped_source)
        );
        lines.push(FrameLine { plain, styled });

        if cluster.sites.len() == 1 && line_number == cluster.start_line {
            let column = cluster.sites[0].column.max(1) as usize;
            let caret_room = OUTPUT_MEASURE_WIDTH.saturating_sub(line_number_width + 6);
            let caret_offset = column.saturating_sub(1).min(caret_room);
            let prefix = format!("{}| ", " ".repeat(line_number_width + 3));
            lines.push(FrameLine {
                plain: format!("{prefix}{}^", " ".repeat(caret_offset)),
                styled: format!(
                    "{}{}{}",
                    stderr_dim(&prefix),
                    " ".repeat(caret_offset),
                    stderr_error("^")
                ),
            });
        }
    }

    Some(box_frame(lines, OUTPUT_MEASURE_WIDTH))
}

fn neutralize_source_line(source: &str) -> String {
    source
        .chars()
        .filter(|character| {
            let code_point = u32::from(*character);
            *character == '\t' || !(code_point <= 0x1f || (0x7f..=0x9f).contains(&code_point))
        })
        .collect()
}

fn safe_source_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    let canonical_root = root.canonicalize().ok()?;
    let canonical_source = canonical_root.join(relative).canonicalize().ok()?;
    canonical_source
        .starts_with(&canonical_root)
        .then_some(canonical_source)
}

fn box_frame(lines: Vec<FrameLine>, inner_width: usize) -> Vec<String> {
    let horizontal = stderr_dim(&"─".repeat(inner_width + 2));
    let mut framed = vec![format!(
        "{}{horizontal}{}",
        stderr_dim("┌"),
        stderr_dim("┐")
    )];
    for line in lines {
        let padding = inner_width.saturating_sub(line.plain.width());
        framed.push(format!(
            "{} {}{} {}",
            stderr_dim("│"),
            line.styled,
            " ".repeat(padding),
            stderr_dim("│")
        ));
    }
    framed.push(format!(
        "{}{horizontal}{}",
        stderr_dim("└"),
        stderr_dim("┘")
    ));
    framed
}

fn highlight_rust_source(source: &str) -> String {
    const KEYWORDS: [&str; 29] = [
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "struct", "trait",
    ];
    let characters: Vec<_> = source.chars().collect();
    let mut output = String::new();
    let mut index = 0;
    while index < characters.len() {
        if characters[index] == '/' && characters.get(index + 1).copied() == Some('/') {
            let rest: String = characters[index..].iter().collect();
            output.push_str(&stderr_dim(&rest));
            break;
        }
        if matches!(characters[index], '"' | '\'') {
            let quote = characters[index];
            let start = index;
            index += 1;
            while index < characters.len() {
                if characters[index] == '\\' {
                    index = (index + 2).min(characters.len());
                    continue;
                }
                let closes = characters[index] == quote;
                index += 1;
                if closes {
                    break;
                }
            }
            let token: String = characters[start..index].iter().collect();
            output.push_str(&stderr_success(&token));
            continue;
        }
        if characters[index].is_alphabetic() || characters[index] == '_' {
            let start = index;
            index += 1;
            while index < characters.len()
                && (characters[index].is_alphanumeric() || characters[index] == '_')
            {
                index += 1;
            }
            let token: String = characters[start..index].iter().collect();
            if KEYWORDS.contains(&token.as_str()) || token == "where" || token == "use" {
                output.push_str(&stderr_info(&token));
            } else {
                output.push_str(&token);
            }
            continue;
        }
        if characters[index].is_ascii_digit() {
            let start = index;
            index += 1;
            while index < characters.len()
                && (characters[index].is_ascii_digit() || characters[index] == '_')
            {
                index += 1;
            }
            let token: String = characters[start..index].iter().collect();
            output.push_str(&stderr_warn(&token));
            continue;
        }
        output.push(characters[index]);
        index += 1;
    }
    output
}

#[derive(Clone, Copy, Default)]
struct CategoryTally {
    category_index: usize,
    errors: usize,
    warnings: usize,
    infos: usize,
}

fn category_tallies(diagnostics: &[&CanonicalDiagnostic]) -> Vec<CategoryTally> {
    let mut tallies = [CategoryTally::default(); DISPLAY_CATEGORIES.len()];
    for diagnostic in diagnostics {
        let tally = &mut tallies[category_rank(&diagnostic.category)];
        match diagnostic.severity {
            Severity::Error => tally.errors += 1,
            Severity::Warning => tally.warnings += 1,
            Severity::Info => tally.infos += 1,
        }
    }
    tallies
        .into_iter()
        .enumerate()
        .filter_map(|(category_index, mut tally)| {
            tally.category_index = category_index;
            (tally.errors + tally.warnings + tally.infos > 0).then_some(tally)
        })
        .collect()
}

fn category_unit_count(tallies: &[CategoryTally]) -> usize {
    tallies
        .iter()
        .map(|tally| tally.errors + tally.warnings + tally.infos)
        .sum()
}

fn category_tally_text(
    tallies: &[CategoryTally],
    revealed_unit_count: usize,
    line_separator: &str,
) -> String {
    let mut remaining = revealed_unit_count.min(category_unit_count(tallies));
    let mut lines = Vec::with_capacity(tallies.len());
    for tally in tallies {
        let shown_errors = tally.errors.min(remaining);
        remaining -= shown_errors;
        let shown_warnings = tally.warnings.min(remaining);
        remaining -= shown_warnings;
        let shown_infos = tally.infos.min(remaining);
        remaining -= shown_infos;
        let mut parts = Vec::new();
        if tally.errors > 0 {
            parts.push(stderr_error(&count_label(shown_errors, "error")));
        }
        if tally.warnings > 0 {
            parts.push(stderr_warn(&stderr_dim(&count_label(
                shown_warnings,
                "warning",
            ))));
        }
        if tally.infos > 0 {
            parts.push(stderr_info(&stderr_dim(&count_label(shown_infos, "info"))));
        }
        lines.push(format!(
            "  {} {} {}",
            stderr_bold(DISPLAY_CATEGORIES[tally.category_index]),
            stderr_dim("›"),
            parts.join(&stderr_dim(", "))
        ));
    }
    format!("{}\n", lines.join(line_separator))
}

fn render_category_tallies(output: &mut String, tallies: &[CategoryTally]) {
    output.push_str(&category_tally_text(
        tallies,
        category_unit_count(tallies),
        "\n",
    ));
}

/// Migration grouping: highest-impact root causes before individual
/// occurrences, using the shared threshold so the terminal and the handoff
/// switch at the same point (US-015 AC-3, AC-4).
fn render_migration_advisory(
    output: &mut String,
    root_causes: &[crate::diagnostics::RootCauseGroup],
) {
    let eligible_groups: Vec<_> = root_causes
        .iter()
        .filter(|group| {
            group.trust_tier != "audit-only" && group.aggregation_policy != "audit-only"
        })
        .cloned()
        .collect();
    let eligible_count = eligible_groups.iter().map(|group| group.occurrences).sum();
    if !crate::ordering::use_migration_grouping(eligible_count, &eligible_groups) {
        return;
    }
    if eligible_groups.is_empty() {
        return;
    }
    let _ = writeln!(
        output,
        "  {} {}",
        stderr_warn("⚠"),
        stderr_bold("Migration-scale: sample before any sweep")
    );
    let _ = writeln!(
        output,
        "{}",
        stderr_dim("    Validate one Fix first sample, then get owner sign-off.")
    );
}

fn render_summary(
    output: &mut String,
    result: &ReportV1,
    root_causes: &[crate::diagnostics::RootCauseGroup],
    selected_categories: &[ScanCategory],
    verbose: bool,
) -> Option<ScoreAnimation> {
    let headline_project = multi_project_headline_project(result);
    let score = if result.projects.len() > 1 && result.summary.score_authoritative {
        headline_project.and_then(|project| project.score)
    } else {
        result.score.filter(|_| result.summary.score_authoritative)
    };
    let Some(score) = score else {
        render_unavailable_score(output, result);
        return None;
    };
    let label = headline_project.map_or_else(
        || result.score_label,
        |_| Some(score_label_for_score(score)),
    )?;

    let projection_diagnostics: Vec<_> = headline_project.map_or_else(
        || result.diagnostics.iter().collect(),
        |project| project.diagnostics.iter().collect(),
    );
    let top_root_causes: HashSet<_> = root_causes
        .iter()
        .filter(|group| group.score_impact == ScoreImpact::Scored)
        .take(crate::ordering::PROTECTED_ROOT_CAUSE_GROUPS)
        .map(|group| group.key.clone())
        .collect();
    let potential_score = projected_score(
        &projection_diagnostics,
        &top_root_causes,
        selected_categories,
    )
    .filter(|potential| *potential > score);
    let authority = if result.completeness.state == CompletenessState::Complete {
        "Core complete"
    } else {
        "Core partial"
    };
    render_score_lead(
        output,
        score,
        label,
        authority,
        potential_score,
        selected_categories,
        verbose,
    );
    if !verbose {
        return None;
    }
    let bar_width = score_bar_width_from_environment();
    let header_start = output.len();
    output.push_str(&score_header(
        score,
        score,
        label,
        potential_score,
        bar_width,
        0,
    ));
    let header_end = output.len();

    let projection_ready_offset = potential_score.map(|potential| {
        let improvement = potential - score;
        let _ = writeln!(
            output,
            "{}{}{}",
            stdout_dim("  Projected score "),
            stdout_score(&potential.to_string(), potential),
            stdout_dim(&format!(
                " (+{improvement}) after resolving the selected root causes"
            ))
        );
        output.len()
    });
    if !selected_categories.is_empty() {
        let names = selected_category_names(selected_categories);
        let _ = writeln!(
            output,
            "{}",
            stdout_dim(&format!("  Categories: {}", names.join(", ")))
        );
    }
    Some(ScoreAnimation {
        range: header_start..header_end,
        score,
        label,
        potential_score,
        projection_ready_offset,
        bar_width,
    })
}

fn render_unavailable_score(output: &mut String, result: &ReportV1) {
    let message = result.summary.score_reasons.first().map_or_else(
        || {
            if result.source_file_count == 0 {
                "Score unavailable because no Rust source files were analyzed.".to_string()
            } else if matches!(
                result.outcome,
                ReportOutcome::Partial | ReportOutcome::Failed
            ) {
                "Score not shown: some checks could not complete.".to_string()
            } else {
                "Score unavailable because required analysis did not complete.".to_string()
            }
        },
        Clone::clone,
    );
    let prefix = "  Score unavailable · ";
    let detail = clip_with_ellipsis(
        &format!("{message}. Next: rerun with --verbose"),
        OUTPUT_MEASURE_WIDTH.saturating_sub(prefix.width()),
    );
    let _ = writeln!(
        output,
        "  {} · {}",
        stdout_error("Score unavailable"),
        stdout_dim(&detail)
    );
}

fn render_score_lead(
    output: &mut String,
    score: u32,
    label: crate::diagnostics::ScoreLabel,
    authority: &str,
    potential_score: Option<u32>,
    selected_categories: &[ScanCategory],
    verbose: bool,
) {
    let _ = writeln!(
        output,
        "  {} · {}",
        stdout_score(&format!("{score} / 100 {label}"), score),
        stdout_dim(authority)
    );
    if verbose {
        output.push('\n');
        return;
    }
    if let Some(potential) = potential_score {
        let improvement = potential - score;
        let _ = writeln!(
            output,
            "{}",
            stdout_dim(&format!(
                "  Projected {potential} (+{improvement}) after the first root causes"
            ))
        );
    }
    if !selected_categories.is_empty() {
        let names = selected_category_names(selected_categories);
        let categories = clip_with_ellipsis(
            &format!("  Categories: {}", names.join(", ")),
            OUTPUT_MEASURE_WIDTH,
        );
        let _ = writeln!(output, "{}", stdout_dim(&categories));
    }
}

fn multi_project_headline_project(result: &ReportV1) -> Option<&crate::diagnostics::ProjectReport> {
    if result.projects.len() <= 1 {
        return None;
    }
    let mut worst = None;
    for project in &result.projects {
        let Some(score) = project.score.filter(|_| project.score_authoritative) else {
            continue;
        };
        if worst.is_none_or(|(_, worst_score)| score < worst_score) {
            worst = Some((project, score));
        }
    }
    worst.map(|(project, _)| project)
}

fn projected_score(
    diagnostics: &[&CanonicalDiagnostic],
    excluded_root_causes: &HashSet<String>,
    selected_categories: &[ScanCategory],
) -> Option<u32> {
    if excluded_root_causes.is_empty() {
        return None;
    }
    let categories: Vec<_> = selected_categories
        .iter()
        .copied()
        .map(scan_category_to_category)
        .collect();
    let diagnostics = diagnostics.iter().copied().filter(|diagnostic| {
        diagnostic
            .visible_on
            .iter()
            .any(|surface| surface == "score")
            && diagnostic
                .root_cause_key
                .as_ref()
                .is_none_or(|key| !excluded_root_causes.contains(key))
    });
    calculate_score_for_canonical(diagnostics, &categories).map(|value| value.0)
}

#[cfg(test)]
fn score_bar(score: u32, potential_score: Option<u32>, width: usize) -> String {
    score_bar_for_values(
        f64::from(score),
        score,
        potential_score.map(f64::from),
        width,
    )
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "score ratios are clamped to 0..=100 before conversion to a bounded terminal width"
)]
fn score_fill_count(score: f64, width: usize) -> usize {
    ((score.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize
}

fn score_bar_for_values(
    display_score: f64,
    color_score: u32,
    potential_score: Option<f64>,
    width: usize,
) -> String {
    let current_fill = score_fill_count(display_score, width);
    let potential_fill =
        potential_score.map_or(current_fill, |potential| score_fill_count(potential, width));
    let potential_fill = potential_fill.clamp(current_fill, width);
    let gain = potential_fill - current_fill;
    let empty = width - potential_fill;
    format!(
        "{}{}{}",
        stdout_score(&"█".repeat(current_fill), color_score),
        stdout_dim(&stdout_score(&"▓".repeat(gain), color_score)),
        stdout_dim(&"░".repeat(empty))
    )
}

fn raw_score_bar(display_score: u32, width: usize) -> String {
    let filled = score_fill_count(f64::from(display_score), width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn score_face_lines(score: u32) -> [String; 4] {
    let face = doctor_face(score);
    [
        "┌─────┐".to_string(),
        format!("│ {} │", face.0),
        format!("│ {} │", face.1),
        "└─────┘".to_string(),
    ]
}

fn score_line(
    display_score: u32,
    final_score: u32,
    label: crate::diagnostics::ScoreLabel,
) -> String {
    format!(
        "{} {} {}",
        stdout_score(&display_score.to_string(), final_score),
        stdout_dim("/ 100"),
        stdout_score(&label.to_string(), final_score)
    )
}

fn raw_score_line(display_score: u32, label: crate::diagnostics::ScoreLabel) -> String {
    format!("{display_score} / 100 {label}")
}

fn score_header_line(face_line: &str, right_column: &str) -> String {
    if right_column.is_empty() {
        format!("  {face_line}")
    } else {
        format!("  {face_line}  {right_column}")
    }
}

fn score_header(
    final_score: u32,
    display_score: u32,
    label: crate::diagnostics::ScoreLabel,
    potential_score: Option<u32>,
    width: usize,
    rainbow_frame: u32,
) -> String {
    let face = score_face_lines(final_score);
    let score_line = score_line(display_score, final_score, label);
    let bar = if final_score == 100 && display_score == 100 {
        let offset = score_header_line(&face[1], "").chars().count() + 2;
        rainbow_text(&raw_score_bar(display_score, width), rainbow_frame, offset)
    } else {
        score_bar_for_values(
            f64::from(display_score),
            final_score,
            potential_score.map(f64::from),
            width,
        )
    };
    let right = [score_line, bar, branding_line(), String::new()];
    let mut output = String::new();
    for (face_line, right_column) in face.iter().zip(right) {
        let _ = writeln!(
            output,
            "{}",
            score_header_line(&stdout_score(face_line, final_score), &right_column)
        );
    }
    output.push('\n');
    output
}

fn join_score_header_frame(lines: &[String; 4]) -> String {
    format!(
        "{}\n\r{}\n\r{}\n\r{}\n",
        lines[0], lines[1], lines[2], lines[3]
    )
}

fn rainbow_score_header_frame(
    final_score: u32,
    display_score: u32,
    label: crate::diagnostics::ScoreLabel,
    frame: u32,
    width: usize,
) -> String {
    let face = score_face_lines(final_score);
    let right = [
        raw_score_line(display_score, label),
        raw_score_bar(display_score, width),
        format!("Rust Doctor ({BRAND_URL})"),
        String::new(),
    ];
    let lines = std::array::from_fn(|index| {
        rainbow_text(&score_header_line(&face[index], &right[index]), frame, 0)
    });
    join_score_header_frame(&lines)
}

#[derive(Clone, Copy)]
struct RgbColor {
    red: u8,
    green: u8,
    blue: u8,
}

fn encode_srgb(value: f64) -> f64 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        value.powf(1.0 / 2.4).mul_add(1.055, -0.055)
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the color channel is rounded only after clamping to the u8 domain"
)]
const fn clamp_color_channel(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn oklch_to_rgb(lightness: f64, chroma: f64, hue: f64) -> RgbColor {
    let hue_radians = hue.to_radians();
    let lab_a = chroma * hue_radians.cos();
    let lab_b = chroma * hue_radians.sin();
    let long_cone = lightness
        .mul_add(
            1.0,
            0.396_337_777_4f64.mul_add(lab_a, 0.215_803_757_3 * lab_b),
        )
        .powi(3);
    let medium_cone =
        0.063_854_172_8f64.mul_add(-lab_b, 0.105_561_345_8f64.mul_add(-lab_a, lightness));
    let medium_cone = medium_cone.powi(3);
    let short_cone =
        1.291_485_548f64.mul_add(-lab_b, 0.089_484_177_5f64.mul_add(-lab_a, lightness));
    let short_cone = short_cone.powi(3);
    RgbColor {
        red: clamp_color_channel(
            encode_srgb(4.076_741_662_1f64.mul_add(
                long_cone,
                (-3.307_711_591_3f64).mul_add(medium_cone, 0.230_969_929_2 * short_cone),
            )) * 255.0,
        ),
        green: clamp_color_channel(
            encode_srgb((-1.268_438_004_6f64).mul_add(
                long_cone,
                2.609_757_401_1f64.mul_add(medium_cone, -0.341_319_396_5 * short_cone),
            )) * 255.0,
        ),
        blue: clamp_color_channel(
            encode_srgb((-0.004_196_086_3f64).mul_add(
                long_cone,
                (-0.703_418_614_7f64).mul_add(medium_cone, 1.707_614_701 * short_cone),
            )) * 255.0,
        ),
    }
}

fn rainbow_text(text: &str, frame: u32, offset: usize) -> String {
    if !stdout_supports_color() {
        return text.to_string();
    }
    colored_rainbow_text(text, frame, offset)
}

fn stdout_supports_color() -> bool {
    let probe = format!(
        "{}",
        "x".if_supports_color(Stream::Stdout, |value| value.red())
    );
    probe != "x"
}

#[allow(
    clippy::cast_precision_loss,
    reason = "terminal text indices and animation frame counts are tiny compared with f64 precision"
)]
fn colored_rainbow_text(text: &str, frame: u32, offset: usize) -> String {
    text.chars().enumerate().fold(
        String::with_capacity(text.len()),
        |mut output, (index, character)| {
            if character == ' ' {
                output.push(character);
                return output;
            }
            let hue = f64::from(frame).mul_add(
                RAINBOW_HUE_SHIFT_PER_FRAME,
                ((index + offset) as f64 / RAINBOW_GRADIENT_WIDTH) * 360.0,
            ) % 360.0;
            let color = oklch_to_rgb(RAINBOW_OKLCH_LIGHTNESS, RAINBOW_OKLCH_CHROMA, hue);
            let _ = write!(
                output,
                "\x1b[38;2;{};{};{}m{character}\x1b[39m",
                color.red, color.green, color.blue
            );
            output
        },
    )
}

fn score_bar_width_from_environment() -> usize {
    score_bar_width(crate::run::stdout_columns())
}

fn score_bar_width(columns: Option<usize>) -> usize {
    const RIGHT_COLUMN_OFFSET: usize = 11;
    const RIGHT_EDGE_SAFETY_COLUMNS: usize = 1;
    const MINIMUM_WIDTH: usize = 10;
    columns.map_or(SCORE_BAR_WIDTH, |columns| {
        columns
            .saturating_sub(RIGHT_COLUMN_OFFSET + RIGHT_EDGE_SAFETY_COLUMNS)
            .clamp(MINIMUM_WIDTH, SCORE_BAR_WIDTH)
    })
}

fn render_project_summary(
    output: &mut String,
    result: &ReportV1,
    show_warnings: bool,
    verbose: bool,
) {
    if result.projects.len() <= 1 {
        return;
    }
    let entries: Vec<_> = result
        .projects
        .iter()
        .map(|project| project_summary_entry(project, show_warnings))
        .collect();
    let longest_name = entries
        .iter()
        .map(|(name, ..)| name.width())
        .max()
        .unwrap_or(0);
    let name_width = if verbose {
        longest_name
    } else {
        longest_name.min(14)
    };
    output.push('\n');
    let entry_count = entries.len();
    let displayed = if verbose {
        entry_count
    } else {
        entry_count.min(3)
    };
    for (name, score, errors, warnings) in entries.into_iter().take(displayed) {
        let name = if verbose {
            name
        } else {
            clip_with_ellipsis(&name, name_width)
        };
        let padded_name = format!("{name:<name_width$}");
        let Some(score) = score else {
            let total = errors + warnings;
            let line = format!("  {padded_name}  no score  {}", count_label(total, "issue"));
            let _ = writeln!(
                output,
                "{}",
                stdout_dim(&clip_with_ellipsis(&line, OUTPUT_MEASURE_WIDTH))
            );
            continue;
        };
        let mut issue_parts = Vec::new();
        let mut plain_issue_parts = Vec::new();
        if errors > 0 {
            issue_parts.push(stdout_error(&count_label(errors, "error")));
            plain_issue_parts.push(count_label(errors, "error"));
        }
        if warnings > 0 {
            issue_parts.push(stdout_warn(&count_label(warnings, "warning")));
            plain_issue_parts.push(count_label(warnings, "warning"));
        }
        if verbose {
            let _ = writeln!(
                output,
                "  {}  {}  {}  {}",
                stdout_score(&padded_name, score),
                stdout_score(&format!("{score:>3}"), score),
                stdout_score(score_band_label(score), score),
                issue_parts.join(&stdout_dim(", "))
            );
        } else {
            let line = format!(
                "  {padded_name}  {score:>3}  {}  {}",
                score_band_label(score),
                plain_issue_parts.join(", ")
            );
            let _ = writeln!(
                output,
                "{}",
                stdout_score(&clip_with_ellipsis(&line, OUTPUT_MEASURE_WIDTH), score)
            );
        }
    }
    if entry_count > displayed {
        let _ = writeln!(
            output,
            "  {}",
            stdout_dim(&format!("+{} more projects", entry_count - displayed))
        );
    }
}

fn project_summary_entry(
    project: &crate::diagnostics::ProjectReport,
    show_warnings: bool,
) -> (String, Option<u32>, usize, usize) {
    let visible = |diagnostic: &&CanonicalDiagnostic| {
        !is_audit_observation(diagnostic)
            && diagnostic
                .visible_on
                .iter()
                .any(|surface| surface == "terminal")
    };
    let errors = project
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error && visible(diagnostic))
        .count();
    let warnings = project
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            show_warnings && diagnostic.severity == Severity::Warning && visible(diagnostic)
        })
        .count();
    (project_name(project), project.score, errors, warnings)
}

fn project_name(project: &crate::diagnostics::ProjectReport) -> String {
    if let Some(name) = Path::new(&project.package_root)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
    {
        return name.to_string();
    }
    if let Some((source, fragment)) = project.cargo_package_id.rsplit_once('#') {
        if let Some((name, _version)) = fragment.split_once('@')
            && !name.is_empty()
        {
            return name.to_string();
        }
        if let Some(name) = source
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
        {
            return name.to_string();
        }
    }
    project
        .cargo_package_id
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("project")
        .to_string()
}

fn render_footer(output: &mut String, result: &ReportV1, show_default_share: bool) {
    let _ = writeln!(output, "\n{}\n", stdout_dim(&section_divider()));
    if show_default_share && let Ok(url) = crate::share::build_url(result) {
        let _ = writeln!(output, "  {} {}", stdout_bold("Share:"), stdout_info(&url));
        render_footer_description(output, "Tell others how you did on socials");
        output.push('\n');
    }
    let _ = writeln!(
        output,
        "  {} {}",
        stdout_bold("Docs:"),
        stdout_info(DOCS_URL)
    );
    render_footer_description(
        output,
        "Learn more about fixing issues, setting up CI/CD, and configuring rules with a config file",
    );
    output.push('\n');
    let _ = writeln!(
        output,
        "  {} {}",
        stdout_bold("GitHub:"),
        stdout_info(GITHUB_URL)
    );
    render_footer_description(output, "Report issues and star the repository!");
}

fn render_footer_description(output: &mut String, description: &str) {
    for line in wrap_text(description, OUTPUT_MEASURE_WIDTH) {
        let _ = writeln!(output, "{}", stdout_dim(&format!("  {line}")));
    }
}

fn branding_line() -> String {
    format!("Rust Doctor {}", stdout_dim(&format!("({BRAND_URL})")))
}

fn doctor_face(score: u32) -> (&'static str, &'static str) {
    match score_label(score) {
        crate::diagnostics::ScoreLabel::Great => ("◠ ◠", " ▽ "),
        crate::diagnostics::ScoreLabel::NeedsWork => ("• •", " ─ "),
        crate::diagnostics::ScoreLabel::Critical => ("x x", " ▽ "),
    }
}

const fn category_rank(category: &Category) -> usize {
    match category {
        Category::Security => 0,
        Category::Correctness | Category::ErrorHandling | Category::Async | Category::Framework => {
            1
        }
        Category::Performance => 2,
        Category::Dependencies | Category::Cargo => 3,
        Category::Architecture | Category::Style => 4,
    }
}

const fn display_category(category: &Category) -> &'static str {
    DISPLAY_CATEGORIES[category_rank(category)]
}

fn selected_category_names(categories: &[ScanCategory]) -> Vec<&'static str> {
    let mut names = Vec::new();
    for category in categories {
        let name = display_category(&scan_category_to_category(*category));
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

fn incomplete_check_names(result: &ReportV1) -> Vec<String> {
    let mut names = Vec::new();
    for check in result.projects.iter().flat_map(|project| &project.checks) {
        if matches!(
            check.status,
            CheckStatus::Skipped
                | CheckStatus::Failed
                | CheckStatus::TimedOut
                | CheckStatus::Cancelled
        ) && !names.contains(&check.name)
        {
            names.push(check.name.clone());
        }
    }
    for failure in &result.audit.analysis_failures {
        if !names.contains(&failure.check) {
            names.push(failure.check.clone());
        }
    }
    names
}

fn join_check_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [name] => name.clone(),
        [first, second] => format!("{first} and {second}"),
        _ => {
            let last_index = names.len() - 1;
            format!(
                "{}, and {}",
                names[..last_index].join(", "),
                names[last_index]
            )
        }
    }
}

fn incomplete_checks_phrase_with_fallback(
    result: &ReportV1,
    count: usize,
    fallback_noun: &str,
) -> String {
    let names = incomplete_check_names(result);
    if names.is_empty() {
        format!("{count} {fallback_noun}")
    } else {
        let noun = if names.len() == 1 { "check" } else { "checks" };
        format!("{} {noun}", join_check_names(&names))
    }
}

const fn scan_category_to_category(category: ScanCategory) -> Category {
    match category {
        ScanCategory::ErrorHandling => Category::ErrorHandling,
        ScanCategory::Performance => Category::Performance,
        ScanCategory::Security => Category::Security,
        ScanCategory::Correctness => Category::Correctness,
        ScanCategory::Architecture => Category::Architecture,
        ScanCategory::Dependencies => Category::Dependencies,
        ScanCategory::Async => Category::Async,
        ScanCategory::Framework => Category::Framework,
        ScanCategory::Cargo => Category::Cargo,
        ScanCategory::Style => Category::Style,
    }
}

fn section_divider() -> String {
    format!("  {}", "─".repeat(OUTPUT_MEASURE_WIDTH))
}

fn file_count_label(count: usize) -> String {
    count_label(count, "file")
}

fn count_label(count: usize, noun: &str) -> String {
    let suffix = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{suffix}")
}

pub(super) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut result = Vec::new();
    for paragraph in text.lines() {
        if paragraph.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let next_width = if line.is_empty() {
                word.width()
            } else {
                line.width() + 1 + word.width()
            };
            if !line.is_empty() && next_width > width {
                result.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            result.push(line);
        }
    }
    result
}

fn take_width(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|character| {
            let character_width = character.width().unwrap_or(0);
            if used + character_width > width {
                return false;
            }
            used += character_width;
            true
        })
        .collect()
}

fn clip_with_ellipsis(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".to_string();
    }
    let mut clipped = take_width(text, width - 1);
    clipped.push('…');
    clipped
}

fn score_band_label(score: u32) -> &'static str {
    match score_label_for_score(score) {
        crate::diagnostics::ScoreLabel::Great => "Great",
        crate::diagnostics::ScoreLabel::NeedsWork => "Needs work",
        crate::diagnostics::ScoreLabel::Critical => "Critical",
    }
}

fn score_label_for_score(score: u32) -> crate::diagnostics::ScoreLabel {
    score_label(score)
}

fn stdout_score(text: &str, score: u32) -> String {
    match score_label(score) {
        crate::diagnostics::ScoreLabel::Great => stdout_success(text),
        crate::diagnostics::ScoreLabel::NeedsWork => stdout_warn(text),
        crate::diagnostics::ScoreLabel::Critical => stdout_error(text),
    }
}

fn colorize_stderr_by_severity(text: &str, severity: Severity) -> String {
    match severity {
        Severity::Error => stderr_error(text),
        Severity::Warning => stderr_warn(text),
        Severity::Info => stderr_info(text),
    }
}

fn stdout_error(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stdout, |value| value.red())
    )
}

fn stdout_warn(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stdout, |value| value.yellow())
    )
}

fn stdout_info(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stdout, |value| value.cyan())
    )
}

fn stdout_success(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stdout, |value| value.green())
    )
}

fn stdout_dim(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stdout, |value| value.dimmed())
    )
}

fn stdout_bold(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stdout, |value| value.bold())
    )
}

fn stderr_error(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stderr, |value| value.red())
    )
}

fn stderr_warn(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stderr, |value| value.yellow())
    )
}

fn stderr_info(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stderr, |value| value.cyan())
    )
}

fn stderr_success(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stderr, |value| value.green())
    )
}

fn stderr_dim(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stderr, |value| value.dimmed())
    )
}

fn stderr_bold(text: &str) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stderr, |value| value.bold())
    )
}

fn print_pass_timings(timings: &[(String, std::time::Duration)]) {
    eprintln!();
    eprintln!(
        "{}",
        "Pass timings:".if_supports_color(Stream::Stderr, |value| value.dimmed())
    );
    let max_name_len = timings
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    for (name, duration) in timings {
        eprintln!(
            "  {:<width$}  {:.1}s",
            name.if_supports_color(Stream::Stderr, |value| value.dimmed()),
            duration.as_secs_f64(),
            width = max_name_len,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{
        AuditMetadata, CanonicalDiagnostic, CompletenessState, DiagnosticOwnership,
        DimensionScores, ProjectReport, ReportCompleteness, ReportSummary, ScanMode, ScoreLabel,
        SourcePosition, SourceRange, SourceSurface,
    };

    fn make_result(root: &Path, score: u32, diagnostics: Vec<CanonicalDiagnostic>) -> ReportV1 {
        let errors = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count();
        let warnings = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Warning)
            .count();
        let infos = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Info)
            .count();
        let mut report = ReportV1::failure(root, ScanMode::Full, "test", "");
        report.outcome = if diagnostics.is_empty() {
            ReportOutcome::Clean
        } else {
            ReportOutcome::Findings
        };
        report.resolved_root = Some(root.to_string_lossy().into_owned());
        report.completeness = ReportCompleteness {
            state: CompletenessState::Complete,
            planned_files: 10,
            analyzed_files: 10,
            completed_checks: 1,
            skipped_checks: 0,
            failed_checks: 0,
            timed_out_checks: 0,
            cancelled_checks: 0,
            required_checks: 1,
            required_completed_checks: 1,
            score_authoritative: true,
            abstentions: 0,
        };
        report.diagnostics = diagnostics;
        report.summary = ReportSummary {
            score: Some(score),
            score_label: Some(ScoreLabel::NeedsWork),
            error_count: errors,
            warning_count: warnings,
            info_count: infos,
            diagnostic_count: errors + warnings + infos,
            score_authoritative: true,
            score_reasons: Vec::new(),
        };
        report.audit = AuditMetadata::default();
        report.score = Some(score);
        report.score_label = Some(ScoreLabel::NeedsWork);
        report.dimension_scores = Some(DimensionScores {
            security: score,
            reliability: score,
            maintainability: score,
            performance: score,
            dependencies: score,
        });
        report.source_file_count = 10;
        report.elapsed = 0.7;
        report.elapsed_ms = 700;
        report.error_count = errors;
        report.warning_count = warnings;
        report.info_count = infos;
        report.error = None;
        report
    }

    fn make_diagnostic(
        rule: &str,
        severity: Severity,
        category: Category,
        line: u32,
    ) -> CanonicalDiagnostic {
        CanonicalDiagnostic {
            advisory: false,
            provider: "rust-doctor".to_string(),
            rule: rule.to_string(),
            title: format!("{rule} title"),
            category,
            severity,
            message: format!("test impact message for {rule}"),
            help: Some(format!("fix {rule} before shipping")),
            url: format!("{BRAND_URL}/rules/{rule}"),
            tags: vec!["heuristic".to_string()],
            analysis_kind: "synast".to_string(),
            confidence: "medium".to_string(),
            original_level: severity.to_string(),
            ownership: DiagnosticOwnership::Package {
                package_id: "fixture".to_string(),
            },
            source_surface: SourceSurface::Library,
            location: DiagnosticLocation::Source {
                path: "src/lib.rs".to_string(),
                range: SourceRange {
                    start: SourcePosition {
                        line,
                        column: 5,
                        byte_offset: None,
                    },
                    end: SourcePosition {
                        line,
                        column: 5,
                        byte_offset: None,
                    },
                },
            },
            related_locations: vec![],
            macro_expansion: None,
            fixes: vec![],
            visible_on: vec!["terminal".to_string(), "score".to_string()],
            site_id: format!("{rule}:{line}"),
            baseline_key: format!("{rule}:{line}"),
            namespace_fallback: false,
            priority: Some("p2".to_string()),
            trust_tier: "calibrated-heuristic".to_string(),
            score_eligible: true,
            score_impact: crate::diagnostics::ScoreImpact::Scored,
            aggregation_policy: "bounded-occurrence".to_string(),
            root_cause_key: Some(format!("rule:{rule}")),
            evidence_summary: "Syntactic Rust AST evidence.".to_string(),
            limitations: Vec::new(),
            fix_recipe: None,
            suppressed: false,
        }
    }

    fn make_project_report(
        result: &ReportV1,
        name: &str,
        score: Option<u32>,
        score_authoritative: bool,
        diagnostics: Vec<CanonicalDiagnostic>,
    ) -> ProjectReport {
        let mut completeness = result.completeness.clone();
        completeness.score_authoritative = score_authoritative;
        ProjectReport {
            cargo_package_id: format!("path+file:///tmp/{name}#{name}@1.0.0"),
            package_root: format!("/tmp/{name}"),
            targets: vec![],
            framework_capabilities: vec![],
            framework_gates: vec![],
            planned_files: vec![],
            analyzed_files: vec![],
            checks: vec![],
            skipped_reasons: vec![],
            completeness,
            score,
            score_authoritative,
            dimensions: Vec::new(),
            elapsed_ms: 700,
            diagnostics,
        }
    }

    fn render(result: &ReportV1, verbose: bool) -> TerminalOutput {
        build_terminal_output(
            result,
            RenderOptions {
                verbose,
                show_warnings: true,
                selected_categories: &[],
                show_default_share: false,
                render_static_scan_summary: true,
                show_agent_guidance: false,
            },
        )
    }

    #[test]
    fn default_render_matches_react_doctor_structure() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("src")).unwrap();
        std::fs::write(
            directory.path().join("src/lib.rs"),
            "pub fn before() {}\npub async fn risky() {}\npub fn after() {}\n",
        )
        .unwrap();
        let diagnostics = vec![
            make_diagnostic(
                "unauthenticated-server-action",
                Severity::Error,
                Category::Security,
                2,
            ),
            make_diagnostic(
                "excessive-clone",
                Severity::Warning,
                Category::Performance,
                3,
            ),
        ];
        let output = render(&make_result(directory.path(), 68, diagnostics), false);

        assert!(output.stderr.contains("✔ Scanned 10 files in 0.7s"));
        assert!(output.stderr.contains("Fix first"));
        assert!(
            output
                .stderr
                .contains("unauthenticated-server-action title"),
            "{}",
            output.stderr
        );
        assert!(output.stderr.contains("Next:"));
        assert!(!output.stderr.contains("Top 1 error you should fix"));
        assert!(!output.stderr.contains("src/lib.rs:2"));
        assert!(output.stderr.contains("2 scored"));
        assert!(output.stderr.contains("0 advisory"));
        assert!(output.stderr.contains("0 audit"));
        assert!(!output.stderr.contains("Security › 1 error"));
        assert!(output.stderr.contains(VERBOSE_COMMAND));

        assert!(output.stdout.contains("68 / 100 Needs work"));
        assert!(output.stdout.contains("Core complete"));
        let plain_stdout = strip_ansi(&output.stdout);
        let first_line = plain_stdout
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap();
        assert!(first_line.contains("68 / 100 Needs work"));
        assert!(
            output.stdout.lines().count() + output.stderr.lines().count() <= 30,
            "default output exceeded 30 lines:\nstdout:\n{}\nstderr:\n{}",
            output.stdout,
            output.stderr
        );
        assert!(!output.stdout.contains("Rust Doctor ("));
        assert!(!output.stdout.contains("Docs:"));
        assert!(!output.stdout.contains("GitHub:"));
        insta::assert_snapshot!("terminal_default_stderr", &output.stderr);
        insta::assert_snapshot!("terminal_default_stdout", &output.stdout);
    }

    #[test]
    fn default_fix_first_and_incomplete_evidence_are_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostics = (0..60)
            .map(|index| {
                make_diagnostic(
                    &format!("scored-rule-{}", index % 5),
                    Severity::Error,
                    Category::Correctness,
                    index + 1,
                )
            })
            .collect();
        let mut result = make_result(directory.path(), 68, diagnostics);
        let checks: Vec<_> = (0..5)
            .map(|index| crate::diagnostics::CheckState {
                name: format!("analyzer-{index}"),
                required: index == 0,
                status: CheckStatus::Failed,
                reason: Some(format!("failure-{index}-{}", "context".repeat(20))),
            })
            .collect();
        result.projects = (0..5)
            .map(|index| {
                let mut project = make_project_report(
                    &result,
                    &format!("fixture-{index}-{}", "project".repeat(20)),
                    Some(68),
                    true,
                    vec![],
                );
                project.checks.clone_from(&checks);
                project
            })
            .collect();
        result.completeness.state = CompletenessState::Partial;

        let output = render(&result, false);
        assert_eq!(output.stderr.matches("    Next:").count(), 3);
        assert_eq!(output.stderr.matches("    analyzer-").count(), 3);
        assert!(output.stderr.contains("+2 more"));
        assert!(output.stdout.contains("+2 more projects"));
        assert!(
            output.stderr.find("Migration-scale").unwrap()
                < output.stderr.find("Fix first").unwrap()
        );
        assert!(
            output.stdout.lines().count() + output.stderr.lines().count() <= 30,
            "worst-case default output exceeded 30 lines:\nstdout:\n{}\nstderr:\n{}",
            output.stdout,
            output.stderr
        );
        assert!(
            output
                .stderr
                .lines()
                .all(|line| strip_ansi(line).width() <= OUTPUT_MEASURE_WIDTH),
            "{}",
            output.stderr
        );
        assert!(
            output
                .stdout
                .lines()
                .all(|line| strip_ansi(line).width() <= OUTPUT_MEASURE_WIDTH),
            "{}",
            output.stdout
        );
    }

    #[test]
    fn interactive_render_opens_on_the_blank_line_that_separates_it_from_the_spinner() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("src")).unwrap();
        std::fs::write(directory.path().join("src/lib.rs"), "pub fn risky() {}\n").unwrap();
        let diagnostics = vec![make_diagnostic(
            "unauthenticated-server-action",
            Severity::Error,
            Category::Security,
            1,
        )];
        // Interactive runs get the scan summary from the live spinner instead of
        // the report, so the report itself owns the blank line React Doctor
        // prints between the summary and the findings.
        let output = build_terminal_output(
            &make_result(directory.path(), 68, diagnostics),
            RenderOptions {
                verbose: false,
                show_warnings: true,
                selected_categories: &[],
                show_default_share: false,
                render_static_scan_summary: false,
                show_agent_guidance: false,
            },
        );

        assert!(output.stderr.starts_with('\n'));
        assert!(!output.stderr.starts_with("\n\n"));
        assert!(output.stderr.contains("Fix first"));
    }

    #[test]
    fn unranked_recommendation_groups_ignore_site_id_order() {
        let mut semantic_first =
            make_diagnostic("semantic-first", Severity::Error, Category::Security, 8);
        let DiagnosticLocation::Source { path, .. } = &mut semantic_first.location else {
            unreachable!();
        };
        *path = "src/a.rs".to_string();
        semantic_first.site_id = "ffff".to_string();

        let mut semantic_second =
            make_diagnostic("semantic-second", Severity::Error, Category::Security, 2);
        let DiagnosticLocation::Source { path, .. } = &mut semantic_second.location else {
            unreachable!();
        };
        *path = "src/z.rs".to_string();
        semantic_second.site_id = "0000".to_string();

        let mut report_order = [semantic_first, semantic_second];
        report_order.sort_by(|left, right| left.site_id.cmp(&right.site_id));
        let mut diagnostics: Vec<_> = report_order.iter().collect();
        sort_terminal_diagnostics(&mut diagnostics);
        let rules: Vec<_> = diagnostic_groups(&diagnostics)
            .into_iter()
            .map(|group| group.rule)
            .collect();

        assert_eq!(rules, ["semantic-first", "semantic-second"]);
    }

    #[test]
    fn code_frame_neutralizes_osc52_and_c1_controls_but_preserves_tabs() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("src")).unwrap();
        std::fs::write(
            directory.path().join("src/lib.rs"),
            "\tpub fn risky() {} // \x1b]52;c;dGVzdA==\x07\u{009b}31m\n",
        )
        .unwrap();
        let cluster = DiagnosticCluster {
            sites: vec![DiagnosticSite {
                start_line: 1,
                end_line: 1,
                column: 2,
            }],
            path: "src/lib.rs",
            start_line: 1,
            end_line: 1,
        };
        let frame = owo_colors::with_override(false, || {
            build_code_frame(&directory.path().to_string_lossy(), &cluster).unwrap()
        });
        let rendered = frame.join("\n");

        assert!(rendered.contains('\t'));
        assert!(!rendered.contains("\x1b]52;"));
        assert!(!rendered.contains('\x07'));
        assert!(!rendered.contains('\u{009b}'));
    }

    #[test]
    fn verbose_render_lists_warning_details_without_a_code_frame() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("src")).unwrap();
        std::fs::write(directory.path().join("src/lib.rs"), "fn warning() {}\n").unwrap();
        let diagnostic =
            make_diagnostic("warning-rule", Severity::Warning, Category::Architecture, 1);
        let output = render(&make_result(directory.path(), 90, vec![diagnostic]), true);
        assert!(
            output
                .stderr
                .contains("⚠ Maintainability: warning-rule title")
        );
        assert!(output.stderr.contains("src/lib.rs:1"));
        assert!(!output.stderr.contains("┌"));
    }

    #[test]
    fn terminal_separates_scored_advisory_and_audit_inventories() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("src")).unwrap();
        std::fs::write(directory.path().join("src/lib.rs"), "fn inventory() {}\n").unwrap();

        let scored = make_diagnostic(
            "hardcoded-secrets",
            Severity::Warning,
            Category::Security,
            1,
        );
        let mut advisory = make_diagnostic(
            "excessive-clone",
            Severity::Warning,
            Category::Performance,
            2,
        );
        advisory.advisory = true;
        advisory.score_eligible = false;
        advisory.score_impact = ScoreImpact::Advisory;
        advisory.visible_on = vec![
            "terminal".to_string(),
            "sarif".to_string(),
            "mcp".to_string(),
        ];

        let mut unsafe_block = make_diagnostic(
            "unsafe-block-audit",
            Severity::Warning,
            Category::Security,
            3,
        );
        unsafe_block.advisory = true;
        unsafe_block.priority = Some("p3".to_string());
        unsafe_block.trust_tier = "audit-only".to_string();
        unsafe_block.score_eligible = false;
        unsafe_block.score_impact = ScoreImpact::Ineligible;
        unsafe_block.aggregation_policy = "audit-only".to_string();
        unsafe_block.visible_on = vec![
            "terminal".to_string(),
            "sarif".to_string(),
            "mcp".to_string(),
        ];

        let mut unsafe_dependency = unsafe_block.clone();
        unsafe_dependency.rule = "unsafe-dependency".to_string();
        unsafe_dependency.title = "Unsafe dependency exposure".to_string();
        unsafe_dependency.site_id = "unsafe-dependency:4".to_string();

        let report = make_result(
            directory.path(),
            90,
            vec![scored, advisory, unsafe_block, unsafe_dependency],
        );
        let output = render(&report, true);

        assert!(output.stderr.contains("1 scored issue"));
        assert!(output.stderr.contains("1 advisory finding"));
        assert!(output.stderr.contains("2 audit observations"));
        assert!(output.stderr.contains("Security › 1 warning"));
        assert!(
            output
                .stderr
                .contains("[audit observation · no score or CI impact]")
        );
        assert!(!output.stderr.contains("Security › 3 warnings"));
    }

    #[test]
    fn confirmed_rustsec_advisory_is_not_an_audit_observation() {
        let directory = tempfile::tempdir().unwrap();
        let mut advisory = make_diagnostic(
            "RUSTSEC-2026-0001",
            Severity::Error,
            Category::Dependencies,
            1,
        );
        advisory.provider = "rustsec".to_string();
        advisory.trust_tier = "unknown".to_string();
        advisory.score_eligible = false;
        advisory.score_impact = ScoreImpact::Ineligible;
        advisory.aggregation_policy = "unmapped".to_string();
        advisory.visible_on.push("ci-failure".to_string());
        advisory.visible_on.push("pr-comment".to_string());
        let output = render(&make_result(directory.path(), 90, vec![advisory]), true);

        assert!(output.stderr.contains("1 advisory finding"));
        assert!(output.stderr.contains("0 audit observations"));
        assert!(output.stderr.contains("Dependencies › 1 error"));
    }

    #[test]
    fn audit_only_volume_never_triggers_remediation_migration_advice() {
        let directory = tempfile::tempdir().unwrap();
        let mut diagnostics = Vec::new();
        for group in 0..5 {
            for occurrence in 0..10 {
                let mut diagnostic = make_diagnostic(
                    &format!("audit-rule-{group}"),
                    Severity::Warning,
                    Category::Security,
                    group * 10 + occurrence + 1,
                );
                diagnostic.advisory = true;
                diagnostic.trust_tier = "audit-only".to_string();
                diagnostic.score_eligible = false;
                diagnostic.score_impact = ScoreImpact::Ineligible;
                diagnostic.aggregation_policy = "audit-only".to_string();
                diagnostics.push(diagnostic);
            }
        }
        let output = render(&make_result(directory.path(), 100, diagnostics), false);
        assert!(output.stderr.contains("50 audit"));
        assert!(
            !output
                .stderr
                .contains("Migration-scale: sample before any sweep")
        );
    }

    #[test]
    fn clean_render_prints_success_and_score() {
        let directory = tempfile::tempdir().unwrap();
        let mut result = make_result(directory.path(), 100, vec![]);
        result.score_label = Some(ScoreLabel::Great);
        result.summary.score_label = Some(ScoreLabel::Great);
        let output = render(&result, false);
        assert!(output.stdout.contains("No issues found!"));
        assert!(output.stdout.contains("100 / 100 Great"));
        assert!(!output.stdout.contains("Docs:"));
        assert!(!output.stdout.contains("GitHub:"));
        assert!(!output.stderr.contains("0 scored issues"));
    }

    #[test]
    fn empty_changed_and_staged_scopes_use_reference_messages() {
        let directory = tempfile::tempdir().unwrap();
        let mut result = make_result(directory.path(), 100, vec![]);
        result.source_file_count = 0;
        result.reporting_scope = "changed".to_string();
        let changed = render(&result, false);
        assert!(changed.stderr.contains("No changed source files found."));
        assert!(!changed.stderr.contains("No Rust source files found."));

        result.reporting_scope = "staged".to_string();
        let staged = render(&result, false);
        assert!(staged.stderr.contains("No staged source files found."));
        assert!(!staged.stderr.contains("No Rust source files found."));
    }

    #[test]
    fn clean_render_explains_diagnostics_demoted_from_the_terminal_surface() {
        let directory = tempfile::tempdir().unwrap();
        let mut diagnostic =
            make_diagnostic("design-cleanup", Severity::Warning, Category::Style, 1);
        diagnostic.visible_on = vec!["score".to_string()];
        let output = render(&make_result(directory.path(), 99, vec![diagnostic]), false);

        assert!(output.stdout.contains(
            "No issues found! (1 demoted from the terminal surface: see config.surfaces.)"
        ));
        assert!(!output.stderr.contains("design-cleanup"));
    }

    #[test]
    fn hidden_warnings_are_not_counted_as_displayed_issues() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostic =
            make_diagnostic("warning-rule", Severity::Warning, Category::Architecture, 1);
        let result = make_result(directory.path(), 99, vec![diagnostic]);
        let output = build_terminal_output(
            &result,
            RenderOptions {
                verbose: false,
                show_warnings: false,
                selected_categories: &[],
                show_default_share: false,
                render_static_scan_summary: false,
                show_agent_guidance: false,
            },
        );
        assert!(output.stdout.contains("No issues found!"));
        assert!(!output.stderr.contains("warning-rule"));
    }

    #[test]
    fn score_bar_keeps_the_reference_width() {
        let bar = score_bar(50, Some(75), SCORE_BAR_WIDTH);
        let plain = strip_ansi(&bar);
        assert_eq!(plain.width(), SCORE_BAR_WIDTH);
        assert_eq!(
            plain.chars().filter(|character| *character == '█').count(),
            25
        );
        assert_eq!(
            plain.chars().filter(|character| *character == '▓').count(),
            13
        );
    }

    #[test]
    fn score_bar_width_is_clamped_to_the_terminal() {
        assert_eq!(score_bar_width(None), 50);
        assert_eq!(score_bar_width(Some(200)), 50);
        assert_eq!(score_bar_width(Some(40)), 28);
        assert_eq!(score_bar_width(Some(10)), 10);
    }

    #[test]
    fn animation_metadata_rebuilds_the_canonical_final_frames() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostics = vec![
            make_diagnostic("security-rule", Severity::Error, Category::Security, 1),
            make_diagnostic(
                "performance-rule",
                Severity::Warning,
                Category::Performance,
                2,
            ),
        ];
        let output = render(&make_result(directory.path(), 68, diagnostics), false);

        assert!(output.animation.category.is_none());
        assert!(output.animation.score.is_none());
    }

    #[test]
    fn animated_output_keeps_findings_and_score_on_their_original_streams() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostics = vec![
            make_diagnostic("security-rule", Severity::Error, Category::Security, 1),
            make_diagnostic(
                "performance-rule",
                Severity::Warning,
                Category::Performance,
                2,
            ),
        ];
        let output = render(&make_result(directory.path(), 68, diagnostics), false);
        let mut stderr = Vec::new();
        let mut stdout = Vec::new();
        let mut delays = Vec::new();
        write_animated_terminal_output(
            &output,
            &mut stderr,
            &mut stdout,
            true,
            true,
            &mut |delay| {
                delays.push(delay);
            },
        )
        .unwrap();
        let stderr = String::from_utf8(stderr).unwrap();
        let stdout = String::from_utf8(stdout).unwrap();

        assert!(stderr.contains("2 scored"));
        assert!(stderr.contains("Fix first"));
        assert!(!stderr.contains("/ 100"));
        assert!(stdout.contains("/ 100"));
        assert!(!stdout.contains("Rust Doctor"));
        assert!(!stdout.contains("2 scored"));
        assert!(!delays.contains(&SCORE_HEADER_ANIMATION_FRAME_DELAY));
        assert!(!delays.contains(&CATEGORY_COUNTUP_FRAME_DELAY));
        assert!(!stderr.contains("\x1b[2A"));
        assert!(!stdout.contains("\x1b[5A"));
    }

    #[test]
    fn redirected_stderr_stays_static_while_stdout_score_and_projection_animate() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostics = vec![
            make_diagnostic("security-rule", Severity::Error, Category::Security, 1),
            make_diagnostic(
                "performance-rule",
                Severity::Warning,
                Category::Performance,
                2,
            ),
        ];
        let output = render(&make_result(directory.path(), 68, diagnostics), false);
        let mut stderr = Vec::new();
        let mut stdout = Vec::new();
        let mut delays = Vec::new();
        write_animated_terminal_output(
            &output,
            &mut stderr,
            &mut stdout,
            false,
            true,
            &mut |delay| {
                delays.push(delay);
            },
        )
        .unwrap();
        let stderr = String::from_utf8(stderr).unwrap();
        let stdout = String::from_utf8(stdout).unwrap();

        assert_eq!(stderr, output.stderr);
        for rows in 1..=DISPLAY_CATEGORIES.len() {
            assert!(!stderr.contains(&format!("\x1b[{rows}A")));
            assert!(!stderr.contains(&format!("\x1b[{rows}B")));
        }
        assert!(!stdout.contains("\x1b[2A"));
        assert!(!delays.contains(&SCORE_HEADER_ANIMATION_FRAME_DELAY));
        assert!(!delays.contains(&CATEGORY_COUNTUP_FRAME_DELAY));
    }

    #[test]
    fn verbose_output_keeps_details_static_but_animates_the_final_projected_score() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostics = vec![make_diagnostic(
            "security-rule",
            Severity::Error,
            Category::Security,
            1,
        )];
        let output = render(&make_result(directory.path(), 68, diagnostics), true);
        let mut stderr = Vec::new();
        let mut stdout = Vec::new();
        let mut delays = Vec::new();
        write_animated_terminal_output(
            &output,
            &mut stderr,
            &mut stdout,
            false,
            false,
            &mut |delay| delays.push(delay),
        )
        .unwrap();
        let stderr = String::from_utf8(stderr).unwrap();
        let stdout = String::from_utf8(stdout).unwrap();

        assert_eq!(stderr, output.stderr);
        assert_eq!(
            stdout.matches("\x1b[2A").count(),
            SCORE_HEADER_ANIMATION_FRAME_COUNT as usize
        );
        assert!(!stdout.contains(&format!("\x1b[{SCORE_PROJECTION_BAR_ROWS_ABOVE_CURSOR}B")));
        assert!(stdout.contains('▓'));
        assert_eq!(
            delays
                .iter()
                .filter(|delay| **delay == SCORE_HEADER_ANIMATION_FRAME_DELAY)
                .count(),
            SCORE_HEADER_ANIMATION_FRAME_COUNT as usize
        );
        assert!(!delays.contains(&CATEGORY_COUNTUP_FRAME_DELAY));
        assert!(!delays.contains(&SCORE_PROJECTION_FRAME_DELAY));
    }

    #[test]
    fn category_animation_caps_its_partial_frame_budget() {
        let tallies = [CategoryTally {
            category_index: 0,
            errors: 70,
            warnings: 30,
            infos: 0,
        }];
        let total = category_unit_count(&tallies);
        let units_per_step = total.div_ceil(CATEGORY_COUNTUP_MAX_STEPS).max(1);
        assert_eq!((0..total).step_by(units_per_step).count(), 20);
        assert!((0..total).step_by(units_per_step).count() <= CATEGORY_COUNTUP_MAX_STEPS);
    }

    #[test]
    fn perfect_score_runs_the_rainbow_frames_and_omits_the_footer_when_clean() {
        let directory = tempfile::tempdir().unwrap();
        let mut result = make_result(directory.path(), 100, vec![]);
        result.score_label = Some(ScoreLabel::Great);
        result.summary.score_label = Some(ScoreLabel::Great);
        let output = render(&result, true);
        let mut stderr = Vec::new();
        let mut stdout = Vec::new();
        let mut delays = Vec::new();
        write_animated_terminal_output(
            &output,
            &mut stderr,
            &mut stdout,
            true,
            true,
            &mut |delay| {
                delays.push(delay);
            },
        )
        .unwrap();
        let stdout = String::from_utf8(stdout).unwrap();

        assert!(stdout.contains("100"));
        assert!(stdout.contains("\x1b[5A"));
        assert!(!stdout.contains("Docs:"));
        assert_eq!(
            delays
                .iter()
                .filter(|delay| **delay == PERFECT_SCORE_RAINBOW_FRAME_DELAY)
                .count(),
            PERFECT_SCORE_RAINBOW_FRAME_COUNT as usize
                + SCORE_HEADER_ANIMATION_FRAME_COUNT as usize
        );
    }

    #[test]
    fn rainbow_uses_the_reference_oklch_color_for_its_first_character() {
        assert_eq!(
            colored_rainbow_text("A", 0, 0),
            "\x1b[38;2;201;103;136mA\x1b[39m"
        );
    }

    #[test]
    fn perfect_score_omits_rainbow_ansi_when_stdout_colors_are_disabled() {
        owo_colors::with_override(false, || {
            let directory = tempfile::tempdir().unwrap();
            let mut result = make_result(directory.path(), 100, vec![]);
            result.score_label = Some(ScoreLabel::Great);
            result.summary.score_label = Some(ScoreLabel::Great);
            let output = render(&result, true);

            assert!(output.stdout.contains("100 / 100 Great"));
            assert!(output.stdout.contains(&"█".repeat(SCORE_BAR_WIDTH)));
            assert!(!output.stdout.contains("\x1b[38;2;"));

            let mut stderr = Vec::new();
            let mut stdout = Vec::new();
            write_animated_terminal_output(
                &output,
                &mut stderr,
                &mut stdout,
                true,
                true,
                &mut |_| {},
            )
            .unwrap();
            let stdout = String::from_utf8(stdout).unwrap();

            assert!(stdout.contains("100 / 100 Great"));
            assert!(stdout.contains(&"█".repeat(SCORE_BAR_WIDTH)));
            assert!(!stdout.contains("\x1b[38;2;"));
        });
    }

    #[test]
    fn osc8_file_urls_encode_paths_without_changing_the_visible_location() {
        let path = Path::new("/tmp/rust doctor/é.rs");
        assert_eq!(
            path_to_file_url(path),
            "file:///tmp/rust%20doctor/%C3%A9.rs"
        );
        let linked = format!(
            "\x1b]8;;{}\x1b\\src/lib.rs:2\x1b]8;;\x1b\\",
            path_to_file_url(Path::new("/tmp/src/lib.rs"))
        );
        assert!(linked.contains("src/lib.rs:2"));
        assert!(linked.starts_with("\x1b]8;;file:///tmp/src/lib.rs"));
    }

    #[test]
    fn incomplete_empty_render_never_claims_the_project_is_clean() {
        let directory = tempfile::tempdir().unwrap();
        let mut result = make_result(directory.path(), 100, vec![]);
        result.outcome = ReportOutcome::Partial;
        result.summary.score_authoritative = false;
        result.completeness.state = CompletenessState::Incomplete;
        result.completeness.failed_checks = 1;
        let output = render(&result, false);
        assert!(output.stderr.contains("results are incomplete"));
        assert!(!output.stdout.contains("No issues found!"));
        assert!(!output.stdout.contains("100 / 100"));
        assert!(output.stdout.contains("Score not shown:"));
        assert!(
            output
                .stdout
                .lines()
                .all(|line| strip_ansi(line).width() <= OUTPUT_MEASURE_WIDTH)
        );
    }

    #[test]
    fn multi_project_lines_use_only_visible_errors_and_requested_warnings() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostics = vec![
            make_diagnostic("error-rule", Severity::Error, Category::Correctness, 1),
            make_diagnostic("warning-rule", Severity::Warning, Category::Performance, 2),
            make_diagnostic("info-rule", Severity::Info, Category::Style, 3),
        ];
        let mut result = make_result(directory.path(), 80, diagnostics.clone());
        let project = ProjectReport {
            cargo_package_id: "path+file:///tmp/first#first@1.0.0".to_string(),
            package_root: "/tmp/first".to_string(),
            targets: vec![],
            framework_capabilities: vec![],
            framework_gates: vec![],
            planned_files: vec![],
            analyzed_files: vec![],
            checks: vec![],
            skipped_reasons: vec![],
            completeness: result.completeness.clone(),
            score: Some(80),
            score_authoritative: true,
            dimensions: Vec::new(),
            elapsed_ms: 700,
            diagnostics,
        };
        let mut second = project.clone();
        second.cargo_package_id = "path+file:///tmp/second#second@1.0.0".to_string();
        second.package_root = "/tmp/second".to_string();
        result.projects = vec![project, second];

        let mut warnings_hidden = String::new();
        render_project_summary(&mut warnings_hidden, &result, false, false);
        assert!(warnings_hidden.contains("first"));
        assert!(warnings_hidden.contains("second"));
        assert!(warnings_hidden.contains("1 error"));
        assert!(!warnings_hidden.contains("warning"));
        assert!(!warnings_hidden.contains("info"));

        let mut warnings_shown = String::new();
        render_project_summary(&mut warnings_shown, &result, true, false);
        assert!(warnings_shown.contains("1 error"));
        assert!(warnings_shown.contains("1 warning"));
        assert!(!warnings_shown.contains("info"));
    }

    #[test]
    fn multi_project_headline_is_hidden_when_another_required_project_failed() {
        let directory = tempfile::tempdir().unwrap();
        let mut result = make_result(directory.path(), 90, vec![]);
        result.outcome = ReportOutcome::Partial;
        result.completeness.state = CompletenessState::Incomplete;
        result.completeness.failed_checks = 1;
        result.completeness.score_authoritative = false;
        result.summary.score_authoritative = false;
        result.projects = vec![
            make_project_report(&result, "healthy", Some(90), true, vec![]),
            make_project_report(&result, "worst", Some(68), true, vec![]),
            make_project_report(&result, "failed", None, false, vec![]),
        ];

        let output = render(&result, false);

        assert_eq!(
            multi_project_headline_project(&result).map(project_name),
            Some("worst".to_string())
        );
        assert!(!output.stdout.contains("/ 100"));
        assert!(output.stdout.contains("Score unavailable"));
        assert!(output.stdout.contains("Score not shown"));
        assert!(output.stdout.contains("failed"));
        assert!(output.stdout.contains("no score"));
    }

    #[test]
    fn multi_project_projection_uses_displayed_rules_to_rescore_the_headline_project() {
        let directory = tempfile::tempdir().unwrap();
        // Score Core V2 only counts catalog rules, so the projection needs real
        // score-eligible identifiers rather than synthetic names.
        const OTHER_RULES: [&str; 2] = ["missing-msrv", "msrv-outdated"];
        const WORST_RULES: [&str; 3] = [
            "clippy::almost_swapped",
            "clippy::await_holding_lock",
            "clippy::eq_op",
        ];
        let other_diagnostics: Vec<_> = OTHER_RULES
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                make_diagnostic(
                    rule,
                    Severity::Error,
                    Category::Security,
                    u32::try_from(index).unwrap() + 1,
                )
            })
            .collect();
        let mut worst_diagnostics: Vec<_> = WORST_RULES
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                make_diagnostic(
                    rule,
                    Severity::Error,
                    Category::Security,
                    u32::try_from(index).unwrap() + 1,
                )
            })
            .collect();
        let mut global = make_diagnostic(
            "clippy::invalid_regex",
            Severity::Error,
            Category::Security,
            20,
        );
        global.ownership = DiagnosticOwnership::Workspace;
        worst_diagnostics.push(global.clone());

        let mut all_diagnostics = other_diagnostics.clone();
        all_diagnostics.extend(worst_diagnostics.clone());
        let mut result = make_result(directory.path(), 68, all_diagnostics);
        let mut other_project_diagnostics = other_diagnostics;
        other_project_diagnostics.push(global);
        result.projects = vec![
            make_project_report(&result, "other", Some(90), true, other_project_diagnostics),
            make_project_report(&result, "worst", Some(68), true, worst_diagnostics),
        ];

        let headline = multi_project_headline_project(&result).unwrap();
        assert!(
            headline
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.ownership == DiagnosticOwnership::Workspace)
        );
        let scoped: Vec<_> = headline.diagnostics.iter().collect();
        let mut all: Vec<_> = result.diagnostics.iter().collect();
        sort_terminal_diagnostics(&mut all);
        let root_causes = crate::ordering::root_cause_groups(all.iter().copied());
        let top_root_causes: Vec<_> = root_causes
            .into_iter()
            .filter(|group| group.score_impact == ScoreImpact::Scored)
            .take(crate::ordering::PROTECTED_ROOT_CAUSE_GROUPS)
            .collect();
        assert_eq!(top_root_causes.len(), 3);
        let all_top_keys: HashSet<_> = top_root_causes
            .iter()
            .map(|group| group.key.clone())
            .collect();
        let expected_projection = projected_score(&scoped, &all_top_keys, &[]).unwrap();
        let workspace_union_projection = projected_score(&all, &all_top_keys, &[]).unwrap();
        assert_ne!(expected_projection, workspace_union_projection);

        let output = render(&result, false);
        let displayed_positions: Vec<_> = top_root_causes
            .iter()
            .map(|group| output.stderr.find(&group.title).unwrap())
            .collect();
        assert!(
            displayed_positions
                .windows(2)
                .all(|positions| positions[0] < positions[1])
        );
        assert!(
            output
                .stdout
                .contains(&format!("(+{})", expected_projection - 68))
        );
        assert!(
            !output
                .stdout
                .contains(&format!("(+{})", workspace_union_projection - 68))
        );
    }

    #[test]
    fn project_name_handles_modern_and_legacy_cargo_package_ids() {
        let directory = tempfile::tempdir().unwrap();
        let result = make_result(directory.path(), 100, vec![]);
        let project = ProjectReport {
            cargo_package_id: "path+file:///tmp/rust-doctor#0.2.0".to_string(),
            package_root: ".".to_string(),
            targets: vec![],
            framework_capabilities: vec![],
            framework_gates: vec![],
            planned_files: vec![],
            analyzed_files: vec![],
            checks: vec![],
            skipped_reasons: vec![],
            completeness: result.completeness,
            score: Some(100),
            score_authoritative: true,
            dimensions: Vec::new(),
            elapsed_ms: 0,
            diagnostics: vec![],
        };
        assert_eq!(project_name(&project), "rust-doctor");

        let mut registry = project.clone();
        registry.cargo_package_id =
            "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0".to_string();
        assert_eq!(project_name(&registry), "serde");

        let mut legacy = project;
        legacy.cargo_package_id = "legacy 0.1.0 (path+file:///tmp/legacy)".to_string();
        assert_eq!(project_name(&legacy), "legacy");
    }

    #[test]
    fn non_interactive_render_includes_agent_guidance() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostic =
            make_diagnostic("warning-rule", Severity::Warning, Category::Architecture, 1);
        let result = make_result(directory.path(), 90, vec![diagnostic]);
        let output = build_terminal_output(
            &result,
            RenderOptions {
                verbose: false,
                show_warnings: true,
                selected_categories: &[],
                show_default_share: false,
                render_static_scan_summary: true,
                show_agent_guidance: true,
            },
        );
        assert!(output.stdout.contains("Agent guidance"));
        assert!(
            output
                .stdout
                .contains("Treat Rust Doctor diagnostics as starting hypotheses")
        );

        let verbose = build_terminal_output(
            &result,
            RenderOptions {
                verbose: true,
                show_warnings: true,
                selected_categories: &[],
                show_default_share: false,
                render_static_scan_summary: true,
                show_agent_guidance: true,
            },
        );
        assert!(
            verbose
                .stderr
                .contains("Curl with no cache & follow the canonical fix")
        );
        assert!(!verbose.stderr.contains("Learn more:"));
    }

    fn strip_ansi(value: &str) -> String {
        let mut output = String::new();
        let mut characters = value.chars().peekable();
        while let Some(character) = characters.next() {
            if character == '\u{1b}' && characters.peek() == Some(&'[') {
                let _ = characters.next();
                for next in characters.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                output.push(character);
            }
        }
        output
    }
}
