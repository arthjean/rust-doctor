//! Terminal text measured and made safe, for every renderer this crate feeds.
//!
//! Both reports draw text a scanned workspace produced: a Clippy message, a
//! path, a rule help. [`sanitize`] and [`sanitize_multiline`] are the one gate
//! it goes through, so nothing it contains can move the cursor or repaint the
//! screen, and [`display_width`] is the one ruler the layouts measure with.
//! They are public because the interactive report lives in the binary crate
//! and must share them: a second sanitizer is a second set of escape sequences
//! to get wrong.
//!
//! There were two. This module scanned a `Vec<char>` by index and advanced two
//! characters past any `ESC` it did not recognize, so `ESC ( B` left a bare `B`
//! in a frame and no sequence was ever cancelled by `CAN` or `SUB`; `report.rs`
//! carried a second, complete grammar for the text it publishes. The complete
//! one is below, and what separated the two callers was never the grammar but
//! whether a newline survives it.


use unicode_width::UnicodeWidthChar;

/// Drops every control character, and every escape sequence whole, including
/// the C1 forms and the string sequences that end on `BEL` or `ESC \`.
///
/// This is what a terminal frame draws through: a frame is one row, and a
/// newline in it moves the cursor away from where the next rewind expects it.
pub fn sanitize(content: &str) -> String {
    strip_escapes(content, false)
}

/// The same grammar, keeping the newline and the tab.
///
/// This is what the JSON report publishes through: a Clippy message arrives as
/// Cargo wrote it, several lines with an indented span under them, and dropping
/// its newlines would run the whole diagnostic together on one line.
pub fn sanitize_multiline(content: &str) -> String {
    strip_escapes(content, true)
}

fn strip_escapes(content: &str, keep_whitespace: bool) -> String {
    let characters: Vec<char> = content.chars().collect();
    let mut output = String::with_capacity(content.len());
    let mut index = 0;
    while let Some(character) = characters.get(index).copied() {
        if opens_escape(character) {
            index = escape_end(&characters, index);
            continue;
        }
        index = index.saturating_add(1);
        if character.is_control() {
            if keep_whitespace && matches!(character, '\n' | '\t') {
                output.push(character);
            }
            continue;
        }
        output.push(character);
    }
    output
}

/// Whether a character opens an escape sequence: `ESC`, or one of the C1
/// controls that stand for a two-character `ESC` form.
pub(crate) const fn opens_escape(character: char) -> bool {
    matches!(
        character,
        '\u{001b}' | '\u{0090}' | '\u{0098}' | '\u{009b}' | '\u{009d}' | '\u{009e}' | '\u{009f}'
    )
}

/// The index just past the escape sequence that opens at `start`.
///
/// This is the crate's one escape grammar. The code frame needs the index
/// because it maps every source character to the column it renders at, and the
/// two sanitizers need the same answer as a string: expressing the grammar
/// twice is how a frame and a published message came to disagree on where a
/// sequence ends.
pub(crate) fn escape_end(characters: &[char], start: usize) -> usize {
    let after = start.saturating_add(1);
    match characters.get(start).copied() {
        // The C1 forms stand for `ESC` plus their introducer, already read.
        Some('\u{009b}') => control_sequence_end(characters, after),
        Some('\u{009d}') => string_sequence_end(characters, after, true),
        Some('\u{0090}' | '\u{0098}' | '\u{009e}' | '\u{009f}') => {
            string_sequence_end(characters, after, false)
        }
        Some('\u{001b}') => match characters.get(after).copied() {
            Some('[') => control_sequence_end(characters, after.saturating_add(1)),
            Some(']') => string_sequence_end(characters, after.saturating_add(1), true),
            Some('P' | 'X' | '^' | '_') => {
                string_sequence_end(characters, after.saturating_add(1), false)
            }
            // A sequence carrying intermediate bytes: `ESC ( B` designates a
            // character set, and its final byte closes it.
            Some('\u{20}'..='\u{2f}') => {
                let mut index = after.saturating_add(1);
                while matches!(characters.get(index), Some('\u{20}'..='\u{2f}')) {
                    index = index.saturating_add(1);
                }
                if matches!(characters.get(index), Some('\u{30}'..='\u{7e}')) {
                    index = index.saturating_add(1);
                }
                index
            }
            // Every other form is two characters whole: `ESC c`, `ESC 7`.
            Some(_) => after.saturating_add(1),
            None => after,
        },
        _ => after,
    }
}

/// A control sequence runs to its final byte, and `CAN` or `SUB` cancels it.
fn control_sequence_end(characters: &[char], mut index: usize) -> usize {
    while let Some(character) = characters.get(index).copied() {
        index = index.saturating_add(1);
        if matches!(character, '\u{0018}' | '\u{001a}')
            || ('\u{40}'..='\u{7e}').contains(&character)
        {
            break;
        }
    }
    index
}

/// A string sequence runs to `ST`, to `BEL` where that terminates it, or to a
/// cancel.
fn string_sequence_end(characters: &[char], mut index: usize, bell_terminates: bool) -> usize {
    while let Some(character) = characters.get(index).copied() {
        if matches!(character, '\u{0018}' | '\u{001a}')
            || character == '\u{009c}'
            || (bell_terminates && character == '\u{0007}')
        {
            return index.saturating_add(1);
        }
        if character == '\u{001b}' && characters.get(index.saturating_add(1)) == Some(&'\\') {
            return index.saturating_add(2);
        }
        index = index.saturating_add(1);
    }
    index
}

/// Width of already-sanitized text in terminal columns, counting a wide
/// character as two and a combining one as none.
pub fn display_width(content: &str) -> usize {
    content.chars().fold(0usize, |width, character| {
        width.saturating_add(UnicodeWidthChar::width(character).unwrap_or(0))
    })
}

/// Cuts on a column boundary and marks the cut with an ellipsis.
pub fn truncate(content: &str, width: usize) -> String {
    if display_width(content) <= width {
        return content.to_owned();
    }
    if width == 0 {
        return String::new();
    }

    let mut truncated = String::new();
    let budget = width.saturating_sub(1);
    let mut used = 0usize;
    for character in content.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used.saturating_add(character_width) > budget {
            break;
        }
        truncated.push(character);
        used = used.saturating_add(character_width);
    }
    truncated.push('…');
    truncated
}

/// Word-wraps into rows no wider than `width` columns, breaking a word that
/// does not fit on its own.
pub fn wrap(content: &str, width: usize) -> Vec<String> {
    if content.is_empty() || width == 0 {
        return vec![String::new()];
    }

    let mut remaining = content;
    let mut lines = Vec::new();
    while !remaining.is_empty() {
        let mut used = 0usize;
        let mut last_break = None;
        let mut overflow = None;
        for (index, character) in remaining.char_indices() {
            let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
            if used.saturating_add(character_width) > width {
                overflow = Some(index);
                break;
            }
            used = used.saturating_add(character_width);
            if index > 0 && character.is_whitespace() {
                last_break = Some(index);
            }
        }

        let Some(hard_split) = overflow else {
            lines.push(remaining.to_owned());
            break;
        };
        if hard_split == 0 {
            let character_length = remaining.chars().next().map_or(0, char::len_utf8);
            lines.push("…".to_owned());
            remaining = &remaining[character_length..];
            continue;
        }

        let split = last_break.unwrap_or(hard_split);
        lines.push(remaining[..split].trim_end().to_owned());
        remaining = remaining[split..].trim_start();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_uses_terminal_columns_and_removes_complete_escape_sequences() {
        let sanitized = sanitize("wide \u{001b}[31m界界\u{001b}[0m text");
        assert_eq!(sanitized, "wide 界界 text");
        let lines = wrap(&sanitized, 8);
        assert!(lines.iter().all(|line| display_width(line) <= 8));
        assert_eq!(lines, ["wide", "界界", "text"]);
    }

    /// The grammar is complete in both directions, and the only thing that
    /// separates the two entry points is the newline.
    #[test]
    fn one_grammar_covers_every_escape_form_and_only_the_newline_differs() {
        // A designating sequence with an intermediate byte, whole.
        assert_eq!(sanitize("a\u{001b}(Bb"), "ab");
        // A two-character form, whole.
        assert_eq!(sanitize("a\u{001b}cb"), "ab");
        // `CAN` cancels a control sequence in progress.
        assert_eq!(sanitize("a\u{001b}[31\u{0018}b"), "ab");
        // An OSC ending on `BEL`, and a C1 CSI.
        assert_eq!(sanitize("a\u{001b}]0;title\u{0007}b"), "ab");
        assert_eq!(sanitize("a\u{009b}31mb"), "ab");
        // The one difference between the two readers.
        assert_eq!(sanitize("one\ntwo\tthree"), "onetwothree");
        assert_eq!(sanitize_multiline("one\ntwo\tthree"), "one\ntwo\tthree");
        assert_eq!(sanitize_multiline("a\u{001b}[31mb\u{0000}c"), "abc");
    }

    #[test]
    fn truncation_counts_wide_and_combining_characters() {
        assert_eq!(truncate("界界界", 5), "界界…");
        assert_eq!(display_width("e\u{301}"), 1);
    }
}
