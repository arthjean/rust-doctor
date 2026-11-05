use std::io::{BufRead, Read};

use cargo_metadata::Message;
use serde::Deserialize;
use serde_json::Value;

use crate::internal_error::InternalError;

const LINE_MAX_BYTES: usize = 1024 * 1024;

const MESSAGE_MAX_COUNT: usize = 100_000;

#[derive(Debug, Default)]
pub(crate) struct ScanExecution {
    pub(crate) command: Vec<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) exit_success: Option<bool>,
    pub(crate) build_finished: Option<bool>,
    pub(crate) noise_lines: usize,
    pub(crate) malformed_messages: usize,
    pub(crate) messages: Vec<CapturedMessage>,
    pub(crate) errors: Vec<InternalError>,
    pub(crate) notices: Vec<InternalError>,
    pub(crate) cargo_cause: Option<String>,
    pub(crate) deadline_exceeded: bool,
    pub(crate) not_evaluated: Vec<&'static str>,
    pub(crate) removed_lint_flags: Vec<String>,
    pub(crate) unlinted: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum CapturedMessage {
    Compiler(CompilerMessageData),
    Known(Box<Message>),
    Unknown,
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
    #[serde(default)]
    pub(crate) children: Vec<CapturedDiagnostic>,
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
    #[serde(default)]
    pub(crate) suggested_replacement: Option<String>,
    #[serde(default)]
    pub(crate) suggestion_applicability: Option<String>,
    #[serde(default)]
    pub(crate) text: Vec<CapturedSpanLine>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedSpanLine {
    pub(crate) text: String,
    pub(crate) highlight_start: usize,
    pub(crate) highlight_end: usize,
}

impl CapturedSpanLine {
    pub(crate) fn highlighted(&self) -> String {
        let start = self.highlight_start.saturating_sub(1);
        let length = self.highlight_end.saturating_sub(self.highlight_start);
        if length == 0 || start.saturating_add(length) > self.text.chars().count() {
            return self.text.clone();
        }
        self.text.chars().skip(start).take(length).collect()
    }
}

#[derive(Debug, Default)]
pub(super) struct CollectedMessages {
    pub(super) build_finished: Option<bool>,
    pub(super) noise_lines: usize,
    pub(super) malformed_messages: usize,
    pub(super) messages: Vec<CapturedMessage>,
    pub(super) errors: Vec<InternalError>,
}

enum Record {
    Line { bytes: Vec<u8>, truncated: bool },
    End,
}

#[cfg(test)]
pub(super) fn collect(reader: impl BufRead) -> CollectedMessages {
    collect_observed(reader, &mut |_| {})
}

pub(super) fn collect_observed(
    reader: impl BufRead,
    observe: &mut dyn FnMut(&Message),
) -> CollectedMessages {
    let mut collected = CollectedMessages::default();
    let mut reader = reader;

    loop {
        match read_record(&mut reader) {
            Ok(Record::End) => break,
            Ok(Record::Line {
                truncated: true, ..
            }) => collected.malformed_messages += 1,
            Ok(Record::Line { bytes, .. }) => {
                let Ok(record) = str::from_utf8(&bytes) else {
                    collected.malformed_messages += 1;
                    continue;
                };
                capture_record(record, &mut collected, observe);
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

fn read_record(reader: &mut impl BufRead) -> std::io::Result<Record> {
    let mut bytes = Vec::new();
    let read = (&mut *reader)
        .take(LINE_MAX_BYTES as u64)
        .read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Ok(Record::End);
    }
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

fn capture_record(
    record: &str,
    collected: &mut CollectedMessages,
    observe: &mut dyn FnMut(&Message),
) {
    let normalized = record.trim_start_matches(|character: char| character.is_ascii_whitespace());
    if normalized.is_empty() {
        if !record.is_empty() {
            collected.noise_lines += 1;
        }
    } else if normalized.starts_with('{') {
        capture_json_line(normalized, collected, observe);
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

fn capture_json_line(
    line: &str,
    collected: &mut CollectedMessages,
    observe: &mut dyn FnMut(&Message),
) {
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
            collected.messages.push(CapturedMessage::Unknown);
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
            observe(&message);
            collected
                .messages
                .push(CapturedMessage::Known(Box::new(message)));
        }
        Err(_) => collected.malformed_messages += 1,
    }
}

#[cfg(test)]
mod tests;
