use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use serde::Serialize;

use super::GroupLocation;
use crate::DiagnosticSpan;
use crate::repo_hygiene::{REDACTED, REDACTED_MIN_CHARS, masked};
use crate::terminal_text::{escape_end, opens_escape};
use crate::workspace_path;

#[cfg(test)]
mod tests;

const FRAME_CONTEXT_LINES: usize = 2;
const FRAME_MAX_LINES: usize = 5;
const FRAME_MAX_COLUMNS: usize = 160;

const LINE_MAX_BYTES: usize = 8 * 1024;

const SCAN_MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeFrame {
    pub location: String,
    pub lines: Vec<CodeFrameLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeFrameLine {
    pub number: usize,
    pub text: String,
    pub primary: bool,
    pub marker: Option<CodeFrameMarker>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CodeFrameMarker {
    pub column_start: usize,
    pub column_end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodeFrameUnavailableReason {
    OutsideWorkspace,
    Unavailable,
    Binary,
    InvalidUtf8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeFrameUnavailable {
    pub location: Option<String>,
    pub reason: CodeFrameUnavailableReason,
    pub message: String,
}

impl CodeFrame {
    pub fn gutter_width(&self) -> usize {
        self.lines
            .iter()
            .map(|line| line.number.to_string().len())
            .max()
            .unwrap_or(1)
    }
}

pub fn code_frame(
    workspace_root: &Path,
    location: &GroupLocation,
) -> Result<CodeFrame, CodeFrameUnavailable> {
    read_code_frame(workspace_root, location, || {})
}

fn read_code_frame(
    workspace_root: &Path,
    location: &GroupLocation,
    before_open: impl FnOnce(),
) -> Result<CodeFrame, CodeFrameUnavailable> {
    let Some(relative_source) = workspace_path::decode_normalized_relative(&location.path) else {
        return Err(unavailable(
            None,
            CodeFrameUnavailableReason::OutsideWorkspace,
        ));
    };
    let safe_location = format_location(location);
    let fail = |reason| unavailable(Some(safe_location.clone()), reason);

    let canonical_root = workspace_root
        .canonicalize()
        .map_err(|_| fail(CodeFrameUnavailableReason::Unavailable))?;
    let canonical_source = canonical_root
        .join(relative_source)
        .canonicalize()
        .map_err(|_| fail(CodeFrameUnavailableReason::Unavailable))?;
    if canonical_source == canonical_root || !canonical_source.starts_with(&canonical_root) {
        return Err(fail(CodeFrameUnavailableReason::OutsideWorkspace));
    }
    if !fs::symlink_metadata(&canonical_source).is_ok_and(|metadata| metadata.is_file()) {
        return Err(fail(CodeFrameUnavailableReason::Unavailable));
    }

    before_open();
    let file =
        File::open(&canonical_source).map_err(|_| fail(CodeFrameUnavailableReason::Unavailable))?;
    let opened_metadata = file
        .metadata()
        .map_err(|_| fail(CodeFrameUnavailableReason::Unavailable))?;
    let revalidated = canonical_source
        .canonicalize()
        .map_err(|_| fail(CodeFrameUnavailableReason::Unavailable))?;
    let current_metadata =
        fs::metadata(&revalidated).map_err(|_| fail(CodeFrameUnavailableReason::Unavailable))?;
    if !opened_metadata.is_file()
        || !revalidated.starts_with(&canonical_root)
        || !workspace_path::same_file(&opened_metadata, &current_metadata)
    {
        return Err(fail(CodeFrameUnavailableReason::OutsideWorkspace));
    }

    let start = window_start(location);
    let (window, carry) = match read_window(file, start, SCAN_MAX_BYTES) {
        Ok(read) => read,
        Err(reason) => return Err(fail(reason)),
    };
    frame_from_window(location, start, &window, carry)
        .ok_or_else(|| fail(CodeFrameUnavailableReason::Unavailable))
}

fn window_start(location: &GroupLocation) -> usize {
    location
        .span
        .line_start
        .max(1)
        .saturating_sub(FRAME_CONTEXT_LINES)
        .max(1)
}

fn read_window(
    file: File,
    start: usize,
    budget: u64,
) -> Result<(Vec<String>, Carry), CodeFrameUnavailableReason> {
    let last = start.saturating_add(FRAME_MAX_LINES).saturating_sub(1);
    let mut reader = BufReader::new(file.take(budget));
    let mut window = Vec::with_capacity(FRAME_MAX_LINES);
    let mut raw = Vec::new();
    let mut carry = Carry::default();
    for number in 1..=last {
        raw.clear();
        let read = reader
            .read_until(b'\n', &mut raw)
            .map_err(|_| CodeFrameUnavailableReason::Unavailable)?;
        if read == 0 {
            break;
        }
        if raw.last() != Some(&b'\n') && reader.get_ref().limit() == 0 {
            break;
        }
        if number < start {
            let line = String::from_utf8_lossy(&raw);
            track_private_key(&line, &mut carry.key_body);
            fallback_literals(&line, &mut carry.fallback);
            continue;
        }
        window.push(retain_line(&raw)?);
    }
    Ok((window, carry))
}

fn retain_line(raw: &[u8]) -> Result<String, CodeFrameUnavailableReason> {
    let line = raw.strip_suffix(b"\n").unwrap_or(raw);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.contains(&0) {
        return Err(CodeFrameUnavailableReason::Binary);
    }
    let end = bounded_end(line).ok_or(CodeFrameUnavailableReason::InvalidUtf8)?;
    line.get(..end)
        .and_then(|kept| std::str::from_utf8(kept).ok())
        .map(str::to_owned)
        .ok_or(CodeFrameUnavailableReason::InvalidUtf8)
}

fn bounded_end(line: &[u8]) -> Option<usize> {
    if line.len() <= LINE_MAX_BYTES {
        return Some(line.len());
    }
    (LINE_MAX_BYTES.saturating_sub(3)..=LINE_MAX_BYTES)
        .rev()
        .find(|end| line.get(*end).is_some_and(|byte| !is_continuation(*byte)))
}

const fn is_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

fn frame_from_window(
    location: &GroupLocation,
    start: usize,
    window: &[String],
    carry: Carry,
) -> Option<CodeFrame> {
    let primary = location.span.line_start.max(1);
    if primary.saturating_sub(start) >= window.len() {
        return None;
    }
    let mut lines = Vec::with_capacity(window.len());
    let mut carry = carry;
    for (offset, source) in window.iter().enumerate() {
        let number = start.saturating_add(offset);
        let redacted = redact_line(source, &mut carry);
        let sanitized = sanitize_line(&redacted.text);
        let is_primary = number == primary;
        let marker = is_primary.then(|| marker_for(&redacted.span(&location.span), &sanitized));
        lines.push(CodeFrameLine {
            number,
            text: sanitized.text,
            primary: is_primary,
            marker,
        });
    }
    Some(CodeFrame {
        location: format_location(location),
        lines,
    })
}

fn marker_for(span: &DiagnosticSpan, sanitized: &SanitizedLine) -> CodeFrameMarker {
    let start = sanitized.source_column(span.column_start);
    let end = if span.line_end > span.line_start {
        sanitized.width
    } else {
        sanitized.source_column(span.column_end)
    };
    let last_column = sanitized.width.saturating_add(1).min(FRAME_MAX_COLUMNS);
    let column_start = start.saturating_add(1).min(last_column).max(1);
    CodeFrameMarker {
        column_start,
        column_end: end
            .saturating_add(1)
            .min(last_column)
            .max(column_start.saturating_add(1)),
    }
}

struct RedactedLine {
    text: String,
    edits: Vec<ColumnEdit>,
}

struct ColumnEdit {
    start: usize,
    end: usize,
    width: usize,
}

impl RedactedLine {
    fn span(&self, span: &DiagnosticSpan) -> DiagnosticSpan {
        DiagnosticSpan {
            column_start: self.column(span.column_start),
            column_end: self.column(span.column_end),
            ..span.clone()
        }
    }

    fn column(&self, column: usize) -> usize {
        let index = column.saturating_sub(1);
        let mut added = 0usize;
        let mut removed = 0usize;
        for edit in &self.edits {
            if index >= edit.end {
                added = added.saturating_add(edit.width);
                removed = removed.saturating_add(edit.end.saturating_sub(edit.start));
            } else if index > edit.start {
                return edit
                    .start
                    .saturating_sub(removed)
                    .saturating_add(added)
                    .saturating_add(1);
            } else {
                break;
            }
        }
        index.saturating_sub(removed).saturating_add(added).saturating_add(1)
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Carry {
    key_body: bool,
    fallback: bool,
}

fn redact_line(source: &str, carry: &mut Carry) -> RedactedLine {
    let mut text = String::with_capacity(source.len());
    let mut edits = Vec::new();
    let mut copied = 0usize;
    if carry.key_body {
        let body_end = crate::repo_hygiene::open_body_end(source);
        if body_end > 0 || source.is_empty() {
            text.push_str(REDACTED);
            edits.push(ColumnEdit {
                start: 0,
                end: source.get(..body_end).map_or(0, |body| body.chars().count()),
                width: REDACTED.len(),
            });
        }
        if body_end == source.len() {
            return RedactedLine { text, edits };
        }
        carry.key_body = false;
        copied = body_end;
    }
    track_private_key(source, &mut carry.key_body);
    let mut ranges = crate::repo_hygiene::credential_ranges(source);
    ranges.extend(fallback_literals(source, &mut carry.fallback));
    ranges.sort_unstable();
    for (start, end) in ranges {
        let start = start.max(copied);
        if end <= start {
            continue;
        }
        let (Some(before), Some(value)) = (source.get(copied..start), source.get(start..end))
        else {
            continue;
        };
        text.push_str(before);
        let replacement = masked(value);
        let char_start = source.get(..start).map_or(0, |head| head.chars().count());
        edits.push(ColumnEdit {
            start: char_start,
            end: char_start.saturating_add(value.chars().count()),
            width: replacement.chars().count(),
        });
        text.push_str(&replacement);
        copied = end;
    }
    text.push_str(source.get(copied..).unwrap_or_default());
    RedactedLine { text, edits }
}

fn track_private_key(line: &str, open: &mut bool) {
    if *open && line.contains("-----END ") {
        *open = false;
    }
    if crate::repo_hygiene::key_continues(line) {
        *open = true;
    }
}

const FALLBACK_CALLS: [&str; 2] = ["unwrap_or(", "unwrap_or_else("];

fn fallback_literals(line: &str, open: &mut bool) -> Vec<(usize, usize)> {
    let mut arguments: Vec<usize> = Vec::new();
    if std::mem::take(open) {
        arguments.push(0);
    }
    for call in FALLBACK_CALLS {
        arguments.extend(line.match_indices(call).map(|(at, call)| at.saturating_add(call.len())));
    }
    let mut ranges = Vec::new();
    for argument in arguments {
        let at = skip_fallback_head(line, argument);
        if line.get(at..).is_none_or(str::is_empty) {
            *open = true;
        } else if let Some((start, end)) = string_literal(line, at)
            && line.get(start..end).is_some_and(|text| text.chars().count() >= REDACTED_MIN_CHARS)
        {
            ranges.push((start, end));
        }
    }
    ranges
}

fn skip_fallback_head(line: &str, mut at: usize) -> usize {
    loop {
        let Some(rest) = line.get(at..) else {
            return at;
        };
        let trimmed = rest.trim_start();
        at = at.saturating_add(rest.len().saturating_sub(trimmed.len()));
        let skipped = if let Some(closure) = trimmed.strip_prefix('|') {
            closure.find('|').map(|close| close.saturating_add(2))
        } else if trimmed.starts_with("move ") {
            Some("move ".len())
        } else if trimmed.starts_with('{') {
            Some(1)
        } else {
            ["String::from(", "OsString::from("]
                .into_iter()
                .find(|conversion| trimmed.starts_with(conversion))
                .map(str::len)
        };
        match skipped {
            Some(length) => at = at.saturating_add(length),
            None => return at,
        }
    }
}

fn string_literal(line: &str, at: usize) -> Option<(usize, usize)> {
    let rest = line.get(at..)?;
    let hashes = rest.strip_prefix('r').map_or(0, |raw| raw.len().saturating_sub(raw.trim_start_matches('#').len()));
    let prefix = if rest.starts_with('r') { 1usize.saturating_add(hashes) } else { 0 };
    rest.get(prefix..)?.starts_with('"').then_some(())?;
    let start = at.saturating_add(prefix).saturating_add(1);
    let body = line.get(start..)?;
    let length = if prefix == 0 {
        let mut escaped = false;
        body.char_indices()
            .find(|(_, character)| {
                let closes = *character == '"' && !escaped;
                escaped = *character == '\\' && !escaped;
                closes
            })
            .map(|(offset, _)| offset)
    } else {
        body.find(&format!("\"{}", "#".repeat(hashes)))
    };
    Some((start, start.saturating_add(length.unwrap_or(body.len()))))
}

struct SanitizedLine {
    text: String,
    source_columns: Vec<usize>,
    width: usize,
}

impl SanitizedLine {
    fn source_column(&self, column: usize) -> usize {
        self.source_columns
            .get(column.saturating_sub(1))
            .copied()
            .unwrap_or_else(|| self.source_columns.last().copied().unwrap_or(0))
    }
}

fn sanitize_line(source: &str) -> SanitizedLine {
    let characters: Vec<char> = source.chars().collect();
    let mut output = String::new();
    let mut source_columns = vec![0; characters.len().saturating_add(1)];
    let mut source_width = 0usize;
    let mut width = 0usize;
    let mut accepting_output = true;
    let mut index = 0;
    while let Some(character) = characters.get(index).copied() {
        let after = index.saturating_add(1);
        set_column(&mut source_columns, index, source_width);
        if opens_escape(character) {
            let next = escape_end(&characters, index);
            fill_columns(&mut source_columns, after, next, source_width);
            index = next;
            continue;
        }
        let token = if character == '\t' {
            " ".repeat(4 - (source_width % 4))
        } else if character.is_control() {
            set_column(&mut source_columns, after, source_width);
            index = after;
            continue;
        } else if character.is_ascii() {
            character.to_string()
        } else {
            format!("\\u{{{:X}}}", character as u32)
        };
        source_width = source_width.saturating_add(token.len());
        if accepting_output && source_width <= FRAME_MAX_COLUMNS {
            output.push_str(&token);
            width = source_width;
        } else {
            accepting_output = false;
        }
        set_column(&mut source_columns, after, source_width);
        index = after;
    }
    SanitizedLine {
        text: output,
        source_columns,
        width,
    }
}

fn set_column(columns: &mut [usize], index: usize, width: usize) {
    if let Some(slot) = columns.get_mut(index) {
        *slot = width;
    }
}

fn fill_columns(columns: &mut [usize], from: usize, to: usize, width: usize) {
    let last = to.min(columns.len().saturating_sub(1));
    if let Some(covered) = columns.get_mut(from..=last) {
        covered.fill(width);
    }
}

fn format_location(location: &GroupLocation) -> String {
    format!(
        "{}:{}:{}",
        location.path,
        location.span.line_start.max(1),
        location.span.column_start.max(1)
    )
}

fn unavailable(
    location: Option<String>,
    reason: CodeFrameUnavailableReason,
) -> CodeFrameUnavailable {
    let message = match reason {
        CodeFrameUnavailableReason::OutsideWorkspace => {
            "Code frame unavailable outside the workspace."
        }
        CodeFrameUnavailableReason::Unavailable
        | CodeFrameUnavailableReason::Binary
        | CodeFrameUnavailableReason::InvalidUtf8 => "Code frame unavailable.",
    };
    CodeFrameUnavailable {
        location,
        reason,
        message: message.to_owned(),
    }
}
