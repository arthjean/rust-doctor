use unicode_width::UnicodeWidthChar;

pub fn sanitize(content: &str) -> String {
    strip_escapes(content, false)
}

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

pub(crate) const fn opens_escape(character: char) -> bool {
    matches!(
        character,
        '\u{001b}' | '\u{0090}' | '\u{0098}' | '\u{009b}' | '\u{009d}' | '\u{009e}' | '\u{009f}'
    )
}

pub(crate) fn escape_end(characters: &[char], start: usize) -> usize {
    let after = start.saturating_add(1);
    match characters.get(start).copied() {
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
            Some(_) => after.saturating_add(1),
            None => after,
        },
        _ => after,
    }
}

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

pub fn display_width(content: &str) -> usize {
    content.chars().fold(0usize, |width, character| {
        width.saturating_add(UnicodeWidthChar::width(character).unwrap_or(0))
    })
}

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
            remaining = remaining.get(character_length..).unwrap_or_default();
            continue;
        }

        let split = last_break.unwrap_or(hard_split);
        let Some((line, rest)) = remaining.split_at_checked(split) else {
            lines.push(remaining.to_owned());
            break;
        };
        lines.push(line.trim_end().to_owned());
        remaining = rest.trim_start();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_and_truncation_return_on_every_line_of_the_non_ascii_table() {
        for line in crate::boundary_table::lines() {
            for width in 0..=4 {
                assert!(!wrap(&line, width).is_empty(), "{line:?} at {width}");
                assert!(display_width(&truncate(&line, width)) <= width.max(1), "{line:?} at {width}");
            }
        }
    }

    #[test]
    fn wrapping_uses_terminal_columns_and_removes_complete_escape_sequences() {
        let sanitized = sanitize("wide \u{001b}[31m界界\u{001b}[0m text");
        assert_eq!(sanitized, "wide 界界 text");
        let lines = wrap(&sanitized, 8);
        assert!(lines.iter().all(|line| display_width(line) <= 8));
        assert_eq!(lines, ["wide", "界界", "text"]);
    }

    #[test]
    fn one_grammar_covers_every_escape_form_and_only_the_newline_differs() {
        assert_eq!(sanitize("a\u{001b}(Bb"), "ab");
        assert_eq!(sanitize("a\u{001b}cb"), "ab");
        assert_eq!(sanitize("a\u{001b}[31\u{0018}b"), "ab");
        assert_eq!(sanitize("a\u{001b}]0;title\u{0007}b"), "ab");
        assert_eq!(sanitize("a\u{009b}31mb"), "ab");
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
