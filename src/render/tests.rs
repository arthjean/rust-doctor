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
        occurrences: 1,
    }];
    let summary = Summary::from_diagnostics(&diagnostics);
    InspectReport {
        schema_version: crate::report::SCHEMA_VERSION,
        audit: Audit::build(1, Status::Complete, &diagnostics),
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
        assert!(plain.lines().all(|line| display_width(line) <= width));
        assert!(!plain.contains("\u{1b}["));

        let colored = rendered(&report(), width, true, false);
        assert!(colored.contains("\u{1b}["));
        for line in colored.lines() {
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
    assert!(output.lines().all(|line| display_width(line) <= 80));
}

#[test]
fn partial_and_missing_scores_suppress_share_and_projection() {
    // The score is made partial by the scan that produced it, not by poking the flag: the audit
    // has to stay reproducible from its own diagnostics or the report refuses to render.
    let mut partial = report();
    partial.status = Status::Incomplete;
    partial.complete = false;
    partial.audit = Audit::build(1, Status::Incomplete, &partial.diagnostics);
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
    flat.audit = Audit::build(1, Status::Complete, &flat.diagnostics);
    let score = flat.audit.score.as_ref().unwrap();
    assert_eq!(score.projected_after_top_three, Some(score.value));
    assert!(!score.projected_rule_ids.is_empty());
    let output = rendered(&flat, 80, false, false);
    assert!(output.contains("100 / 100 Great"));
    assert!(!output.contains("projected"));

    let mut raising = report();
    raising.diagnostics[0].code = Some("rust_doctor::source::dynamic_shell_command".to_owned());
    raising.diagnostics[0].category = Some("security".to_owned());
    raising.audit = Audit::build(1, Status::Complete, &raising.diagnostics);
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
    failed.audit = Audit::build(12, Status::Failed, &failed.diagnostics);
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
    report.audit = Audit::build(1, Status::Complete, &report.diagnostics[..1]);
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
    assert!(one.contains("clippy::indexing_slicing reports here but is left out"));

    let two = withheld_sentence(&[
        "clippy::indexing_slicing".to_owned(),
        "clippy::string_slice".to_owned(),
    ])
    .expect("two withheld rules are a sentence");
    assert!(two.contains("clippy::indexing_slicing and clippy::string_slice report here"));

    let many = withheld_sentence(&[
        "clippy::indexing_slicing".to_owned(),
        "clippy::string_slice".to_owned(),
        "clippy::print_stderr".to_owned(),
        "clippy::panic".to_owned(),
    ])
    .expect("four withheld rules are a sentence");
    assert!(
        many.contains("clippy::indexing_slicing, clippy::string_slice and 2 more"),
        "{many}"
    );
    assert!(
        !many.contains("clippy::print_stderr"),
        "past two names the sentence counts rather than enumerates: {many}"
    );
}

/// A reader who misses the loudest rule from what to fix finds out why in
/// the same breath, without opening the JSON.
#[test]
fn the_terminal_says_why_a_noisy_rule_is_missing_from_what_to_fix() {
    let mut report = report();
    report.diagnostics = vec![
        diagnostic("clippy::indexing_slicing", "reliability", 60),
        diagnostic(
            "rust_doctor::cargo::duplicate_major_versions",
            "dependencies",
            2,
        ),
    ];
    report.audit = Audit::build(1, Status::Complete, &report.diagnostics);
    report.summary = Summary::from_diagnostics(&report.diagnostics);

    let output = rendered(&report, 100, false, false);

    assert!(
        output.contains("Fix the top 1 rules"),
        "the quiet rule is what the report recommends: {output}"
    );
    assert!(
        output.contains("clippy::indexing_slicing reports here but is left out"),
        "the loud rule's absence is explained: {output}"
    );
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
        occurrences,
    }
}
