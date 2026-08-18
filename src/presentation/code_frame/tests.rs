//! Tests of the code frame.
//!
//! They live in their own file for the reason the frame itself checks: the
//! crate passes its own `oversized_unit` rule, and a module carrying the
//! workspace check, the reader, the sanitizer and their tests in one file would
//! not. They also live here rather than beside the rest of the presentation,
//! which is what lets the reader and its bounds stay private to this module.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::DiagnosticSpan;

static TEMPORARY: AtomicUsize = AtomicUsize::new(0);

fn temporary_root(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/presentation-tests")
        .join(format!(
            "{}-{name}-{}",
            std::process::id(),
            TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn at(path: &str, line: usize, column_start: usize, column_end: usize) -> GroupLocation {
    GroupLocation {
        path: path.to_owned(),
        span: DiagnosticSpan {
            line_start: line,
            column_start,
            line_end: line,
            column_end,
        },
    }
}

fn primary(frame: &CodeFrame) -> &CodeFrameLine {
    frame
        .lines
        .iter()
        .find(|line| line.primary)
        .expect("a frame always carries its reported line")
}

/// The frame passes the rule the report publishes. `oversized_unit` reports a
/// file at a thousand lines, and this module holds that bound: these tests have
/// a file of their own for that reason, and a file that grows back past it
/// fails here rather than on a self-scan nobody reads.
#[test]
fn the_frame_holds_the_size_bound_the_report_reports_for() {
    for own in [include_str!("../code_frame.rs"), include_str!("tests.rs")] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the code frame is {lines} lines long, over the {} it reports",
            crate::structure::FILE_LINES
        );
    }
}

#[test]
fn frame_is_bounded_marks_primary_location_and_neutralizes_controls() {
    let root = temporary_root("bounded");
    fs::create_dir_all(root.join("src")).unwrap();
    let long = "x".repeat(200);
    fs::write(
        root.join("src/lib.rs"),
        format!("one\ntwo\n\tlet value = 1;\x1b]52;secret\u{009b}31m{long}\nfour\nfive\nsix\n"),
    )
    .unwrap();
    let location = at("src/lib.rs", 3, 5, 8);

    let frame = code_frame(&root, &location).unwrap();
    assert_eq!(frame.location, "src/lib.rs:3:5");
    assert!(frame.lines.len() <= FRAME_MAX_LINES);
    assert_eq!(frame.lines.iter().filter(|line| line.primary).count(), 1);
    assert_eq!(
        primary(&frame).marker,
        Some(CodeFrameMarker {
            column_start: 8,
            column_end: 11,
        })
    );
    assert!(
        frame
            .lines
            .iter()
            .all(|line| line.text.len() <= FRAME_MAX_COLUMNS)
    );
    let rendered = serde_json::to_string(&frame).unwrap();
    assert!(!rendered.contains("52;secret"));
    assert!(!rendered.contains('\u{001b}'));
    assert!(!rendered.contains('\u{009b}'));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn frame_resolves_normalized_paths_and_maps_source_to_terminal_columns() {
    let root = temporary_root("normalized-path");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/100%.rs"),
        format!("\t界e\u{301}{}\n", "界".repeat(100)),
    )
    .unwrap();
    let location = at("src/100%25.rs", 1, 5, 6);

    let frame = code_frame(&root, &location).unwrap();
    let line = &frame.lines[0];
    assert_eq!(frame.location, "src/100%25.rs:1:5");
    assert_eq!(
        line.marker,
        Some(CodeFrameMarker {
            column_start: 21,
            column_end: 29,
        })
    );
    assert!(line.text.len() <= FRAME_MAX_COLUMNS);
    assert!(line.text.matches("\\u{754C}").count() < 100);

    let raw_path = at("src/100%.rs", 1, 5, 6);
    assert_eq!(
        code_frame(&root, &raw_path).unwrap_err().reason,
        CodeFrameUnavailableReason::OutsideWorkspace
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hostile_paths_binary_and_invalid_utf8_never_render_source() {
    let root = temporary_root("hostile");
    fs::write(root.join("binary.rs"), b"safe\0secret").unwrap();
    fs::write(root.join("invalid.rs"), [0xff, 0xfe]).unwrap();
    for (path, reason) in [
        ("../secret.rs", CodeFrameUnavailableReason::OutsideWorkspace),
        (
            "/private/secret.rs",
            CodeFrameUnavailableReason::OutsideWorkspace,
        ),
        ("binary.rs", CodeFrameUnavailableReason::Binary),
        ("invalid.rs", CodeFrameUnavailableReason::InvalidUtf8),
    ] {
        let error = code_frame(&root, &at(path, 1, 1, 2)).unwrap_err();
        assert_eq!(error.reason, reason, "{path}");
        assert!(error.message.len() < 1024);
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn symlink_escape_and_replacement_between_validation_and_open_are_rejected() {
    use std::os::unix::fs::symlink;

    let root = temporary_root("race-root");
    let outside = temporary_root("race-outside");
    fs::write(outside.join("secret.rs"), "TOP_SECRET\n").unwrap();
    symlink(outside.join("secret.rs"), root.join("escape.rs")).unwrap();
    assert_eq!(
        code_frame(&root, &at("escape.rs", 1, 1, 2))
            .unwrap_err()
            .reason,
        CodeFrameUnavailableReason::OutsideWorkspace
    );

    fs::write(root.join("replace.rs"), "safe\n").unwrap();
    let error = read_code_frame(&root, &at("replace.rs", 1, 1, 2), || {
        fs::remove_file(root.join("replace.rs")).unwrap();
        symlink(outside.join("secret.rs"), root.join("replace.rs")).unwrap();
    })
    .unwrap_err();
    assert_eq!(error.reason, CodeFrameUnavailableReason::OutsideWorkspace);

    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

/// The reader walks lines rather than a byte prefix, so a finding deep in a
/// file is framed exactly like one at its top. A cap on the bytes decoded left
/// everything past the first kilobytes of a file with no frame at all, which is
/// most of a file the crate's own `oversized_unit` rule reports on.
#[test]
fn a_line_past_any_byte_prefix_is_still_framed() {
    let root = temporary_root("deep-line");
    fs::create_dir_all(root.join("src")).unwrap();
    let mut source = String::new();
    for number in 1..=1005 {
        if number == 1000 {
            source.push_str("    let reported = deep_in_the_file();\n");
        } else {
            source.push_str("    // filler carrying this file well past any byte prefix\n");
        }
    }
    let path = root.join("src/deep.rs");
    fs::write(&path, &source).unwrap();
    assert!(
        fs::metadata(&path).unwrap().len() > 8 * 1024,
        "the fixture has to be larger than the byte cap it is a regression for"
    );

    let frame = code_frame(&root, &at("src/deep.rs", 1000, 9, 17)).unwrap();
    assert_eq!(
        frame
            .lines
            .iter()
            .map(|line| line.number)
            .collect::<Vec<_>>(),
        vec![998, 999, 1000, 1001, 1002]
    );
    let reported = primary(&frame);
    assert_eq!(reported.number, 1000);
    assert_eq!(reported.text, "    let reported = deep_in_the_file();");
    assert_eq!(
        reported.marker,
        Some(CodeFrameMarker {
            column_start: 9,
            column_end: 17,
        })
    );
    fs::remove_dir_all(root).unwrap();
}

/// Decoding only the lines that are kept is what keeps a character straddling a
/// bound from making a valid file look like it is not UTF-8. Under the byte
/// prefix this replaced, one accented character at the wrong offset cost the
/// file every frame it had, including the ones on its first line.
#[test]
fn a_character_straddling_a_read_bound_leaves_a_valid_file_readable() {
    let root = temporary_root("straddle");
    fs::create_dir_all(root.join("src")).unwrap();
    let mut source = String::from("    let reported = 1;\n");
    let filler = "    // padding up to the boundary a byte prefix used to cut\n";
    let opener = "    // ";
    while source.len() + filler.len() + opener.len() <= 8191 {
        source.push_str(filler);
    }
    let padding = 8191 - source.len() - opener.len();
    source.push_str(opener);
    source.push_str(&"y".repeat(padding));
    assert_eq!(source.len(), 8191);
    source.push_str("é and the rest of the comment\n");
    fs::write(root.join("src/accent.rs"), &source).unwrap();
    assert!(std::str::from_utf8(source.as_bytes()).is_ok());

    let frame = code_frame(&root, &at("src/accent.rs", 1, 9, 17)).unwrap();
    assert_eq!(primary(&frame).text, "    let reported = 1;");
    fs::remove_dir_all(root).unwrap();
}

/// The line bound is applied on a character boundary, so cutting a very long
/// line never turns it into a decoding failure either.
#[test]
fn a_line_cut_at_its_own_bound_is_still_decoded() {
    let root = temporary_root("long-line");
    fs::create_dir_all(root.join("src")).unwrap();
    let mut line = "z".repeat(LINE_MAX_BYTES - 1);
    line.push('é');
    line.push_str(&"z".repeat(64));
    fs::write(root.join("src/long.rs"), format!("{line}\n")).unwrap();

    let frame = code_frame(&root, &at("src/long.rs", 1, 1, 2)).unwrap();
    let reported = primary(&frame);
    assert_eq!(reported.text, "z".repeat(FRAME_MAX_COLUMNS));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_window_never_leaves_the_file_it_frames() {
    let root = temporary_root("window-edges");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/short.rs"), "one\ntwo\nthree\n").unwrap();

    for (line, expected) in [(1, vec![1, 2, 3]), (2, vec![1, 2, 3]), (3, vec![1, 2, 3])] {
        let frame = code_frame(&root, &at("src/short.rs", line, 1, 2)).unwrap();
        assert_eq!(
            frame
                .lines
                .iter()
                .map(|line| line.number)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(primary(&frame).number, line);
    }

    assert_eq!(
        code_frame(&root, &at("src/short.rs", 4, 1, 2))
            .unwrap_err()
            .reason,
        CodeFrameUnavailableReason::Unavailable
    );
    fs::remove_dir_all(root).unwrap();
}

/// The caret stays inside the text the frame shows. A span pointing past what
/// the sanitizer had room for stops where the text stops rather than at the
/// frame's own edge, where it would hang in empty columns.
#[test]
fn a_marker_never_points_past_the_text_the_frame_shows() {
    let root = temporary_root("marker-bound");
    fs::create_dir_all(root.join("src")).unwrap();
    // The emoji sanitizes to nine columns and would overflow, so the rendered
    // line stops five columns short of the frame's width.
    let source = format!("{}\u{1F469}{}", "a".repeat(155), "b".repeat(40));
    fs::write(root.join("src/wide.rs"), format!("{source}\n")).unwrap();

    let frame = code_frame(&root, &at("src/wide.rs", 1, 156, 190)).unwrap();
    let reported = primary(&frame);
    let marker = reported.marker.unwrap();
    assert_eq!(reported.text.len(), 155);
    assert!(
        marker.column_start <= reported.text.len() + 1,
        "the caret starts at {} for {} columns of text",
        marker.column_start,
        reported.text.len()
    );
    assert!(
        marker.column_end <= reported.text.len() + 2,
        "the caret ends at {} for {} columns of text",
        marker.column_end,
        reported.text.len()
    );
    assert!(marker.column_end > marker.column_start);
    fs::remove_dir_all(root).unwrap();
}

/// A span that ends on a later line has no end column on the line the frame
/// shows. It marks to the end of what is shown rather than borrowing a column
/// from a line the reader cannot see.
#[test]
fn a_span_ending_on_a_later_line_marks_to_the_end_of_the_line_shown() {
    let root = temporary_root("multi-line-span");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/wide_span.rs"),
        "use std::fmt;\nfn wide_span() {\n    body();\n}\n",
    )
    .unwrap();
    let location = GroupLocation {
        path: "src/wide_span.rs".to_owned(),
        span: DiagnosticSpan {
            line_start: 2,
            column_start: 4,
            line_end: 4,
            column_end: 2,
        },
    };

    let frame = code_frame(&root, &location).unwrap();
    let reported = primary(&frame);
    assert_eq!(reported.text, "fn wide_span() {");
    assert_eq!(
        reported.marker,
        Some(CodeFrameMarker {
            column_start: 4,
            column_end: 17,
        })
    );
    fs::remove_dir_all(root).unwrap();
}

/// A line the scan budget cut short is dropped rather than shown. The frame
/// would otherwise render a fragment as if it were the source, and the reader
/// has no way to tell one from the other.
#[test]
fn a_line_the_scan_budget_cut_short_is_never_shown() {
    let root = temporary_root("scan-budget");
    fs::create_dir_all(root.join("src")).unwrap();
    let path = root.join("src/budget.rs");
    fs::write(&path, "one\ntwo\nthree\n").unwrap();

    // Seven bytes reach the end of "one\n" and stop three bytes into "two\n".
    let cut = read_window(File::open(&path).unwrap(), 1, 7).unwrap();
    assert_eq!(cut, vec!["one".to_owned()]);

    // The same read with the budget landing on a line terminator keeps the line.
    let whole = read_window(File::open(&path).unwrap(), 1, 8).unwrap();
    assert_eq!(whole, vec!["one".to_owned(), "two".to_owned()]);

    fs::remove_dir_all(root).unwrap();
}

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
