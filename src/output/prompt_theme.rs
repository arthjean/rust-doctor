//! Interactive prompt theme mirroring React Doctor's `prompts` rendering.
//!
//! React Doctor asks every post-scan question through the npm `prompts`
//! package. This theme reproduces that layout for `dialoguer`: a cyan `?`
//! prefix, a bold question, a gray `›` delimiter, the arrow-keys hint, then a
//! cyan `❯` pointer with an underlined active title followed by its inline gray
//! description.
//!
//! Items use the `"Title\n  Description"` convention already used by the prompts
//! in this crate; the description renders only on the active item, exactly like
//! `prompts` does.

use super::terminal::wrap_text;
use dialoguer::theme::Theme;
use owo_colors::{OwoColorize as _, Stream, Style};
use std::fmt;
use unicode_width::UnicodeWidthStr as _;

const SELECT_HINT: &str = "- Use arrow-keys. Return to submit.";
const MULTI_SELECT_HINT: &str = "- Space to select. Return to submit.";
/// `prompts` renders the pointer plus three spaces, so titles start at column 4.
const ITEM_INDENT: &str = "    ";
const FALLBACK_COLUMNS: usize = 80;

/// Prompt theme shared by every interactive question in the CLI.
pub struct PromptTheme;

impl Theme for PromptTheme {
    fn format_prompt(&self, f: &mut dyn fmt::Write, prompt: &str) -> fmt::Result {
        write_question(f, prompt, "")
    }

    fn format_select_prompt(&self, f: &mut dyn fmt::Write, prompt: &str) -> fmt::Result {
        write_question(f, prompt, SELECT_HINT)
    }

    fn format_select_prompt_item(
        &self,
        f: &mut dyn fmt::Write,
        text: &str,
        active: bool,
    ) -> fmt::Result {
        let (title, description) = split_item(text);
        if !active {
            return write!(f, "{ITEM_INDENT}{title}");
        }
        write!(
            f,
            "{}   {}",
            paint("❯", Style::new().cyan()),
            paint(title, Style::new().cyan().underline())
        )?;
        write_description(f, ITEM_INDENT.width() + title.width(), description)
    }

    fn format_select_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        sel: &str,
    ) -> fmt::Result {
        write_answer(f, prompt, split_item(sel).0)
    }

    fn format_multi_select_prompt(&self, f: &mut dyn fmt::Write, prompt: &str) -> fmt::Result {
        write_question(f, prompt, MULTI_SELECT_HINT)
    }

    fn format_multi_select_prompt_item(
        &self,
        f: &mut dyn fmt::Write,
        text: &str,
        checked: bool,
        active: bool,
    ) -> fmt::Result {
        let (title, description) = split_item(text);
        let radio = if checked {
            paint("◉", Style::new().green())
        } else {
            "◯".to_string()
        };
        let pointer = if active {
            paint("❯", Style::new().cyan())
        } else {
            " ".to_string()
        };
        if !active {
            return write!(f, "{radio} {pointer} {title}");
        }
        write!(
            f,
            "{radio} {pointer} {}",
            paint(title, Style::new().cyan().underline())
        )?;
        write_description(f, ITEM_INDENT.width() + title.width(), description)
    }

    fn format_multi_select_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selections: &[&str],
    ) -> fmt::Result {
        let titles: Vec<_> = selections
            .iter()
            .map(|selection| split_item(selection).0)
            .collect();
        write_answer(f, prompt, &titles.join(", "))
    }

    fn format_confirm_prompt(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        default: Option<bool>,
    ) -> fmt::Result {
        let options = match default {
            Some(true) => "(Y/n)",
            Some(false) => "(y/N)",
            None => "(y/n)",
        };
        write_question(f, prompt, options)
    }

    fn format_confirm_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selection: Option<bool>,
    ) -> fmt::Result {
        let answer = match selection {
            Some(true) => "yes",
            Some(false) => "no",
            None => "",
        };
        write_answer(f, prompt, answer)
    }
}

/// Split an item into its title and its optional indented description.
fn split_item(text: &str) -> (&str, &str) {
    text.split_once('\n')
        .map_or((text, ""), |(title, description)| {
            (title, description.trim())
        })
}

/// Render the pending question line, then any extra context lines below it.
fn write_question(f: &mut dyn fmt::Write, prompt: &str, trailing: &str) -> fmt::Result {
    let (headline, details) = prompt.split_once('\n').unwrap_or((prompt, ""));
    write!(
        f,
        "{} {} {}",
        paint("?", Style::new().cyan()),
        paint(headline, Style::new().bold()),
        paint("›", gray())
    )?;
    if !trailing.is_empty() {
        write!(f, " {}", paint(trailing, gray()))?;
    }
    for line in details.lines() {
        write!(f, "\n{}", paint(line, gray()))?;
    }
    Ok(())
}

/// Render the answered question line once the prompt is submitted.
fn write_answer(f: &mut dyn fmt::Write, prompt: &str, answer: &str) -> fmt::Result {
    let headline = prompt.split('\n').next().unwrap_or(prompt);
    write!(
        f,
        "{} {} {}",
        paint("✔", Style::new().green()),
        paint(headline, Style::new().bold()),
        paint("›", gray())
    )?;
    if answer.is_empty() {
        return Ok(());
    }
    write!(f, " {answer}")
}

/// Append the active item's description inline, or below it when it would wrap.
fn write_description(f: &mut dyn fmt::Write, used: usize, description: &str) -> fmt::Result {
    if description.is_empty() {
        return Ok(());
    }
    let columns = crate::run::stdout_columns().unwrap_or(FALLBACK_COLUMNS);
    let inline = format!(" - {description}");
    if !description.contains('\n') && used + inline.width() < columns {
        return write!(f, "{}", paint(&inline, gray()));
    }
    let width = columns.saturating_sub(ITEM_INDENT.width()).max(1);
    for line in wrap_text(description, width) {
        write!(f, "\n{}", paint(&format!("{ITEM_INDENT}{line}"), gray()))?;
    }
    Ok(())
}

/// `prompts` paints hints, delimiters, and descriptions with ANSI bright black.
const fn gray() -> Style {
    Style::new().bright_black()
}

fn paint(text: &str, style: Style) -> String {
    format!(
        "{}",
        text.if_supports_color(Stream::Stdout, |value| value.style(style))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialoguer::console::strip_ansi_codes;

    fn rendered(render: impl FnOnce(&mut String) -> fmt::Result) -> String {
        let mut buffer = String::new();
        render(&mut buffer).expect("writing to a String never fails");
        strip_ansi_codes(&buffer).into_owned()
    }

    #[test]
    fn select_question_matches_the_react_doctor_layout() {
        let line = rendered(|buffer| {
            PromptTheme.format_select_prompt(buffer, "What would you like to do next?")
        });
        assert_eq!(
            line,
            "? What would you like to do next? › - Use arrow-keys. Return to submit."
        );
    }

    #[test]
    fn active_item_carries_the_pointer_and_its_inline_description() {
        let item = "Claude Code\n  Open claude here with the top issues as a prompt";
        let active = rendered(|buffer| PromptTheme.format_select_prompt_item(buffer, item, true));
        let inactive =
            rendered(|buffer| PromptTheme.format_select_prompt_item(buffer, item, false));
        assert_eq!(
            active,
            "❯   Claude Code - Open claude here with the top issues as a prompt"
        );
        assert_eq!(inactive, "    Claude Code");
        // Both rows put the title at the same column: the pointer replaces one
        // of the four leading spaces rather than shifting the title.
        assert_eq!(title_column(&active), title_column(&inactive));
    }

    fn title_column(line: &str) -> usize {
        line.find("Claude Code")
            .map_or(0, |offset| line[..offset].width())
    }

    #[test]
    fn answered_question_keeps_only_the_selected_title() {
        let line = rendered(|buffer| {
            PromptTheme.format_select_prompt_selection(
                buffer,
                "What would you like to do next?",
                "Claude Code\n  Open claude here with the top issues as a prompt",
            )
        });
        assert_eq!(line, "✔ What would you like to do next? › Claude Code");
    }

    #[test]
    fn multi_select_items_carry_the_radio_then_the_pointer() {
        let checked = rendered(|buffer| {
            PromptTheme.format_multi_select_prompt_item(buffer, "alpha\n  crates/alpha", true, true)
        });
        let unchecked = rendered(|buffer| {
            PromptTheme.format_multi_select_prompt_item(buffer, "beta\n  crates/beta", false, false)
        });
        assert_eq!(checked, "◉ ❯ alpha - crates/alpha");
        assert_eq!(unchecked, "◯   beta");
    }

    #[test]
    fn confirm_prompt_shows_the_default_answer_in_parentheses() {
        let pending = rendered(|buffer| {
            PromptTheme.format_confirm_prompt(buffer, "Install the skill?", Some(true))
        });
        let answered = rendered(|buffer| {
            PromptTheme.format_confirm_prompt_selection(buffer, "Install the skill?", Some(false))
        });
        assert_eq!(pending, "? Install the skill? › (Y/n)");
        assert_eq!(answered, "✔ Install the skill? › no");
    }

    #[test]
    fn multi_line_prompts_keep_their_context_under_the_question() {
        let line = rendered(|buffer| {
            PromptTheme.format_select_prompt(
                buffer,
                "Add Rust Doctor to GitHub Actions?\n  Scan every pull request.",
            )
        });
        assert_eq!(
            line,
            "? Add Rust Doctor to GitHub Actions? › - Use arrow-keys. Return to submit.\n  Scan every pull request."
        );
    }
}
