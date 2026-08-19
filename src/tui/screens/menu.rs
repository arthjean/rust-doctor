//! The menu screens, and the keys they all answer to.
//!
//! The landing, the agent handoff and the two GitHub Actions screens are the
//! same shape: a title block, a list of actions with a cursor, a key hint, and
//! sometimes a notice under the actions. [`menu`] draws that shape once and
//! [`input`] reads it once, so a screen contributes only its title and what
//! selecting means. Four copies of the same six-line key match, each with its
//! own hard-coded upper bound, is how a menu ends up pointing past its actions.

use console::Key;

use crate::tui::model::{ACTION_MENU_ITEM_GAP_ROWS, ACTION_MENU_MARGIN_ROWS, HORIZONTAL_PADDING_COLUMNS};
use crate::tui::text::{Color, Line, Span, Style};

pub const GITHUB_ACTIONS_SETUP_URL: &str = "https://github.com/arthjean/rust-doctor#github-actions";

pub const HINT_QUIT: &str = "q quit";
pub const HINT_MENU: &str = "↑/↓ move · enter select · q quit";
pub const HINT_MENU_BACK: &str = "↑/↓ move · enter select · esc back · q quit";
pub const HINT_MENU_SKIP: &str = "↑/↓ move · enter select · esc skip · q quit";

const POINTER: &str = "❯";
const POINTER_SMALL: &str = "›";

/// One entry of an action menu, with the block of description lines the
/// reference only reveals under the selected entry.
pub struct Action {
    pub label: String,
    pub description: Vec<Line>,
}

impl Action {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: Vec::new(),
        }
    }

    pub fn described(label: impl Into<String>, description: Vec<Line>) -> Self {
        Self {
            label: label.into(),
            description,
        }
    }
}

/// A notice a menu shows under its actions: what became of the workflow the
/// GitHub Actions screens offered to write.
pub struct Notice {
    pub succeeded: bool,
    pub message: String,
}

// ------------------------------------------------------------------- input

/// What a key did to a menu.
pub enum MenuInput {
    Moved,
    Selected(usize),
    Back,
    Ignored,
}

/// Reads one key against a cursor over `count` entries.
///
/// `Selected` always names an entry that exists, and an empty menu selects
/// nothing at all, so no caller has to guard an index it was handed.
pub fn input(key: Key, selected: &mut usize, count: usize) -> MenuInput {
    match key {
        Key::Escape => MenuInput::Back,
        Key::ArrowUp | Key::Char('k') => {
            *selected = selected.saturating_sub(1);
            MenuInput::Moved
        }
        Key::ArrowDown | Key::Char('j') => {
            *selected = selected.saturating_add(1).min(count.saturating_sub(1));
            MenuInput::Moved
        }
        Key::Enter => match count.checked_sub(1) {
            Some(last) => MenuInput::Selected((*selected).min(last)),
            None => MenuInput::Ignored,
        },
        _ => MenuInput::Ignored,
    }
}

// ------------------------------------------------------------------ drawing

/// `width` is the hard bound on a row, not a suggestion: an action label is
/// written by this crate and can be longer than a narrow terminal, and a row
/// that wraps desynchronizes the cursor rewind for every frame after it.
pub struct Menu<'a> {
    pub title: Vec<Line>,
    pub actions: &'a [Action],
    pub selected: usize,
    pub notice: Option<&'a Notice>,
    pub hint: &'a str,
    pub width: usize,
}

pub fn menu(screen: Menu<'_>) -> Vec<Line> {
    let mut lines = screen.title;
    lines.extend(action_menu(screen.actions, screen.selected));
    if let Some(notice) = screen.notice {
        let style = Style::color(if notice.succeeded {
            Color::Green
        } else {
            Color::Yellow
        });
        lines.push(Line::text(&notice.message, style));
    }
    lines.extend(vec![Line::blank(); ACTION_MENU_MARGIN_ROWS]);
    lines.push(Line::text(screen.hint, Style::DIM));
    lines
        .into_iter()
        .map(|line| line.truncate_end(screen.width))
        .collect()
}

fn action_menu(actions: &[Action], selected: usize) -> Vec<Line> {
    let mut lines = vec![Line::blank(); ACTION_MENU_MARGIN_ROWS];
    for (index, action) in actions.iter().enumerate() {
        let is_selected = index == selected;
        let style = if is_selected {
            Style::color(Color::Cyan).bold()
        } else {
            Style::PLAIN
        };
        let pointer = if is_selected { POINTER } else { POINTER_SMALL };
        lines.push(Line::text(format!("{pointer} {}", action.label), style));
        if is_selected {
            lines.extend(action.description.iter().cloned());
        }
        if index + 1 < actions.len() {
            lines.extend(vec![Line::blank(); ACTION_MENU_ITEM_GAP_ROWS]);
        }
    }
    lines
}

// ------------------------------------------------------------------- titles

pub struct LandingNotices {
    pub issue_count: usize,
    pub incomplete: Option<String>,
}

/// The score block, then at most one line about the scan itself. An incomplete
/// scan says so and nothing else: a workspace whose checks did not all run has
/// not earned "no issues found".
pub fn landing_title(header: Vec<Line>, notices: &LandingNotices) -> Vec<Line> {
    let mut lines = header;
    let notice = notices.incomplete.as_ref().map_or_else(
        || {
            (notices.issue_count == 0).then(|| {
                Line::text("✔ No issues found. Nice work.", Style::color(Color::Green))
            })
        },
        |message| {
            Some(Line::text(
                format!("⚠ {message}"),
                Style::color(Color::Yellow),
            ))
        },
    );
    if let Some(notice) = notice {
        lines.push(Line::blank());
        lines.push(notice);
    }
    lines
}

pub fn handoff_title() -> Vec<Line> {
    vec![Line::text("Choose how to continue", Style::BOLD).indented(HORIZONTAL_PADDING_COLUMNS)]
}

/// Either GitHub Actions screen: its own heading, then the justification both
/// of them carry. The two used to be two functions identical but for the
/// heading, which the tool's own `duplicate_function_body` named.
fn ci_title(heading: Line, width: usize) -> Vec<Line> {
    let mut lines = vec![heading];
    lines.extend(ci_justification(width));
    lines
}

/// The recommendation screen, reached before the reader has chosen anything.
pub fn ci_recommendation_title(width: usize) -> Vec<Line> {
    ci_title(
        Line::text("Add Rust Doctor to GitHub Actions first", Style::BOLD)
            .indented(HORIZONTAL_PADDING_COLUMNS),
        width,
    )
}

/// The setup screen, which asks rather than recommends.
pub fn ci_setup_title(width: usize) -> Vec<Line> {
    ci_title(
        Line::blank()
            .with(Span::new("?", Style::color(Color::Cyan).bold()))
            .with(Span::new(
                " Add Rust Doctor to GitHub Actions?",
                Style::BOLD,
            )),
        width,
    )
}

pub fn ci_justification(width: usize) -> Vec<Line> {
    [
        Line::text(
            "Scan every pull request to prevent new Rust issues while you fix the backlog.",
            Style::DIM,
        ),
        Line::text(
            "Baseline scope on a pull request, so only what the change introduces is judged.",
            Style::DIM,
        ),
        Line::of(Span::linked(
            GITHUB_ACTIONS_SETUP_URL,
            Style::color(Color::Cyan),
            GITHUB_ACTIONS_SETUP_URL,
        )),
    ]
    .into_iter()
    .map(|line| {
        line.indented(HORIZONTAL_PADDING_COLUMNS)
            .truncate_end(width)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|line| line.render(false, false)).collect()
    }

    /// Every menu answers to the same keys, and none of them can be pushed past
    /// the entries it shows or before its first one.
    #[test]
    fn a_menu_cursor_never_leaves_the_entries_it_indexes() {
        let mut selected = 0usize;
        assert!(matches!(
            input(Key::ArrowUp, &mut selected, 3),
            MenuInput::Moved
        ));
        assert_eq!(selected, 0);
        for _ in 0..10 {
            input(Key::Char('j'), &mut selected, 3);
        }
        assert_eq!(selected, 2);
        assert!(matches!(
            input(Key::Enter, &mut selected, 3),
            MenuInput::Selected(2)
        ));
        // A cursor left over from a longer list still selects a real entry.
        let mut stale = 7usize;
        assert!(matches!(
            input(Key::Enter, &mut stale, 2),
            MenuInput::Selected(1)
        ));
        // An empty menu selects nothing rather than an index that is not there.
        let mut empty = 0usize;
        assert!(matches!(input(Key::Enter, &mut empty, 0), MenuInput::Ignored));
        assert!(matches!(
            input(Key::Char('x'), &mut empty, 0),
            MenuInput::Ignored
        ));
        assert!(matches!(input(Key::Escape, &mut empty, 0), MenuInput::Back));
    }

    #[test]
    fn the_menu_points_at_the_selection_and_gaps_every_entry() {
        let actions = [
            Action::new("Review 2 issues"),
            Action::described(
                "Add to GitHub Actions (Recommended)",
                vec![Line::text("why", Style::DIM)],
            ),
            Action::new("Hand off to an agent"),
        ];
        assert_eq!(
            visible(&menu(Menu {
                title: Vec::new(),
                actions: &actions,
                selected: 1,
                notice: None,
                hint: HINT_MENU,
                width: 80,
            })),
            [
                "",
                "› Review 2 issues",
                "",
                "❯ Add to GitHub Actions (Recommended)",
                "why",
                "",
                "› Hand off to an agent",
                "",
                HINT_MENU,
            ]
        );
    }

    /// A notice sits between the actions and the hint, so the reader sees what
    /// happened without leaving the screen that offered it.
    #[test]
    fn a_menu_notice_lands_under_the_actions_and_above_the_hint() {
        let actions = [Action::new("Yes, add the workflow")];
        let notice = Notice {
            succeeded: false,
            message: "Could not add the workflow: permission denied".to_owned(),
        };
        assert_eq!(
            visible(&menu(Menu {
                title: Vec::new(),
                actions: &actions,
                selected: 0,
                notice: Some(&notice),
                hint: HINT_MENU_BACK,
                width: 80,
            })),
            [
                "",
                "❯ Yes, add the workflow",
                "Could not add the workflow: permission denied",
                "",
                HINT_MENU_BACK,
            ]
        );
    }

    /// An action label is written by this crate and can outrun a narrow
    /// terminal, so the menu cuts it rather than letting the row wrap.
    #[test]
    fn a_label_longer_than_the_terminal_is_cut_rather_than_wrapped() {
        let actions = [Action::new("Add to GitHub Actions first (Recommended)")];
        for width in [12usize, 24, 39] {
            let rendered = menu(Menu {
                title: ci_recommendation_title(width),
                actions: &actions,
                selected: 0,
                notice: None,
                hint: HINT_MENU_SKIP,
                width,
            });
            for line in rendered {
                assert!(line.width() <= width, "a row overflowed {width} columns");
            }
        }
    }

    #[test]
    fn the_landing_reports_a_clean_workspace_before_its_menu() {
        let notices = LandingNotices {
            issue_count: 0,
            incomplete: None,
        };
        let rendered = visible(&menu(Menu {
            title: landing_title(Vec::new(), &notices),
            actions: &[],
            selected: 0,
            notice: None,
            hint: HINT_QUIT,
            width: 80,
        }));
        assert_eq!(
            rendered,
            ["", "✔ No issues found. Nice work.", "", "", HINT_QUIT]
        );
    }

    /// A scan that did not finish has not earned "no issues found", so it says
    /// what did not run instead.
    #[test]
    fn an_incomplete_scan_replaces_the_clean_report_rather_than_joining_it() {
        let notices = LandingNotices {
            issue_count: 0,
            incomplete: Some("structure checks did not complete.".to_owned()),
        };
        let rendered = visible(&landing_title(Vec::new(), &notices));
        assert_eq!(rendered, ["", "⚠ structure checks did not complete."]);
    }
}
