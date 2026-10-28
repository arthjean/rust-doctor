//! The credential shapes the repository pass matches, and what keeps the
//! shapes that are not credentials out.
//!
//! A shape alone is not a leak: a PEM parser names its header as a constant,
//! AWS documents `AKIA...EXAMPLE`, and every tutorial writes
//! `postgres://user:pass@localhost`. Each match therefore has to carry the
//! part a real credential has, a key body under the header or a host a
//! deployment reaches, and is vetoed when the value, or the line just before
//! it, says it is an illustration.
//!
//! Masking reads the same shapes without the vetoes: a code frame that hides
//! a placeholder costs nothing, and one that prints a key costs everything.

/// One high-confidence credential shape: a fixed prefix, a minimum tail of
/// admissible characters, and a published name. Entropy alone never triggers,
/// which is the documented lesson of the tools that tried.
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

/// Words that mark a value as an illustration rather than a secret.
///
/// `bogus` came from the corpus: vibesql's token-precedence script passes
/// `sk-ant-oat01-deliberately-bogus` to prove the environment wins, and it held
/// the score four points lower as a hardcoded credential.
const PLACEHOLDER_WORDS: [&str; 6] = ["bogus", "dummy", "example", "fake", "placeholder", "sample"];

/// Bytes before a match that are read for a placeholder word: enough for
/// `let example_key = "` or `// sample:` and no more of the line.
const VETO_WINDOW: usize = 40;

/// The shortest base64 run that reads as a key body: a PEM body line is 64
/// characters, and 38 is the base64 of a 28-byte value, longer than any
/// identifier or word a parser keeps near its header.
const KEY_BODY_RUN: usize = 38;

/// Non-empty lines after a header read for a key body.
const KEY_BODY_LINES: usize = 3;

/// Schemes whose URL carries a password in its user information.
const DATABASE_SCHEMES: [&str; 6] = ["mongodb", "mongodb+srv", "mysql", "postgres", "postgresql", "redis"];

/// Passwords a template writes in place of one.
const PLACEHOLDER_PASSWORDS: [&str; 3] = ["changeme", "pass", "password"];

/// First credential of a line, its pattern name and the byte range of the
/// matched token, read with the lines that follow it. A prefix inside a longer
/// identifier is not a match, and a vetoed match gives way to the next one.
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

/// Every credential the rule would match on a line, as sorted, disjoint byte
/// ranges.
///
/// The rule reports the first match of a line, but a code frame prints the
/// whole line and the lines around it, so whatever renders source text masks
/// every match rather than the one a finding names, vetoed or not.
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

/// What the report keeps of a credential: its first four characters, which
/// name the kind of key as react-doctor keeps the type prefix visible, and
/// never more. A value too short to spare four characters loses all of them.
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

/// Whether any line of `text` holds a credential the masking would hide.
pub(crate) fn carries_credential(text: &str) -> bool {
    text.lines().any(|line| !credential_ranges(line).is_empty())
}

/// `text` with every credential of every line masked as a code frame masks
/// it, a private key body a header left open included: each line of it is
/// masked whole until the line holding its footer. Diagnostic text runs
/// through this once, at normalization, so every renderer reads it already
/// masked.
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

/// The byte ranges of one pattern on a line, left to right.
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

/// Is the match at `range` an illustration? Its own text or the bytes just
/// before it name a placeholder, or a `...` truncation follows it.
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

/// What opens a PEM block's last line.
const FOOTER: &str = "-----END ";

/// Whether a PEM private key header on `line` leaves its body open below it:
/// no footer follows the header on the line.
pub(crate) fn key_continues(line: &str) -> bool {
    private_key_block(line).is_some_and(|(_, end)| !line.get(end..).unwrap_or_default().contains(FOOTER))
}

/// Where the body of a key a line above left open ends on `line`: at the end
/// of the line until its footer, then before the footer when body text sits
/// in front of it on the footer's line, and at its start when nothing does.
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

/// What masking hides of a line holding a private key header: the header to
/// its footer when the line holds both, as a one-line PEM escaped with `\n`
/// does, the header to the end of the line when a body sits on the line, and
/// the header alone when nothing but escapes and punctuation follow it.
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
    // A real PEM label is uppercase words: `RSA`, `OPENSSH`, or nothing at
    // all. Anything else is prose about key blocks, not a key block, which is
    // what keeps this rule off documentation that spells the pattern out.
    line.get(label_start..suffix_start)?
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte == b' ')
        .then_some((start, suffix_start + suffix.len()))
}

/// Does a key body follow a PEM header: a base64 run of `KEY_BODY_RUN`
/// characters on the rest of the header's line or on one of the next
/// `KEY_BODY_LINES` non-empty lines, before the block ends? A header kept as
/// a constant by a parser has an `-----END` or code after it, never a body.
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

/// The part of a line before the block's `-----END` marker, and whether the
/// marker is on it.
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

/// The password of a database URL, and whether the URL reaches a deployment.
struct DatabasePassword {
    range: (usize, usize),
    /// Neither a placeholder password nor a local host: `localhost`, or a
    /// name without a dot, which is a container or a development machine.
    deployed: bool,
}

/// Every `scheme://user:password@host` of a line whose scheme is a database's.
fn database_passwords(line: &str) -> Vec<DatabasePassword> {
    let bytes = line.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = line.get(from..).and_then(|rest| rest.find("://")) {
        let separator = from + offset;
        from = separator + 3;
        // On bytes: a scheme is ASCII, so the byte after the last one outside
        // it is a char boundary even when a multibyte character precedes it,
        // as a typographic quote before `postgres://` does.
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

/// A password a template writes: a stock word, an environment variable, or a
/// slot a template engine or `format!` fills.
fn placeholder_password(password: &str) -> bool {
    let wrapped = |open: &str, close: &str| password.starts_with(open) && password.ends_with(close);
    PLACEHOLDER_PASSWORDS.contains(&password.to_ascii_lowercase().as_str())
        || password.starts_with('$')
        || wrapped("<", ">")
        || wrapped("{", "}")
}

#[cfg(test)]
mod tests;
