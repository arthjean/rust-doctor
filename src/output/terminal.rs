use crate::diagnostics::{
    CanonicalDiagnostic, DiagnosticLocation, DimensionScores, ReportV1, Severity,
};
use owo_colors::{OwoColorize, Stream};
use std::collections::HashMap;
use unicode_width::UnicodeWidthStr;

use super::score::{SCORE_GOOD_THRESHOLD, SCORE_OK_THRESHOLD};

const SCORE_BAR_WIDTH: usize = 40;

/// Short caption framing the aggregate score as a direction, not a precise
/// grade (US-012). The per-dimension scores below tell you *where* to act.
const COMPASS_CAPTION: &str = "compass, not thermometer — see per-dimension scores";

/// Render full scan results to stdout/stderr.
pub fn render_terminal(
    result: &ReportV1,
    pass_timings: &[(String, std::time::Duration)],
    verbose: bool,
    show_warnings: bool,
) {
    if let Some(baseline) = &result.baseline {
        if baseline.baseline_degraded {
            eprintln!(
                "Baseline degraded to files scope: {}",
                baseline
                    .degraded_reason
                    .as_deref()
                    .unwrap_or("unknown reason")
            );
        } else {
            eprintln!(
                "Baseline {}: {} introduced, {} fixed, {} cross-file match(es)",
                baseline.base_commit,
                baseline.new_count,
                baseline.fixed_count,
                baseline.cross_file_match_count
            );
        }
    }
    if result.score.is_some() && !result.summary.score_authoritative {
        eprintln!("Score is non-authoritative because required analysis is incomplete");
    }

    // Handle zero files — still show diagnostics (e.g., audit/machete findings)
    if result.source_file_count == 0 && result.diagnostics.is_empty() {
        eprintln!(
            "{}",
            "No Rust source files found".if_supports_color(Stream::Stderr, |t| t.yellow())
        );
        return;
    }

    // Print diagnostics grouped by severity
    let diagnostics: Vec<&CanonicalDiagnostic> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            (show_warnings || diagnostic.severity != Severity::Warning)
                && diagnostic
                    .visible_on
                    .iter()
                    .any(|value| value == "terminal")
        })
        .collect();
    if !diagnostics.is_empty() {
        print_diagnostics(&diagnostics, verbose);
        eprintln!();
    }
    if verbose && !result.audit.suppressed_security.is_empty() {
        eprintln!(
            "Security audit: {} finding(s) suppressed by project configuration",
            result.audit.suppressed_security.len()
        );
        for diagnostic in &result.audit.suppressed_security {
            eprintln!("  {}: {}", diagnostic.rule, diagnostic.message);
        }
        eprintln!();
    }

    // Print score box
    print_score_box(result);

    // Print pass timings in verbose mode
    if verbose && !pass_timings.is_empty() {
        print_pass_timings(pass_timings);
    }
}

// ── Box layout helpers ───────────────────────────────────────────────────

fn dim(s: &str) -> String {
    format!("{}", s.if_supports_color(Stream::Stdout, |t| t.dimmed()))
}

fn pad_line(inner_width: usize, content: &str, plain_len: usize) -> String {
    let padding = inner_width.saturating_sub(plain_len + 2);
    format!(
        "  {} {} {}{}",
        dim("│"),
        content,
        " ".repeat(padding),
        dim("│")
    )
}

fn empty_line(inner_width: usize) -> String {
    format!("  {} {}{}", dim("│"), " ".repeat(inner_width - 2), dim("│"))
}

// ── Score box section renderers ──────────────────────────────────────────

/// Render the doctor face and brand header.
fn render_header(inner_width: usize, score: u32) {
    let (eyes, mouth) = if score >= SCORE_GOOD_THRESHOLD {
        ("◠ ◠", " ▽ ")
    } else if score >= SCORE_OK_THRESHOLD {
        ("• •", " ─ ")
    } else {
        ("x x", " △ ")
    };

    println!("{}", pad_line(inner_width, "┌─────┐", 7));
    println!(
        "{}",
        pad_line(
            inner_width,
            &format!("│ {} │", colorize_by_score(eyes, score)),
            7
        )
    );
    println!(
        "{}",
        pad_line(
            inner_width,
            &format!("│ {} │", colorize_by_score(mouth, score)),
            7
        )
    );
    println!("{}", pad_line(inner_width, "└─────┘", 7));
    println!(
        "{}",
        pad_line(
            inner_width,
            &format!(
                "{}",
                "rust-doctor".if_supports_color(Stream::Stdout, |t| t.bold())
            ),
            11,
        )
    );
    println!("{}", empty_line(inner_width));
}

/// Render the dimension score bars.
fn render_dimension_bars(inner_width: usize, ds: &DimensionScores, dim_text: &str) {
    let colored_dim = format!(
        "{}: {}  {}: {}  {}: {}  {}: {}  {}: {}",
        "Security".if_supports_color(Stream::Stdout, |t| t.dimmed()),
        colorize_by_score(&ds.security.to_string(), ds.security),
        "Reliability".if_supports_color(Stream::Stdout, |t| t.dimmed()),
        colorize_by_score(&ds.reliability.to_string(), ds.reliability),
        "Maintainability".if_supports_color(Stream::Stdout, |t| t.dimmed()),
        colorize_by_score(&ds.maintainability.to_string(), ds.maintainability),
        "Performance".if_supports_color(Stream::Stdout, |t| t.dimmed()),
        colorize_by_score(&ds.performance.to_string(), ds.performance),
        "Dependencies".if_supports_color(Stream::Stdout, |t| t.dimmed()),
        colorize_by_score(&ds.dependencies.to_string(), ds.dependencies),
    );
    println!("{}", pad_line(inner_width, &colored_dim, dim_text.width()));
    println!("{}", empty_line(inner_width));
}

/// Render the stats footer (error/warning counts, skipped passes).
fn render_stats_footer(
    inner_width: usize,
    result: &ReportV1,
    stats: &str,
    skipped_text: Option<&str>,
) {
    let colored_info_part = if result.info_count > 0 {
        format!(
            "  {} {} info(s)",
            "ℹ".if_supports_color(Stream::Stdout, |t| t.cyan()),
            result.info_count
        )
    } else {
        String::new()
    };
    let colored_stats = format!(
        "{} {} error(s)  {} {} warning(s){colored_info_part}  {} files  {:.1}s",
        colorize_by_score(
            if result.error_count > 0 { "✗" } else { "✓" },
            if result.error_count > 0 { 0 } else { 100 }
        ),
        result.error_count,
        colorize_by_score(
            if result.warning_count > 0 {
                "⚠"
            } else {
                "✓"
            },
            if result.warning_count > 0 { 49 } else { 100 }
        ),
        result.warning_count,
        result.source_file_count,
        result.elapsed,
    );
    println!("{}", pad_line(inner_width, &colored_stats, stats.width()));

    if let Some(text) = skipped_text {
        println!("{}", empty_line(inner_width));
        let colored_skip = format!(
            "{} {} pass(es) skipped — run: {}",
            "⊘".if_supports_color(Stream::Stdout, |t| t.yellow()),
            result.skipped_passes.len(),
            "rust-doctor --install-deps".if_supports_color(Stream::Stdout, |t| t.bold()),
        );
        println!("{}", pad_line(inner_width, &colored_skip, text.width()));
    }
}

// ── Main score box ───────────────────────────────────────────────────────

/// Print the ASCII doctor box with score.
fn print_score_box(result: &ReportV1) {
    let (Some(score), Some(label), Some(ds)) = (
        result.score,
        result.score_label,
        result.dimension_scores.as_ref(),
    ) else {
        return;
    };

    // Build content lines for width calculation
    let score_text = format!("{score} / 100  {label}");
    let bar = build_score_bar(score);
    let dim_text = format!(
        "Security: {}  Reliability: {}  Maintainability: {}  Performance: {}  Dependencies: {}",
        ds.security, ds.reliability, ds.maintainability, ds.performance, ds.dependencies
    );
    let info_part = if result.info_count > 0 {
        format!("  ℹ {} info(s)", result.info_count)
    } else {
        String::new()
    };
    let stats = format!(
        "{} {} error(s)  {} {} warning(s){info_part}  {} files  {:.1}s",
        if result.error_count > 0 { "✗" } else { "✓" },
        result.error_count,
        if result.warning_count > 0 {
            "⚠"
        } else {
            "✓"
        },
        result.warning_count,
        result.source_file_count,
        result.elapsed,
    );
    let skipped_text = if result.skipped_passes.is_empty() {
        None
    } else {
        Some(format!(
            "⊘ {} pass(es) skipped — run: rust-doctor --install-deps",
            result.skipped_passes.len()
        ))
    };

    // Calculate box width
    let mut widths = vec![
        7_usize,
        score_text.width(),
        COMPASS_CAPTION.width(),
        bar.plain.width(),
        dim_text.width(),
        stats.width(),
    ];
    if let Some(ref text) = skipped_text {
        widths.push(text.width());
    }
    let max_width = widths.into_iter().max().unwrap_or(40).max(40);
    let iw = max_width + 2;

    // Render box
    println!("  {}{}{}", dim("┌"), dim(&"─".repeat(iw)), dim("┐"));

    render_header(iw, score);

    // Score + compass caption + bar
    let colored_score = colorize_by_score(&score_text, score);
    println!("{}", pad_line(iw, &colored_score, score_text.width()));
    println!(
        "{}",
        pad_line(iw, &dim(COMPASS_CAPTION), COMPASS_CAPTION.width())
    );
    println!("{}", empty_line(iw));
    println!("{}", pad_line(iw, &bar.colored, bar.plain.width()));
    println!("{}", empty_line(iw));

    render_dimension_bars(iw, ds, &dim_text);
    render_stats_footer(iw, result, &stats, skipped_text.as_deref());

    println!("  {}{}{}", dim("└"), dim(&"─".repeat(iw)), dim("┘"));
}

struct ScoreBar {
    plain: String,
    colored: String,
}

fn build_score_bar(score: u32) -> ScoreBar {
    let filled = ((f64::from(score) / 100.0) * SCORE_BAR_WIDTH as f64).round() as usize;
    let empty = SCORE_BAR_WIDTH - filled;

    let filled_str = "█".repeat(filled);
    let empty_str = "░".repeat(empty);

    let plain = format!("{filled_str}{empty_str}");
    let dimmed_empty = empty_str.if_supports_color(Stream::Stdout, |t| t.dimmed());
    let colored = format!("{}{}", colorize_by_score(&filled_str, score), dimmed_empty);

    ScoreBar { plain, colored }
}

fn colorize_by_score(text: &str, score: u32) -> String {
    if score >= SCORE_GOOD_THRESHOLD {
        format!("{}", text.if_supports_color(Stream::Stdout, |t| t.green()))
    } else if score >= SCORE_OK_THRESHOLD {
        format!("{}", text.if_supports_color(Stream::Stdout, |t| t.yellow()))
    } else {
        format!("{}", text.if_supports_color(Stream::Stdout, |t| t.red()))
    }
}

/// Print per-pass timing table to stderr (verbose mode only).
fn print_pass_timings(timings: &[(String, std::time::Duration)]) {
    eprintln!();
    eprintln!(
        "{}",
        "Pass timings:".if_supports_color(Stream::Stderr, |t| t.dimmed())
    );
    // Find the longest pass name for alignment
    let max_name_len = timings
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    for (name, duration) in timings {
        eprintln!(
            "  {:<width$}  {:.1}s",
            name.if_supports_color(Stream::Stderr, |t| t.dimmed()),
            duration.as_secs_f64(),
            width = max_name_len,
        );
    }
}

/// A grouped diagnostic: a rule with its occurrence count and representative info.
struct DiagGroup<'a> {
    rule: &'a str,
    severity: Severity,
    message: &'a str,
    help: Option<&'a str>,
    count: usize,
    occurrences: Vec<DiagOccurrence<'a>>,
}

struct DiagOccurrence<'a> {
    file_path: std::borrow::Cow<'a, str>,
    line: Option<u32>,
    column: Option<u32>,
}

/// Print diagnostics grouped by rule, errors first.
#[allow(clippy::too_many_lines)]
fn print_diagnostics(diagnostics: &[&CanonicalDiagnostic], verbose: bool) {
    // Group by rule — borrow from diagnostics to avoid cloning strings
    let mut groups: HashMap<&str, DiagGroup<'_>> = HashMap::new();
    for &d in diagnostics {
        let entry = groups.entry(&d.rule).or_insert_with(|| DiagGroup {
            rule: &d.rule,
            severity: d.severity,
            message: &d.message,
            help: d.help.as_deref(),
            count: 0,
            occurrences: vec![],
        });
        entry.count += 1;
        let (file_path, line, column) = match &d.location {
            DiagnosticLocation::Source { path, range } => (
                std::borrow::Cow::Borrowed(path.as_str()),
                Some(range.start.line),
                Some(range.start.column),
            ),
            DiagnosticLocation::Project => (std::borrow::Cow::Borrowed("<project>"), None, None),
        };
        entry.occurrences.push(DiagOccurrence {
            file_path,
            line,
            column,
        });
    }

    // Sort: errors first, then warnings, then info
    let mut sorted: Vec<_> = groups.into_values().collect();
    sorted.sort_by(|a, b| {
        let severity_ord = |s: &Severity| match s {
            Severity::Error => 0,
            Severity::Warning => 1,
            Severity::Info => 2,
        };
        severity_ord(&a.severity)
            .cmp(&severity_ord(&b.severity))
            .then(a.rule.cmp(b.rule))
    });

    for group in &sorted {
        let symbol = match group.severity {
            Severity::Error => format!("{}", "✗".if_supports_color(Stream::Stderr, |t| t.red())),
            Severity::Warning => {
                format!("{}", "⚠".if_supports_color(Stream::Stderr, |t| t.yellow()))
            }
            Severity::Info => {
                format!("{}", "ℹ".if_supports_color(Stream::Stderr, |t| t.cyan()))
            }
        };

        eprint!("  {symbol} {}", group.message);
        if group.count > 1 {
            eprint!(
                " {}",
                format!("({})", group.count).if_supports_color(Stream::Stderr, |t| t.dimmed())
            );
        }
        // Mark syn-only heuristic findings so the user calibrates confidence (US-013).
        if diagnostics.iter().any(|diagnostic| {
            diagnostic.rule == group.rule && diagnostic.tags.iter().any(|tag| tag == "heuristic")
        }) {
            eprint!(
                " {}",
                "~heuristic".if_supports_color(Stream::Stderr, |t| t.dimmed())
            );
        }
        eprintln!();

        if let Some(ref help) = group.help {
            eprintln!(
                "    {}",
                help.if_supports_color(Stream::Stderr, |t| t.dimmed())
            );
        }

        if verbose {
            for occ in &group.occurrences {
                let location: std::borrow::Cow<'_, str> = match (occ.line, occ.column) {
                    (Some(l), Some(c)) => format!("{}:{}:{}", occ.file_path, l, c).into(),
                    (Some(l), None) => format!("{}:{}", occ.file_path, l).into(),
                    _ => std::borrow::Cow::Borrowed(occ.file_path.as_ref()),
                };
                eprintln!(
                    "    {}",
                    location.if_supports_color(Stream::Stderr, |t| t.dimmed())
                );
            }
        }

        eprintln!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{
        AuditMetadata, CanonicalDiagnostic, CompletenessState, DiagnosticLocation, DimensionScores,
        ReportCompleteness, ReportOutcome, ReportSummary, ScanMode, ScoreLabel, SourcePosition,
        SourceRange,
    };
    use std::path::Path;

    fn make_result(
        score: u32,
        diagnostics: Vec<CanonicalDiagnostic>,
        errors: usize,
        warnings: usize,
        infos: usize,
    ) -> ReportV1 {
        let mut report = ReportV1::failure(Path::new("."), ScanMode::Full, "test", "");
        report.outcome = ReportOutcome::Findings;
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
        };
        report.diagnostics = diagnostics;
        report.summary = ReportSummary {
            score: Some(score),
            score_label: Some(ScoreLabel::Great),
            error_count: errors,
            warning_count: warnings,
            info_count: infos,
            diagnostic_count: errors + warnings + infos,
            score_authoritative: true,
        };
        report.audit = AuditMetadata::default();
        report.score = Some(score);
        report.score_label = Some(ScoreLabel::Great);
        report.dimension_scores = Some(DimensionScores {
            security: score,
            reliability: score,
            maintainability: score,
            performance: score,
            dependencies: score,
        });
        report.source_file_count = 10;
        report.elapsed = 0.5;
        report.error_count = errors;
        report.warning_count = warnings;
        report.info_count = infos;
        report.error = None;
        report
    }

    fn make_diagnostic(rule: &str, severity: Severity) -> CanonicalDiagnostic {
        CanonicalDiagnostic {
            provider: "rust-doctor".to_string(),
            rule: rule.to_string(),
            title: rule.to_string(),
            category: crate::diagnostics::Category::ErrorHandling,
            severity,
            message: format!("test message for {rule}"),
            help: Some(format!("fix {rule}")),
            url: String::new(),
            tags: vec!["heuristic".to_string()],
            analysis_kind: "synast".to_string(),
            confidence: "medium".to_string(),
            original_level: severity.to_string(),
            ownership: crate::diagnostics::DiagnosticOwnership::Package {
                package_id: "fixture".to_string(),
            },
            source_surface: crate::diagnostics::SourceSurface::Library,
            location: DiagnosticLocation::Source {
                path: "src/lib.rs".to_string(),
                range: SourceRange {
                    start: SourcePosition {
                        line: 10,
                        column: 5,
                        byte_offset: None,
                    },
                    end: SourcePosition {
                        line: 10,
                        column: 5,
                        byte_offset: None,
                    },
                },
            },
            related_locations: vec![],
            macro_expansion: None,
            fixes: vec![],
            visible_on: vec!["terminal".to_string()],
            site_id: rule.to_string(),
            baseline_key: rule.to_string(),
            namespace_fallback: false,
        }
    }

    // --- build_score_bar ---

    #[test]
    fn test_score_bar_full() {
        let bar = build_score_bar(100);
        assert_eq!(bar.plain.chars().count(), SCORE_BAR_WIDTH);
        assert!(!bar.plain.contains('░'));
    }

    #[test]
    fn test_score_bar_empty() {
        let bar = build_score_bar(0);
        assert_eq!(bar.plain.chars().count(), SCORE_BAR_WIDTH);
        assert!(!bar.plain.contains('█'));
    }

    #[test]
    fn test_score_bar_half() {
        let bar = build_score_bar(50);
        let filled = bar.plain.chars().filter(|&c| c == '█').count();
        let empty = bar.plain.chars().filter(|&c| c == '░').count();
        assert_eq!(filled + empty, SCORE_BAR_WIDTH);
        assert_eq!(filled, 20);
    }

    // --- colorize_by_score ---

    #[test]
    fn test_colorize_high_score_contains_text() {
        // NO_COLOR may suppress ANSI codes; just verify the text is present
        let result = colorize_by_score("test", 90);
        assert!(result.contains("test"));
    }

    #[test]
    fn test_colorize_low_score_contains_text() {
        let result = colorize_by_score("test", 20);
        assert!(result.contains("test"));
    }

    // --- render_terminal (integration) ---

    #[test]
    fn test_render_terminal_with_diagnostics() {
        let diags = vec![
            make_diagnostic("rule-a", Severity::Error),
            make_diagnostic("rule-b", Severity::Warning),
        ];
        let result = make_result(70, diags, 1, 1, 0);
        // Should not panic — output goes to stdout/stderr
        render_terminal(&result, &[], false, true);
    }

    #[test]
    fn test_render_terminal_zero_diagnostics() {
        let result = make_result(100, vec![], 0, 0, 0);
        render_terminal(&result, &[], false, true);
    }

    #[test]
    fn test_render_terminal_verbose() {
        let diags = vec![make_diagnostic("rule-a", Severity::Warning)];
        let result = make_result(80, diags, 0, 1, 0);
        render_terminal(&result, &[], true, true);
    }

    #[test]
    fn test_render_terminal_with_skipped_passes() {
        let mut result = make_result(90, vec![], 0, 0, 0);
        result.skipped_passes = vec!["cargo-audit".to_string(), "cargo-deny".to_string()];
        render_terminal(&result, &[], false, true);
    }

    #[test]
    fn test_render_terminal_zero_files_no_diagnostics() {
        let mut result = make_result(100, vec![], 0, 0, 0);
        result.source_file_count = 0;
        // Should print "No Rust source files found" and return early
        render_terminal(&result, &[], false, true);
    }

    // --- print_diagnostics grouping ---

    #[test]
    fn test_print_diagnostics_groups_by_rule() {
        let diags = [
            make_diagnostic("same-rule", Severity::Warning),
            make_diagnostic("same-rule", Severity::Warning),
            make_diagnostic("other-rule", Severity::Error),
        ];
        // Should not panic; diagnostics are grouped by rule
        let refs: Vec<_> = diags.iter().collect();
        print_diagnostics(&refs, false);
    }

    #[test]
    fn test_print_diagnostics_sorts_errors_first() {
        let diags = [
            make_diagnostic("warn-rule", Severity::Warning),
            make_diagnostic("info-rule", Severity::Info),
            make_diagnostic("err-rule", Severity::Error),
        ];
        // Should print errors, then warnings, then info
        let refs: Vec<_> = diags.iter().collect();
        print_diagnostics(&refs, true);
    }
}
