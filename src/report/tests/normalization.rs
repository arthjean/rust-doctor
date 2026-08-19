//! Tests for what a producer's finding becomes once it is a diagnostic:
//! identity, severity, spans, ordering, deduplication and everything the
//! published text has taken out of it.
//!
//! The fixtures come from the parent module, which carries the pipeline half.

use std::path::Path;

use serde_json::Value;

use super::super::normalize::*;
use super::super::sanitize::*;
use super::super::{DiagnosticSource, DiagnosticSpan, Severity, Status};
use super::*;
use crate::execution::ScanExecution;
use crate::internal_error::InternalError;

#[test]
fn normalizes_text_paths_severity_and_deduplicates() {
    let workspace = fixture("clean").canonicalize().unwrap();
    let source = workspace.join("src/lib.rs");
    let raw_message = format!(
        "\u{1b}[31mmessage {} /home/person  \r\nnext\t\r",
        workspace.display()
    );
    let home = HomePaths {
        lexical: Some("/home/person".to_owned()),
        canonical: None,
    };
    let messages = vec![
        compiler_message(
            Some("clippy::needless_return"),
            "warning",
            &raw_message,
            source.to_str().unwrap(),
            3,
        ),
        compiler_message(
            Some("clippy::needless_return"),
            "warning",
            &raw_message,
            source.to_str().unwrap(),
            3,
        ),
    ];
    let diagnostics = normalize_diagnostics(&messages, Some(&workspace), None, &home);

    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.source, DiagnosticSource::Clippy);
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(diagnostic.category, None);
    assert_eq!(diagnostic.message, "message . <home>\nnext\n");
    assert_eq!(diagnostic.help, None);
    assert_eq!(diagnostic.path.as_deref(), Some("src/lib.rs"));
    assert_eq!(diagnostic.occurrences, 2);
    assert_eq!(diagnostic.id.len(), 64);
    let tuple = serde_json::to_vec(&(
        diagnostic.source.as_str(),
        diagnostic.code.as_deref(),
        diagnostic.path.as_deref(),
        diagnostic.span.as_ref(),
        diagnostic.severity.as_str(),
        diagnostic.message.as_str(),
    ))
    .unwrap();
    assert_eq!(diagnostic.id, blake3::hash(&tuple).to_hex().to_string());
}

#[test]
fn external_paths_are_null_and_future_severity_is_unknown() {
    let messages = vec![compiler_message(
        None,
        "future-level",
        "future",
        "/outside/project/lib.rs",
        1,
    )];
    let workspace = fixture("clean").canonicalize().unwrap();
    let diagnostics =
        normalize_diagnostics(&messages, Some(&workspace), None, &HomePaths::default());

    assert_eq!(diagnostics[0].path, None);
    assert_eq!(diagnostics[0].severity, Severity::Unknown);
    assert_eq!(diagnostics[0].category, None);
    assert_eq!(diagnostics[0].help, None);
    assert!(diagnostics[0].span.is_some());
}

#[test]
fn exact_curated_codes_gain_metadata_without_restamping_severity() {
    let workspace = fixture("clean").canonicalize().unwrap();
    let messages = [
        compiler_message(
            Some("clippy::todo"),
            "error",
            "toolchain-owned message",
            "src/lib.rs",
            2,
        ),
        compiler_message(
            Some("clippy::todo_suffix"),
            "warning",
            "similar code",
            "src/lib.rs",
            3,
        ),
    ];
    let diagnostics =
        normalize_diagnostics(&messages, Some(&workspace), None, &HomePaths::default());

    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(diagnostics[0].category.as_deref(), Some("correctness"));
    assert_eq!(
        diagnostics[0].help.as_deref(),
        Some(
            "Replace todo! with the intended implementation or remove the reachable placeholder."
        )
    );
    assert_eq!(diagnostics[1].category, None);
    assert_eq!(diagnostics[1].help, None);
}

#[test]
fn code_normalization_cannot_turn_a_non_exact_code_into_a_curated_match() {
    let workspace = fixture("clean").canonicalize().unwrap();
    let diagnostics = normalize_diagnostics(
        &[compiler_message(
            Some("\u{1b}[31mclippy::todo\u{1b}[0m"),
            "warning",
            "similar code",
            "src/lib.rs",
            2,
        )],
        Some(&workspace),
        None,
        &HomePaths::default(),
    );

    assert_eq!(diagnostics[0].code.as_deref(), Some("clippy::todo"));
    assert_eq!(diagnostics[0].category, None);
    assert_eq!(diagnostics[0].help, None);
}

#[test]
fn missing_code_or_primary_span_does_not_invent_structured_fields() {
    let workspace = fixture("clean").canonicalize().unwrap();
    let mut without_span =
        compiler_message(Some("clippy::todo"), "warning", "todo", "src/lib.rs", 2);
    if let CapturedMessage::Compiler(message) = &mut without_span {
        message.message.spans.clear();
    }
    let diagnostics = normalize_diagnostics(
        &[
            compiler_message(None, "warning", "todo", "src/lib.rs", 1),
            without_span,
        ],
        Some(&workspace),
        None,
        &HomePaths::default(),
    );

    assert_eq!(diagnostics[0].code, None);
    assert_eq!(diagnostics[0].category, None);
    assert_eq!(diagnostics[0].help, None);
    assert!(diagnostics[1].path.is_none());
    assert!(diagnostics[1].span.is_none());
    assert_eq!(diagnostics[1].category.as_deref(), Some("correctness"));
}

#[test]
fn editorial_metadata_is_not_part_of_the_v1_fingerprint_tuple() {
    let identity = (
        DiagnosticSource::Clippy,
        Some("clippy::todo"),
        Some("src/lib.rs"),
        Some(DiagnosticSpan {
            line_start: 2,
            column_start: 3,
            line_end: 2,
            column_end: 10,
        }),
        Severity::Warning,
        "toolchain-owned message",
    );
    let first = fingerprint(
        identity.0,
        identity.1,
        identity.2,
        identity.3.as_ref(),
        identity.4,
        identity.5,
    );
    let second = fingerprint(
        identity.0,
        identity.1,
        identity.2,
        identity.3.as_ref(),
        identity.4,
        identity.5,
    );
    let reports = [
        ("correctness", "first editorial help", first),
        ("maintainability", "second editorial help", second),
    ];

    assert_ne!(reports[0].0, reports[1].0);
    assert_ne!(reports[0].1, reports[1].1);
    assert_eq!(reports[0].2, reports[1].2);
}

#[test]
fn multiple_primary_spans_use_the_documented_canonical_order() {
    let mut message = match compiler_message(None, "error", "boom", "src/z.rs", 8) {
        CapturedMessage::Compiler(message) => message,
        _ => unreachable!(),
    };
    message.message.spans.push(CapturedSpan {
        file_name: "src/a.rs".to_owned(),
        line_start: 2,
        line_end: 2,
        column_start: 1,
        column_end: 3,
        is_primary: true,
    });
    let workspace = fixture("clean").canonicalize().unwrap();
    let diagnostics = normalize_diagnostics(
        &[CapturedMessage::Compiler(message)],
        Some(&workspace),
        None,
        &HomePaths::default(),
    );

    assert_eq!(diagnostics[0].path.as_deref(), Some("src/a.rs"));
    assert_eq!(
        diagnostics[0].span.as_ref().map(|span| span.line_start),
        Some(2)
    );
}

#[test]
fn duplicate_context_conflicts_are_arrival_order_independent() {
    let workspace = fixture("clean").canonicalize().unwrap();
    let home = HomePaths::default();
    let first = normalize_diagnostics(
        &[
            compiler_message_for_target("target-a"),
            compiler_message_for_target("target-b"),
        ],
        Some(&workspace),
        None,
        &home,
    );
    let reversed = normalize_diagnostics(
        &[
            compiler_message_for_target("target-b"),
            compiler_message_for_target("target-a"),
        ],
        Some(&workspace),
        None,
        &home,
    );

    assert_eq!(first, reversed);
    assert_eq!(first[0].target, None);
    assert_eq!(first[0].occurrences, 2);
}

#[test]
fn malformed_messages_make_a_started_scan_incomplete() {
    let result = ExecutionResult {
        manifest_path: None,
        metadata: None,
        structure: None,
        cargo_health: None,
        repo: None,
        toolchain: None,
        scan: ClippyExecution::Finished(ScanExecution {
            command: vec!["cargo".to_owned(), "clippy".to_owned()],
            exit_code: Some(0),
            exit_success: Some(true),
            build_finished: Some(true),
            noise_lines: 0,
            malformed_messages: 1,
            messages: Vec::new(),
            errors: Vec::new(),
        }),
        source: None,
        error: None,
    };
    let report = from_execution(result);

    assert_eq!(report.status, Status::Incomplete);
    assert_eq!(
        report
            .errors
            .iter()
            .map(|error| (
                error.stage.as_str(),
                error.code.as_str(),
                error.message.as_str()
            ))
            .collect::<Vec<_>>(),
        [("parsing", "malformed-message", "malformed Cargo message")]
    );
}

#[test]
fn incomplete_scan_reports_each_distinct_normative_cause_once() {
    let duplicate = InternalError {
        stage: "execution",
        code: "build-failed",
        message: "Cargo reported build-finished.success: false".to_owned(),
    };
    let result = ExecutionResult {
        manifest_path: None,
        metadata: None,
        structure: None,
        cargo_health: None,
        repo: None,
        toolchain: None,
        scan: ClippyExecution::Finished(ScanExecution {
            command: vec!["cargo".to_owned(), "clippy".to_owned()],
            exit_code: Some(101),
            exit_success: Some(false),
            build_finished: Some(false),
            noise_lines: 0,
            malformed_messages: 2,
            messages: Vec::new(),
            errors: vec![duplicate],
        }),
        source: None,
        error: None,
    };
    let report = from_execution(result);
    let errors: Vec<_> = report
        .errors
        .iter()
        .map(|error| {
            (
                error.stage.as_str(),
                error.code.as_str(),
                error.message.as_str(),
            )
        })
        .collect();

    assert_eq!(report.status, Status::Incomplete);
    assert_eq!(
        errors,
        [
            (
                "execution",
                "build-failed",
                "Cargo reported build-finished.success: false"
            ),
            ("execution", "clippy-exit", "Clippy exited with status 101"),
            ("parsing", "malformed-message", "malformed Cargo message"),
        ]
    );
}

#[test]
fn missing_exit_and_build_finished_have_explicit_causes() {
    let result = ExecutionResult {
        manifest_path: None,
        metadata: None,
        structure: None,
        cargo_health: None,
        repo: None,
        toolchain: None,
        scan: ClippyExecution::Finished(ScanExecution {
            command: vec!["cargo".to_owned(), "clippy".to_owned()],
            exit_code: None,
            exit_success: None,
            build_finished: None,
            noise_lines: 0,
            malformed_messages: 0,
            messages: Vec::new(),
            errors: Vec::new(),
        }),
        source: None,
        error: None,
    };
    let report = from_execution(result);

    assert_eq!(report.status, Status::Incomplete);
    assert_eq!(report.errors.len(), 2);
    assert!(report.errors.iter().any(|error| {
        error.code == "clippy-exit" && error.message == "Clippy terminated without an exit code"
    }));
    assert!(report.errors.iter().any(|error| {
        error.code == "build-finished-missing"
            && error.message == "Cargo did not emit build-finished"
    }));
}

/// US-011: a repository-pass failure degrades the scan, never aborts it.
/// The error surfaces at stage `repo`, the diagnostics of every other
/// producer stay published, and the incomplete status drops the
/// authoritative flag.
#[test]
fn repo_errors_surface_at_stage_repo_and_make_the_scan_incomplete() {
    let metadata = cargo_health_metadata(&[]);
    let path = metadata.workspace_root.join("src/lib.rs").to_string();
    let result = ExecutionResult {
        manifest_path: None,
        metadata: Some(metadata),
        structure: None,
        cargo_health: None,
        repo: Some(crate::repo_hygiene::RepoScan {
            findings: Vec::new(),
            errors: vec![crate::repo_hygiene::RepoError {
                code: "git-unavailable",
                message: "Repository hygiene skipped: git was not available.",
            }],
        }),
        toolchain: None,
        scan: ClippyExecution::Finished(scan(
            vec![compiler_message(
                Some("clippy::todo"),
                "warning",
                "todo survives the repo failure",
                &path,
                2,
            )],
            0,
            true,
            true,
        )),
        source: None,
        error: None,
    };
    let report = from_execution(result);

    assert_eq!(report.status, Status::Incomplete);
    assert!(report.errors.iter().any(|error| {
        error.stage == "repo"
            && error.code == "git-unavailable"
            && error.message == "Repository hygiene skipped: git was not available."
    }));
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].code.as_deref(), Some("clippy::todo"));
    assert!(!report.audit.score.as_ref().is_some_and(|score| score.authoritative));
}

#[test]
fn twenty_message_permutations_render_identically() {
    let base = [
        compiler_message(Some("E0001"), "error", "alpha", "src/e.rs", 7),
        compiler_message(Some("clippy::lint"), "warning", "beta", "src/b.rs", 3),
        compiler_message(None, "help", "gamma", "src/c.rs", 4),
        compiler_message(None, "future", "delta", "/external/d.rs", 1),
        compiler_message(None, "note", "epsilon", "src/a.rs", 2),
    ];
    let workspace = fixture("clean").canonicalize().unwrap();
    let mut expected = None;
    let mut order = [0, 1, 2, 3, 4];
    let mut seen = BTreeSet::new();

    for permutation in 0..20 {
        assert!(seen.insert(order));
        let messages: Vec<_> = order
            .iter()
            .map(|&index| clone_compiler_message(&base[index]))
            .collect();
        let diagnostics =
            normalize_diagnostics(&messages, Some(&workspace), None, &HomePaths::default());
        let mut rendered = Vec::new();
        crate::render::render_json(&report_with_diagnostics(diagnostics), &mut rendered)
            .unwrap();
        match expected.as_ref() {
            Some(expected) => assert_eq!(&rendered, expected),
            None => expected = Some(rendered),
        }
        if permutation < 19 {
            assert!(next_permutation(&mut order));
        }
    }
    assert_eq!(seen.len(), 20);
}

#[test]
fn sanitizes_workspace_and_both_home_forms_from_errors() {
    let home = HomePaths {
        lexical: Some("/linked/home".to_owned()),
        canonical: Some("/real/home".to_owned()),
    };
    let sanitized = sanitize_text(
        "\u{1b}[31m/work/project failed in /linked/home and /real/home \r\n",
        Some(Path::new("/work/project")),
        &home,
    );

    assert_eq!(sanitized, ". failed in <home> and <home>\n");
}

#[test]
fn lexical_home_is_redacted_when_canonicalization_fails() {
    let home =
        HomePaths::from_path(Some(PathBuf::from("/definitely/missing/rust-doctor-home")));

    assert!(home.canonical.is_none());
    assert_eq!(
        sanitize_text(
            "failed in /definitely/missing/rust-doctor-home/project",
            None,
            &home
        ),
        "failed in <home>/project"
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_lexical_home_display_is_redacted() {
    let path = PathBuf::from(OsString::from_vec(
        b"/definitely/missing/rust-doctor-\xff-home".to_vec(),
    ));
    let displayed = path.to_string_lossy().into_owned();
    let home = HomePaths::from_path(Some(path));

    assert_eq!(home.lexical.as_deref(), Some(displayed.as_str()));
    assert_eq!(
        sanitize_text(&format!("failed in {displayed}/project"), None, &home),
        "failed in <home>/project"
    );
}

#[test]
fn strips_ecma_48_control_sequences_and_payloads() {
    let value = concat!(
        "a\u{001b}[31mb\u{001b}[0mc",
        "\u{001b}]osc\u{0007}d",
        "\u{001b}Pdcs\u{001b}\\e",
        "\u{001b}Xsos\u{001b}\\f",
        "\u{001b}^pm\u{001b}\\g",
        "\u{001b}_apc\u{001b}\\h",
        "\u{001b}(Bi",
        "\u{009b}31mj",
        "\u{009d}osc\u{009c}k",
    );

    assert_eq!(normalize_text(value), "abcdefghijk");
}

#[test]
fn ecma_48_can_and_sub_cancel_sequences_without_consuming_following_text() {
    let value = concat!(
        "a\u{001b}[31\u{0018}b",
        "\u{001b}Pdiscarded\u{001a}c",
        "\u{009b}32\u{0018}d",
        "\u{009d}discarded\u{001a}e",
    );

    assert_eq!(normalize_text(value), "abcde");
}

#[test]
fn control_characters_in_internal_paths_are_encoded_before_rendering() {
    let workspace = fixture("clean").canonicalize().unwrap();
    let diagnostics = normalize_diagnostics(
        &[compiler_message(
            Some("clippy::lint"),
            "warning",
            "message",
            "src/100%\u{001b}[31mline\n.rs",
            1,
        )],
        Some(&workspace),
        None,
        &HomePaths::default(),
    );

    assert_eq!(
        diagnostics[0].path.as_deref(),
        Some("src/100%25%1B[31mline%0A.rs")
    );
    let report = report_with_diagnostics(diagnostics);
    let mut terminal = Vec::new();
    crate::render::render_terminal(&report, &mut terminal).unwrap();
    let terminal = String::from_utf8(terminal).unwrap();
    assert!(!terminal.contains('\u{001b}'));
    assert!(terminal.contains("src/100%25%1B[31mline%0A.rs:1:2"));

    let mut json = Vec::new();
    crate::render::render_json(&report, &mut json).unwrap();
    let json: Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(
        json["diagnostics"][0]["path"],
        "src/100%25%1B[31mline%0A.rs"
    );
}
