//! Every frame the report paints, and the actions each screen offers.
//!
//! It carries an `impl App` of its own rather than sitting in `tui.rs` beside
//! the state machine, because the two answer different questions and together
//! they put the block over the five hundred lines `oversized_unit` reports an
//! impl at: this half is what a state looks like, `input.rs` is what a key
//! does to it.

use super::screens::{Action, ScoreVariant, ViewerState};
use super::text::{self, Line};
use super::{
    App, CI_ACTIONS, EXIT_HINT, LandingAction, Layout, RECOMMENDATION_ACTIONS, View, actions_from,
    incomplete_message, pluralize, screens, visible_start,
};

impl App<'_> {
    pub(super) fn frame(&self, layout: Layout) -> Vec<Line> {
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

    pub(super) fn landing_frame(&self, selected: usize, layout: Layout) -> Vec<Line> {
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

    pub(super) fn score_header(&self, variant: ScoreVariant, width: usize) -> Vec<Line> {
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
    pub(super) fn landing_actions(&self) -> Vec<LandingAction> {
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

    pub(super) fn landing_action(&self, kind: LandingAction, width: usize) -> Action {
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
    pub(super) fn landing_on(&self, action: LandingAction) -> View {
        self.landing_where(|kind| kind == action)
    }

    pub(super) fn landing_where(&self, wanted: impl Fn(LandingAction) -> bool) -> View {
        View::Landing {
            selected: self
                .landing_actions()
                .into_iter()
                .position(wanted)
                .unwrap_or(0),
        }
    }

    pub(super) fn handoff_actions(&self) -> Vec<Action> {
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
}
