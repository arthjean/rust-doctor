use unicode_width::UnicodeWidthChar;

pub(crate) fn sanitize(content: &str) -> String {
    let characters: Vec<_> = content.chars().collect();
    let mut sanitized = String::with_capacity(content.len());
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if character == '\u{001b}' {
            index = skip_escape_sequence(&characters, index);
            continue;
        }
        if matches!(
            character,
            '\u{0090}' | '\u{0098}' | '\u{009b}' | '\u{009d}' | '\u{009e}' | '\u{009f}'
        ) {
            index = skip_c1_sequence(&characters, index);
            continue;
        }
        if !character.is_control() {
            sanitized.push(character);
        }
        index += 1;
    }
    sanitized
}

pub(crate) fn display_width(content: &str) -> usize {
    content.chars().fold(0usize, |width, character| {
        width.saturating_add(UnicodeWidthChar::width(character).unwrap_or(0))
    })
}

pub(crate) fn truncate(content: &str, width: usize) -> String {
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

pub(crate) fn wrap(content: &str, width: usize) -> Vec<String> {
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

pub(crate) fn skip_escape_sequence(characters: &[char], start: usize) -> usize {
    let Some(kind) = characters.get(start + 1).copied() else {
        return characters.len();
    };
    match kind {
        '[' => skip_control_sequence(characters, start + 2),
        ']' | 'P' | 'X' | '^' | '_' => skip_string_sequence(characters, start + 2),
        _ => (start + 2).min(characters.len()),
    }
}

pub(crate) fn skip_c1_sequence(characters: &[char], start: usize) -> usize {
    if characters[start] == '\u{009b}' {
        skip_control_sequence(characters, start + 1)
    } else {
        skip_string_sequence(characters, start + 1)
    }
}

fn skip_control_sequence(characters: &[char], mut index: usize) -> usize {
    while let Some(character) = characters.get(index) {
        index += 1;
        if ('@'..='~').contains(character) {
            break;
        }
    }
    index
}

fn skip_string_sequence(characters: &[char], mut index: usize) -> usize {
    while let Some(character) = characters.get(index) {
        if *character == '\u{0007}' {
            return index + 1;
        }
        if *character == '\u{001b}' && characters.get(index + 1) == Some(&'\\') {
            return index + 2;
        }
        index += 1;
    }
    index
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

    #[test]
    fn truncation_counts_wide_and_combining_characters() {
        assert_eq!(truncate("界界界", 5), "界界…");
        assert_eq!(display_width("e\u{301}"), 1);
    }
}
