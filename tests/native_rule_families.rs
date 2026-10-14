#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! The source rule families Clippy cannot express, scanned through the
//! `inspect` a user runs: a blocking call inside async code, SQL text
//! assembled from a runtime value, a shell command line built from one, the
//! tokio calls that panic, an unchecked HTTP status, a secret that falls back
//! to a literal, and the web controls switched off. The fixture stubs the
//! crates it calls (`tokio`, `tokio-macros`, `futures`, `reqwest`, `sqlx`,
//! `diesel`, `rusqlite`, `postgres`, `tokio-postgres`, `sea-orm`,
//! `tower-http`, `actix-cors`, `cookie`, `jsonwebtoken`) with their surface
//! and none of their behavior, so the scan compiles offline and the detectors
//! see the paths real code writes.

use std::fs;
use std::path::{Path, PathBuf};

use rust_doctor::{DiagnosticContext, InspectRequest, Status, inspect};
use serde_json::{Value, json};

const BLOCKING: &str = "rust_doctor::source::blocking_call_in_async";
const SQL: &str = "rust_doctor::source::sql_built_from_format";
const SHELL: &str = "rust_doctor::source::dynamic_shell_command";
/// Every native source rule this fixture holds a positive and a negative for,
/// with the fewest silences its oracle must record.
const SERVICE_RULES: [(&str, usize); 8] = [
    ("rust_doctor::source::block_in_place_on_current_thread", 3),
    ("rust_doctor::source::block_on_in_async", 2),
    ("rust_doctor::source::unchecked_http_status", 4),
    ("rust_doctor::source::secret_env_fallback", 5),
    ("rust_doctor::source::permissive_cors", 3),
    ("rust_doctor::source::insecure_auth_cookie", 2),
    ("rust_doctor::source::disabled_jwt_validation", 2),
    (BLOCKING, 9),
];

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
        .filter(|diagnostic| {
            diagnostic
                .code
                .as_deref()
                .is_some_and(|code| code.starts_with("rust_doctor::source::"))
        })
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
    for (code, minimum) in SERVICE_RULES.into_iter().chain([(SQL, 9), (SHELL, 5)]) {
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
    assert_eq!(blocking.len() - in_tests.len(), 20);
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

/// A rule is published as not evaluated when the workspace resolves a crate
/// it was never validated on, and reports nothing there: the fixture writes a
/// current-thread `block_in_place` against a tokio 0.2 stub, outside the
/// rule's `>=1, <2`. The two not-evaluated reasons never cost the score its
/// authority, since they say the rule had nothing to judge.
#[test]
fn a_crate_outside_the_validated_range_closes_the_rule_and_reports_nothing() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/source-kernel/version-gate");
    let report = inspect(InspectRequest::new(fixture));
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code.as_deref().is_none_or(|code| !code.starts_with("rust_doctor::source::"))),
        "a closed gate reported a finding"
    );
    let published = serde_json::to_value(&report).unwrap();
    let reason = |rule: &str| {
        published["policy"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == rule)
            .map(|entry| entry["not_evaluated"].clone())
            .unwrap()
    };
    assert_eq!(reason("rust_doctor::source::block_in_place_on_current_thread"), "crate-version-unsupported");
    assert_eq!(reason("rust_doctor::source::block_on_in_async"), "crate-version-unsupported");
    assert_eq!(reason("rust_doctor::source::disabled_tls_verification"), "crate-absent");
    assert_eq!(reason("rust_doctor::source::blocking_call_in_async"), Value::Null);
    assert_eq!(published["audit"]["score"]["authoritative"], true);
}
