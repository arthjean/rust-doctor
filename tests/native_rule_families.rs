#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! The source rule families Clippy cannot express, scanned through the
//! `inspect` a user runs: a blocking call inside async code, SQL text
//! assembled from a runtime value, and a shell command line built from one. The fixture stubs the crates it calls
//! (`tokio`, `reqwest`, `sqlx`, `diesel`, `rusqlite`, `postgres`,
//! `tokio-postgres`, `sea-orm`) with their surface and none of their behavior, so the
//! scan compiles offline and the detectors see the paths real code writes.

use std::fs;
use std::path::{Path, PathBuf};

use rust_doctor::{DiagnosticContext, InspectRequest, Status, inspect};
use serde_json::{Value, json};

const BLOCKING: &str = "rust_doctor::source::blocking_call_in_async";
const SQL: &str = "rust_doctor::source::sql_built_from_format";
const SHELL: &str = "rust_doctor::source::dynamic_shell_command";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/source-kernel/native-families")
}

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/source-kernel/native-families/oracle.json")).unwrap()
}

#[test]
fn both_families_report_exactly_their_positive_sites() {
    let report = inspect(InspectRequest::new(fixture()));
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);

    let observed: Vec<Value> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic.code.as_deref(), Some(BLOCKING | SQL | SHELL)))
        .map(|diagnostic| {
            let span = diagnostic.span.as_ref().unwrap();
            let mut finding = json!({
                "code": diagnostic.code,
                "path": diagnostic.path,
                "span": {
                    "line_start": span.line_start,
                    "column_start": span.column_start,
                    "line_end": span.line_end,
                    "column_end": span.column_end,
                },
            });
            if let Some(context) = diagnostic.context {
                finding["context"] = serde_json::to_value(context).unwrap();
            }
            finding
        })
        .collect();
    assert_eq!(json!(observed), oracle()["findings"]);

    let oracle = oracle();
    for (code, minimum) in [(BLOCKING, 9), (SQL, 9), (SHELL, 5)] {
        let cases = oracle["negative_cases"][code].as_array().unwrap();
        assert!(cases.len() >= minimum, "{code} records too few silences");
        for case in cases {
            let path = case["path"].as_str().unwrap();
            let symbol = case["symbol"].as_str().unwrap();
            let source = fs::read_to_string(fixture().join(path)).unwrap();
            assert!(source.contains(&format!("fn {symbol}")), "missing {code} case {symbol}");
            assert!(
                report.diagnostics.iter().all(|diagnostic| {
                    diagnostic.code.as_deref() != Some(code) || diagnostic.path.as_deref() != Some(path)
                }),
                "a negative case of {code} emitted a finding: {symbol}"
            );
        }
    }
}

/// A blocking call a test makes is reported, and carries the context that
/// keeps it from weighing: in a test target by Cargo's target kind, and in a
/// `#[cfg(test)]` module of a shipped file by the attribute.
#[test]
fn a_blocking_call_in_test_code_carries_the_test_context() {
    let report = inspect(InspectRequest::new(fixture()));
    let blocking: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.as_deref() == Some(BLOCKING))
        .collect();

    let in_tests: Vec<_> = blocking
        .iter()
        .filter(|diagnostic| diagnostic.context.is_some())
        .map(|diagnostic| (diagnostic.path.as_deref().unwrap(), diagnostic.context))
        .collect();
    assert_eq!(
        in_tests,
        [
            ("service/src/blocking.rs", Some(DiagnosticContext::Tests)),
            ("service/tests/blocking.rs", Some(DiagnosticContext::Tests)),
        ]
    );
    assert_eq!(blocking.len() - in_tests.len(), 19);
}

#[test]
fn both_rules_publish_their_category_tier_and_the_fix() {
    let report = inspect(InspectRequest::new(fixture()));
    let published = serde_json::to_value(&report).unwrap();
    let rule = |id: &str| {
        published["policy"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rule| rule["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(rule(BLOCKING)["category"], "reliability");
    assert_eq!(rule(BLOCKING)["tier"], "P2");
    assert_eq!(rule(SQL)["category"], "security");
    assert_eq!(rule(SQL)["tier"], "P1");

    let help = |id: &str| {
        report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code.as_deref() == Some(id))
            .and_then(|diagnostic| diagnostic.help.clone())
            .unwrap()
    };
    assert!(help(BLOCKING).contains("synchronous helper of the same file"), "{}", help(BLOCKING));
    for fix in [".bind()", "query!"] {
        assert!(help(SQL).contains(fix), "{}", help(SQL));
    }
}

/// US-017: the call in async code to a synchronous helper carries the finding,
/// and its message names the helper and the blocking call it reaches.
#[test]
fn a_helper_finding_names_the_helper_and_what_it_reaches() {
    let report = inspect(InspectRequest::new(fixture()));
    let messages: Vec<&str> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code.as_deref() == Some(BLOCKING)
                && diagnostic.path.as_deref() == Some("service/src/blocking_reach.rs")
        })
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert!(
        messages.iter().any(|message| message.contains("`synchronous_helper`") && message.contains("`std::fs::read`")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|message| message.contains("`inner`") && message.contains("`std::thread::sleep`")),
        "{messages:?}"
    );
}
