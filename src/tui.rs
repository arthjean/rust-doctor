//! The interactive report.
//!
//! A transposition of React Doctor's Ink application: the same five screens,
//! the same geometry, the same keys. It takes over stdout when both ends are a
//! real terminal and the run asked for neither `--json`, `--yes` nor
//! `--verbose`; every other run keeps the linear report `render::terminal`
//! writes, which is what pipes, CI and agents read.
//!
//! Frames are rewritten in place the way Ink does: the cursor climbs back over
//! the previous frame, every row clears to its end, and whatever the shorter
//! frame left below is erased. No alternate screen, so the last frame survives
//! in the scrollback exactly as the reference leaves it.
//!
//! Each screen carries its own cursor, in [`View`]. A flat set of cursors
//! beside a view tag says nothing about which one is live, and the two that
//! went out of step with the screen they indexed both produced a defect: a menu
//! pointing past its own actions, and a failed write reported onto a screen
//! that does not show notices.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use console::Term;
use rust_doctor::presentation::ReportPresentation;
use rust_doctor::score_block;
use rust_doctor::{InspectReport, Status};

use crate::handoff::{self, AvailableAgent};

mod canvas;
mod frames;
mod input;
mod model;
mod screens;
mod text;
mod workflow;

use canvas::Canvas;
use model::{Entry, Layout, Row, build_entries, build_rows, pluralize, project_name, resolve_layout};
use screens::{Action, CopyFeedback, Notice};

/// Frames the projected gain grows over, once the score has finished counting
/// up. The count-up itself is [`score_block::COUNT_UP_FRAME_COUNT`], shared
/// with the linear report; what follows it is this report's own.
const PROJECTION_FRAME_COUNT: u32 = 16;
const PROJECTION_FRAME_DELAY: Duration = Duration::from_millis(35);

const EXIT_HINT: &str = "esc back · q to quit";

const CI_ACTIONS: [&str; 2] = ["Yes, add the workflow", "Open the GitHub Actions guide"];
const RECOMMENDATION_ACTIONS: [&str; 2] = [
    "Add to GitHub Actions first (Recommended)",
    "Continue without GitHub Actions",
];
/// Both GitHub Actions menus open on the entry that writes the workflow, and
/// both handlers match on it by this name rather than on a bare zero.
const INSTALL_ACTION: usize = 0;

/// What the reader asked for on the way out. Anything that takes the terminal
/// over, launching an agent above all, happens after the loop has given it
/// back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Quit,
    LaunchAgent(usize),
    CopyPrompt,
}

pub struct Session<'a> {
    pub report: &'a InspectReport,
    pub presentation: &'a ReportPresentation,
    pub workspace_root: &'a Path,
    pub agents: &'a [AvailableAgent],
    pub color: bool,
    pub animate: bool,
}

pub struct Completed {
    pub outcome: Outcome,
    /// Set when the reader added the GitHub Actions workflow, so the caller can
    /// say so once the screen is its own again.
    pub installed_workflow: Option<PathBuf>,
}

pub fn run(session: &Session<'_>) -> io::Result<Completed> {
    let mut canvas = Canvas::new(Term::stdout(), session.color);
    if !canvas.is_attended() {
        return Ok(Completed {
            outcome: Outcome::Quit,
            installed_workflow: None,
        });
    }
    let mut app = App::new(session);
    let outcome = app.run(&mut canvas);
    canvas.restore();
    outcome.map(|outcome| Completed {
        outcome,
        installed_workflow: app.installed_workflow,
    })
}

// ------------------------------------------------------------------- the app

/// The screen on show, and the cursor that belongs to it. A cursor exists only
/// while its screen does, so it can never index a list it does not come from.
enum View {
    Landing {
        selected: usize,
    },
    Issues,
    /// The GitHub Actions screen reached from the landing menu.
    Ci {
        selected: usize,
        notice: Option<Notice>,
    },
    /// The one offered on the way to an agent.
    HandoffCi {
        selected: usize,
        notice: Option<Notice>,
    },
    Handoff {
        selected: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LandingAction {
    Review,
    AddToCi,
    Handoff,
}

enum Flow {
    Continue,
    Exit(Outcome),
}

struct App<'a> {
    session: &'a Session<'a>,
    project_name: String,
    rows: Vec<Row>,
    entries: Vec<Entry>,
    view: View,
    ci_available: bool,
    selected_entry: usize,
    offset: usize,
    read: BTreeSet<usize>,
    copy_feedback: Option<CopyFeedback>,
    awaiting_second_g: bool,
    installed_workflow: Option<PathBuf>,
    displayed_score: u8,
    displayed_projection: Option<u8>,
    show_actions: bool,
}

impl<'a> App<'a> {
    fn new(session: &'a Session<'a>) -> Self {
        let rows = build_rows(session.presentation);
        let entries = build_entries(&rows);
        let selected_entry = entries
            .iter()
            .position(|entry| entry.row_index().is_some())
            .unwrap_or(0);
        Self {
            project_name: project_name(session.report, session.workspace_root),
            rows,
            entries,
            view: View::Landing { selected: 0 },
            ci_available: workflow::can_install(session.workspace_root),
            selected_entry,
            offset: 0,
            read: BTreeSet::new(),
            copy_feedback: None,
            awaiting_second_g: false,
            installed_workflow: None,
            displayed_score: 0,
            displayed_projection: None,
            show_actions: false,
            session,
        }
    }

    fn run(&mut self, canvas: &mut Canvas) -> io::Result<Outcome> {
        self.reveal_score(canvas)?;
        // The cursor is hidden only once the loop owns the terminal. The reveal
        // sleeps between frames with the terminal in cooked mode, so a Ctrl-C
        // there kills the process before anything can put the cursor back; the
        // loop reads Ctrl-C as a key and leaves cleanly.
        canvas.hide_cursor()?;
        self.show_actions = true;
        loop {
            let layout = self.paint(canvas)?;
            match self.handle(canvas.read_key()?, layout) {
                Flow::Continue => {}
                Flow::Exit(outcome) => {
                    self.paint(canvas)?;
                    return Ok(outcome);
                }
            }
        }
    }

    /// The count-up of `use-animated-score.ts`: the value eases from zero over
    /// the shared count-up frames, then the projected gain grows out of it.
    fn reveal_score(&mut self, canvas: &mut Canvas) -> io::Result<()> {
        let Some(score) = self.session.report.audit.score.as_ref() else {
            return Ok(());
        };
        let target = score
            .projected_after_top_three
            .filter(|projection| *projection > score.value);
        if !self.session.animate {
            self.displayed_score = score.value;
            self.displayed_projection = target;
            return Ok(());
        }
        for frame in 0..=score_block::COUNT_UP_FRAME_COUNT {
            self.displayed_score =
                score_block::eased(0, score.value, frame, score_block::COUNT_UP_FRAME_COUNT);
            self.paint(canvas)?;
            if frame < score_block::COUNT_UP_FRAME_COUNT {
                sleep(score_block::COUNT_UP_FRAME_DELAY);
            }
        }
        self.displayed_score = score.value;
        let Some(target) = target else {
            return Ok(());
        };
        for frame in 1..=PROJECTION_FRAME_COUNT {
            self.displayed_projection = Some(score_block::eased(
                score.value,
                target,
                frame,
                PROJECTION_FRAME_COUNT,
            ));
            self.paint(canvas)?;
            sleep(PROJECTION_FRAME_DELAY);
        }
        self.displayed_projection = Some(target);
        Ok(())
    }

    /// Draws one frame and answers with the layout it drew for, so the keys
    /// that follow are read against the geometry the reader is looking at
    /// rather than against a second, independently resolved one.
    fn paint(&self, canvas: &mut Canvas) -> io::Result<Layout> {
        let (columns, rows) = canvas.geometry();
        let layout = resolve_layout(columns, rows, self.entries.len());
        canvas.draw(self.frame(layout), columns, rows)?;
        Ok(layout)
    }

    // ------------------------------------------------------------- rendering
}

fn actions_from(labels: &[&str]) -> Vec<Action> {
    labels.iter().map(|label| Action::new(*label)).collect()
}

// --------------------------------------------------------------- derivations

fn incomplete_message(report: &InspectReport) -> Option<String> {
    if report.status == Status::Complete {
        return None;
    }
    let mut stages: Vec<&str> = report
        .errors
        .iter()
        .map(|error| error.stage.as_str())
        .collect();
    stages.dedup();
    if stages.is_empty() {
        return Some("The scan did not complete: results are incomplete.".to_owned());
    }
    Some(format!(
        "{} checks did not complete: results are incomplete.",
        stages.join(" and ")
    ))
}

/// Where a viewport of `height` rows starts so that `selected` stays inside it.
/// This is the only rule the list scrolls by: the selection keeps it up to
/// date, and the frame reapplies it in case the terminal was resized between
/// two keystrokes.
fn visible_start(item_count: usize, offset: usize, selected: usize, height: usize) -> usize {
    if height == 0 {
        return 0;
    }
    let bounded = offset.min(item_count.saturating_sub(height));
    if selected < bounded {
        return selected;
    }
    if selected >= bounded + height {
        return selected.saturating_sub(height).saturating_add(1);
    }
    bounded
}

#[cfg(test)]
mod tests;
