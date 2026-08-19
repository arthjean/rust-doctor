//! Tests of the stream reader, in a file of their own so that both halves of
//! the module stay under the size bound `oversized_unit` reports at.

use std::io::Cursor;

use super::*;

const FINISHED: &str = "{\"reason\":\"build-finished\",\"success\":true}\n";

const COMPILER_MESSAGE: &str = concat!(
    "{\"reason\":\"compiler-message\",",
    "\"package_id\":\"path+file:///project#example@0.1.0\",",
    "\"target\":{\"name\":\"example\"},",
    "\"message\":{\"message\":\"diagnostic\",",
    "\"code\":null,\"level\":\"warning\",\"spans\":[]}}\n",
);

/// What the reader answered, in the four counts a line is judged on.
#[derive(Debug, PartialEq, Eq)]
struct Counts {
    noise: usize,
    malformed: usize,
    messages: usize,
    build_finished: Option<bool>,
}

fn counts(input: impl Into<Vec<u8>>) -> Counts {
    let collected = collect(Cursor::new(input.into()));
    assert!(collected.errors.is_empty(), "{:?}", collected.errors);
    Counts {
        noise: collected.noise_lines,
        malformed: collected.malformed_messages,
        messages: collected.messages.len(),
        build_finished: collected.build_finished,
    }
}

/// Every kind of line the stream can carry, and the count it lands in.
///
/// One table rather than one test each: the cases it replaced were six copies
/// of the same four assertions, which is exactly what this crate's own
/// `duplicate_function_body` rule reports on.
#[test]
fn every_kind_of_line_lands_in_its_own_count() {
    let whitespace_prefixed: String = [' ', '\t', '\n', '\r', '\u{000c}']
        .into_iter()
        .map(|prefix| format!("{prefix}{FINISHED}"))
        .collect();
    let oversized = format!("{}\n{FINISHED}", "{".repeat(LINE_MAX_BYTES + 4_096));
    let mut not_utf8 = b"\xff\xfe not utf-8\n".to_vec();
    not_utf8.extend_from_slice(FINISHED.as_bytes());

    let cases: [(&str, Vec<u8>, Counts); 6] = [
        (
            "ASCII whitespace before an object is not part of the record",
            whitespace_prefixed.into_bytes(),
            Counts {
                noise: 0,
                malformed: 0,
                messages: 5,
                build_finished: Some(true),
            },
        ),
        (
            "an empty record is nothing, a whitespace-only one is noise",
            format!("\n \t\u{000c}\r\n{FINISHED}").into_bytes(),
            Counts {
                noise: 1,
                malformed: 0,
                messages: 1,
                build_finished: Some(true),
            },
        ),
        (
            "a third-party prefix contaminates its own line and no neighbor",
            format!("{COMPILER_MESSAGE}third-party prefix{COMPILER_MESSAGE}{FINISHED}")
                .into_bytes(),
            Counts {
                noise: 0,
                malformed: 1,
                messages: 2,
                build_finished: Some(true),
            },
        ),
        (
            "a last line with no newline is a whole line, never a cut one",
            FINISHED.trim_end().as_bytes().to_vec(),
            Counts {
                noise: 0,
                malformed: 0,
                messages: 1,
                build_finished: Some(true),
            },
        ),
        (
            "a line past the budget is dropped without costing the stream its tail",
            oversized.into_bytes(),
            Counts {
                noise: 0,
                malformed: 1,
                messages: 1,
                build_finished: Some(true),
            },
        ),
        (
            "a line that is not UTF-8 stops at itself, not at the stream",
            not_utf8,
            Counts {
                noise: 0,
                malformed: 1,
                messages: 1,
                build_finished: Some(true),
            },
        ),
    ];

    for (why, input, expected) in cases {
        assert_eq!(counts(input), expected, "{why}");
    }
}

/// A reason the toolchain has not published yet is kept whole rather than
/// dropped, and a truncated object is not.
#[test]
fn a_future_reason_is_kept_and_a_cut_object_is_not() {
    let input = format!(
        "third-party output\n{FINISHED}\
         {{\"reason\":\"future-message\",\"value\":1}}\n\
         {{\"reason\":\n"
    );
    let collected = collect(Cursor::new(input));

    assert_eq!(collected.noise_lines, 1);
    assert_eq!(collected.malformed_messages, 1);
    assert_eq!(collected.messages.len(), 2);
    assert!(matches!(
        &collected.messages[0],
        CapturedMessage::Known(message)
            if matches!(message.as_ref(), Message::BuildFinished(_))
    ));
    assert!(matches!(collected.messages[1], CapturedMessage::Unknown));
}

/// A severity the toolchain has not published yet reaches the report as it was
/// written: the reader classifies the record, never the diagnostic.
#[test]
fn a_future_diagnostic_severity_is_preserved() {
    let input = concat!(
        "{\"reason\":\"compiler-message\",",
        "\"package_id\":\"path+file:///project#example@0.1.0\",",
        "\"target\":{\"name\":\"example\"},",
        "\"message\":{\"message\":\"future diagnostic\",",
        "\"code\":null,\"level\":\"future-level\",\"spans\":[]}}\n",
    );
    let collected = collect(Cursor::new(input));

    assert_eq!(collected.malformed_messages, 0);
    assert!(matches!(
        &collected.messages[0],
        CapturedMessage::Compiler(message) if message.message.level == "future-level"
    ));
}

/// A workspace whose procedural macros emit without end stops the scan at a
/// published bound and says so, rather than growing until the allocator gives
/// up. The error lands at the `parsing` stage, which is what drops the
/// authoritative flag.
#[test]
fn a_flood_of_messages_stops_at_the_published_bound() {
    let collected = collect(Cursor::new(COMPILER_MESSAGE.repeat(MESSAGE_MAX_COUNT + 16)));

    assert_eq!(collected.messages.len(), MESSAGE_MAX_COUNT);
    assert_eq!(
        collected
            .errors
            .iter()
            .map(|error| (error.stage, error.code))
            .collect::<Vec<_>>(),
        [("parsing", "message-limit")]
    );
}
