//! Tests of the linear report.
//!
//! They live in their own file for the reason the report itself checks: the
//! crate passes its own `oversized_unit` rule, and a module carrying its eight
//! section renderers and their tests in one file would not.

use super::*;
use crate::terminal_text::display_width;
use crate::{
    Audit, BlockingLevel, DeltaMatch, DeltaReport, DeltaSummary, Diagnostic, DiagnosticSource,
    DiagnosticSpan, GateReport, GateStatus, InspectReport, ReportError, ScanReport, Severity,
    Summary, ToolchainReport,
};

/// The report passes the rule it publishes. `oversized_unit` reports a file at
/// a thousand lines, and this module holds that bound: the score block and
/// these tests have files of their own for that reason, and a file that grows
/// back past it fails here rather than on a self-scan nobody reads.
#[test]
fn the_report_holds_the_size_bound_it_reports_for() {
    for own in [
        include_str!("../render.rs"),
        include_str!("score_header.rs"),
        include_str!("score_header/tests.rs"),
        include_str!("tests.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the linear report is {lines} lines long, over the {} it reports",
            crate::structure::FILE_LINES
        );
    }
}

struct ClosedWriter;

impl Write for ClosedWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn report() -> InspectReport {
    let diagnostics = vec![Diagnostic {
        context: None,
        id: "id".to_owned(),
        source: DiagnosticSource::Clippy,
        code: Some("clippy::todo".to_owned()),
        base_severity: Severity::Warning,
        severity: Severity::Warning,
        category: Some("correctness".to_owned()),
        message: "replace the placeholder".to_owned(),
        help: Some("Implement the intended behavior.".to_owned()),
        package: Some("package".to_owned()),
        target: Some("target".to_owned()),
        path: Some("src/lib.rs".to_owned()),
        span: Some(DiagnosticSpan {
            line_start: 2,
            column_start: 3,
            line_end: 2,
            column_end: 4,
        }),
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        suggestion: None,
        occurrences: 1,
    }];
    report_of(diagnostics, 1, 100)
}

/// The report a set of diagnostics produces over a workspace of `source_files`
/// files and `production_lines` lines of production Rust.
///
/// The audit is built from the diagnostics rather than written down beside
/// them, because every entry point of this report replays
/// `InspectReport::is_valid`, which recomputes the whole block: a score poked
/// into a fixture never reaches a renderer.
fn report_of(
    diagnostics: Vec<Diagnostic>,
    source_files: usize,
    production_lines: usize,
) -> InspectReport {
    let summary = Summary::from_diagnostics(&diagnostics);
    InspectReport {
        schema_version: crate::report::SCHEMA_VERSION,
        audit: Audit::build(
            source_files,
            production_lines,
            Status::Complete,
            &diagnostics,
        ),
        status: Status::Complete,
        complete: true,
        policy: None,
        scope: None,
        project: None,
        toolchain: ToolchainReport {
            rustc: None,
            cargo: None,
            clippy: None,
        },
        scan: ScanReport {
            command: None,
            exit_code: Some(0),
            build_finished: Some(true),
            noise_lines: Some(0),
        },
        diagnostics,
        delta: None,
        errors: Vec::new(),
        summary,
        gate: GateReport {
            blocking: BlockingLevel::Error,
            status: GateStatus::Passed,
            blocking_diagnostics: Some(0),
        },
    }
}

fn rendered(report: &InspectReport, width: usize, color: bool, verbose: bool) -> String {
    let mut output = Vec::new();
    render_terminal_with_options(
        report,
        &mut output,
        TerminalOptions {
            workspace_root: Path::new("tests/fixtures/kernel-contract/todo"),
            elapsed: Duration::from_millis(1250),
            verbose,
            width,
            color,
            animate: false,
        },
    )
    .unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn json_is_one_document_followed_by_newline() {
    let mut output = Vec::new();
    render_json(&report(), &mut output).unwrap();
    assert_eq!(output.last(), Some(&b'\n'));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output).unwrap()["schema_version"],
        crate::report::SCHEMA_VERSION
    );
}

#[test]
fn invalid_report_is_rejected_before_any_output() {
    let mut report = report();
    report.audit.score.as_mut().unwrap().value = 101;
    let mut json = Vec::new();
    assert!(matches!(
        render_json(&report, &mut json),
        Err(RenderError::InvalidReport)
    ));
    assert!(json.is_empty());

    let mut terminal = Vec::new();
    assert!(matches!(
        render_terminal(&report, &mut terminal),
        Err(RenderError::InvalidReport)
    ));
    assert!(terminal.is_empty());
}

#[test]
fn terminal_sections_follow_the_normative_order() {
    let output = rendered(&report(), 100, false, false);
    let labels = [
        "Scope:",
        "Scanned ",
        "Top warning:",
        "All 1 occurrences across 1 findings",
        "Bugs:",
        "Run with --verbose",
        // Top border of the score block: the stable marker of the section
        // since the value reads `N / 100 Label`.
        "┌─────┐",
        "Share:",
        "Docs:",
        "GitHub:",
    ];
    let mut previous = 0usize;
    for label in labels {
        let position = output.find(label).expect("section should be rendered");
        assert!(position >= previous, "{label} was out of order\n{output}");
        previous = position;
    }
    assert!(output.contains("GitHub: https://github.com/arthjean/rust-doctor"));
}

#[test]
fn widths_and_color_policy_are_stable() {
    for width in [80, 100, 140] {
        let plain = rendered(&report(), width, false, false);
        assert!(
            plain
                .lines()
                .filter(|line| !is_link_row(line))
                .all(|line| display_width(line) <= width)
        );
        assert!(!plain.contains("\u{1b}["));

        let colored = rendered(&report(), width, true, false);
        assert!(colored.contains("\u{1b}["));
        for line in colored.lines().filter(|line| !is_link_row(line)) {
            // `sanitize` strips every escape sequence, including the
            // truecolor of a perfect score's bar, which the original
            // hard-coded code list did not cover.
            let visible = sanitize(line);
            assert!(display_width(&visible) <= width, "{visible}");
        }
    }
}

#[test]
fn wide_dynamic_text_respects_terminal_columns() {
    let mut report = report();
    report.diagnostics[0].message = "界".repeat(80);
    let output = rendered(&report, 80, false, false);
    assert!(
        output
            .lines()
            .filter(|line| !is_link_row(line))
            .all(|line| display_width(line) <= 80)
    );
}

/// A row carrying a URL is the one row the width does not bound: a link cut in two is two
/// strings no terminal opens, so the renderer writes it whole and the terminal soft-wraps it.
fn is_link_row(row: &str) -> bool {
    sanitize(row).contains("https://")
}

/// The rule link is written whole whatever the width, and never cut mid-way.
#[test]
fn a_rule_link_is_never_wrapped() {
    let mut report = report();
    report.diagnostics[0].code = Some("rust_doctor::structure::near_duplicate_function_body".to_owned());
    report.diagnostics[0].category = Some("maintainability".to_owned());
    report.audit = Audit::build(1, 100, Status::Complete, &report.diagnostics);
    report.summary = Summary::from_diagnostics(&report.diagnostics);
    let output = rendered(&report, 40, false, false);
    let link = "Rule: https://rust-doctor.com/rules/rust_doctor%3A%3Astructure%3A%3Anear_duplicate_function_body";
    assert!(output.lines().any(|line| line == link), "{output}");
}

#[test]
fn partial_and_missing_scores_suppress_share_and_projection() {
    // The score is made partial by the scan that produced it, not by poking the flag: the audit
    // has to stay reproducible from its own diagnostics or the report refuses to render.
    let mut partial = report();
    partial.status = Status::Incomplete;
    partial.complete = false;
    partial.audit = Audit::build(1, 100, Status::Incomplete, &partial.diagnostics);
    let score = partial.audit.score.as_ref().unwrap();
    assert!(!score.authoritative);
    assert_eq!(score.projected_after_top_three, None);
    assert!(score.projected_rule_ids.is_empty());
    let output = rendered(&partial, 80, false, false);
    assert!(output.contains("Core partial"));
    assert!(!output.contains("Share:"));
    assert!(!output.contains("projected"));

    partial.audit.source_files = 0;
    partial.audit.score = None;
    let output = rendered(&partial, 80, false, false);
    assert!(output.contains("Score unavailable: no Rust files were analyzed."));
    assert!(!output.contains("Share:"));
}

#[test]
fn projection_is_rendered_only_when_it_raises_the_score() {
    let mut flat = report();
    flat.diagnostics[0].code = Some("clippy::dbg_macro".to_owned());
    flat.diagnostics[0].category = Some("maintainability".to_owned());
    // Five hundred kilolines: one `dbg!` in a workspace that size is a density the exponential
    // cannot round away from a hundred, which is what makes the projection flat.
    flat.audit = Audit::build(5_000, 500_000, Status::Complete, &flat.diagnostics);
    let score = flat.audit.score.as_ref().unwrap();
    assert_eq!(score.projected_after_top_three, Some(score.value));
    assert!(!score.projected_rule_ids.is_empty());
    let output = rendered(&flat, 80, false, false);
    assert!(output.contains("100 / 100 Great"));
    assert!(!output.contains("projected"));

    let mut raising = report();
    raising.diagnostics[0].code = Some("rust_doctor::source::dynamic_shell_command".to_owned());
    raising.diagnostics[0].category = Some("security".to_owned());
    raising.audit = Audit::build(1, 100, Status::Complete, &raising.diagnostics);
    let score = raising.audit.score.as_ref().unwrap();
    assert_eq!(score.value, 40);
    assert_eq!(score.projected_after_top_three, Some(100));
    let output = rendered(&raising, 80, false, false);
    assert!(output.contains("to reach a projected 100/100"));
    assert!(
        output.contains(
            "Capped at 40/100 by a P0 finding: rust_doctor::source::dynamic_shell_command"
        )
    );
}

/// A failed scan publishes what it attempted and what failed, and nothing it
/// did not measure.
///
/// The failure used to be followed by a `100 / 100` face and `No issues
/// found.`, because a run ending in the toolchain preflight still carries an
/// inventory of the workspace it never scanned: the score is built from that
/// count against zero diagnostics, and zero findings score 100. The old test
/// missed it by building the audit over zero files, which is the one input for
/// which no score exists at all.
#[test]
fn a_failed_scan_publishes_its_failure_and_nothing_it_did_not_measure() {
    let mut failed = report();
    failed.status = Status::Failed;
    failed.complete = false;
    failed.diagnostics.clear();
    failed.audit = Audit::build(12, 1_200, Status::Failed, &failed.diagnostics);
    failed.summary = Summary::default();
    failed.errors = vec![ReportError {
        stage: "execution".to_owned(),
        code: "clippy-unavailable".to_owned(),
        message: "Clippy could not report a version".to_owned(),
    }];
    assert!(failed.audit.score.is_some(), "the input has a score to hide");

    let output = rendered(&failed, 80, false, false);

    assert!(output.contains("Scope: full codebase"));
    assert!(output.contains("Scan failed: Clippy could not report a version"));
    for claim in [
        "/ 100",
        "Rust Doctor",
        "No issues found.",
        "Scanned",
        "occurrences",
        "Categories:",
        "Gate",
        "https://",
        "Share:",
        "Docs:",
        "GitHub:",
    ] {
        assert!(!output.contains(claim), "a failed scan claimed {claim:?}");
    }
}

#[test]
fn verbose_lists_all_groups_without_the_cta() {
    let output = rendered(&report(), 100, false, true);
    assert!(output.contains("Warning: Todo (1 occurrences)"));
    assert!(output.contains("Rule ID: clippy::todo"));
    assert!(!output.contains("Run with --verbose"));
}

#[test]
fn broken_pipe_is_typed_and_detectable() {
    let error = render_terminal(&report(), ClosedWriter).unwrap_err();
    assert!(error.is_broken_pipe());
}

#[test]
fn baseline_hides_pre_existing_details_and_keeps_introduced_and_fixed() {
    let mut report = report();
    let mut pre_existing = report.diagnostics[0].clone();
    pre_existing.id = "pre-existing-current".to_owned();
    pre_existing.message = "must stay hidden".to_owned();
    report.diagnostics.push(pre_existing);
    let mut fixed = report.diagnostics[0].clone();
    fixed.id = "fixed-baseline".to_owned();
    fixed.message = "removed debt".to_owned();
    report.delta = Some(DeltaReport {
        fingerprint_version: 1,
        base_diagnostics: 2,
        current_diagnostics: 2,
        introduced: vec!["id".to_owned()],
        pre_existing: vec![DeltaMatch {
            current_id: "pre-existing-current".to_owned(),
            baseline_id: "pre-existing-baseline".to_owned(),
        }],
        fixed: vec![fixed],
        summary: DeltaSummary {
            introduced: 1,
            pre_existing: 1,
            fixed: 1,
            cross_file_matches: 1,
        },
    });
    report.audit = Audit::build(1, 100, Status::Complete, &report.diagnostics[..1]);
    report.summary = Summary::from_diagnostics(&report.diagnostics);

    let output = rendered(&report, 100, false, false);

    assert!(output.contains("replace the placeholder"));
    assert!(output.contains("Fixed: src/lib.rs:2:3 warning [clippy::todo] removed debt"));
    assert!(!output.contains("must stay hidden"));
}

/// The sentence exists to explain an absence, so it names enough to be
/// recognised and counts the rest.
#[test]
fn the_withheld_sentence_names_two_rules_and_counts_the_rest() {
    assert_eq!(withheld_sentence(&[]), None);

    let one = withheld_sentence(&["clippy::indexing_slicing".to_owned()])
        .expect("one withheld rule is a sentence");
    assert!(
        one.contains("clippy::indexing_slicing (98% noise on 40 sites) reports here but is left \
                      out"),
        "{one}"
    );

    let two = withheld_sentence(&[
        "clippy::indexing_slicing".to_owned(),
        "clippy::string_slice".to_owned(),
    ])
    .expect("two withheld rules are a sentence");
    assert!(
        two.contains(
            "clippy::indexing_slicing (98% noise on 40 sites) and clippy::string_slice (98% \
             noise on 40 sites) report here"
        ),
        "{two}"
    );

    let many = withheld_sentence(&[
        "clippy::indexing_slicing".to_owned(),
        "clippy::string_slice".to_owned(),
        "clippy::print_stderr".to_owned(),
        "clippy::panic".to_owned(),
    ])
    .expect("four withheld rules are a sentence");
    assert!(
        many.contains(
            "clippy::indexing_slicing (98% noise on 40 sites), clippy::string_slice (98% noise \
             on 40 sites) and 2 more"
        ),
        "{many}"
    );
    assert!(
        !many.contains("clippy::print_stderr"),
        "past two names the sentence counts rather than enumerates: {many}"
    );
}

/// A reader can weigh the ranking, because every rule it names carries the
/// sample its rate rests on.
///
/// The loud rule is measured wrong on all forty sites the corpus reviewed and
/// the quiet one right on the one site it showed, so the quiet one leads
/// despite thirty times fewer findings. Without the two samples printed beside
/// the two rates, that order reads as a defect of the tool: a reader who sees
/// the rule with sixty findings ranked second has no way to tell a measurement
/// from a bug.
#[test]
fn the_terminal_names_the_sample_behind_every_rate_it_ranks_by() {
    let mut report = report();
    report.diagnostics = vec![
        diagnostic("clippy::indexing_slicing", "reliability", 60),
        diagnostic(
            "rust_doctor::cargo::duplicate_major_versions",
            "dependencies",
            2,
        ),
    ];
    report.audit = Audit::build(1, 100, Status::Complete, &report.diagnostics);
    report.summary = Summary::from_diagnostics(&report.diagnostics);

    let output = rendered(&report, 100, false, false);

    // The report wraps at the terminal width, so the two names are looked for in
    // the whole frame rather than on the line the projection starts on.
    let unwrapped = output.replace('\n', " ");
    assert!(
        unwrapped.find("rust_doctor::cargo::duplicate_major_versions (33% noise on 1 site)")
            < unwrapped.find("clippy::indexing_slicing (98% noise on 40 sites)"),
        "the rule measured right on its one site leads, and both samples are named: {output}"
    );
}

/// A rule the corpus never adjudicated is named as unmeasured, never as a rate.
///
/// It is ranked at the middle of the interval, and printing that middle as a
/// percentage would publish an assumption as an observation. `clippy::todo` is
/// catalogued and carries no entry in the shipped table.
#[test]
fn the_terminal_names_an_unmeasured_rule_as_unmeasured() {
    let mut report = report();
    report.diagnostics = vec![diagnostic("clippy::todo", "maintainability", 4)];
    report.audit = Audit::build(1, 100, Status::Complete, &report.diagnostics);
    report.summary = Summary::from_diagnostics(&report.diagnostics);

    let output = rendered(&report, 100, false, false);

    assert!(
        output.contains("clippy::todo (unmeasured)"),
        "an unmeasured rule is named, not scored: {output}"
    );
    assert!(
        !output.contains("clippy::todo (50%"),
        "the half-weight default is a ranking choice, never published as a measurement: {output}"
    );
}

/// The added text is bounded by the terminal like every other line.
///
/// Forty columns is the narrowest the interactive report ever draws at, and the
/// linear one normalizes to `MIN_WIDTH` below it. Either way no row may pass the
/// width the writer was given: the rule ids the ranking names are long, and the
/// sample note lengthens each of them.
#[test]
fn the_measurement_note_never_pushes_a_row_past_the_width() {
    let mut report = report();
    report.diagnostics = vec![
        diagnostic("clippy::indexing_slicing", "reliability", 60),
        diagnostic(
            "rust_doctor::cargo::duplicate_major_versions",
            "dependencies",
            2,
        ),
    ];
    report.audit = Audit::build(1, 100, Status::Complete, &report.diagnostics);
    report.summary = Summary::from_diagnostics(&report.diagnostics);

    for width in [40, 80, 120] {
        let output = rendered(&report, width, false, false);
        let bound = width.max(MIN_WIDTH);
        for row in output.lines().filter(|row| !is_link_row(row)) {
            assert!(
                display_width(row) <= bound,
                "a row of {} columns at width {width}: {row}",
                display_width(row)
            );
        }
    }
}

fn diagnostic(code: &str, category: &str, occurrences: usize) -> Diagnostic {
    Diagnostic {
        context: None,
        id: format!("finding-{code}"),
        source: DiagnosticSource::Clippy,
        code: Some(code.to_owned()),
        base_severity: Severity::Warning,
        severity: Severity::Warning,
        category: Some(category.to_owned()),
        message: "finding".to_owned(),
        help: None,
        package: None,
        target: None,
        path: Some("src/lib.rs".to_owned()),
        span: None,
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        suggestion: None,
        occurrences,
    }
}

// -------------------------------------------- the four values of the block

/// The five rules the search fires, one per dimension the score reads, each
/// catalogued so the block stays authoritative: an uncatalogued rule drops the
/// flag and the label reads `Core partial` instead of its band.
// Unmeasured rules, so the search reads the curve and not a discount: `expect_used` used to
// stand for reliability, and the corpus adjudicated it wrong on four sites out of five, which
// leaves the dimension at 28 however many sites fire.
const PER_KILOLINE_RULES: [(&str, &str); 3] = [
    ("clippy::todo", "correctness"),
    ("clippy::dbg_macro", "maintainability"),
    ("clippy::manual_memcpy", "performance"),
];
const WORKSPACE_RULE: (&str, &str) = ("rust_doctor::cargo::test_only_dependency", "dependencies");
const SECURITY_RULE: (&str, &str) = (
    "rust_doctor::source::disabled_tls_verification",
    "security",
);

/// `count` distinct sites of one rule. One diagnostic is one site, so the
/// numerator of its dimension is the count itself at warning severity.
fn sites(rule: (&str, &str), count: usize) -> impl Iterator<Item = Diagnostic> {
    let (code, category) = rule;
    (0..count).map(move |index| Diagnostic {
        id: format!("{code}-{index}"),
        path: Some(format!("src/site{index}.rs")),
        ..diagnostic(code, category, 1)
    })
}

/// A workspace of `production_lines` lines holding `count` sites in each of the
/// four dimensions with no ceiling below the top band, and `security` sites in
/// the one that caps the whole score at forty.
fn scored(count: usize, security: usize, production_lines: usize) -> InspectReport {
    let diagnostics: Vec<Diagnostic> = PER_KILOLINE_RULES
        .into_iter()
        .chain([WORKSPACE_RULE])
        .flat_map(|rule| sites(rule, count))
        .chain(sites(SECURITY_RULE, security))
        .collect();
    report_of(diagnostics, count.max(1), production_lines)
}

fn value_of(report: &InspectReport) -> u8 {
    report.audit.score.as_ref().map_or(0, |score| score.value)
}

/// A report whose audit really scores `target`, found rather than declared.
///
/// The value cannot be written into the block: every entry point of this
/// report replays `InspectReport::is_valid`, which recomputes the score from
/// the diagnostics and the line count. So the four values the block has to
/// draw are searched for over the two knobs core-v3 reads, how many sites
/// fired and how large the workspace is. A density only falls as its
/// denominator grows, so the score is monotonic in the line count and a
/// bisection lands on the smallest workspace reaching the target; the helper
/// answers nothing rather than a neighbor.
fn scoring(target: u8) -> Option<InspectReport> {
    const COUNTS: [usize; 11] = [0, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89];
    const SECURITY: [usize; 3] = [0, 12, 60];
    for security in SECURITY {
        for count in COUNTS {
            let (mut low, mut high) = (1usize, 4_000_000usize);
            while low < high {
                let middle = low + (high - low) / 2;
                if value_of(&scored(count, security, middle)) >= target {
                    high = middle;
                } else {
                    low = middle + 1;
                }
            }
            let report = scored(count, security, low);
            if value_of(&report) == target {
                return Some(report);
            }
        }
    }
    None
}

/// The rows of the block, at the width every entry point normalizes to.
fn block_rows(report: &InspectReport) -> Vec<String> {
    let output = rendered(report, MIN_WIDTH, false, false);
    let label = crate::score_block::label_text(
        report.audit.score.as_ref().expect("a scored report").label,
        true,
    );
    let start = output
        .lines()
        .position(|line| line.contains(&format!("/ {} {label}", crate::score_block::PERFECT_SCORE)))
        .expect("the block draws its score line");
    output
        .lines()
        .skip(start)
        .take(crate::score_block::BLOCK_ROWS)
        .map(str::to_owned)
        .collect()
}

/// The four values of the story, drawn through the report itself at the width
/// it guarantees. Each one is a workspace the audit really scored: the bar, the
/// face and the band are what core-v3 produced, not what a fixture claimed.
#[test]
fn the_block_draws_the_bar_the_face_and_the_band_of_every_score_it_publishes() {
    for target in [0u8, 37, 73, 100] {
        let report = scoring(target).expect("the search reaches every value the block draws");
        let score = report.audit.score.as_ref().expect("a scored report");
        assert_eq!(score.value, target);
        assert!(score.authoritative, "a catalogued rule set scores for real");
        assert_eq!(
            score.label,
            crate::score_block::label_for(target),
            "the band of {target}"
        );

        let rows = block_rows(&report);
        let faces = crate::score_block::face_rows(score.label);
        let bar_width = crate::score_block::bar_width(MIN_WIDTH).expect("the block fits");
        let filled = crate::score_block::bar_fill(target, bar_width);
        let expected = [
            format!(
                "  {}  {target} / {} {}",
                faces[0],
                crate::score_block::PERFECT_SCORE,
                crate::score_block::label_text(score.label, true)
            ),
            format!(
                "  {}  {}{}",
                faces[1],
                "█".repeat(filled),
                "░".repeat(bar_width - filled)
            ),
            format!("  {}  {}", faces[2], crate::score_block::BRANDING_NAME),
            format!("  {}", faces[3]),
        ];
        assert_eq!(rows.len(), crate::score_block::BLOCK_ROWS);
        for (row, expected) in rows.iter().zip(expected) {
            assert!(
                row.starts_with(expected.trim_end()),
                "at {target}, {row:?} does not start with {expected:?}"
            );
            assert!(
                display_width(row) <= MIN_WIDTH,
                "at {target}, {row:?} overflows {MIN_WIDTH} columns"
            );
        }
    }
}

/// The help is the rule's and is printed once under the rule, not once under each of its
/// sites: on this crate's own scan it was the same sentence twenty-two times.
#[test]
fn verbose_prints_the_help_once_per_group() {
    let mut report = report();
    let mut second = report.diagnostics[0].clone();
    second.id = "second".to_owned();
    second.path = Some("src/other.rs".to_owned());
    report.diagnostics.push(second);
    report.audit = Audit::build(1, 100, Status::Complete, &report.diagnostics);
    report.summary = Summary::from_diagnostics(&report.diagnostics);

    let output = rendered(&report, 100, false, true);
    assert_eq!(output.matches("Help: Implement the intended behavior.").count(), 1, "{output}");
    assert_eq!(output.matches("src/lib.rs:2:3").count(), 1);
    assert_eq!(output.matches("src/other.rs:2:3").count(), 1);
}

/// The toolchain's replacement is printed under its site, on one row, and qualified whenever
/// the toolchain does not vouch for it outright.
#[test]
fn a_replacement_is_printed_under_its_site() {
    let mut report = report();
    report.diagnostics[0].suggestion = Some(crate::Suggestion {
        replacement: "let value = compute();\n    value".to_owned(),
        applicability: crate::Applicability::MachineApplicable,
    });
    let output = rendered(&report, 100, false, true);
    assert!(output.contains("Replace with: let value = compute(); value"), "{output}");

    report.diagnostics[0].suggestion = Some(crate::Suggestion {
        replacement: "todo!()".to_owned(),
        applicability: crate::Applicability::MaybeIncorrect,
    });
    let output = rendered(&report, 100, false, true);
    assert!(output.contains("Replace with: todo!() (maybe-incorrect)"), "{output}");
}

/// A group every site of which sits outside production code is one line of the verbose
/// report, since it is shown and weighs nothing, and `--json` carries every site.
#[test]
fn an_unscored_group_is_folded_onto_one_line() {
    let mut report = report();
    report.diagnostics[0].context = Some(crate::DiagnosticContext::Tests);
    report.audit = Audit::build(1, 100, Status::Complete, &report.diagnostics);
    report.summary = Summary::from_diagnostics(&report.diagnostics);

    let output = rendered(&report, 100, false, true);
    assert!(
        output.contains("Unscored: Todo (1 occurrences outside production code, clippy::todo)"),
        "{output}"
    );
    assert!(!output.contains("Rule ID: clippy::todo"), "{output}");
    assert!(!output.contains("src/lib.rs:2:3"), "{output}");
}
