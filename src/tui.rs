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

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use console::{Key, Term};
use rust_doctor::presentation::ReportPresentation;
use rust_doctor::{InspectReport, Status};

use crate::handoff::AvailableAgent;

mod model;
mod screens;
mod text;
mod workflow;

use model::{Entry, PERFECT_SCORE, Row, build_entries, build_rows, pluralize, resolve_layout};
use screens::{Action, CiFeedback, CopyFeedback, LandingNotices, ScoreVariant, ViewerState};
use text::{Line, sanitize};

const SCORE_FRAME_COUNT: u32 = 40;
const SCORE_FRAME_DELAY: Duration = Duration::from_millis(50);
const PROJECTION_FRAME_COUNT: u32 = 16;
const PROJECTION_FRAME_DELAY: Duration = Duration::from_millis(35);
const ISSUE_PROMPT_MAX_SITES: usize = 8;
const EXIT_HINT: &str = "esc back · q to quit";

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
    let term = Term::stdout();
    if !term.features().is_attended() {
        return Ok(Completed {
            outcome: Outcome::Quit,
            installed_workflow: None,
        });
    }
    let mut canvas = Canvas {
        term,
        color: session.color,
        drawn: 0,
    };
    let mut app = App::new(session);
    canvas.term.hide_cursor()?;
    let outcome = app.run(&mut canvas);
    let _ = canvas.term.show_cursor();
    let _ = canvas.term.flush();
    outcome.map(|outcome| Completed {
        outcome,
        installed_workflow: app.installed_workflow,
    })
}

// ---------------------------------------------------------------------- canvas

struct Canvas {
    term: Term,
    color: bool,
    drawn: usize,
}

impl Canvas {
    /// Terminal geometry, as columns by rows.
    fn geometry(&self) -> (usize, usize) {
        let (rows, columns) = self.term.size();
        (usize::from(columns).max(1), usize::from(rows).max(1))
    }

    fn draw(&mut self, frame: &[Line], height: usize) -> io::Result<()> {
        let mut buffer = String::new();
        if self.drawn > 0 {
            let _ = write!(buffer, "\u{1b}[{}A", self.drawn);
        }
        buffer.push('\r');
        let mut written = 0usize;
        for line in frame.iter().take(height) {
            buffer.push_str(&line.render(self.color, self.color));
            // Clearing to the end of the row is what lets a frame shrink
            // without leaving the previous one's tail behind.
            buffer.push_str("\u{1b}[K\r\n");
            written = written.saturating_add(1);
        }
        buffer.push_str("\u{1b}[J");
        self.term.write_str(&buffer)?;
        self.term.flush()?;
        self.drawn = written;
        Ok(())
    }
}

// ------------------------------------------------------------------- the app

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Landing,
    Issues,
    Ci,
    HandoffCi,
    Handoff,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
    landing_selected: usize,
    handoff_selected: usize,
    ci_selected: usize,
    recommendation_selected: usize,
    ci_feedback: Option<CiFeedback>,
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
            project_name: project_name(session),
            rows,
            entries,
            view: View::Landing,
            landing_selected: 0,
            handoff_selected: 0,
            ci_selected: 0,
            recommendation_selected: 0,
            ci_feedback: None,
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
        self.show_actions = true;
        loop {
            let (columns, rows) = canvas.geometry();
            let frame = self.frame(columns, rows);
            canvas.draw(&frame, rows.saturating_sub(1).max(1))?;
            match self.handle(canvas.term.read_key_raw()?, columns, rows) {
                Flow::Continue => {}
                Flow::Exit(outcome) => {
                    let frame = self.frame(columns, rows);
                    canvas.draw(&frame, rows.saturating_sub(1).max(1))?;
                    return Ok(outcome);
                }
            }
        }
    }

    /// The count-up of `use-animated-score.ts`: the value eases from zero over
    /// forty frames, then the projected gain grows out of it over sixteen more.
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
        for frame in 0..=SCORE_FRAME_COUNT {
            self.displayed_score = eased(0, score.value, frame, SCORE_FRAME_COUNT);
            self.paint(canvas)?;
            if frame < SCORE_FRAME_COUNT {
                sleep(SCORE_FRAME_DELAY);
            }
        }
        self.displayed_score = score.value;
        let Some(target) = target else {
            return Ok(());
        };
        for frame in 1..=PROJECTION_FRAME_COUNT {
            self.displayed_projection =
                Some(eased(score.value, target, frame, PROJECTION_FRAME_COUNT));
            self.paint(canvas)?;
            sleep(PROJECTION_FRAME_DELAY);
        }
        self.displayed_projection = Some(target);
        Ok(())
    }

    fn paint(&self, canvas: &mut Canvas) -> io::Result<()> {
        let (columns, rows) = canvas.geometry();
        let frame = self.frame(columns, rows);
        canvas.draw(&frame, rows.saturating_sub(1).max(1))
    }

    // ------------------------------------------------------------- rendering

    fn frame(&self, columns: usize, rows: usize) -> Vec<Line> {
        let layout = resolve_layout(columns, rows, self.entries.len());
        let header_width = match layout.arrangement {
            model::Arrangement::Split => layout.list_column_width,
            _ => layout.width,
        };
        match self.view {
            View::Landing => {
                let actions: Vec<Action> = self
                    .landing_actions(layout.width)
                    .into_iter()
                    .map(|(_, action)| action)
                    .collect();
                screens::landing(
                    self.score_header(ScoreVariant::Landing, header_width),
                    &self.notices(),
                    &actions,
                    self.landing_selected,
                    self.show_actions,
                )
            }
            View::Handoff => {
                screens::agent_handoff(&self.handoff_actions(), self.handoff_selected)
            }
            View::HandoffCi => screens::ci_recommendation(
                &recommendation_actions(),
                self.recommendation_selected,
                layout.width,
            ),
            View::Ci => screens::ci_setup(
                &ci_actions(),
                self.ci_selected,
                self.ci_feedback.as_ref(),
                layout.width,
            ),
            View::Issues => {
                let header = if layout.shows_viewer_score_header {
                    self.score_header(ScoreVariant::Viewer, header_width)
                } else {
                    Vec::new()
                };
                let read = |index: usize| self.read.contains(&index);
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
                    read: &read,
                    workspace_root: self.session.workspace_root,
                    copy_feedback: self.copy_feedback,
                    exit_hint: EXIT_HINT,
                };
                screens::viewer(&state, layout)
            }
        }
    }

    fn score_header(&self, variant: ScoreVariant, width: usize) -> Vec<Line> {
        let (score, projection) = match variant {
            ScoreVariant::Landing => (self.displayed_score, self.displayed_projection),
            ScoreVariant::Viewer => (
                self.session
                    .report
                    .audit
                    .score
                    .as_ref()
                    .map_or(0, |score| score.value),
                None,
            ),
        };
        screens::score_header(
            variant,
            self.session.report.audit.score.as_ref(),
            &self.project_name,
            self.session.presentation.finding_count,
            width,
            score,
            projection,
        )
    }

    fn notices(&self) -> LandingNotices {
        LandingNotices {
            issue_count: self.rows.len(),
            incomplete: incomplete_message(self.session.report),
            empty_state: None,
        }
    }

    fn landing_actions(&self, width: usize) -> Vec<(LandingAction, Action)> {
        let mut actions = Vec::new();
        if !self.rows.is_empty() {
            actions.push((
                LandingAction::Review,
                Action::new(format!("Review {}", pluralize(self.rows.len(), "issue"))),
            ));
        }
        if self.ci_available {
            actions.push((
                LandingAction::AddToCi,
                Action::described(
                    "Add to GitHub Actions (Recommended)",
                    screens::ci_justification(width),
                ),
            ));
        }
        if !self.rows.is_empty() {
            actions.push((LandingAction::Handoff, Action::new("Hand off to an agent")));
        }
        actions
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

    fn handle(&mut self, key: Key, columns: usize, rows: usize) -> Flow {
        if matches!(key, Key::Char('q') | Key::CtrlC) {
            return Flow::Exit(Outcome::Quit);
        }
        match self.view {
            View::Landing => self.handle_landing(key, columns),
            View::Handoff => self.handle_handoff(key),
            View::HandoffCi => self.handle_recommendation(key),
            View::Ci => self.handle_ci(key),
            View::Issues => self.handle_issues(key, columns, rows),
        }
    }

    fn handle_landing(&mut self, key: Key, columns: usize) -> Flow {
        let actions = self.landing_actions(columns);
        match key {
            Key::Escape => return Flow::Exit(Outcome::Quit),
            Key::ArrowUp | Key::Char('k') => {
                self.landing_selected = self.landing_selected.saturating_sub(1);
            }
            Key::ArrowDown | Key::Char('j') => {
                self.landing_selected =
                    (self.landing_selected + 1).min(actions.len().saturating_sub(1));
            }
            Key::Enter => match actions.get(self.landing_selected).map(|(kind, _)| *kind) {
                Some(LandingAction::Review) => {
                    self.mark_read();
                    self.view = View::Issues;
                }
                Some(LandingAction::AddToCi) => {
                    self.ci_feedback = None;
                    self.ci_selected = 0;
                    self.view = View::Ci;
                }
                Some(LandingAction::Handoff) => {
                    self.handoff_selected = 0;
                    self.view = if self.ci_available {
                        self.recommendation_selected = 0;
                        View::HandoffCi
                    } else {
                        View::Handoff
                    };
                }
                None => {}
            },
            _ => {}
        }
        Flow::Continue
    }

    fn handle_handoff(&mut self, key: Key) -> Flow {
        let count = self.session.agents.len() + 1;
        match key {
            Key::Escape => self.view = View::Landing,
            Key::ArrowUp | Key::Char('k') => {
                self.handoff_selected = self.handoff_selected.saturating_sub(1);
            }
            Key::ArrowDown | Key::Char('j') => {
                self.handoff_selected = (self.handoff_selected + 1).min(count - 1);
            }
            Key::Enter => {
                return Flow::Exit(if self.handoff_selected < self.session.agents.len() {
                    Outcome::LaunchAgent(self.handoff_selected)
                } else {
                    Outcome::CopyPrompt
                });
            }
            _ => {}
        }
        Flow::Continue
    }

    fn handle_recommendation(&mut self, key: Key) -> Flow {
        match key {
            Key::Escape => self.view = View::Handoff,
            Key::ArrowUp | Key::Char('k') => {
                self.recommendation_selected = self.recommendation_selected.saturating_sub(1);
            }
            Key::ArrowDown | Key::Char('j') => {
                self.recommendation_selected = (self.recommendation_selected + 1).min(1);
            }
            Key::Enter => {
                if self.recommendation_selected == 0 {
                    self.install_workflow();
                }
                self.view = View::Handoff;
            }
            _ => {}
        }
        Flow::Continue
    }

    fn handle_ci(&mut self, key: Key) -> Flow {
        match key {
            Key::Escape => {
                self.ci_feedback = None;
                self.view = View::Landing;
            }
            Key::ArrowUp | Key::Char('k') => {
                self.ci_selected = self.ci_selected.saturating_sub(1);
            }
            Key::ArrowDown | Key::Char('j') => self.ci_selected = (self.ci_selected + 1).min(1),
            Key::Enter => {
                if self.ci_selected == 0 {
                    self.install_workflow();
                    return Flow::Exit(Outcome::Quit);
                }
                let opened = open_url(screens::GITHUB_ACTIONS_SETUP_URL);
                self.ci_feedback = Some(CiFeedback {
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
            _ => {}
        }
        Flow::Continue
    }

    fn handle_issues(&mut self, key: Key, columns: usize, rows: usize) -> Flow {
        let height = resolve_layout(columns, rows, self.entries.len()).list_height;
        let is_second_g = self.awaiting_second_g && key == Key::Char('g');
        if key != Key::Char('g') {
            self.awaiting_second_g = false;
        }
        match key {
            Key::Escape => {
                // Coming back from the issues, the menu lands on what to do
                // next rather than on the review the reader just left.
                self.landing_selected = self
                    .landing_actions(columns)
                    .iter()
                    .position(|(kind, _)| *kind != LandingAction::Review)
                    .unwrap_or(0);
                self.copy_feedback = None;
                self.view = View::Landing;
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
        if height > 0 {
            if next < self.offset {
                self.offset = next;
            } else if next >= self.offset + height {
                self.offset = next + 1 - height;
            }
        }
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
        let Some(row) = self
            .entries
            .get(self.selected_entry)
            .and_then(Entry::row_index)
            .and_then(|index| self.rows.get(index))
        else {
            return;
        };
        let prompt = issue_prompt(row, &self.project_name);
        self.copy_feedback = Some(match crate::handoff::copy_text(&prompt) {
            Ok(()) => CopyFeedback::Copied,
            Err(_) => CopyFeedback::Failed,
        });
    }

    fn install_workflow(&mut self) {
        match workflow::install(self.session.workspace_root) {
            Ok(path) => {
                self.ci_available = false;
                self.installed_workflow = Some(path);
            }
            Err(error) => {
                self.ci_feedback = Some(CiFeedback {
                    succeeded: false,
                    message: format!("Could not add the workflow: {error}"),
                });
            }
        }
    }
}

fn ci_actions() -> Vec<Action> {
    vec![
        Action::new("Yes, add the workflow"),
        Action::new("Open the GitHub Actions guide"),
    ]
}

fn recommendation_actions() -> Vec<Action> {
    vec![
        Action::new("Add to GitHub Actions first (Recommended)"),
        Action::new("Continue without GitHub Actions"),
    ]
}

// --------------------------------------------------------------- derivations

fn project_name(session: &Session<'_>) -> String {
    if let Some(project) = session.report.project.as_ref()
        && let [package] = project.packages.as_slice()
    {
        return package.name.clone();
    }
    session
        .workspace_root
        .canonicalize()
        .ok()
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str().map(str::to_owned))
        .unwrap_or_else(|| "workspace".to_owned())
}

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

fn ease_out_cubic(progress: f64) -> f64 {
    1.0 - (1.0 - progress).powi(3)
}

/// Value shown at one frame of an eased run from `from` to `to`.
fn eased(from: u8, to: u8, frame: u32, frames: u32) -> u8 {
    let progress = ease_out_cubic(f64::from(frame) / f64::from(frames));
    let value = f64::from(to)
        .mul_add(progress, f64::from(from) * (1.0 - progress))
        .round();
    if value <= 0.0 {
        0
    } else if value >= f64::from(PERFECT_SCORE) {
        PERFECT_SCORE
    } else {
        value as u8
    }
}

/// The clipboard payload of one rule, the transposition of
/// `build-issue-prompt.ts`. Every field is sanitized before it is joined, so a
/// message carrying an escape sequence cannot travel through the clipboard.
fn issue_prompt(row: &Row, project_name: &str) -> String {
    let severity = if row.is_error() { "ERROR" } else { "WARN" };
    let mut lines = vec![
        format!(
            "Fix exactly one Rust Doctor rule in {}:",
            sanitize(project_name)
        ),
        String::new(),
        format!(
            "{severity} {}: {} ({}, ×{})",
            sanitize(row.category.as_str()),
            sanitize(&row.title),
            sanitize(&row.rule_id),
            row.site_count
        ),
        sanitize(&row.message),
    ];
    if let Some(help) = &row.help {
        lines.push(String::new());
        lines.push(format!("Suggested fix: {}", sanitize(help)));
    }
    lines.extend([
        String::new(),
        "Scope:".to_owned(),
        format!("- Fix only {}.", sanitize(&row.rule_id)),
        "- Fix the root cause; do not suppress, disable, or silence the rule.".to_owned(),
        "- Keep unrelated refactors out of this pass.".to_owned(),
        String::new(),
        "Affected sites:".to_owned(),
    ]);
    lines.extend(
        row.sites
            .iter()
            .take(ISSUE_PROMPT_MAX_SITES)
            .map(|site| format!("- {}", sanitize(site))),
    );
    let remaining = row.sites.len().saturating_sub(ISSUE_PROMPT_MAX_SITES);
    if remaining > 0 {
        lines.push(format!("- +{remaining} more sites"));
    }
    if !row.rule_url.is_empty() {
        lines.push(String::new());
        lines.push(format!("Learn more: {}", sanitize(&row.rule_url)));
    }
    lines.extend([
        String::new(),
        format!(
            "Verify with `rust-doctor . --yes --verbose` and confirm {} is gone before moving on.",
            sanitize(&row.rule_id)
        ),
    ]);
    lines.join("\n")
}

/// Hands a URL to the desktop. Both streams go to the void: a browser launcher
/// that writes to stdout would land in the middle of a frame.
fn open_url(url: &str) -> bool {
    let (program, arguments): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };
    Command::new(program)
        .args(arguments)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_doctor::AuditCategoryName;
    use rust_doctor::Severity;

    fn row() -> Row {
        Row {
            rule_id: "clippy::unwrap_used".to_owned(),
            title: "Unwrap used".to_owned(),
            category: AuditCategoryName::Bugs,
            severity: Severity::Warning,
            site_count: 12,
            location: "src/lib.rs:4".to_owned(),
            message: "called `unwrap` on a value".to_owned(),
            help: Some("Use `?` or `unwrap_or`.".to_owned()),
            rule_url: "https://rust-doctor.vercel.app/rules/clippy".to_owned(),
            frame: None,
            sites: (1..=10).map(|line| format!("src/lib.rs:{line}")).collect(),
        }
    }

    #[test]
    fn the_issue_prompt_names_one_rule_bounds_its_sites_and_ends_on_the_rescan() {
        let prompt = issue_prompt(&row(), "workspace");
        assert!(prompt.starts_with("Fix exactly one Rust Doctor rule in workspace:"));
        assert!(prompt.contains("WARN Bugs: Unwrap used (clippy::unwrap_used, ×12)"));
        assert!(prompt.contains("- Fix only clippy::unwrap_used."));
        assert_eq!(prompt.matches("- src/lib.rs:").count(), ISSUE_PROMPT_MAX_SITES);
        assert!(prompt.contains("- +2 more sites"));
        assert!(prompt.ends_with("confirm clippy::unwrap_used is gone before moving on."));
    }

    #[test]
    fn hostile_report_text_never_reaches_the_clipboard_payload() {
        let mut hostile = row();
        hostile.message = "safe\u{1b}[2Jwiped".to_owned();
        hostile.title = "title\nsecond line".to_owned();
        let prompt = issue_prompt(&hostile, "workspace");
        assert!(prompt.contains("safewiped"));
        assert!(prompt.contains("titlesecond line"));
        assert!(!prompt.contains('\u{1b}'));
    }

    #[test]
    fn the_count_up_starts_at_zero_lands_on_the_score_and_never_goes_back() {
        for value in [1u8, 42, 99, 100] {
            let counted: Vec<u8> = (0..=SCORE_FRAME_COUNT)
                .map(|frame| eased(0, value, frame, SCORE_FRAME_COUNT))
                .collect();
            assert_eq!(counted[0], 0);
            assert_eq!(counted[counted.len() - 1], value);
            assert!(counted.windows(2).all(|pair| pair[0] <= pair[1]));
        }
        assert_eq!(eased(60, 90, PROJECTION_FRAME_COUNT, PROJECTION_FRAME_COUNT), 90);
        assert_eq!(eased(60, 90, 0, PROJECTION_FRAME_COUNT), 60);
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
        assert_eq!(
            incomplete_message(&complete_report(Status::Complete, &[])),
            None
        );
        assert_eq!(
            incomplete_message(&complete_report(Status::Incomplete, &["structure", "repo"])),
            Some("structure and repo checks did not complete: results are incomplete.".to_owned())
        );
    }

    fn complete_report(status: Status, stages: &[&str]) -> InspectReport {
        use rust_doctor::{
            Audit, BlockingLevel, GateReport, GateStatus, ReportError, ScanReport, Summary,
            ToolchainReport,
        };
        InspectReport {
            schema_version: rust_doctor::SCHEMA_VERSION,
            audit: Audit::build(0, status, &[]),
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
}
