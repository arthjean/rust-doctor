//! The state machine's tests.
//!
//! They live in their own file because they carry a small harness: an
//! `InspectReport`, a `ReportPresentation` and a scratch workspace. What they
//! cover is the part of the report that has no rendering to assert against,
//! the transitions between screens and the cursors that index them, which is
//! where both of the defects this module was rewritten for were living.

use super::*;
use crate::test_scratch::scratch;
use console::Key;
use rust_doctor::presentation::{DiagnosticGroup, GroupDiagnostic};
use rust_doctor::{AuditCategoryName, Severity};
use std::fs;

/// The interactive report passes the rule the linear one publishes. `tui.rs`
/// carried a 535-line `impl App` that `oversized_unit` reports an impl block
/// at five hundred, and no test named it: the frames and the input handlers
/// have files of their own for that reason.
#[test]
fn the_interactive_report_holds_the_size_bound_the_report_reports_for() {
    for own in [
        include_str!("../tui.rs"),
        include_str!("canvas.rs"),
        include_str!("frames.rs"),
        include_str!("input.rs"),
        include_str!("model.rs"),
        include_str!("screens.rs"),
        include_str!("screens/menu.rs"),
        include_str!("screens/viewer.rs"),
        include_str!("text.rs"),
        include_str!("tests.rs"),
        include_str!("workflow.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < rust_doctor::OVERSIZED_UNIT_FILE_LINES,
            "a file of the interactive report is {lines} lines long, over the {} it reports",
            rust_doctor::OVERSIZED_UNIT_FILE_LINES
        );
    }
}

#[test]
fn the_viewport_follows_the_selection_in_both_directions() {
    assert_eq!(visible_start(20, 0, 0, 5), 0);
    assert_eq!(visible_start(20, 0, 7, 5), 3);
    assert_eq!(visible_start(20, 10, 2, 5), 2);
    assert_eq!(visible_start(20, 18, 19, 5), 15);
    assert_eq!(visible_start(3, 0, 2, 5), 0);
    assert_eq!(visible_start(20, 4, 5, 0), 0);
}

#[test]
fn an_incomplete_scan_names_the_stages_that_did_not_finish() {
    assert_eq!(incomplete_message(&report(Status::Complete, &[])), None);
    assert_eq!(
        incomplete_message(&report(Status::Incomplete, &["structure", "repo"])),
        Some("structure and repo checks did not complete: results are incomplete.".to_owned())
    );
}

/// Adding the workflow removes the entry that offered it, so coming back to
/// the landing has to land on an entry that is still there. Keeping a
/// cursor beside the view left the menu pointing at nothing and Enter inert.
#[test]
fn installing_the_workflow_leaves_the_landing_pointing_at_a_real_entry() {
    let workspace = scratch("tui-tests", "landing-cursor");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let report = report(Status::Complete, &[]);
    let presentation = presentation(&["clippy::a", "clippy::b"]);
    let session = session(&report, &presentation, &workspace);
    let mut app = App::new(&session);
    app.ci_available = true;

    // Landing: Review, Add to GitHub Actions, Hand off. Walk to the last.
    press(&mut app, &[Key::Char('j'), Key::Char('j'), Key::Enter]);
    assert!(matches!(app.view, View::HandoffCi { selected: 0, .. }));

    // Add the workflow, then leave the agent screen.
    press(&mut app, &[Key::Enter]);
    assert!(app.installed_workflow.is_some());
    assert!(!app.ci_available);
    assert!(matches!(app.view, View::Handoff { .. }));

    press(&mut app, &[Key::Escape]);
    let selected =
        landing_cursor(&app.view).expect("escaping the handoff returns to the landing");
    let actions = app.landing_actions();
    assert_eq!(actions.len(), 2, "the CI entry is gone once it is done");
    assert_eq!(actions.get(selected), Some(&LandingAction::Handoff));

    fs::remove_dir_all(workspace).unwrap();
}

/// A workflow that could not be written says so where it was offered. The
/// agent handoff renders no notice, so moving on to it swallowed the reason
/// and left the reader thinking the file had been added.
#[test]
fn a_workflow_that_could_not_be_written_reports_it_on_the_screen_that_offered_it() {
    // A regular file cannot host `.github/workflows`, so the write fails
    // without needing an unwritable directory.
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let report = report(Status::Complete, &[]);
    let presentation = presentation(&["clippy::a"]);
    let session = session(&report, &presentation, &workspace);
    let mut app = App::new(&session);
    app.ci_available = true;

    app.view = View::HandoffCi {
        selected: 0,
        notice: None,
    };
    press(&mut app, &[Key::Enter]);

    assert!(
        matches!(app.view, View::HandoffCi { .. }),
        "a failed write must keep the screen that offered it"
    );
    let notice = ci_notice(&app.view).expect("the failure must be reported");
    assert!(!notice.succeeded);
    assert!(notice.message.starts_with("Could not add the workflow:"));
    assert!(app.installed_workflow.is_none());
    assert!(app.ci_available, "nothing was written, so nothing is done");

    // The same failure on the landing's own CI screen keeps the reader
    // there rather than quitting on a write that never happened.
    app.view = View::Ci {
        selected: 0,
        notice: None,
    };
    assert!(matches!(app.handle_ci(Key::Enter), Flow::Continue));
    assert!(matches!(app.view, View::Ci { notice: Some(_), .. }));
}

/// Leaving the issue list lands on what to do next, never on the review the
/// reader just came out of.
#[test]
fn escaping_the_issue_list_lands_on_what_to_do_next() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).to_owned();
    let report = report(Status::Complete, &[]);
    let presentation = presentation(&["clippy::a"]);
    let session = session(&report, &presentation, &workspace);
    let mut app = App::new(&session);

    app.ci_available = true;
    press(&mut app, &[Key::Enter]);
    assert!(matches!(app.view, View::Issues));
    press(&mut app, &[Key::Escape]);
    let selected =
        landing_cursor(&app.view).expect("escaping the issues returns to the landing");
    assert_eq!(
        app.landing_actions().get(selected),
        Some(&LandingAction::AddToCi)
    );

    // With no CI entry left, "next" is the agent handoff.
    app.ci_available = false;
    app.view = View::Issues;
    press(&mut app, &[Key::Escape]);
    let selected =
        landing_cursor(&app.view).expect("escaping the issues returns to the landing");
    assert_eq!(
        app.landing_actions().get(selected),
        Some(&LandingAction::Handoff)
    );
}

/// `q` and Ctrl-C leave from wherever the reader is, without a screen
/// having to handle them.
#[test]
fn quitting_works_from_every_screen() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).to_owned();
    let report = report(Status::Complete, &[]);
    let presentation = presentation(&["clippy::a"]);
    let session = session(&report, &presentation, &workspace);
    let mut app = App::new(&session);
    let layout = resolve_layout(160, 48, app.entries.len());

    for view in [
        View::Landing { selected: 0 },
        View::Issues,
        View::Handoff { selected: 0 },
        View::HandoffCi {
            selected: 0,
            notice: None,
        },
        View::Ci {
            selected: 0,
            notice: None,
        },
    ] {
        for key in [Key::Char('q'), Key::CtrlC] {
            app.view = match view {
                View::Landing { selected } => View::Landing { selected },
                View::Issues => View::Issues,
                View::Handoff { selected } => View::Handoff { selected },
                View::HandoffCi { selected, .. } => View::HandoffCi {
                    selected,
                    notice: None,
                },
                View::Ci { selected, .. } => View::Ci {
                    selected,
                    notice: None,
                },
            };
            assert!(matches!(
                app.handle(key, layout),
                Flow::Exit(Outcome::Quit)
            ));
        }
    }
}

/// Every screen draws at its declared geometry without a row reaching the
/// last column, whatever the terminal measures.
#[test]
fn no_screen_overflows_the_terminal_it_was_resolved_for() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).to_owned();
    let report = report(Status::Complete, &[]);
    let presentation = presentation(&["clippy::a", "clippy::b", "clippy::c"]);
    let session = session(&report, &presentation, &workspace);
    let mut app = App::new(&session);
    app.show_actions = true;
    app.ci_available = true;

    for (columns, rows) in [(40usize, 12usize), (100, 24), (160, 48), (24, 8)] {
        let layout = resolve_layout(columns, rows, app.entries.len());
        for view in 0..5 {
            app.view = match view {
                0 => View::Landing { selected: 0 },
                1 => View::Issues,
                2 => View::Handoff { selected: 0 },
                3 => View::HandoffCi {
                    selected: 0,
                    notice: None,
                },
                _ => View::Ci {
                    selected: 0,
                    notice: None,
                },
            };
            for line in app.frame(layout) {
                assert!(
                    line.width() < columns,
                    "a row of width {} filled a {columns}-column terminal",
                    line.width()
                );
            }
        }
    }
}

/// The block at the narrowest terminal the interactive report draws one in.
///
/// Forty columns is over `score_block::MIN_BLOCK_COLUMNS`, so the block is
/// drawn rather than folded into a single line, and every row still leaves the
/// guard column the rewind needs. The frame comes from `App::frame`, the entry
/// point the loop paints through, so the geometry asserted here is the geometry
/// a reader gets.
#[test]
fn the_block_fits_forty_columns_at_every_score_the_report_publishes() {
    for value in [0u8, 37, 73, 100] {
        let report = scored_report(value);
        let presentation = presentation(&["clippy::a", "clippy::b"]);
        let session = session(&report, &presentation, Path::new("."));
        let mut app = App::new(&session);
        app.view = View::Issues;

        let layout = resolve_layout(40, 40, app.entries.len());
        assert!(
            layout.shows_viewer_score_header,
            "forty columns by forty rows draws the block"
        );
        assert!(layout.score_header_width >= score_block::MIN_BLOCK_COLUMNS);
        let bar_width = score_block::bar_width(layout.score_header_width)
            .expect("the block fits forty columns");

        let frame: Vec<String> = app
            .frame(layout)
            .iter()
            .map(|line| line.render(false, false))
            .collect();
        let label = score_block::label_for(value);
        let faces = score_block::face_rows(label);
        let block = frame
            .get(..score_block::BLOCK_ROWS)
            .expect("the block opens the screen");
        assert!(block[0].starts_with(&format!(
            "  {}  {value} / {} {}",
            faces[0],
            score_block::PERFECT_SCORE,
            score_block::label_text(label, true)
        )));
        assert!(block[1].starts_with(&format!("  {}  ", faces[1])));
        assert_eq!(
            block[1].matches('\u{2588}').count(),
            score_block::bar_fill(value, bar_width),
            "the bar of {value} at forty columns"
        );
        assert_eq!(
            block[1].matches('\u{2591}').count(),
            bar_width - score_block::bar_fill(value, bar_width)
        );
        assert!(block[2].contains(score_block::BRANDING_NAME));
        assert_eq!(block[3], format!("  {}  ", faces[3]));
        for row in block {
            assert!(
                rust_doctor::terminal_text::display_width(row) <= 40 - score_block::RIGHT_EDGE_SAFETY_COLUMNS,
                "at {value}, {row:?} reaches the guard column"
            );
        }
    }
}

// ------------------------------------------------------------- fixtures

fn landing_cursor(view: &View) -> Option<usize> {
    match view {
        View::Landing { selected } => Some(*selected),
        _ => None,
    }
}

fn ci_notice(view: &View) -> Option<&Notice> {
    match view {
        View::Ci { notice, .. } | View::HandoffCi { notice, .. } => notice.as_ref(),
        _ => None,
    }
}

fn press(app: &mut App<'_>, keys: &[Key]) {
    let layout = resolve_layout(160, 48, app.entries.len());
    for key in keys {
        app.handle(key.clone(), layout);
    }
}

fn session<'a>(
    report: &'a InspectReport,
    presentation: &'a ReportPresentation,
    workspace_root: &'a Path,
) -> Session<'a> {
    Session {
        report,
        presentation,
        workspace_root,
        agents: &[],
        color: false,
        animate: false,
    }
}

fn presentation(rule_ids: &[&str]) -> ReportPresentation {
    ReportPresentation {
        groups: rule_ids
            .iter()
            .map(|rule_id| DiagnosticGroup {
                rule_id: (*rule_id).to_owned(),
                title: (*rule_id).to_owned(),
                rule_url: String::new(),
                severity: Severity::Warning,
                category: Some(AuditCategoryName::Bugs),
                occurrences: 1,
                diagnostics: vec![GroupDiagnostic {
                    message: "message".to_owned(),
                    help: None,
                    base_severity: Severity::Warning,
                    severity: Severity::Warning,
                    path: None,
                    span: None,
                    related: Vec::new(),
                    occurrences: 1,
                }],
            })
            .collect(),
        migration_advisories: Vec::new(),
        issue_count: rule_ids.len(),
        finding_count: rule_ids.len(),
    }
}

/// The report with a score of `value` on it.
///
/// The interactive report validates nothing: it draws the block the audit
/// published. That the four values below are values core-v3 really produces,
/// and that each carries the band `score_block::label_for` gives it, is what
/// the linear report's own test establishes over reports the audit built.
fn scored_report(value: u8) -> InspectReport {
    use rust_doctor::{AuditScore, ScoreDimensions};

    let mut report = report(Status::Complete, &[]);
    report.audit.score = Some(AuditScore {
        model: rust_doctor::SCORE_MODEL.to_owned(),
        value,
        label: score_block::label_for(value),
        authoritative: true,
        dimensions: ScoreDimensions {
            security: value,
            reliability: value,
            maintainability: value,
            performance: value,
            dependencies: value,
        },
        worst_tier: None,
        applied_ceiling: None,
        projected_after_top_three: None,
        projected_rule_ids: Vec::new(),
        withheld_rule_ids: Vec::new(),
    });
    report
}

fn report(status: Status, stages: &[&str]) -> InspectReport {
    use rust_doctor::{
        Audit, BlockingLevel, GateReport, GateStatus, ReportError, ScanReport, Summary,
        ToolchainReport,
    };
    InspectReport {
        schema_version: rust_doctor::SCHEMA_VERSION,
        audit: Audit::build(0, 0, status, &[]),
        status,
        complete: status == Status::Complete,
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
        diagnostics: Vec::new(),
        delta: None,
        errors: stages
            .iter()
            .map(|stage| ReportError {
                stage: (*stage).to_owned(),
                code: "failed".to_owned(),
                message: "stage failed".to_owned(),
            })
            .collect(),
        summary: Summary::default(),
        gate: GateReport {
            blocking: BlockingLevel::Error,
            status: GateStatus::Passed,
            blocking_diagnostics: Some(0),
        },
    }
}
