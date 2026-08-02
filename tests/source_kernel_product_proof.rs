#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rust_doctor::{Diagnostic, DiagnosticSource, InspectReport, InspectRequest, Status, inspect};
use serde_json::{Value, json};

const TLS: &str = "rust_doctor::source::disabled_tls_verification";
const SHELL: &str = "rust_doctor::source::dynamic_shell_command";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/source-kernel")
        .join(name)
}

fn content_hashes(root: &Path) -> BTreeMap<String, blake3::Hash> {
    fn visit(root: &Path, directory: &Path, hashes: &mut BTreeMap<String, blake3::Hash>) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                visit(root, &path, hashes);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                hashes.insert(relative, blake3::hash(&fs::read(path).unwrap()));
            }
        }
    }

    let mut hashes = BTreeMap::new();
    visit(root, root, &mut hashes);
    hashes
}

fn source_findings(report: &InspectReport) -> Vec<&Diagnostic> {
    report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.source == DiagnosticSource::RustDoctor
                && matches!(diagnostic.code.as_deref(), Some(TLS | SHELL))
        })
        .collect()
}

#[test]
fn native_source_findings_join_the_existing_v3_report() {
    let root = fixture("precision");
    let before = content_hashes(&root);
    let report = inspect(InspectRequest::new(&root));
    let after = content_hashes(&root);

    assert_eq!(before, after);
    assert_eq!(report.schema_version, 8);
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    assert!(report.complete);
    assert_eq!(report.exit_code(), 0);
    assert_eq!(
        report.scan.command.as_deref(),
        Some(
            [
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--no-deps",
                "--message-format=json",
                "--",
                "-W",
                "clippy::dbg_macro",
                "-W",
                "clippy::mem_forget",
                "-W",
                "clippy::non_send_fields_in_send_ty",
                "-W",
                "clippy::permissions_set_readonly_false",
                "-W",
                "clippy::suspicious_command_arg_space",
                "-W",
                "clippy::todo",
                "-W",
                "clippy::unimplemented",
                "-W",
                "clippy::zombie_processes",
            ]
            .map(str::to_owned)
            .as_slice()
        )
    );

    let native = source_findings(&report);
    assert_eq!(native.len(), 10, "{:?}", report.diagnostics);
    assert_eq!(
        native
            .iter()
            .filter(|diagnostic| diagnostic.code.as_deref() == Some(TLS))
            .count(),
        5
    );
    assert_eq!(
        native
            .iter()
            .filter(|diagnostic| diagnostic.code.as_deref() == Some(SHELL))
            .count(),
        5
    );
    assert!(native.iter().all(|diagnostic| {
        diagnostic.id.len() == 64
            && diagnostic.occurrences == 1
            && diagnostic.path.is_some()
            && diagnostic.span.is_some()
            && diagnostic.category.as_deref() == Some("security")
    }));
    let shared = native
        .iter()
        .find(|diagnostic| diagnostic.path.as_deref() == Some("app/src/shared.rs"))
        .unwrap();
    assert_eq!(shared.package.as_deref(), Some("source-kernel-app"));
    assert_eq!(shared.target, None);
}

#[test]
fn adversarial_matrix_matches_exact_positive_spans_and_closed_negative_cases() {
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/source-kernel/precision/oracle.json")).unwrap();
    let precision = inspect(InspectRequest::new(fixture("precision")));
    let errors = inspect(InspectRequest::new(fixture("errors")));

    let actual: Vec<_> = source_findings(&precision)
        .into_iter()
        .map(|diagnostic| {
            let span = diagnostic.span.as_ref().unwrap();
            json!({
                "code": diagnostic.code,
                "path": diagnostic.path,
                "span": {
                    "line_start": span.line_start,
                    "column_start": span.column_start,
                    "line_end": span.line_end,
                    "column_end": span.column_end,
                }
            })
        })
        .collect();
    assert_eq!(json!(actual), oracle["findings"]);

    for code in [TLS, SHELL] {
        let cases = oracle["negative_cases"][code].as_array().unwrap();
        assert!(cases.len() >= 12);
        for case in cases {
            let workspace = case["workspace"].as_str().unwrap();
            let path = case["path"].as_str().unwrap();
            let symbol = case["symbol"].as_str().unwrap();
            let source = fs::read_to_string(fixture(workspace).join(path)).unwrap();
            assert!(
                source.contains(&format!("fn {symbol}")),
                "missing {code} case {case}"
            );
            let report = if workspace == "precision" {
                &precision
            } else {
                &errors
            };
            assert!(
                report.diagnostics.iter().all(|diagnostic| {
                    diagnostic.code.as_deref() != Some(code)
                        || diagnostic.path.as_deref() != Some(path)
                }),
                "negative case emitted a finding: {case}"
            );
        }
    }

    let rendered = serde_json::to_string(&precision).unwrap();
    for forbidden in [
        "RD_SOURCE_PAYLOAD_8f9c",
        "RD_SOURCE_URL_8f9c",
        "RD_SOURCE_CREDENTIAL_8f9c",
        "RD_SOURCE_PATH_8f9c",
        "RD_SOURCE_ANSI_8f9c",
        "\u{1b}",
    ] {
        assert!(!rendered.contains(forbidden), "report leaked {forbidden:?}");
    }
}

#[test]
fn source_errors_make_a_started_scan_incomplete_without_losing_valid_findings() {
    let report = inspect(InspectRequest::new(fixture("errors")));

    assert_eq!(report.status, Status::Incomplete);
    assert!(!report.complete);
    assert_eq!(report.exit_code(), 1);
    let source_error_codes: Vec<_> = report
        .errors
        .iter()
        .filter(|error| error.stage == "source")
        .map(|error| error.code.as_str())
        .collect();
    assert_eq!(
        source_error_codes,
        [
            "module-ambiguous",
            "module-not-found",
            "parse-error",
            "parse-error",
            "path-outside-workspace",
            "path-outside-workspace",
        ]
    );
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_deref() == Some(SHELL))
            .count(),
        2
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code.as_deref() != Some(TLS))
    );
    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains(env!("CARGO_MANIFEST_DIR")));
    assert!(!rendered.contains("secret"));
    assert!(!rendered.contains("echo {user}"));
}

#[test]
fn failures_before_scan_execution_do_not_run_the_source_kernel() {
    let report = inspect(InspectRequest::new(fixture("does-not-exist")));

    assert_eq!(report.status, Status::Failed);
    assert_eq!(report.exit_code(), 2);
    assert!(report.diagnostics.is_empty());
    assert!(report.errors.iter().all(|error| error.stage != "source"));
}
