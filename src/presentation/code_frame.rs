use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::Path;

use serde::Serialize;

use super::GroupLocation;
use crate::workspace_path;

const FRAME_MAX_BYTES: u64 = 8 * 1024;
pub(super) const FRAME_MAX_LINES: usize = 5;
pub(super) const FRAME_MAX_COLUMNS: usize = 160;

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

pub fn code_frame(
    workspace_root: &Path,
    location: &GroupLocation,
) -> Result<CodeFrame, CodeFrameUnavailable> {
    read_code_frame(workspace_root, location, || {})
}

pub(super) fn read_code_frame(
    workspace_root: &Path,
    location: &GroupLocation,
    before_open: impl FnOnce(),
) -> Result<CodeFrame, CodeFrameUnavailable> {
    if !is_safe_relative_path(&location.path) {
        return Err(unavailable(
            None,
            CodeFrameUnavailableReason::OutsideWorkspace,
        ));
    }
    let safe_location = Some(format_location(location));
    let canonical_root = workspace_root.canonicalize().map_err(|_| {
        unavailable(
            safe_location.clone(),
            CodeFrameUnavailableReason::Unavailable,
        )
    })?;
    let relative_source =
        workspace_path::decode_normalized_relative(&location.path).ok_or_else(|| {
            unavailable(
                safe_location.clone(),
                CodeFrameUnavailableReason::OutsideWorkspace,
            )
        })?;
    let candidate = canonical_root.join(relative_source);
    let canonical_source = candidate.canonicalize().map_err(|_| {
        unavailable(
            safe_location.clone(),
            CodeFrameUnavailableReason::Unavailable,
        )
    })?;
    if canonical_source == canonical_root || !canonical_source.starts_with(&canonical_root) {
        return Err(unavailable(
            safe_location,
            CodeFrameUnavailableReason::OutsideWorkspace,
        ));
    }

    before_open();
    let file = File::open(&canonical_source).map_err(|_| {
        unavailable(
            safe_location.clone(),
            CodeFrameUnavailableReason::Unavailable,
        )
    })?;
    let opened_metadata = file.metadata().map_err(|_| {
        unavailable(
            safe_location.clone(),
            CodeFrameUnavailableReason::Unavailable,
        )
    })?;
    let revalidated = canonical_source.canonicalize().map_err(|_| {
        unavailable(
            safe_location.clone(),
            CodeFrameUnavailableReason::Unavailable,
        )
    })?;
    let current_metadata = fs::metadata(&revalidated).map_err(|_| {
        unavailable(
            safe_location.clone(),
            CodeFrameUnavailableReason::Unavailable,
        )
    })?;
    if !opened_metadata.is_file()
        || !revalidated.starts_with(&canonical_root)
        || !same_file(&opened_metadata, &current_metadata)
    {
        return Err(unavailable(
            safe_location,
            CodeFrameUnavailableReason::OutsideWorkspace,
        ));
    }

    let mut bytes = Vec::with_capacity(FRAME_MAX_BYTES as usize);
    file.take(FRAME_MAX_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            unavailable(
                safe_location.clone(),
                CodeFrameUnavailableReason::Unavailable,
            )
        })?;
    if bytes.contains(&0) {
        return Err(unavailable(
            safe_location,
            CodeFrameUnavailableReason::Binary,
        ));
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| {
        unavailable(
            safe_location.clone(),
            CodeFrameUnavailableReason::InvalidUtf8,
        )
    })?;
    frame_from_source(location, source)
        .ok_or_else(|| unavailable(safe_location, CodeFrameUnavailableReason::Unavailable))
}

fn frame_from_source(location: &GroupLocation, source: &str) -> Option<CodeFrame> {
    let source_lines: Vec<_> = source.lines().collect();
    let primary = location.span.line_start.max(1);
    if primary > source_lines.len() {
        return None;
    }
    let mut start = primary.saturating_sub(2).max(1);
    let mut end = start
        .saturating_add(FRAME_MAX_LINES.saturating_sub(1))
        .min(source_lines.len());
    if primary > end {
        start = primary
            .saturating_sub(FRAME_MAX_LINES.saturating_sub(1))
            .max(1);
        end = primary;
    }

    let mut lines = Vec::with_capacity(end.saturating_sub(start).saturating_add(1));
    for number in start..=end {
        let sanitized = sanitize_line(source_lines[number - 1]);
        let is_primary = number == primary;
        let marker = is_primary.then(|| {
            let start = sanitized.source_column(location.span.column_start);
            let end = sanitized.source_column(location.span.column_end);
            let last_marker_column = if sanitized.truncated {
                FRAME_MAX_COLUMNS
            } else {
                sanitized.width.saturating_add(1).min(FRAME_MAX_COLUMNS)
            };
            let column_start = start.saturating_add(1).min(last_marker_column).max(1);
            let column_end = end
                .saturating_add(1)
                .max(column_start.saturating_add(1))
                .min(FRAME_MAX_COLUMNS.saturating_add(1));
            CodeFrameMarker {
                column_start,
                column_end,
            }
        });
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

struct SanitizedLine {
    text: String,
    source_columns: Vec<usize>,
    width: usize,
    truncated: bool,
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
    let characters: Vec<_> = source.chars().collect();
    let mut output = String::new();
    let mut source_columns = vec![0; characters.len().saturating_add(1)];
    let mut source_width = 0usize;
    let mut output_width = 0usize;
    let mut accepting_output = true;
    let mut index = 0;
    while index < characters.len() {
        source_columns[index] = source_width;
        let character = characters[index];
        if character == '\u{001b}' {
            let next = skip_escape_sequence(&characters, index);
            source_columns[index + 1..=next.min(characters.len())].fill(source_width);
            index = next;
            continue;
        }
        if matches!(
            character,
            '\u{0090}' | '\u{0098}' | '\u{009b}' | '\u{009d}' | '\u{009e}' | '\u{009f}'
        ) {
            let next = skip_c1_sequence(&characters, index);
            source_columns[index + 1..=next.min(characters.len())].fill(source_width);
            index = next;
            continue;
        }
        let token = if character == '\t' {
            " ".repeat(4 - (source_width % 4))
        } else if character.is_control() {
            source_columns[index + 1] = source_width;
            index += 1;
            continue;
        } else if character.is_ascii() {
            character.to_string()
        } else {
            format!("\\u{{{:X}}}", character as u32)
        };
        source_width = source_width.saturating_add(token.len());
        if accepting_output && source_width <= FRAME_MAX_COLUMNS {
            output.push_str(&token);
            output_width = source_width;
        } else if !token.is_empty() {
            accepting_output = false;
        }
        source_columns[index + 1] = source_width;
        index += 1;
    }
    SanitizedLine {
        text: output,
        source_columns,
        width: output_width,
        truncated: !accepting_output,
    }
}

fn skip_escape_sequence(characters: &[char], start: usize) -> usize {
    let Some(kind) = characters.get(start + 1).copied() else {
        return characters.len();
    };
    match kind {
        '[' => skip_control_sequence(characters, start + 2),
        ']' | 'P' | 'X' | '^' | '_' => skip_string_sequence(characters, start + 2),
        _ => (start + 2).min(characters.len()),
    }
}

fn skip_c1_sequence(characters: &[char], start: usize) -> usize {
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

fn format_location(location: &GroupLocation) -> String {
    format!(
        "{}:{}:{}",
        location.path,
        location.span.line_start.max(1),
        location.span.column_start.max(1)
    )
}

fn is_safe_relative_path(path: &str) -> bool {
    workspace_path::decode_normalized_relative(path).is_some()
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

#[cfg(unix)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && left.permissions().readonly() == right.permissions().readonly()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_column_mapping_uses_full_unicode_sequence_width() {
        let line = sanitize_line("👩‍🔬x");

        assert_eq!(line.text, "\\u{1F469}\\u{200D}\\u{1F52C}x");
        assert_eq!(line.text.len(), 27);
        assert_eq!(line.source_column(4), 26);
        assert_eq!(line.source_column(5), 27);
    }

    #[test]
    fn terminal_width_is_exact_because_non_ascii_is_escaped() {
        assert_eq!(sanitize_line("e\u{301}").text, "e\\u{301}");
        assert_eq!(sanitize_line("א\u{5b0}").text, "\\u{5D0}\\u{5B0}");
        assert_eq!(
            sanitize_line("❤️1️⃣").text,
            "\\u{2764}\\u{FE0F}1\\u{FE0F}\\u{20E3}"
        );
        assert_eq!(sanitize_line("a\tb").text, "a   b");
    }
}
