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
