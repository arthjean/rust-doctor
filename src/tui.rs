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

use console::{Key, Term};
use rust_doctor::presentation::ReportPresentation;
use rust_doctor::score_block;
use rust_doctor::{InspectReport, Status};

use crate::handoff::{self, AvailableAgent};

mod canvas;
mod model;
mod screens;
mod text;
mod workflow;

use canvas::Canvas;
use model::{Entry, Layout, Row, build_entries, build_rows, pluralize, project_name, resolve_layout};
use screens::{Action, CopyFeedback, MenuInput, Notice, ScoreVariant, ViewerState};
use text::Line;

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

    fn frame(&self, layout: Layout) -> Vec<Line> {
        match &self.view {
            View::Landing { selected } => self.landing_frame(*selected, layout),
            View::Handoff { selected } => {
                let actions = self.handoff_actions();
                screens::menu(screens::Menu {
                    title: screens::handoff_title(),
                    actions: &actions,
                    selected: *selected,
                    notice: None,
                    hint: screens::HINT_MENU_BACK,
                    width: layout.frame_width,
                })
            }
            View::HandoffCi { selected, notice } => {
                let actions = actions_from(&RECOMMENDATION_ACTIONS);
                screens::menu(screens::Menu {
                    title: screens::ci_recommendation_title(layout.width),
                    actions: &actions,
                    selected: *selected,
                    notice: notice.as_ref(),
                    hint: screens::HINT_MENU_SKIP,
                    width: layout.frame_width,
                })
            }
            View::Ci { selected, notice } => {
                let actions = actions_from(&CI_ACTIONS);
                screens::menu(screens::Menu {
                    title: screens::ci_setup_title(layout.width),
                    actions: &actions,
                    selected: *selected,
                    notice: notice.as_ref(),
                    hint: screens::HINT_MENU_BACK,
                    width: layout.frame_width,
                })
            }
            View::Issues => {
                let header = if layout.shows_viewer_score_header {
                    self.score_header(ScoreVariant::Viewer, layout.score_header_width)
                } else {
                    Vec::new()
                };
                let state = ViewerState {
                    rows: &self.rows,
                    entries: &self.entries,
                    header,
                    selected_entry: self.selected_entry,
                    visible_start: visible_start(
                        self.entries.len(),
                        self.offset,
                        self.selected_entry,
                        layout.list_height,
                    ),
                    read: &self.read,
                    workspace_root: self.session.workspace_root,
                    copy_feedback: self.copy_feedback,
                    exit_hint: EXIT_HINT,
                };
                screens::viewer(&state, layout)
            }
        }
    }

    fn landing_frame(&self, selected: usize, layout: Layout) -> Vec<Line> {
        let title = screens::landing_title(
            self.score_header(ScoreVariant::Landing, layout.score_header_width),
            &screens::LandingNotices {
                issue_count: self.rows.len(),
                incomplete: incomplete_message(self.session.report),
            },
        );
        // Nothing is offered while the score is still counting up: the reader
        // is watching a number, not reading a menu.
        if !self.show_actions {
            return title;
        }
        let actions: Vec<Action> = self
            .landing_actions()
            .into_iter()
            .map(|kind| self.landing_action(kind, layout.width))
            .collect();
        let hint = if actions.is_empty() {
            screens::HINT_QUIT
        } else {
            screens::HINT_MENU
        };
        screens::menu(screens::Menu {
            title,
            actions: &actions,
            selected,
            notice: None,
            hint,
            width: layout.frame_width,
        })
    }

    fn score_header(&self, variant: ScoreVariant, width: usize) -> Vec<Line> {
        let score = self.session.report.audit.score.as_ref();
        let (displayed, projection) = match variant {
            ScoreVariant::Landing => (self.displayed_score, self.displayed_projection),
            ScoreVariant::Viewer => (score.map_or(0, |score| score.value), None),
        };
        screens::score_header(
            variant,
            score,
            &self.project_name,
            self.session.presentation.finding_count,
            width,
            displayed,
            projection,
        )
    }

    /// What the landing offers, in order. No width and no labels: building a
    /// wrapped justification paragraph on the path that only needs to know
    /// which entry the cursor is on is work thrown away on every keystroke.
    fn landing_actions(&self) -> Vec<LandingAction> {
        let mut actions = Vec::new();
        if !self.rows.is_empty() {
            actions.push(LandingAction::Review);
        }
        if self.ci_available {
            actions.push(LandingAction::AddToCi);
        }
        if !self.rows.is_empty() {
            actions.push(LandingAction::Handoff);
        }
        actions
    }

    fn landing_action(&self, kind: LandingAction, width: usize) -> Action {
        match kind {
            LandingAction::Review => Action::new(format!(
                "Review {}",
                pluralize(self.rows.len(), "issue")
            )),
            LandingAction::AddToCi => Action::described(
                "Add to GitHub Actions (Recommended)",
                screens::ci_justification(width),
            ),
            LandingAction::Handoff => Action::new("Hand off to an agent"),
        }
    }

    /// The landing with its cursor on `action`, or on its first entry when the
    /// landing no longer offers it. Every way back to the landing states which
    /// entry it means, so a menu can never point past what it shows.
    fn landing_on(&self, action: LandingAction) -> View {
        self.landing_where(|kind| kind == action)
    }

    fn landing_where(&self, wanted: impl Fn(LandingAction) -> bool) -> View {
        View::Landing {
            selected: self
                .landing_actions()
                .into_iter()
                .position(wanted)
                .unwrap_or(0),
        }
    }

    fn handoff_actions(&self) -> Vec<Action> {
        let mut actions: Vec<Action> = self
            .session
            .agents
            .iter()
            .map(|agent| Action::new(agent.label()))
            .collect();
        actions.push(Action::described(
            "Copy prompt",
            vec![Line::text(
                "Paste into any agent or edit it first",
                text::Style::DIM,
            )],
        ));
        actions
    }

    // --------------------------------------------------------------- input

    fn handle(&mut self, key: Key, layout: Layout) -> Flow {
        if matches!(key, Key::Char('q') | Key::CtrlC) {
            return Flow::Exit(Outcome::Quit);
        }
        match self.view {
            View::Landing { .. } => self.handle_landing(key),
            View::Handoff { .. } => self.handle_handoff(key),
            View::HandoffCi { .. } => self.handle_recommendation(key),
            View::Ci { .. } => self.handle_ci(key),
            View::Issues => self.handle_issues(key, layout.list_height),
        }
    }

    fn handle_landing(&mut self, key: Key) -> Flow {
        let actions = self.landing_actions();
        let View::Landing { selected } = &mut self.view else {
            return Flow::Continue;
        };
        match screens::input(key, selected, actions.len()) {
            MenuInput::Back => return Flow::Exit(Outcome::Quit),
            MenuInput::Selected(index) => match actions.get(index) {
                Some(LandingAction::Review) => {
                    self.mark_read();
                    self.view = View::Issues;
                }
                Some(LandingAction::AddToCi) => {
                    self.view = View::Ci {
                        selected: 0,
                        notice: None,
                    };
                }
                Some(LandingAction::Handoff) => {
                    self.view = if self.ci_available {
                        View::HandoffCi {
                            selected: 0,
                            notice: None,
                        }
                    } else {
                        View::Handoff { selected: 0 }
                    };
                }
                None => {}
            },
            MenuInput::Moved | MenuInput::Ignored => {}
        }
        Flow::Continue
    }

    fn handle_handoff(&mut self, key: Key) -> Flow {
        let agents = self.session.agents.len();
        let View::Handoff { selected } = &mut self.view else {
            return Flow::Continue;
        };
        match screens::input(key, selected, agents.saturating_add(1)) {
            MenuInput::Back => {
                self.view = self.landing_on(LandingAction::Handoff);
            }
            MenuInput::Selected(index) => {
                return Flow::Exit(if index < agents {
                    Outcome::LaunchAgent(index)
                } else {
                    Outcome::CopyPrompt
                });
            }
            MenuInput::Moved | MenuInput::Ignored => {}
        }
        Flow::Continue
    }

    fn handle_recommendation(&mut self, key: Key) -> Flow {
        let View::HandoffCi { selected, .. } = &mut self.view else {
            return Flow::Continue;
        };
        match screens::input(key, selected, RECOMMENDATION_ACTIONS.len()) {
            MenuInput::Selected(INSTALL_ACTION) => {
                // A write that failed keeps the reader on the screen that
                // offered it: the agent handoff has nowhere to show a notice,
                // so moving on would swallow the reason.
                match self.install_workflow() {
                    Some(notice) => self.set_ci_notice(notice),
                    None => self.view = View::Handoff { selected: 0 },
                }
            }
            MenuInput::Back | MenuInput::Selected(_) => {
                self.view = View::Handoff { selected: 0 };
            }
            MenuInput::Moved | MenuInput::Ignored => {}
        }
        Flow::Continue
    }

    fn handle_ci(&mut self, key: Key) -> Flow {
        let View::Ci { selected, .. } = &mut self.view else {
            return Flow::Continue;
        };
        match screens::input(key, selected, CI_ACTIONS.len()) {
            MenuInput::Back => {
                self.view = self.landing_on(LandingAction::AddToCi);
            }
            MenuInput::Selected(INSTALL_ACTION) => match self.install_workflow() {
                // The workflow is written and the caller says so on a screen
                // this report no longer owns.
                None => return Flow::Exit(Outcome::Quit),
                Some(notice) => self.set_ci_notice(notice),
            },
            MenuInput::Selected(_) => {
                let opened = handoff::open_url(screens::GITHUB_ACTIONS_SETUP_URL);
                self.set_ci_notice(Notice {
                    succeeded: opened,
                    message: if opened {
                        "✓ Opened the GitHub Actions guide in your browser".to_owned()
                    } else {
                        format!(
                            "Couldn't open a browser. Visit {}",
                            screens::GITHUB_ACTIONS_SETUP_URL
                        )
                    },
                });
            }
            MenuInput::Moved | MenuInput::Ignored => {}
        }
        Flow::Continue
    }

    fn handle_issues(&mut self, key: Key, height: usize) -> Flow {
        let is_second_g = self.awaiting_second_g && key == Key::Char('g');
        if key != Key::Char('g') {
            self.awaiting_second_g = false;
        }
        match key {
            Key::Escape => {
                // Coming back from the issues, the menu lands on what to do
                // next rather than on the review the reader just left.
                self.copy_feedback = None;
                self.view = self.landing_where(|kind| kind != LandingAction::Review);
            }
            Key::ArrowDown | Key::Char('j') => self.move_to(self.selected_entry + 1, true, height),
            Key::ArrowUp | Key::Char('k') => {
                self.move_to(self.selected_entry.saturating_sub(1), false, height);
            }
            Key::PageDown => self.move_to(self.selected_entry + height, true, height),
            Key::PageUp => {
                self.move_to(self.selected_entry.saturating_sub(height), false, height);
            }
            Key::Char('G') => self.move_to(self.entries.len().saturating_sub(1), false, height),
            Key::Char('g') => {
                if is_second_g {
                    self.move_to(0, true, height);
                } else {
                    self.awaiting_second_g = true;
                }
            }
            Key::Enter => self.copy_issue_context(),
            _ => {}
        }
        Flow::Continue
    }

    fn move_to(&mut self, target: usize, forward: bool, height: usize) {
        if self.entries.is_empty() {
            return;
        }
        let bounded = target.min(self.entries.len() - 1);
        let Some(next) = self.nearest_selectable(bounded, forward) else {
            return;
        };
        self.selected_entry = next;
        self.copy_feedback = None;
        self.mark_read();
        // The viewport follows the selection by exactly one rule, the one the
        // frame reapplies when the terminal has been resized under it.
        self.offset = visible_start(self.entries.len(), self.offset, next, height);
    }

    fn nearest_selectable(&self, from: usize, forward: bool) -> Option<usize> {
        let ahead = self.seek(from, forward);
        ahead.or_else(|| self.seek(from, !forward))
    }

    fn seek(&self, from: usize, forward: bool) -> Option<usize> {
        let selectable = |index: &usize| {
            self.entries
                .get(*index)
                .is_some_and(|entry| entry.row_index().is_some())
        };
        if forward {
            (from..self.entries.len()).find(selectable)
        } else {
            (0..=from).rev().find(selectable)
        }
    }

    fn selected_row(&self) -> Option<&Row> {
        self.entries
            .get(self.selected_entry)
            .and_then(Entry::row_index)
            .and_then(|index| self.rows.get(index))
    }

    fn mark_read(&mut self) {
        if let Some(index) = self
            .entries
            .get(self.selected_entry)
            .and_then(Entry::row_index)
        {
            self.read.insert(index);
        }
    }

    fn copy_issue_context(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let issue = handoff::IssuePrompt {
            project_name: &self.project_name,
            rule_id: &row.rule_id,
            title: &row.title,
            category: row.category.as_str(),
            is_error: row.is_error(),
            site_count: row.site_count,
            message: &row.message,
            help: row.help.as_deref(),
            rule_url: &row.rule_url,
            sites: &row.sites,
        };
        let copied = handoff::build_issue_prompt(&issue)
            .and_then(|payload| handoff::copy_to_clipboard(&payload));
        self.copy_feedback = Some(match copied {
            Ok(()) => CopyFeedback::Copied,
            Err(_) => CopyFeedback::Failed,
        });
    }

    /// Writes the workflow, answering with what the reader has to be told.
    /// Nothing to say means it landed.
    fn install_workflow(&mut self) -> Option<Notice> {
        match workflow::install(self.session.workspace_root) {
            Ok(path) => {
                self.ci_available = false;
                self.installed_workflow = Some(path);
                None
            }
            Err(error) => Some(Notice {
                succeeded: false,
                message: format!("Could not add the workflow: {error}"),
            }),
        }
    }

    fn set_ci_notice(&mut self, notice: Notice) {
        if let View::Ci { notice: slot, .. } | View::HandoffCi { notice: slot, .. } = &mut self.view
        {
            *slot = Some(notice);
        }
    }
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
