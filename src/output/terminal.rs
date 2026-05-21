use crate::cli::CategoryFilter;
use crate::diagnostics::{Category, ScanResult, Severity};
use owo_colors::{OwoColorize, Stream};
use std::collections::HashMap;
use unicode_width::UnicodeWidthStr;

use super::score::{SCORE_GOOD_THRESHOLD, SCORE_OK_THRESHOLD};

const SCORE_BAR_WIDTH: usize = 40;

/// Render full scan results to stdout/stderr.
pub fn render_terminal(
    result: &ScanResult,
    verbose: bool,
    category_filter: Option<CategoryFilter>,
) {
    // Handle zero files — still show diagnostics (e.g., audit/machete findings)
    if result.source_file_count == 0 && result.diagnostics.is_empty() {
        eprintln!(
            "{}",
            "No Rust source files found".if_supports_color(Stream::Stderr, |t| t.yellow())
        );
        return;
    }

    // Print diagnostics grouped by severity
    if !result.diagnostics.is_empty() {
        print_diagnostics(&result.diagnostics, verbose, category_filter);
        eprintln!();
    }

    // Print score box
    print_score_box(result);
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
fn render_dimension_bars(inner_width: usize, result: &ScanResult, dim_text: &str) {
    let ds = &result.dimension_scores;
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
    result: &ScanResult,
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
        result.elapsed.as_secs_f64(),
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
fn print_score_box(result: &ScanResult) {
    let score = result.score;
    let label = &result.score_label;

    // Build content lines for width calculation
    let score_text = format!("{score} / 100  {label}");
    let bar = build_score_bar(score);
    let ds = &result.dimension_scores;
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
        result.elapsed.as_secs_f64(),
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

    // Score + bar
    let colored_score = colorize_by_score(&score_text, score);
    println!("{}", pad_line(iw, &colored_score, score_text.width()));
    println!("{}", empty_line(iw));
    println!("{}", pad_line(iw, &bar.colored, bar.plain.width()));
    println!("{}", empty_line(iw));

    render_dimension_bars(iw, result, &dim_text);
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

/// A grouped diagnostic: a rule with its occurrence count and representative info.
struct DiagGroup<'a> {
    rule: &'a str,
    severity: Severity,
    category: &'a Category,
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

/// Match a `CategoryFilter` variant to a `Category` variant.
const fn matches_category(filter: CategoryFilter, category: &Category) -> bool {
    matches!(
        (filter, category),
        (CategoryFilter::ErrorHandling, Category::ErrorHandling)
            | (CategoryFilter::Performance, Category::Performance)
            | (CategoryFilter::Security, Category::Security)
            | (CategoryFilter::Correctness, Category::Correctness)
            | (CategoryFilter::Architecture, Category::Architecture)
            | (CategoryFilter::Dependencies, Category::Dependencies)
            | (CategoryFilter::Async, Category::Async)
            | (CategoryFilter::Framework, Category::Framework)
            | (CategoryFilter::Cargo, Category::Cargo)
            | (CategoryFilter::Style, Category::Style)
    )
}

/// Print diagnostics grouped by rule, errors first.
#[allow(clippy::too_many_lines)]
fn print_diagnostics(
    diagnostics: &[crate::diagnostics::Diagnostic],
    verbose: bool,
    category_filter: Option<CategoryFilter>,
) {
    // Group by rule — borrow from diagnostics to avoid cloning strings
    let mut groups: HashMap<&str, DiagGroup<'_>> = HashMap::new();
    for d in diagnostics {
        if let Some(cf) = category_filter {
            if !matches_category(cf, &d.category) {
                continue;
            }
        }
        let entry = groups.entry(&d.rule).or_insert_with(|| DiagGroup {
            rule: &d.rule,
            severity: d.severity,
            category: &d.category,
            message: &d.message,
            help: d.help.as_deref(),
            count: 0,
            occurrences: vec![],
        });
        entry.count += 1;
        entry.occurrences.push(DiagOccurrence {
            file_path: d.file_path.to_string_lossy(),
            line: d.line,
            column: d.column,
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
        eprintln!();

        if let Some(ref help) = group.help {
            eprintln!(
                "    {}",
                help.if_supports_color(Stream::Stderr, |t| t.dimmed())
            );
        }

        if verbose || category_filter.is_some_and(|cf| matches_category(cf, group.category)) {
            // When --category is set, only show verbose for matching groups
            let show_verbose =
                category_filter.is_none_or(|cf| matches_category(cf, group.category));

            if show_verbose {
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
        }

        eprintln!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Category, Diagnostic, DimensionScores, ScoreLabel, Severity};
    use std::path::PathBuf;
    use std::time::Duration;

    fn make_result(
        score: u32,
        diagnostics: Vec<Diagnostic>,
        errors: usize,
        warnings: usize,
        infos: usize,
    ) -> ScanResult {
        ScanResult {
            diagnostics,
            score,
            score_label: ScoreLabel::Great,
            dimension_scores: DimensionScores {
                security: score,
                reliability: score,
                maintainability: score,
                performance: score,
                dependencies: score,
            },
            source_file_count: 10,
            elapsed: Duration::from_millis(500),
            skipped_passes: vec![],
            error_count: errors,
            warning_count: warnings,
            info_count: infos,
        }
    }

    fn make_diagnostic(rule: &str, severity: Severity) -> Diagnostic {
        Diagnostic {
            file_path: PathBuf::from("src/lib.rs"),
            rule: rule.to_string(),
            category: Category::ErrorHandling,
            severity,
            message: format!("test message for {rule}"),
            help: Some(format!("fix {rule}")),
            line: Some(10),
            column: Some(5),
            fix: None,
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
        render_terminal(&result, false, None);
    }

    #[test]
    fn test_render_terminal_zero_diagnostics() {
        let result = make_result(100, vec![], 0, 0, 0);
        render_terminal(&result, false, None);
    }

    #[test]
    fn test_render_terminal_verbose() {
        let diags = vec![make_diagnostic("rule-a", Severity::Warning)];
        let result = make_result(80, diags, 0, 1, 0);
        render_terminal(&result, true, None);
    }

    #[test]
    fn test_render_terminal_with_skipped_passes() {
        let mut result = make_result(90, vec![], 0, 0, 0);
        result.skipped_passes = vec!["cargo-audit".to_string(), "cargo-deny".to_string()];
        render_terminal(&result, false, None);
    }

    #[test]
    fn test_render_terminal_zero_files_no_diagnostics() {
        let mut result = make_result(100, vec![], 0, 0, 0);
        result.source_file_count = 0;
        // Should print "No Rust source files found" and return early
        render_terminal(&result, false, None);
    }

    // --- print_diagnostics grouping ---

    #[test]
    fn test_print_diagnostics_groups_by_rule() {
        let diags = vec![
            make_diagnostic("same-rule", Severity::Warning),
            make_diagnostic("same-rule", Severity::Warning),
            make_diagnostic("other-rule", Severity::Error),
        ];
        // Should not panic; diagnostics are grouped by rule
        print_diagnostics(&diags, false, None);
    }

    #[test]
    fn test_print_diagnostics_sorts_errors_first() {
        let diags = vec![
            make_diagnostic("warn-rule", Severity::Warning),
            make_diagnostic("info-rule", Severity::Info),
            make_diagnostic("err-rule", Severity::Error),
        ];
        // Should print errors, then warnings, then info
        print_diagnostics(&diags, true, None);
    }
    #[test]
    fn test_render_terminal_with_category_filter() {
        let diags = vec![
            make_diagnostic("rule-a", Severity::Warning),
            make_diagnostic("rule-b", Severity::Error),
        ];
        let result = make_result(70, diags, 1, 1, 0);
        // Should not panic; verbose shown only for ErrorHandling category
        render_terminal(&result, false, Some(CategoryFilter::ErrorHandling));
    }

    #[test]
    fn test_render_terminal_with_non_matching_category_filter() {
        let diags = vec![make_diagnostic("rule-a", Severity::Warning)];
        let result = make_result(80, diags, 0, 1, 0);
        // Performance filter should not show verbose for ErrorHandling diagnostics
        render_terminal(&result, false, Some(CategoryFilter::Performance));
    }
}
