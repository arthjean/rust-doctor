use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use serde::Serialize;

use super::GroupLocation;
use crate::terminal_text::{skip_c1_sequence, skip_escape_sequence};
use crate::workspace_path;

#[cfg(test)]
mod tests;

/// Lines of context the frame carries above the reported one.
const FRAME_CONTEXT_LINES: usize = 2;
const FRAME_MAX_LINES: usize = 5;
const FRAME_MAX_COLUMNS: usize = 160;

/// Bytes of one line the frame keeps.
///
/// A hundred and sixty columns need at most six hundred and forty bytes of
/// characters, and the rest of this budget absorbs the escape sequences that
/// occupy no column at all. A line cut here loses its tail and nothing else:
/// an introducer that was kept still swallows everything after it, so a cut can
/// never expose the inside of a sequence as text.
const LINE_MAX_BYTES: usize = 8 * 1024;

/// Bytes the reader scans looking for the reported line.
///
/// This is a scan budget, not a window, and the difference is the whole reason
/// the reader walks lines. A cap on the bytes decoded left every finding past
/// the first kilobytes of a file unframeable however short its own line was,
/// and cut a character in half often enough to report a valid file as invalid
/// UTF-8. Reaching this bound costs the frame nothing unless the reported line
/// itself lies beyond it, which is why it sits at the size past which a file
/// has stopped being source a person reads rather than at a size chosen to keep
/// a frame small.
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
    /// Digits the widest line number of the frame needs.
    ///
    /// Both reports lay their gutter out from this rather than each guessing a
    /// width. A guess holds until a frame carries a line number wider than it,
    /// and then the caret row and the source row stop agreeing on where the
    /// text begins, in one report and not the other.
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
    // The path is decoded once. Failing here is the only case that answers with
    // no location at all, since a path this rejected is one the report should
    // not echo back.
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
    // Opening a path that is not a regular file is not merely useless: opening
    // a named pipe blocks in the call itself, before any check on the handle
    // could reject it. The check below the open is the one that decides; this
    // one only keeps a tree carrying a pipe from stopping the report.
    if !fs::symlink_metadata(&canonical_source).is_ok_and(|metadata| metadata.is_file()) {
        return Err(fail(CodeFrameUnavailableReason::Unavailable));
    }

    // Canonicalization alone has a replacement race. Open the checked path, then
    // verify that the live path still resolves inside the workspace and
    // identifies the same file as the handle.
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
    let window = match read_window(file, start, SCAN_MAX_BYTES) {
        Ok(window) => window,
        Err(reason) => return Err(fail(reason)),
    };
    frame_from_window(location, start, &window)
        .ok_or_else(|| fail(CodeFrameUnavailableReason::Unavailable))
}

/// First line the frame shows.
///
/// The window runs from here for `FRAME_MAX_LINES`, which always covers the
/// reported line since only `FRAME_CONTEXT_LINES` of it sit above: anchoring
/// above the line and reading forward is what leaves the reader with no case
/// where it has to walk back to a line it already passed.
fn window_start(location: &GroupLocation) -> usize {
    location
        .span
        .line_start
        .max(1)
        .saturating_sub(FRAME_CONTEXT_LINES)
        .max(1)
}

/// Reads the lines the frame is built from, one line at a time.
///
/// Walking lines is what makes the reported line as reachable at the end of a
/// file as at its top, and decoding only the lines that are kept is what keeps
/// a character straddling a bound from making a valid file look like it is not
/// UTF-8. Both were bugs of the byte prefix this replaced.
///
/// The budget is a parameter rather than the constant read directly, so that
/// the one branch answering what happens when it runs out mid-line is reachable
/// from a test without an eight megabyte fixture.
fn read_window(
    file: File,
    start: usize,
    budget: u64,
) -> Result<Vec<String>, CodeFrameUnavailableReason> {
    let last = start.saturating_add(FRAME_MAX_LINES).saturating_sub(1);
    let mut reader = BufReader::new(file.take(budget));
    let mut window = Vec::with_capacity(FRAME_MAX_LINES);
    let mut raw = Vec::new();
    for number in 1..=last {
        raw.clear();
        let read = reader
            .read_until(b'\n', &mut raw)
            .map_err(|_| CodeFrameUnavailableReason::Unavailable)?;
        if read == 0 {
            break;
        }
        // A line the scan budget cut short is not a line of the file. Dropping
        // it answers `Unavailable` for a reported line that far out rather than
        // rendering a fragment as if it were the source.
        if raw.last() != Some(&b'\n') && reader.get_ref().limit() == 0 {
            break;
        }
        if number < start {
            continue;
        }
        window.push(retain_line(&raw)?);
    }
    Ok(window)
}

/// One kept line, stripped of its terminator, bounded and decoded.
///
/// The NUL check covers what is kept rather than the whole file, which is the
/// same guarantee it always carried: nothing outside the window is ever
/// rendered, so nothing outside it can leak.
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

/// Where a kept line is cut: at `LINE_MAX_BYTES`, walked back to a character
/// boundary so that a cut is never mistaken for a decoding failure.
///
/// A character is at most four bytes, so a boundary is at most three bytes
/// back. Not finding one there means the bytes are not UTF-8, which is the
/// caller's answer rather than a silently emptied line.
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
) -> Option<CodeFrame> {
    let primary = location.span.line_start.max(1);
    if primary.saturating_sub(start) >= window.len() {
        return None;
    }
    let mut lines = Vec::with_capacity(window.len());
    for (offset, source) in window.iter().enumerate() {
        let number = start.saturating_add(offset);
        let sanitized = sanitize_line(source);
        let is_primary = number == primary;
        let marker = is_primary.then(|| marker_for(location, &sanitized));
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

/// The caret under the reported line.
///
/// It never leaves the text the frame rendered. A span that ends on a later
/// line has no end column on this one, so it runs to the end of what is shown
/// rather than borrowing a column from a line the reader cannot see, and a
/// span pointing past a line the sanitizer cut stops where the cut did rather
/// than at the frame's own edge.
fn marker_for(location: &GroupLocation, sanitized: &SanitizedLine) -> CodeFrameMarker {
    let span = &location.span;
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
        if character == '\u{001b}' {
            let next = skip_escape_sequence(&characters, index);
            fill_columns(&mut source_columns, after, next, source_width);
            index = next;
            continue;
        }
        if matches!(
            character,
            '\u{0090}' | '\u{0098}' | '\u{009b}' | '\u{009d}' | '\u{009e}' | '\u{009f}'
        ) {
            let next = skip_c1_sequence(&characters, index);
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
        // Every token above is at least one character wide, so overflowing the
        // frame closes the output for the rest of the line rather than skipping
        // one token and letting a narrower one behind it back in.
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

/// Records the column a source character starts at.
///
/// The map is one longer than the line, so every character and the position
/// after the last one have a slot. Writing through the slot rather than through
/// an index is what makes that a property of the code instead of a proof spread
/// across this function and the two sequence skippers it calls.
fn set_column(columns: &mut [usize], index: usize, width: usize) {
    if let Some(slot) = columns.get_mut(index) {
        *slot = width;
    }
}

/// The same for the run a skipped escape sequence covers, which all start where
/// the sequence did because none of them occupies a column of its own.
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
