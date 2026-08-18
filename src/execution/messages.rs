//! Cargo's `--message-format=json` stream: the shape of what it carries, and
//! the line-by-line reader that turns it into that shape.
//!
//! One rule holds the file together: **every bound here is a budget on work,
//! never a filter on meaning**. A line past `LINE_MAX_BYTES` is dropped as
//! malformed and counted, a stream past `MESSAGE_MAX_COUNT` messages stops with
//! an error at the `parsing` stage, and neither decides which diagnostic the
//! report is allowed to publish. The reader used to have no bound at all, in a
//! crate whose git layer documents that every way out of it is bounded: the
//! scanned workspace's procedural macros decide how many diagnostics Cargo
//! emits, and `has_contaminated_cargo_suffix` costs one JSON parse per `{` of
//! the line it is handed.

use std::io::{BufRead, Read};

use cargo_metadata::Message;
use serde::Deserialize;
use serde_json::Value;

use crate::internal_error::InternalError;

/// What one line of the stream may contribute.
///
/// Cargo emits one object per line, and the largest of them is a compiler
/// message carrying its own rendered form. A megabyte is far past any of them;
/// past it, the only producers are a `RUSTC_WRAPPER` or a wrapper script
/// writing into Cargo's stdout.
const LINE_MAX_BYTES: usize = 1024 * 1024;

/// How many messages one scan may keep.
///
/// The report ranks a finite list and the terminal renders a finite report, so
/// a scan that has already collected this many has collected every diagnostic
/// anyone will read. Stopping here says so; growing until the allocator gives
/// up would not.
const MESSAGE_MAX_COUNT: usize = 100_000;

#[derive(Debug)]
pub(crate) struct ScanExecution {
    pub(crate) command: Vec<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) exit_success: Option<bool>,
    pub(crate) build_finished: Option<bool>,
    pub(crate) noise_lines: usize,
    pub(crate) malformed_messages: usize,
    pub(crate) messages: Vec<CapturedMessage>,
    pub(crate) errors: Vec<InternalError>,
}

#[derive(Debug)]
pub(crate) enum CapturedMessage {
    Compiler(CompilerMessageData),
    Known(Box<Message>),
    Unknown(Value),
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompilerMessageData {
    pub(crate) package_id: String,
    pub(crate) target: CapturedTarget,
    pub(crate) message: CapturedDiagnostic,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedTarget {
    pub(crate) name: String,
    /// Target kind as Cargo declares it: `lib`, `bin`, `test`, `bench`,
    /// `example`, `custom-build`. The build system answers, not a path
    /// heuristic, and it stays exact where `target.test` does not: under
    /// `--all-targets`, Cargo marks `test` true even on a binary with no test
    /// at all, while `kind` keeps separating what the project ships from what
    /// it compiles to check itself.
    #[serde(default)]
    pub(crate) kind: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedDiagnostic {
    pub(crate) message: String,
    pub(crate) code: Option<CapturedDiagnosticCode>,
    pub(crate) level: String,
    #[serde(default)]
    pub(crate) spans: Vec<CapturedSpan>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedDiagnosticCode {
    pub(crate) code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedSpan {
    pub(crate) file_name: String,
    pub(crate) line_start: usize,
    pub(crate) line_end: usize,
    pub(crate) column_start: usize,
    pub(crate) column_end: usize,
    pub(crate) is_primary: bool,
}

#[derive(Debug, Default)]
pub(super) struct CollectedMessages {
    pub(super) build_finished: Option<bool>,
    pub(super) noise_lines: usize,
    pub(super) malformed_messages: usize,
    pub(super) messages: Vec<CapturedMessage>,
    pub(super) errors: Vec<InternalError>,
}

/// One record of the stream: the bytes of a line, and whether the line ended
/// where the reader stopped keeping it.
enum Record {
    Line { bytes: Vec<u8>, truncated: bool },
    End,
}

pub(super) fn collect(reader: impl BufRead) -> CollectedMessages {
    let mut collected = CollectedMessages::default();
    let mut reader = reader;

    loop {
        match read_record(&mut reader) {
            Ok(Record::End) => break,
            // A line the reader had to cut is a line no parser can be given: a
            // truncated JSON object is not the message Cargo emitted, and the
            // suffix scan below is quadratic in the length it was cut from.
            Ok(Record::Line {
                truncated: true, ..
            }) => collected.malformed_messages += 1,
            Ok(Record::Line { bytes, .. }) => {
                // A line that is not UTF-8 is a malformed message, not a
                // failed read: the stream keeps going, and one unreadable line
                // no longer costs the report every message after it.
                let Ok(record) = str::from_utf8(&bytes) else {
                    collected.malformed_messages += 1;
                    continue;
                };
                capture_record(record, &mut collected);
            }
            Err(error) => {
                collected.errors.push(InternalError::new(
                    "parsing",
                    "stdout-read",
                    format!("could not read Clippy stdout: {error}"),
                ));
                break;
            }
        }
        if collected.messages.len() >= MESSAGE_MAX_COUNT {
            collected.errors.push(InternalError::new(
                "parsing",
                "message-limit",
                format!("Cargo emitted more than {MESSAGE_MAX_COUNT} messages"),
            ));
            break;
        }
    }

    collected
}

/// Reads one line, keeping at most `LINE_MAX_BYTES` of it.
///
/// The rest of an oversized line is consumed rather than left in the pipe: the
/// producer is still writing, and stopping the read would block it forever.
fn read_record(reader: &mut impl BufRead) -> std::io::Result<Record> {
    let mut bytes = Vec::new();
    let read = (&mut *reader)
        .take(LINE_MAX_BYTES as u64)
        .read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Ok(Record::End);
    }
    // A last line with no newline is a complete line, not a cut one: only a
    // line that filled the whole budget without ending was cut.
    let truncated = !bytes.ends_with(b"\n") && bytes.len() >= LINE_MAX_BYTES;
    if truncated {
        let mut discarded = Vec::new();
        loop {
            discarded.clear();
            let read = (&mut *reader)
                .take(LINE_MAX_BYTES as u64)
                .read_until(b'\n', &mut discarded)?;
            if read == 0 || discarded.ends_with(b"\n") {
                break;
            }
        }
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    Ok(Record::Line { bytes, truncated })
}

fn capture_record(record: &str, collected: &mut CollectedMessages) {
    let normalized = record.trim_start_matches(|character: char| character.is_ascii_whitespace());
    if normalized.is_empty() {
        if !record.is_empty() {
            collected.noise_lines += 1;
        }
    } else if normalized.starts_with('{') {
        capture_json_line(normalized, collected);
    } else if has_contaminated_cargo_suffix(normalized) {
        collected.malformed_messages += 1;
    } else {
        collected.noise_lines += 1;
    }
}

fn has_contaminated_cargo_suffix(line: &str) -> bool {
    line.char_indices()
        .filter(|(_, character)| *character == '{')
        .any(|(index, _)| {
            line.get(index..).is_some_and(|suffix| {
                serde_json::from_str::<Value>(suffix)
                    .ok()
                    .is_some_and(|value| value.get("reason").and_then(Value::as_str).is_some())
            })
        })
}

fn capture_json_line(line: &str, collected: &mut CollectedMessages) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        collected.malformed_messages += 1;
        return;
    };

    let reason = value.get("reason").and_then(Value::as_str);
    if reason == Some("compiler-message") {
        match serde_json::from_value::<CompilerMessageData>(value) {
            Ok(message) => collected.messages.push(CapturedMessage::Compiler(message)),
            Err(_) => collected.malformed_messages += 1,
        }
        return;
    }

    let known_reason = matches!(
        reason,
        Some("compiler-artifact" | "build-script-executed" | "build-finished")
    );
    if !known_reason {
        if reason.is_some() {
            collected.messages.push(CapturedMessage::Unknown(value));
        } else {
            collected.malformed_messages += 1;
        }
        return;
    }

    match serde_json::from_value::<Message>(value) {
        Ok(message) => {
            if let Message::BuildFinished(finished) = &message {
                collected.build_finished = Some(finished.success);
            }
            collected
                .messages
                .push(CapturedMessage::Known(Box::new(message)));
        }
        Err(_) => collected.malformed_messages += 1,
    }
}

#[cfg(test)]
mod tests;
