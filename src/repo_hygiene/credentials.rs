struct CredentialPattern {
    name: &'static str,
    prefix: &'static str,
    minimum_tail: usize,
    tail: fn(u8) -> bool,
}

const CREDENTIAL_PATTERNS: [CredentialPattern; 5] = [
    CredentialPattern {
        name: "AWS access key id",
        prefix: "AKIA",
        minimum_tail: 16,
        tail: |byte| byte.is_ascii_uppercase() || byte.is_ascii_digit(),
    },
    CredentialPattern {
        name: "GitHub personal access token",
        prefix: "ghp_",
        minimum_tail: 36,
        tail: |byte| byte.is_ascii_alphanumeric(),
    },
    CredentialPattern {
        name: "GitHub fine-grained token",
        prefix: "github_pat_",
        minimum_tail: 22,
        tail: |byte| byte.is_ascii_alphanumeric() || byte == b'_',
    },
    CredentialPattern {
        name: "secret API key",
        prefix: "sk-",
        minimum_tail: 20,
        tail: |byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'),
    },
    CredentialPattern {
        name: "Slack bot token",
        prefix: "xoxb-",
        minimum_tail: 10,
        tail: |byte| byte.is_ascii_alphanumeric() || byte == b'-',
    },
];

const PLACEHOLDER_WORDS: [&str; 6] = ["bogus", "dummy", "example", "fake", "placeholder", "sample"];

const VETO_WINDOW: usize = 40;

const KEY_BODY_RUN: usize = 38;

const KEY_BODY_LINES: usize = 3;

const DATABASE_SCHEMES: [&str; 6] = ["mongodb", "mongodb+srv", "mysql", "postgres", "postgresql", "redis"];

const PLACEHOLDER_PASSWORDS: [&str; 3] = ["changeme", "pass", "password"];

pub(super) fn first_credential(line: &str, following: &[&str]) -> Option<(&'static str, (usize, usize))> {
    if let Some(range) = private_key_block(line)
        && !vetoed(line, range)
        && key_body_follows(line.get(range.1..).unwrap_or_default(), following)
    {
        return Some(("private key block", range));
    }
    CREDENTIAL_PATTERNS
        .iter()
        .find_map(|pattern| {
            pattern_ranges(line, pattern)
                .find(|range| !vetoed(line, *range))
                .map(|range| (pattern.name, range))
        })
        .or_else(|| {
            database_passwords(line)
                .into_iter()
                .find(|password| password.deployed && !vetoed(line, password.range))
                .map(|password| ("database URL password", password.range))
        })
}

pub(crate) fn credential_ranges(line: &str) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = private_key_range(line)
        .into_iter()
        .chain(
            CREDENTIAL_PATTERNS
                .iter()
                .flat_map(|pattern| pattern_ranges(line, pattern)),
        )
        .chain(database_passwords(line).into_iter().map(|password| password.range))
        .collect();
    ranges.sort_unstable();
    let mut disjoint: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match disjoint.last_mut() {
            Some(last) if start < last.1 => last.1 = last.1.max(end),
            _ => disjoint.push((start, end)),
        }
    }
    disjoint
}

pub(crate) const REDACTED: &str = "<redacted>";
const REDACTED_PREFIX_CHARS: usize = 4;
pub(crate) const REDACTED_MIN_CHARS: usize = 8;

pub(crate) fn masked(value: &str) -> String {
    let kept: String = if value.chars().count() >= REDACTED_MIN_CHARS {
        value.chars().take(REDACTED_PREFIX_CHARS).collect()
    } else {
        String::new()
    };
    kept + REDACTED
}

pub(crate) fn carries_credential(text: &str) -> bool {
    text.lines().any(|line| !credential_ranges(line).is_empty())
}

pub(crate) fn mask_credentials(text: &str) -> String {
    if !carries_credential(text) {
        return text.to_owned();
    }
    let mut masked_text = String::with_capacity(text.len());
    let mut key_open = false;
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            masked_text.push('\n');
        }
        let mut line = line;
        if key_open {
            let body_end = open_body_end(line);
            masked_text.push_str(if body_end > 0 || line.is_empty() { REDACTED } else { "" });
            if body_end == line.len() {
                continue;
            }
            line = line.get(body_end..).unwrap_or_default();
        }
        key_open = key_continues(line);
        let mut copied = 0;
        for (start, end) in credential_ranges(line) {
            let (Some(before), Some(value)) = (line.get(copied..start), line.get(start..end))
            else {
                continue;
            };
            masked_text.push_str(before);
            masked_text.push_str(&masked(value));
            copied = end;
        }
        masked_text.push_str(line.get(copied..).unwrap_or_default());
    }
    masked_text
}

fn pattern_ranges<'a>(
    line: &'a str,
    pattern: &'a CredentialPattern,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    let bytes = line.as_bytes();
    let mut from = 0;
    std::iter::from_fn(move || {
        while let Some(offset) = line.get(from..)?.find(pattern.prefix) {
            let start = from + offset;
            let boundary = start == 0
                || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
            let tail_start = start + pattern.prefix.len();
            let tail_length = bytes[tail_start..]
                .iter()
                .take_while(|byte| (pattern.tail)(**byte))
                .count();
            from = tail_start;
            if boundary && tail_length >= pattern.minimum_tail {
                from = tail_start + tail_length;
                return Some((start, from));
            }
        }
        None
    })
}

fn vetoed(line: &str, (start, end): (usize, usize)) -> bool {
    let mut window = start.saturating_sub(VETO_WINDOW);
    while !line.is_char_boundary(window) {
        window -= 1;
    }
    let read = line.get(window..end).unwrap_or_default().to_ascii_lowercase();
    let after = line.get(end..).unwrap_or_default();
    PLACEHOLDER_WORDS.iter().any(|word| read.contains(word))
        || after.starts_with("...")
        || after.starts_with('\u{2026}')
}

const FOOTER: &str = "-----END ";

pub(crate) fn key_continues(line: &str) -> bool {
    private_key_block(line).is_some_and(|(_, end)| !line.get(end..).unwrap_or_default().contains(FOOTER))
}

pub(crate) fn open_body_end(line: &str) -> usize {
    match line.find(FOOTER) {
        None => line.len(),
        Some(footer)
            if line
                .get(..footer)
                .is_some_and(|body| body.contains(|character: char| character.is_ascii_alphanumeric())) =>
        {
            footer
        }
        Some(_) => 0,
    }
}

fn private_key_range(line: &str) -> Option<(usize, usize)> {
    let (start, header_end) = private_key_block(line)?;
    let rest = line.get(header_end..).unwrap_or_default();
    let extent = match rest.find(FOOTER) {
        Some(footer) => {
            let label = footer + FOOTER.len();
            rest.get(label..)
                .and_then(|label_text| label_text.find("-----"))
                .map_or(rest.len(), |close| label + close + "-----".len())
        }
        None if rest
            .replace("\\n", "")
            .replace("\\r", "")
            .contains(|character: char| character.is_ascii_alphanumeric()) =>
        {
            rest.len()
        }
        None => 0,
    };
    Some((start, header_end + extent))
}

fn private_key_block(line: &str) -> Option<(usize, usize)> {
    let start = line.find("-----BEGIN ")?;
    let label_start = start + "-----BEGIN ".len();
    let suffix = "PRIVATE KEY-----";
    let suffix_start = line.get(label_start..)?.find(suffix)? + label_start;
    line.get(label_start..suffix_start)?
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte == b' ')
        .then_some((start, suffix_start + suffix.len()))
}

fn key_body_follows(rest: &str, following: &[&str]) -> bool {
    let (rest, ended) = body_text(rest);
    if carries_body(rest) {
        return true;
    }
    if ended {
        return false;
    }
    for line in following.iter().filter(|line| !line.trim().is_empty()).take(KEY_BODY_LINES) {
        let (text, ends) = body_text(line);
        if carries_body(text) {
            return true;
        }
        if ends {
            return false;
        }
    }
    false
}

fn body_text(line: &str) -> (&str, bool) {
    match line.find(FOOTER) {
        Some(end) => (line.get(..end).unwrap_or_default(), true),
        None => (line, false),
    }
}

fn carries_body(text: &str) -> bool {
    text.split(|character: char| !(character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=')))
        .any(|run| run.len() >= KEY_BODY_RUN)
}

struct DatabasePassword {
    range: (usize, usize),
    deployed: bool,
}

fn database_passwords(line: &str) -> Vec<DatabasePassword> {
    let bytes = line.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = line.get(from..).and_then(|rest| rest.find("://")) {
        let separator = from + offset;
        from = separator + 3;
        let scheme_start = bytes
            .get(..separator)
            .and_then(|before| {
                before
                    .iter()
                    .rposition(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'.' | b'-')))
            })
            .map_or(0, |index| index + 1);
        let scheme = line.get(scheme_start..separator).unwrap_or_default().to_ascii_lowercase();
        if !DATABASE_SCHEMES.contains(&scheme.as_str())
            || (scheme_start > 0 && bytes.get(scheme_start - 1) == Some(&b'_'))
        {
            continue;
        }
        let rest = line.get(from..).unwrap_or_default();
        let authority_end = rest
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '/' | '?' | '#' | '"' | '\'' | '`')
            })
            .unwrap_or(rest.len());
        let authority = rest.get(..authority_end).unwrap_or_default();
        let Some(at) = authority.rfind('@') else {
            continue;
        };
        let Some(colon) = authority.get(..at).and_then(|user_info| user_info.find(':')) else {
            continue;
        };
        let password = authority.get(colon + 1..at).unwrap_or_default();
        if password.is_empty() {
            continue;
        }
        let host = authority.get(at + 1..).unwrap_or_default().split(':').next().unwrap_or_default();
        let local = host.eq_ignore_ascii_case("localhost") || !host.contains('.');
        found.push(DatabasePassword {
            range: (from + colon + 1, from + at),
            deployed: !local && !placeholder_password(password),
        });
    }
    found
}

fn placeholder_password(password: &str) -> bool {
    let wrapped = |open: &str, close: &str| password.starts_with(open) && password.ends_with(close);
    PLACEHOLDER_PASSWORDS.contains(&password.to_ascii_lowercase().as_str())
        || password.starts_with('$')
        || wrapped("<", ">")
        || wrapped("{", "}")
}

#[cfg(test)]
mod tests;
