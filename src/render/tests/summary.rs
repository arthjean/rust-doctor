//! The default view: one framed group, then one row per scored rule, at any
//! width from the floor of 40 columns up.

use super::*;

/// `count` scored groups, one site each, of distinct catalogued Clippy rules.
fn groups(count: usize) -> InspectReport {
    let diagnostics: Vec<Diagnostic> = crate::policy::CATALOG
        .iter()
        .filter(|definition| definition.producer == crate::policy::Producer::Clippy)
        .take(count)
        .enumerate()
        .map(|(index, definition)| Diagnostic {
            id: format!("group-{index:02}"),
            path: Some(format!("src/module_{index:02}.rs")),
            span: Some(crate::DiagnosticSpan {
                line_start: index + 1,
                column_start: 5,
                line_end: index + 1,
                column_end: 9,
            }),
            ..diagnostic(definition.id, definition.category, 1)
        })
        .collect();
    assert_eq!(diagnostics.len(), count);
    report_of(diagnostics, count, 1_000)
}

/// The rows the default view gives the groups after the framed one: each
/// names a rule id and a `path:line:col` site.
fn group_rows(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter(|row| row.contains("  x1  ") && row.contains("src/module_"))
        .collect()
}

#[test]
fn every_scored_rule_gets_a_row_up_to_twenty_then_an_overflow_line() {
    let output = rendered(&groups(3), 140, false, false);
    assert_eq!(output.matches("Rule ID: ").count(), 1, "{output}");
    assert_eq!(group_rows(&output).len(), 2, "{output}");

    let output = rendered(&groups(25), 140, false, false);
    assert_eq!(output.matches("Rule ID: ").count(), 1, "{output}");
    assert_eq!(group_rows(&output).len(), 19, "{output}");
    assert!(output.contains("and 5 more rules: --verbose"), "{output}");
    let row = group_rows(&output)[0];
    assert!(row.starts_with("warning  "), "{row}");
    assert!(row.ends_with(":5"), "a row ends on its first site: {row}");
}

#[test]
fn an_unscored_group_is_counted_in_the_totals_and_given_no_row() {
    let mut report = groups(2);
    report.diagnostics[1].context = Some(crate::DiagnosticContext::Tests);
    report.audit = Audit::build(2, 1_000, Status::Complete, &report.diagnostics);
    let output = rendered(&report, 140, false, false);
    assert!(group_rows(&output).is_empty(), "{output}");
    assert!(output.contains("All 2 occurrences across 2 findings"), "{output}");
}

#[test]
fn every_row_of_the_default_view_fits_every_width_down_to_the_floor() {
    for width in [40, 60, 80, 100, 140] {
        let output = rendered(&groups(25), width, false, false);
        for row in output.lines().filter(|row| !is_link_row(row)) {
            assert!(display_width(row) <= width, "{width}: {row}");
        }
    }
    // A pane narrower than the floor is laid out at the floor.
    let narrow = rendered(&groups(4), 30, false, false);
    assert_eq!(narrow, rendered(&groups(4), MIN_WIDTH, false, false));
}

/// At every width, the floor and below it included, the animated block still
/// rewinds exactly one block per frame: a row wider than the pane would
/// scroll the block away from the cursor-up the animation relies on.
#[test]
fn the_animated_block_rewinds_one_block_per_frame_at_every_width() {
    let report = groups(4);
    for width in [30, MIN_WIDTH, 60, 80, 100, 140] {
        let mut output = Vec::new();
        render_terminal_with_options(
            &report,
            &mut output,
            TerminalOptions {
                workspace_root: Path::new("tests/fixtures/kernel-contract/todo"),
                elapsed: Duration::from_millis(1),
                verbose: false,
                width,
                color: true,
                animate: true,
                requested_scope: None,
            },
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        let rewind = format!("\u{1b}[{}A\r", crate::score_block::BLOCK_ROWS);
        assert_eq!(
            output.matches(&rewind).count() as u32,
            crate::score_block::COUNT_UP_FRAME_COUNT,
            "{width}"
        );
        for row in output.split(&rewind).flat_map(str::lines).filter(|row| !is_link_row(row)) {
            assert!(display_width(&sanitize(row)) <= width.max(MIN_WIDTH), "{width}: {row:?}");
        }
    }
}

/// A group mixing a shipped and a test site labels only the test site, in
/// the default view's frame and in the verbose list of every site.
#[test]
fn only_the_test_site_of_a_mixed_group_is_labeled() {
    let span = |line| crate::DiagnosticSpan {
        line_start: line,
        column_start: 5,
        line_end: line,
        column_end: 9,
    };
    let shipped = Diagnostic {
        id: "shipped".to_owned(),
        span: Some(span(3)),
        ..diagnostic("clippy::todo", "correctness", 1)
    };
    let test = Diagnostic {
        id: "test".to_owned(),
        path: Some("tests/it.rs".to_owned()),
        span: Some(span(7)),
        context: Some(crate::DiagnosticContext::Tests),
        ..diagnostic("clippy::todo", "correctness", 1)
    };
    let report = report_of(vec![shipped, test], 2, 1_000);
    let output = rendered(&report, 140, false, true);
    assert!(output.lines().any(|row| row == "tests/it.rs:7:5 (tests, not scored)"), "{output}");
    assert!(output.lines().any(|row| row == "src/lib.rs:3:5"), "{output}");
    assert_eq!(output.matches("not scored").count(), 1, "{output}");
}

/// An incomplete scan that found nothing says which checks did not finish,
/// in the warning color, and never that nothing was found.
#[test]
fn an_incomplete_scan_with_no_finding_says_what_did_not_complete() {
    let mut report = report_of(Vec::new(), 2, 1_000);
    report.status = Status::Incomplete;
    report.complete = false;
    report.audit = Audit::build(2, 1_000, Status::Incomplete, &report.diagnostics);
    report.errors = vec![ReportError {
        stage: "structure".to_owned(),
        code: "failed".to_owned(),
        message: "stage failed".to_owned(),
    }];
    let output = rendered(&report, 80, true, false);
    assert!(!output.contains("No issues found."), "{output}");
    assert!(
        output.contains("\x1b[33mNo issues detected, but structure checks did not complete."),
        "{output}"
    );
}

#[test]
fn a_report_of_test_files_only_says_why_it_has_no_score() {
    let report = report_of(Vec::new(), 3, 0);
    assert!(report.audit.score.is_none());
    let output = rendered(&report, 80, false, false);
    assert!(
        output.contains("Score unavailable: only tests, benches, examples or build scripts were found."),
        "{output}"
    );
}
