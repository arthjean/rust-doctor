use crate::cli::ScanCategory;
use crate::diagnostics::{
    CanonicalDiagnostic, Category, DiagnosticLocation, ReportOutcome, ReportV1, Severity,
};
use owo_colors::{OwoColorize, Stream};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::{IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::score::{SCORE_GOOD_THRESHOLD, SCORE_OK_THRESHOLD, calculate_score_for_canonical};

const TOP_ERRORS_DISPLAY_COUNT: usize = 3;
const SCORE_BAR_WIDTH: usize = 50;
const OUTPUT_MEASURE_WIDTH: usize = 60;
const CODE_FRAME_LINES_ABOVE: u32 = 1;
const CODE_FRAME_LINES_BELOW: u32 = 1;
const CODE_FRAME_MAX_LINE_LENGTH: usize = 200;
const CODE_FRAME_CLUSTER_REACH: u32 = 3;
const CODE_FRAME_MAX_SPAN: u32 = 20;
const BRAND_URL: &str = "https://rust-doctor.vercel.app";
const DOCS_URL: &str = "https://rust-doctor.vercel.app";
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

/// Render the temporary welcome scene on an interactive terminal, or the
/// static branded header used by pipes, CI, verbose output, and agent shells.
#[allow(
    clippy::redundant_pub_crate,
    reason = "the private terminal module exposes this only through a crate-private re-export"
)]
pub(crate) fn render_welcome(animate: bool) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    if !animate {
        writeln!(stdout, "Rust Doctor v{}", env!("CARGO_PKG_VERSION"))?;
        writeln!(stdout)?;
        return stdout.flush();
    }

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
    )?;
    std::thread::sleep(WELCOME_INTER_LINE_DELAY);
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
    )?;
    std::thread::sleep(WELCOME_HOLD_DELAY);
    write!(stdout, "\x1b[3A\r\x1b[0J")?;
    stdout.flush()
}

fn type_welcome_line(
    stdout: &mut impl std::io::Write,
    prefix: &str,
    text: &str,
    style: fn(&str) -> String,
) -> std::io::Result<()> {
    let characters: Vec<_> = text.chars().collect();
    for length in 1..=characters.len() {
        let fragment: String = characters[..length].iter().collect();
        write!(stdout, "\r{prefix}{}\x1b[K", style(&fragment))?;
        stdout.flush()?;
        std::thread::sleep(WELCOME_TYPEWRITER_DELAY);
    }
    Ok(())
}

fn welcome_text_width() -> usize {
    const PREFIX_WIDTH: usize = 11;
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map_or(usize::MAX, |columns| {
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

    if !rendered.stderr.is_empty() {
        eprint!("{}", rendered.stderr);
        let _ = std::io::stderr().flush();
    }
    if !rendered.stdout.is_empty() {
        print!("{}", rendered.stdout);
        let _ = std::io::stdout().flush();
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
}

fn build_terminal_output(result: &ReportV1, options: RenderOptions<'_>) -> TerminalOutput {
    let mut output = TerminalOutput::default();
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

    let diagnostics: Vec<_> = result
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

    if diagnostics.is_empty() {
        if result.source_file_count == 0 {
            let _ = writeln!(
                output.stderr,
                "\n  {}",
                stderr_warn("No Rust source files found.")
            );
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
                    "No issues detected, but {incomplete_checks} {noun} failed: results are incomplete."
                ))
            );
        } else if result.outcome == ReportOutcome::Clean || !options.show_warnings {
            let _ = writeln!(output.stdout, "  {}\n", stdout_success("No issues found!"));
        }
    } else {
        output.stderr.push('\n');
        render_diagnostics(
            &mut output.stderr,
            &diagnostics,
            options.verbose,
            result.resolved_root.as_deref(),
            options.show_agent_guidance,
            should_render_hyperlinks(options.show_agent_guidance),
        );
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

    render_summary(
        &mut output.stdout,
        result,
        &diagnostics,
        options.selected_categories,
    );
    render_project_summary(&mut output.stdout, result);
    render_footer(&mut output.stdout, result, options.show_default_share);
    output
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

fn render_diagnostics(
    output: &mut String,
    diagnostics: &[&CanonicalDiagnostic],
    verbose: bool,
    root: Option<&str>,
    agent_environment: bool,
    hyperlinks: bool,
) {
    let groups = diagnostic_groups(diagnostics);
    let error_groups: Vec<_> = groups
        .iter()
        .filter(|group| group.severity == Severity::Error)
        .take(TOP_ERRORS_DISPLAY_COUNT)
        .collect();

    if verbose {
        for group in &groups {
            render_diagnostic_group(output, group, true, root, agent_environment, hyperlinks);
        }
    } else if !error_groups.is_empty() {
        let noun = if error_groups.len() == 1 {
            "error"
        } else {
            "errors"
        };
        let _ = writeln!(
            output,
            "  {}\n",
            stderr_bold(&format!("Top {} {noun} you should fix", error_groups.len()))
        );
        for group in &error_groups {
            render_diagnostic_group(output, group, false, root, agent_environment, hyperlinks);
        }
    }

    if !error_groups.is_empty() || verbose {
        let _ = writeln!(output, "\n{}\n", stderr_dim(&section_divider()));
    }

    let total = diagnostics.len();
    let issue_noun = if total == 1 { "issue" } else { "issues" };
    let _ = writeln!(
        output,
        "  {}\n",
        stderr_bold(&format!("All {total} {issue_noun}"))
    );
    render_category_tallies(output, diagnostics);

    let shown_error_rules = error_groups.len();
    if !verbose && total > shown_error_rules {
        let _ = writeln!(
            output,
            "\n  {} {} {}",
            stderr_dim("Run"),
            stderr_info(&stderr_bold(VERBOSE_COMMAND)),
            stderr_dim("to list every error and warning")
        );
    }

    render_migration_advisory(output, &groups);
    output.push('\n');
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
    let _ = writeln!(output, "  {icon} {colored_headline}{badge}");

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
    let source_lines: Vec<_> = source.lines().collect();
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
            .copied()
            .unwrap_or("");
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
    errors: usize,
    warnings: usize,
    infos: usize,
}

fn render_category_tallies(output: &mut String, diagnostics: &[&CanonicalDiagnostic]) {
    let mut tallies = [CategoryTally::default(); DISPLAY_CATEGORIES.len()];
    for diagnostic in diagnostics {
        let tally = &mut tallies[category_rank(&diagnostic.category)];
        match diagnostic.severity {
            Severity::Error => tally.errors += 1,
            Severity::Warning => tally.warnings += 1,
            Severity::Info => tally.infos += 1,
        }
    }
    for (index, tally) in tallies.into_iter().enumerate() {
        if tally.errors + tally.warnings + tally.infos == 0 {
            continue;
        }
        let mut parts = Vec::new();
        if tally.errors > 0 {
            parts.push(stderr_error(&count_label(tally.errors, "error")));
        }
        if tally.warnings > 0 {
            parts.push(stderr_warn(&stderr_dim(&count_label(
                tally.warnings,
                "warning",
            ))));
        }
        if tally.infos > 0 {
            parts.push(stderr_info(&stderr_dim(&count_label(tally.infos, "info"))));
        }
        let _ = writeln!(
            output,
            "  {} {} {}",
            stderr_bold(DISPLAY_CATEGORIES[index]),
            stderr_dim("›"),
            parts.join(&stderr_dim(", "))
        );
    }
}

fn render_migration_advisory(output: &mut String, groups: &[DiagnosticGroup<'_>]) {
    let migration_groups: Vec<_> = groups
        .iter()
        .filter_map(|group| {
            let files: HashSet<_> = group
                .diagnostics
                .iter()
                .filter_map(|diagnostic| match &diagnostic.location {
                    DiagnosticLocation::Source { path, .. } => Some(path.as_str()),
                    DiagnosticLocation::Project => None,
                })
                .collect();
            (files.len() >= 40).then_some((group, files.len()))
        })
        .take(TOP_ERRORS_DISPLAY_COUNT)
        .collect();
    if migration_groups.is_empty() {
        return;
    }
    let _ = writeln!(
        output,
        "\n  {} {}",
        stderr_warn("⚠"),
        stderr_bold("Migration-scale change: sample before you sweep")
    );
    for (group, file_count) in migration_groups {
        let _ = writeln!(
            output,
            "    {} {}",
            group.title,
            stderr_dim(&format!(
                "×{} across {file_count} files",
                group.diagnostics.len()
            ))
        );
    }
    for line in wrap_text(
        "Fix a representative few first and confirm the recipe holds. Then get the code owner's sign-off before changing the rest.",
        OUTPUT_MEASURE_WIDTH,
    ) {
        let _ = writeln!(output, "{}", stderr_dim(&format!("    {line}")));
    }
    let _ = writeln!(
        output,
        "    {} {}",
        stderr_dim("Scope it down one area at a time:"),
        stderr_info("npx rust-doctor@latest <path>")
    );
}

fn render_summary(
    output: &mut String,
    result: &ReportV1,
    diagnostics: &[&CanonicalDiagnostic],
    selected_categories: &[ScanCategory],
) {
    let score = result.score.filter(|_| result.summary.score_authoritative);
    let Some(score) = score else {
        let _ = writeln!(output, "  {}", branding_line());
        let message = if result.source_file_count == 0 {
            "Score unavailable because no Rust source files were analyzed."
        } else if matches!(
            result.outcome,
            ReportOutcome::Partial | ReportOutcome::Failed
        ) {
            "Score not shown: some checks could not complete."
        } else {
            "Score unavailable because required analysis did not complete."
        };
        let _ = writeln!(output, "  {}\n", stdout_dim(message));
        return;
    };
    let Some(label) = result.score_label else {
        return;
    };

    let top_rules: HashSet<_> = diagnostic_groups(diagnostics)
        .into_iter()
        .filter(|group| group.severity == Severity::Error)
        .take(TOP_ERRORS_DISPLAY_COUNT)
        .map(|group| group.rule.to_string())
        .collect();
    let potential_score = projected_score(result, &top_rules, selected_categories)
        .filter(|potential| *potential > score);

    let face = doctor_face(score);
    let score_line = format!(
        "{} {} {}",
        stdout_score(&score.to_string(), score),
        stdout_dim("/ 100"),
        stdout_score(&label.to_string(), score)
    );
    let score_bar = score_bar(score, potential_score, score_bar_width_from_environment());
    let face_lines = [
        "┌─────┐".to_string(),
        format!("│ {} │", face.0),
        format!("│ {} │", face.1),
        "└─────┘".to_string(),
    ];
    let right_lines = [score_line, score_bar, branding_line(), String::new()];
    for (face_line, right_line) in face_lines.iter().zip(right_lines) {
        let colored_face = stdout_score(face_line, score);
        if right_line.is_empty() {
            let _ = writeln!(output, "  {colored_face}");
        } else {
            let _ = writeln!(output, "  {colored_face}  {right_line}");
        }
    }
    output.push('\n');

    if let Some(potential) = potential_score {
        let improvement = potential - score;
        let _ = writeln!(
            output,
            "{}{}{}",
            stdout_dim("  You could improve "),
            stdout_score(&format!("+{improvement}%"), potential),
            stdout_dim(&format!(
                " by fixing the top {TOP_ERRORS_DISPLAY_COUNT} issues"
            ))
        );
    }
    if !selected_categories.is_empty() {
        let mut names = Vec::new();
        for category in selected_categories {
            let name = display_category(&scan_category_to_category(*category));
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let _ = writeln!(
            output,
            "{}",
            stdout_dim(&format!("  Categories: {}", names.join(", ")))
        );
    }
}

fn projected_score(
    result: &ReportV1,
    excluded_rules: &HashSet<String>,
    selected_categories: &[ScanCategory],
) -> Option<u32> {
    if excluded_rules.is_empty() {
        return None;
    }
    let categories: Vec<_> = selected_categories
        .iter()
        .copied()
        .map(scan_category_to_category)
        .collect();
    let diagnostics = result.diagnostics.iter().filter(|diagnostic| {
        diagnostic
            .visible_on
            .iter()
            .any(|surface| surface == "score")
            && !excluded_rules.contains(&diagnostic.rule)
    });
    Some(calculate_score_for_canonical(diagnostics, &categories).0)
}

fn score_bar(score: u32, potential_score: Option<u32>, width: usize) -> String {
    let current_fill = ((f64::from(score) / 100.0) * width as f64).round() as usize;
    let potential_fill = potential_score.map_or(current_fill, |potential| {
        ((f64::from(potential) / 100.0) * width as f64).round() as usize
    });
    let potential_fill = potential_fill.clamp(current_fill, width);
    let gain = potential_fill - current_fill;
    let empty = width - potential_fill;
    format!(
        "{}{}{}",
        stdout_score(&"█".repeat(current_fill), score),
        stdout_dim(&stdout_score(&"▓".repeat(gain), score)),
        stdout_dim(&"░".repeat(empty))
    )
}

fn score_bar_width_from_environment() -> usize {
    score_bar_width(
        std::env::var("COLUMNS")
            .ok()
            .and_then(|value| value.parse().ok()),
    )
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

fn render_project_summary(output: &mut String, result: &ReportV1) {
    if result.projects.len() <= 1 {
        return;
    }
    let entries: Vec<_> = result
        .projects
        .iter()
        .map(|project| {
            let errors = project
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == Severity::Error)
                .count();
            let warnings = project
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == Severity::Warning)
                .count();
            let infos = project
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == Severity::Info)
                .count();
            (
                project_name(project),
                project.score,
                errors,
                warnings,
                infos,
            )
        })
        .collect();
    let longest_name = entries
        .iter()
        .map(|(name, ..)| name.width())
        .max()
        .unwrap_or(0);
    output.push('\n');
    for (name, score, errors, warnings, infos) in entries {
        let padded_name = format!("{name:<longest_name$}");
        let Some(score) = score else {
            let total = errors + warnings + infos;
            let _ = writeln!(
                output,
                "  {}  {}  {}",
                stdout_dim(&padded_name),
                stdout_dim("no score"),
                stdout_dim(&count_label(total, "issue"))
            );
            continue;
        };
        let mut issue_parts = Vec::new();
        if errors > 0 {
            issue_parts.push(stdout_error(&count_label(errors, "error")));
        }
        if warnings > 0 {
            issue_parts.push(stdout_warn(&count_label(warnings, "warning")));
        }
        if infos > 0 {
            issue_parts.push(stdout_info(&count_label(infos, "info")));
        }
        let _ = writeln!(
            output,
            "  {}  {}  {}  {}",
            stdout_score(&padded_name, score),
            stdout_score(&format!("{score:>3}"), score),
            stdout_score(score_band_label(score), score),
            issue_parts.join(&stdout_dim(", "))
        );
    }
}

fn project_name(project: &crate::diagnostics::ProjectReport) -> String {
    let package_id_tail = project
        .cargo_package_id
        .rsplit('#')
        .next()
        .unwrap_or(&project.cargo_package_id);
    let candidate = package_id_tail
        .split('@')
        .next()
        .and_then(|value| value.split_whitespace().next())
        .filter(|value| !value.is_empty());
    candidate.map_or_else(
        || {
            Path::new(&project.package_root)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("project")
                .to_string()
        },
        str::to_string,
    )
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

const fn doctor_face(score: u32) -> (&'static str, &'static str) {
    if score >= SCORE_GOOD_THRESHOLD {
        ("◠ ◠", " ▽ ")
    } else if score >= SCORE_OK_THRESHOLD {
        ("• •", " ─ ")
    } else {
        ("x x", " ▽ ")
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

fn wrap_text(text: &str, width: usize) -> Vec<String> {
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

const fn score_band_label(score: u32) -> &'static str {
    if score >= SCORE_GOOD_THRESHOLD {
        "Great"
    } else if score >= SCORE_OK_THRESHOLD {
        "Needs work"
    } else {
        "Critical"
    }
}

fn stdout_score(text: &str, score: u32) -> String {
    if score >= SCORE_GOOD_THRESHOLD {
        stdout_success(text)
    } else if score >= SCORE_OK_THRESHOLD {
        stdout_warn(text)
    } else {
        stdout_error(text)
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
        DimensionScores, ReportCompleteness, ReportSummary, ScanMode, ScoreLabel, SourcePosition,
        SourceRange, SourceSurface,
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
        assert!(output.stderr.contains("Top 1 error you should fix"));
        assert!(
            output
                .stderr
                .contains("Security: unauthenticated-server-action title")
        );
        assert!(output.stderr.contains("src/lib.rs:2"));
        assert!(output.stderr.contains("pub async fn risky()"));
        assert!(output.stderr.contains("All 2 issues"));
        assert!(output.stderr.contains("Security › 1 error"));
        assert!(output.stderr.contains("Performance › 1 warning"));
        assert!(output.stderr.contains(VERBOSE_COMMAND));

        assert!(output.stdout.contains("68 / 100 Needs work"));
        assert!(
            output
                .stdout
                .contains("Rust Doctor (https://rust-doctor.vercel.app)")
        );
        assert!(output.stdout.contains("Docs:"));
        assert!(output.stdout.contains("GitHub:"));
        insta::assert_snapshot!("terminal_default_stderr", &output.stderr);
        insta::assert_snapshot!("terminal_default_stdout", &output.stdout);
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
    fn clean_render_prints_success_and_score() {
        let directory = tempfile::tempdir().unwrap();
        let mut result = make_result(directory.path(), 100, vec![]);
        result.score_label = Some(ScoreLabel::Great);
        result.summary.score_label = Some(ScoreLabel::Great);
        let output = render(&result, false);
        assert!(output.stdout.contains("No issues found!"));
        assert!(output.stdout.contains("100 / 100 Great"));
        assert!(!output.stderr.contains("All 0 issues"));
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
        assert!(
            output
                .stdout
                .contains("Score not shown: some checks could not complete.")
        );
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
