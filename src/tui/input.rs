//! What a key does, and the effects a chosen action runs.
//!
//! It carries an `impl App` of its own for the reason `frames.rs` does: one
//! impl block holding both halves reaches the five hundred lines
//! `oversized_unit` reports at, and the halves are separable. Every menu still
//! answers to one `screens::input`, so no screen here carries an upper bound
//! of its own.

use console::Key;

use super::model::{Entry, Row};
use super::screens::{CopyFeedback, MenuInput, Notice};
use super::{
    App, CI_ACTIONS, Flow, INSTALL_ACTION, LandingAction, Layout, Outcome, RECOMMENDATION_ACTIONS,
    View, handoff, screens, visible_start, workflow,
};

impl App<'_> {
    pub(super) fn handle(&mut self, key: Key, layout: Layout) -> Flow {
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

    pub(super) fn handle_landing(&mut self, key: Key) -> Flow {
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

    pub(super) fn handle_handoff(&mut self, key: Key) -> Flow {
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

    pub(super) fn handle_recommendation(&mut self, key: Key) -> Flow {
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

    pub(super) fn handle_ci(&mut self, key: Key) -> Flow {
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

    pub(super) fn handle_issues(&mut self, key: Key, height: usize) -> Flow {
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

    pub(super) fn move_to(&mut self, target: usize, forward: bool, height: usize) {
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

    pub(super) fn nearest_selectable(&self, from: usize, forward: bool) -> Option<usize> {
        let ahead = self.seek(from, forward);
        ahead.or_else(|| self.seek(from, !forward))
    }

    pub(super) fn seek(&self, from: usize, forward: bool) -> Option<usize> {
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

    pub(super) fn selected_row(&self) -> Option<&Row> {
        self.entries
            .get(self.selected_entry)
            .and_then(Entry::row_index)
            .and_then(|index| self.rows.get(index))
    }

    pub(super) fn mark_read(&mut self) {
        if let Some(index) = self
            .entries
            .get(self.selected_entry)
            .and_then(Entry::row_index)
        {
            self.read.insert(index);
        }
    }

    pub(super) fn copy_issue_context(&mut self) {
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
    pub(super) fn install_workflow(&mut self) -> Option<Notice> {
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

    pub(super) fn set_ci_notice(&mut self, notice: Notice) {
        if let View::Ci { notice: slot, .. } | View::HandoffCi { notice: slot, .. } = &mut self.view
        {
            *slot = Some(notice);
        }
    }
}
