#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rust_doctor::{
    BlockingLevel, GateStatus, InspectRequest, RuleLevel, RuleOverride, Severity, Status, inspect,
};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/kernel-contract")
        .join(name)
}

fn cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .args(arguments)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap()
}

fn json_report(output: &Output) -> Value {
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn public_request_defaults_and_overrides_use_the_shared_policy_compiler() {
    let path = fixture("todo");
    let default = inspect(InspectRequest::new(&path));
    assert_eq!(default.status, Status::Complete);
    assert_eq!(default.gate.blocking, BlockingLevel::Error);
    assert_eq!(default.gate.status, GateStatus::Passed);

    let overridden = inspect(
        InspectRequest::new(path)
            .with_rule_override(RuleOverride::new("clippy::todo", RuleLevel::Error))
            .with_blocking(BlockingLevel::Error),
    );
    let finding = overridden
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_deref() == Some("clippy::todo"))
        .expect("todo fixture should trigger the curated lint");
    assert_eq!(finding.base_severity, Severity::Warning);
    assert_eq!(finding.severity, Severity::Error);
    assert_eq!(overridden.gate.status, GateStatus::Failed);
    assert_eq!(overridden.exit_code(), 1);
}

#[test]
fn clap_rejects_malformed_policy_inputs_before_inspection() {
    for (flag, value) in [
        ("--rule", "clippy::todo"),
        ("--rule", "=warn"),
        ("--rule", "clippy::todo=deny"),
        ("--category", "security=warning"),
    ] {
        let output = cli(&[
            "inspect",
            "--json",
            flag,
            value,
            "/path/that/must/not/be/inspected",
        ]);
        assert_eq!(output.status.code(), Some(2), "{flag} {value}");
        assert!(output.stdout.is_empty(), "{flag} {value}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("off"), "{stderr}");
        assert!(stderr.contains("warn"), "{stderr}");
        assert!(stderr.contains("error"), "{stderr}");
        assert!(!stderr.contains(value), "policy value leaked: {stderr}");
        assert!(!stderr.contains("Inspecting Cargo workspace"), "{stderr}");
    }

    let hostile = [
        ("--rule", "private/path=deny".to_owned(), "private/path"),
        (
            "--rule",
            format!("{}SECRET_AFTER_BOUND=deny", "a".repeat(129)),
            "SECRET_AFTER_BOUND",
        ),
        (
            "--category",
            "private\u{1b}[31mcategory=deny".to_owned(),
            "private\u{1b}[31mcategory",
        ),
    ];
    for (flag, value, sentinel) in hostile {
        let output = cli(&[
            "inspect",
            "--json",
            flag,
            &value,
            "/path/that/must/not/be/inspected",
        ]);
        assert_eq!(output.status.code(), Some(2), "{flag}");
        assert!(output.stdout.is_empty(), "{flag}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("off"), "{stderr}");
        assert!(stderr.contains("warn"), "{stderr}");
        assert!(stderr.contains("error"), "{stderr}");
        assert!(!stderr.contains(sentinel), "selector leaked: {stderr}");
        assert!(!stderr.contains("Inspecting Cargo workspace"), "{stderr}");
    }

    let output = cli(&[
        "inspect",
        "--blocking",
        "warn",
        "/path/that/must/not/be/inspected",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    for allowed in ["none", "error", "warning"] {
        assert!(stderr.contains(allowed), "{stderr}");
    }
    assert!(!stderr.contains("Inspecting Cargo workspace"));
}

#[test]
fn semantic_policy_failures_are_single_private_v7_reports_before_discovery() {
    let long = format!("{}SECRET_AFTER_BOUND", "a".repeat(129));
    let cases = [
        ("--rule", "unknown::rule".to_owned(), "unknown-rule"),
        ("--category", "unknown".to_owned(), "unknown-category"),
        ("--rule", "bad/path".to_owned(), "invalid-rule-selector"),
        ("--rule", "bad\\path".to_owned(), "invalid-rule-selector"),
        ("--rule", "bad\npath".to_owned(), "invalid-rule-selector"),
        (
            "--rule",
            "bad\u{1b}[31mpath".to_owned(),
            "invalid-rule-selector",
        ),
        ("--rule", long, "invalid-rule-selector"),
    ];

    for (flag, selector, expected_code) in cases {
        let value = format!("{selector}=warn");
        let output = cli(&[
            "inspect",
            "--json",
            flag,
            &value,
            "/path/that/must/not/be/inspected",
        ]);
        assert_eq!(output.status.code(), Some(2), "{selector:?}");
        let report = json_report(&output);
        assert_eq!(report["schema_version"], 15);
        assert_eq!(report["policy"], Value::Null);
        assert_eq!(report["status"], "failed");
        assert_eq!(report["gate"]["status"], "not-evaluated");
        assert_eq!(report["gate"]["blocking_diagnostics"], Value::Null);
        assert_eq!(report["errors"].as_array().unwrap().len(), 1);
        assert_eq!(report["errors"][0]["stage"], "policy");
        assert_eq!(report["errors"][0]["code"], expected_code);
        assert!(report["scan"]["command"].is_null());

        let mut rendered = String::from_utf8_lossy(&output.stdout).into_owned();
        rendered.push_str(&String::from_utf8_lossy(&output.stderr));
        if expected_code.starts_with("invalid-") {
            assert!(
                !rendered.contains(&selector),
                "selector leaked: {selector:?}"
            );
        }
        assert!(!rendered.contains("/path/that/must/not/be/inspected"));
        assert!(!rendered.contains("SECRET_AFTER_BOUND"));
        assert!(!rendered.contains('\u{1b}'));
    }

    let output = cli(&[
        "inspect",
        "--json",
        "--rule",
        "clippy::todo=warn",
        "--rule",
        "clippy::todo=error",
        "/path/that/must/not/be/inspected",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        json_report(&output)["errors"][0]["code"],
        "duplicate-rule-override"
    );
}

#[test]
fn terminal_gate_exposes_effective_policy_threshold_and_count() {
    let path = fixture("todo");
    let output = cli(&[
        "inspect",
        "--rule",
        "clippy::todo=error",
        path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Policy: base severity warning, effective severity error"));
    assert!(stdout.contains("Gate failed: blocking error, 3 blocking diagnostic(s)"));
}

/// The catalog the CLI publishes is the catalog the scan compiles against.
///
/// This is the surface the website reads to state what the tool checks, so a
/// rule added to `src/policy/catalog.rs` has to reach it without anyone
/// republishing a list by hand. Comparing against the library's own catalog
/// rather than a frozen count is what keeps that true as the catalog grows.
#[test]
fn rules_list_publishes_the_shipped_catalog() {
    let output = cli(&["rules", "list", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());

    let published: Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = published.as_array().unwrap();
    let shipped = rust_doctor::catalog();
    assert_eq!(entries.len(), shipped.len());

    for (entry, rule) in entries.iter().zip(shipped.iter()) {
        assert_eq!(entry["id"], rule.id);
        assert_eq!(entry["category"], rule.category);
        assert_eq!(entry["tier"], rule.tier.as_str());
        assert_eq!(entry["help"], rule.help);
        // Every catalogued rule ships at warn: an error by default would be a
        // policy the reader never chose.
        assert_eq!(entry["default_level"], "warn");
    }

    // A producer is named by the identifier's prefix, and the published field
    // has to agree with it, since the website groups by one and explains the
    // other.
    for entry in entries {
        let id = entry["id"].as_str().unwrap();
        let producer = entry["producer"].as_str().unwrap();
        let expected = match id.split("::").next().unwrap() {
            "clippy" => "clippy",
            _ => match id.split("::").nth(1).unwrap() {
                "cargo" => "cargo-health",
                "source" => "source-kernel",
                "structure" => "structure",
                "repo" => "repo",
                // An unknown segment fails the assertion below rather than
                // unwinding, so the report names the rule that introduced it.
                other => other,
            },
        };
        assert_eq!(producer, expected, "{id}");
    }
}

/// The human listing is the same catalog, one rule per line.
#[test]
fn rules_list_without_json_stays_line_oriented() {
    let output = cli(&["rules", "list"]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), rust_doctor::catalog().len());
    assert!(lines.iter().all(|line| line.matches('\t').count() == 3));
}
