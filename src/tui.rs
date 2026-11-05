use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use console::Term;
use rust_doctor::presentation::ReportPresentation;
use rust_doctor::score_block;
use rust_doctor::InspectReport;

use crate::handoff::{self, AvailableAgent};

mod canvas;
mod frames;
mod input;
mod model;
mod screens;
mod text;

use canvas::Canvas;
use model::{Entry, Layout, Row, build_entries, build_rows, pluralize, project_name, resolve_layout};
use screens::{Action, CopyFeedback, Notice};

const PROJECTION_FRAME_COUNT: u32 = 16;
const PROJECTION_FRAME_DELAY: Duration = Duration::from_millis(35);

const EXIT_HINT: &str = "esc back · q to quit";

const CI_ACTIONS: [&str; 2] = ["Yes, add the workflow", "Open the GitHub Actions guide"];
const RECOMMENDATION_ACTIONS: [&str; 2] = [
    "Add to GitHub Actions first (Recommended)",
    "Continue without GitHub Actions",
];
const INSTALL_ACTION: usize = 0;

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
    pub frame_root: &'a Path,
    pub agents: &'a [AvailableAgent],
    pub color: bool,
    pub animate: bool,
}

pub struct Completed {
    pub outcome: Outcome,
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
    let held = rust_doctor::hold_terminal();
    let mut app = App::new(session);
    let outcome = app.run(&mut canvas);
    canvas.restore();
    drop(held);
    outcome.map(|outcome| Completed {
        outcome,
        installed_workflow: app.installed_workflow,
    })
}

enum View {
    Landing {
        selected: usize,
    },
    Issues,
    Ci {
        selected: usize,
        notice: Option<Notice>,
    },
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
            ci_available: crate::ci::can_install(session.workspace_root),
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

    fn paint(&self, canvas: &mut Canvas) -> io::Result<Layout> {
        let (columns, rows) = canvas.geometry();
        let layout = resolve_layout(columns, rows, self.entries.len());
        canvas.draw(self.frame(layout), columns, rows)?;
        Ok(layout)
    }
}

fn actions_from(labels: &[&str]) -> Vec<Action> {
    labels.iter().map(|label| Action::new(*label)).collect()
}

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
