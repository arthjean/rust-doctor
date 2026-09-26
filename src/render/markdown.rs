//! A saved `--json` report rendered as Markdown, for a CI job summary or a pull
//! request comment.
//!
//! It reads the report as it was published rather than as this crate models
//! it: the file is whatever a CI step wrote, possibly by another run on another
//! machine, so the schema version is checked first and every field after that
//! is read defensively. Every piece of text that came from a diagnostic is
//! escaped, because a finding's message is text a scanned repository chose and
//! a comment is rendered for people who never asked to read its markup.

use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde_json::Value;

use crate::SCHEMA_VERSION;
use crate::presentation::rule_url;

/// The largest report file `report markdown` reads.
pub const REPORT_BYTES_LIMIT: u64 = 64 * 1024 * 1024;

/// How many findings the table lists when the caller names no limit.
pub const DEFAULT_FINDING_LIMIT: usize = 20;

/// The longest message a table cell carries, in characters. A comment has a
/// size limit of its own, and one finding should not spend it.
const MESSAGE_CHARACTERS: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkdownError {
    Unreadable(io::ErrorKind),
    TooLarge,
    InvalidJson { line: usize, column: usize },
    SchemaMismatch { found: Option<u64> },
    NotAReport,
}

impl fmt::Display for MarkdownError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(kind) => write!(formatter, "the report could not be read ({kind})"),
            Self::TooLarge => write!(
                formatter,
                "the report is larger than {} MiB",
                REPORT_BYTES_LIMIT / 1024 / 1024
            ),
            Self::InvalidJson { line, column } => write!(
                formatter,
                "the report is not valid JSON (line {line}, column {column})"
            ),
            Self::SchemaMismatch { found: Some(found) } => write!(
                formatter,
                "the report has schema_version {found}, and this binary reads {SCHEMA_VERSION}: render it with the rust-doctor that wrote it"
            ),
            Self::SchemaMismatch { found: None } => write!(
                formatter,
                "the report carries no schema_version, and this binary reads {SCHEMA_VERSION}"
            ),
            Self::NotAReport => write!(formatter, "the file is not a rust-doctor report"),
        }
    }
}

impl std::error::Error for MarkdownError {}

/// Reads the report at `path`, and nothing else: no scan, no process.
pub fn read_saved_report(path: &Path) -> Result<Value, MarkdownError> {
    let unreadable = |error: io::Error| MarkdownError::Unreadable(error.kind());
    let file = File::open(path).map_err(unreadable)?;
    if file.metadata().map_err(unreadable)?.len() > REPORT_BYTES_LIMIT {
        return Err(MarkdownError::TooLarge);
    }
    // The length is read again through the bound: a file that grows after the
    // check, or a pipe that has no length at all, is held to the same limit.
    let mut bytes = Vec::new();
    file.take(REPORT_BYTES_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    if bytes.len() as u64 > REPORT_BYTES_LIMIT {
        return Err(MarkdownError::TooLarge);
    }
    serde_json::from_slice(&bytes).map_err(|error| MarkdownError::InvalidJson {
        line: error.line(),
        column: error.column(),
    })
}

/// The report as Markdown: score, label and authority with its reasons, the
/// baseline counts when the scan compared against one, and a table of up to
/// `limit` findings, the introduced ones in baseline scope.
pub fn render_markdown(report: &Value, limit: usize) -> Result<String, MarkdownError> {
    let found = report.get("schema_version").and_then(Value::as_u64);
    if found != Some(u64::from(SCHEMA_VERSION)) {
        return Err(MarkdownError::SchemaMismatch { found });
    }
    let score = report
        .pointer("/audit/score")
        .filter(|score| score.is_object())
        .ok_or(MarkdownError::NotAReport)?;
    let diagnostics = report
        .get("diagnostics")
        .and_then(Value::as_array)
        .ok_or(MarkdownError::NotAReport)?;

    let mut out = String::from("## rust-doctor\n\n");
    let _ = write!(out, "**Score: {}**", score_text(score));
    if score.get("authoritative").and_then(Value::as_bool) == Some(true) {
        out.push_str(", authoritative.\n");
    } else {
        let reasons: Vec<String> = score
            .get("reasons")
            .and_then(Value::as_array)
            .map(|reasons| reasons.iter().map(|reason| escape(&text(reason))).collect())
            .unwrap_or_default();
        if reasons.is_empty() {
            out.push_str(", not authoritative.\n");
        } else {
            let _ = writeln!(out, ", not authoritative: {}.", reasons.join(", "));
        }
    }

    let baseline = report.pointer("/scope/mode").and_then(Value::as_str) == Some("baseline");
    let delta = report.get("delta").filter(|delta| delta.is_object());
    let findings: Vec<&Value> = match (baseline, delta) {
        (true, Some(delta)) => {
            let count = |field: &str| {
                delta
                    .pointer(&format!("/summary/{field}"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            };
            let _ = writeln!(
                out,
                "\n**Against the base:** {} introduced, {} fixed.",
                count("introduced"),
                count("fixed")
            );
            let introduced: BTreeSet<&str> = delta
                .get("introduced")
                .and_then(Value::as_array)
                .map(|ids| ids.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| introduced.contains(id))
                })
                .collect()
        }
        (true, None) => {
            out.push_str("\nThe comparison against the base was not computed.\n");
            Vec::new()
        }
        (false, _) => diagnostics.iter().collect(),
    };
    render_table(&mut out, findings, limit, baseline);
    Ok(out)
}

fn score_text(score: &Value) -> String {
    let value = score
        .get("value")
        .and_then(Value::as_u64)
        .map_or_else(|| "none".to_owned(), |value| format!("{value}/100"));
    match score.get("label").and_then(Value::as_str) {
        Some(label) => format!("{value} ({})", escape(label)),
        None => value,
    }
}

fn render_table(out: &mut String, mut findings: Vec<&Value>, limit: usize, introduced: bool) {
    let noun = if introduced { "introduced finding" } else { "finding" };
    let total = findings.len();
    if total == 0 {
        let _ = writeln!(out, "\nNo {noun}.");
        return;
    }
    sort_findings(&mut findings);
    let shown = total.min(limit);
    if shown > 0 {
        out.push_str("\n| Rule | Location | Severity | Message |\n|---|---|---|---|\n");
        for finding in findings.iter().take(shown) {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} |",
                rule_cell(finding),
                location(finding),
                escape(&field(finding, "severity")),
                escape(&bounded(&field(finding, "message")))
            );
        }
    }
    let plural = if total == 1 { "" } else { "s" };
    if shown < total {
        let _ = writeln!(out, "\nShowing {shown} of {total} {noun}s.");
    } else {
        let _ = writeln!(out, "\n{total} {noun}{plural}.");
    }
}

/// Errors first, then by place, so the same report always lists the same rows.
/// Each key is read once per finding rather than once per comparison.
fn sort_findings(findings: &mut Vec<&Value>) {
    let mut keyed: Vec<_> = findings
        .iter()
        .map(|finding| {
            let rank = match finding.get("severity").and_then(Value::as_str) {
                Some("error") => 0,
                Some("warning") => 1,
                Some("info") => 2,
                _ => 3,
            };
            let key = (
                rank,
                borrowed(finding, "path"),
                finding.pointer("/span/line_start").and_then(Value::as_u64),
                borrowed(finding, "code"),
                borrowed(finding, "message"),
                borrowed(finding, "id"),
                borrowed(finding, "source"),
            );
            (key, *finding)
        })
        .collect();
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    *findings = keyed.into_iter().map(|(_, finding)| finding).collect();
}

/// A string field without copying it, the empty string when it is anything
/// else: the order runs n log n times over a report of any size.
fn borrowed<'a>(finding: &'a Value, name: &str) -> &'a str {
    finding.get(name).and_then(Value::as_str).unwrap_or("")
}

fn rule_cell(finding: &Value) -> String {
    match finding.get("code").and_then(Value::as_str) {
        // A catalogued id is letters, digits, underscores and colons, which a
        // code span shows as they are. Anything else is escaped text.
        Some(code)
            if !code.is_empty()
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_:".contains(&byte)) =>
        {
            format!("[`{code}`]({})", rule_url(code))
        }
        Some(code) => format!("[{}]({})", escape(code), rule_url(code)),
        None => escape(&field(finding, "source")),
    }
}

fn location(finding: &Value) -> String {
    let path = field(finding, "path");
    let place = match finding.pointer("/span/line_start").and_then(Value::as_u64) {
        Some(line) if !path.is_empty() => format!("{path}:{line}"),
        _ if path.is_empty() => "workspace".to_owned(),
        _ => path,
    };
    escape(&place)
}

fn field(finding: &Value, name: &str) -> String {
    finding.get(name).map(text).unwrap_or_default()
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn bounded(message: &str) -> String {
    let mut characters = message.chars();
    let kept: String = characters.by_ref().take(MESSAGE_CHARACTERS).collect();
    if characters.next().is_some() {
        format!("{kept}…")
    } else {
        kept
    }
}

/// Text made inert for Markdown and HTML inside one table cell: markup
/// characters are escaped, angle brackets and ampersands become entities, a
/// line break becomes a space, and a zero-width space after `@`, `#`, a
/// scheme's `:` and `www` keeps GitHub from turning the text into a mention,
/// an issue reference or a link.
pub fn escape(text: &str) -> String {
    const BREAK: &str = "&#8203;";
    let mut escaped = String::with_capacity(text.len());
    for (index, character) in text.char_indices() {
        let rest = text.get(index + character.len_utf8()..).unwrap_or("");
        match character {
            ':' if rest.starts_with("//") => {
                escaped.push(':');
                escaped.push_str(BREAK);
            }
            '.' if escaped
                .as_bytes()
                .get(escaped.len().saturating_sub(3)..)
                .is_some_and(|tail| tail.eq_ignore_ascii_case(b"www")) =>
            {
                escaped.push_str(BREAK);
                escaped.push('.');
            }
            '#' => {
                escaped.push('#');
                escaped.push_str(BREAK);
            }
            '\\' | '`' | '*' | '_' | '[' | ']' | '|' | '~' => {
                escaped.push('\\');
                escaped.push(character);
            }
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '&' => escaped.push_str("&amp;"),
            '@' => {
                escaped.push('@');
                escaped.push_str(BREAK);
            }
            character if character.is_control() => escaped.push(' '),
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nothing_a_message_carries_survives_as_markup() {
        let hostile = "a | b `code` <script>x</script> [link](http://x) **bold** @someone\nnext\\";
        let escaped = escape(hostile);
        assert_eq!(
            escaped,
            "a \\| b \\`code\\` &lt;script&gt;x&lt;/script&gt; \\[link\\](http:&#8203;//x) \\*\\*bold\\*\\* @&#8203;someone next\\\\"
        );
        assert_eq!(
            escape("see #12, https://evil.example and WWW.evil.example"),
            "see #&#8203;12, https:&#8203;//evil.example and WWW&#8203;.evil.example"
        );
        assert!(!escaped.contains('\n') && !escaped.contains('<'));
    }

    #[test]
    fn a_report_of_another_schema_is_refused_naming_both_versions() {
        let error = render_markdown(&json!({ "schema_version": 7 }), 20).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("schema_version 7"), "{message}");
        assert!(message.contains(&SCHEMA_VERSION.to_string()), "{message}");
        assert_eq!(
            render_markdown(&json!({ "schema_version": SCHEMA_VERSION }), 20),
            Err(MarkdownError::NotAReport)
        );
    }

    #[test]
    fn a_long_message_is_cut_at_its_bound() {
        let long = "x".repeat(MESSAGE_CHARACTERS + 10);
        assert_eq!(bounded(&long).chars().count(), MESSAGE_CHARACTERS + 1);
        assert_eq!(bounded("short"), "short");
    }
}
