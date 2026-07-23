//! End-to-end integration tests for the scan pipeline.
//!
//! These tests create temporary Rust projects with known violations
//! and verify that `scan_project` detects them correctly.

// Integration test crates aren't covered by clippy.toml `allow-*-in-tests`
// (clippy #13981); unwrap/expect are fine in test assertions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rust_doctor::cli::FailOn;
use rust_doctor::config::ResolvedConfig;
use rust_doctor::diagnostics::{ScanResult, Severity};
use rust_doctor::discovery::discover_project;
use rust_doctor::scan::scan_project;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Assert that a scan result contains at least one diagnostic with the given rule name.
#[track_caller]
fn assert_has_rule(result: &ScanResult, rule: &str) {
    let all_rules: Vec<_> = result.diagnostics.iter().map(|d| &d.rule).collect();
    assert!(
        result.diagnostics.iter().any(|d| d.rule == rule),
        "Expected '{rule}' diagnostic but got none. All diags: {all_rules:?}",
    );
}

/// Assert that a scan result does NOT contain diagnostics with any of the given rule names.
#[track_caller]
fn assert_no_rules(result: &ScanResult, rules: &[&str]) {
    let found: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| rules.contains(&d.rule.as_str()))
        .map(|d| &d.rule)
        .collect();
    assert!(
        found.is_empty(),
        "Expected no diagnostics for {rules:?} but got: {found:?}",
    );
}

/// Create a minimal Cargo.toml for a temporary test project.
fn write_cargo_toml(dir: &Path, name: &str) {
    let toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
"#
    );
    fs::write(dir.join("Cargo.toml"), toml).unwrap();
}

/// Create a `ResolvedConfig` that enables custom rules but skips external tools.
fn fast_config() -> ResolvedConfig {
    ResolvedConfig {
        verbose: false,
        diff: None,
        fail_on: FailOn::None,
        ignore_rules: vec![],
        ignore_files: vec![],
        lint: true,
        dependencies: false,
        rules_config: std::collections::HashMap::new(),
        category_config: std::collections::HashMap::new(),
        tag_config: std::collections::HashMap::new(),
        path_overrides: vec![],
        enable_rules: vec![],
        score_fail_below: None,
        respect_inline_disables: true,
        max_parallelism: None,
        evaluation_profile: false,
    }
}

/// Scan a temp project with the given source code and return the result.
fn scan_temp_project(name: &str, source: &str) -> ScanResult {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    write_cargo_toml(dir, name);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), source).unwrap();

    let cargo_toml = dir.join("Cargo.toml");
    let project_info = discover_project(&cargo_toml, true).unwrap();
    let config = fast_config();

    scan_project(&project_info, &config, true, &[], true).unwrap()
}

// ── Existing rule tests ──────────────────────────────────────────────────

#[test]
fn test_scan_detects_unwrap_in_production() {
    let result = scan_temp_project(
        "test-unwrap",
        r#"
pub fn risky() -> String {
    std::env::var("HOME").unwrap()
}
"#,
    );

    assert_has_rule(&result, "unwrap-in-production");
    let diag = result
        .diagnostics
        .iter()
        .find(|d| d.rule == "unwrap-in-production")
        .unwrap();
    assert_eq!(diag.severity, Severity::Warning);
}

#[test]
fn test_scan_detects_hardcoded_secret() {
    let result = scan_temp_project(
        "test-secret",
        r#"
pub fn connect() {
    let api_key = "sk-1234567890abcdef1234567890abcdef";
    println!("{}", api_key);
}
"#,
    );

    assert_has_rule(&result, "hardcoded-secrets");
    let diag = result
        .diagnostics
        .iter()
        .find(|d| d.rule == "hardcoded-secrets")
        .unwrap();
    assert_eq!(diag.severity, Severity::Error);
}

#[test]
fn test_scan_clean_project_has_no_custom_rule_violations() {
    let result = scan_temp_project(
        "test-clean",
        r"
/// Add two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
",
    );

    // Filter out clippy lints and informational pass diagnostics — only check custom rules
    let custom_diags: Vec<&str> = result
        .diagnostics
        .iter()
        .filter(|d| {
            !d.rule.starts_with("clippy::") && d.rule != "skipped-pass" && d.rule != "missing-msrv"
        })
        .map(|d| d.rule.as_str())
        .collect();

    assert!(
        custom_diags.is_empty(),
        "Clean project should have no custom rule violations but got: {custom_diags:?}"
    );
}

// ── New rule tests (US-004) ──────────────────────────────────────────────

#[test]
fn test_scan_detects_large_enum_variant() {
    // The rule checks FIELD COUNT disparity (>3x or >5 fields when min is 0)
    let result = scan_temp_project(
        "test-large-enum",
        r"
pub enum MyEnum {
    Small(u8),
    Large(u8, u8, u8, u8, u8, u8, u8, u8),
}
",
    );

    assert_has_rule(&result, "large-enum-variant");
}

#[test]
fn test_scan_detects_sql_injection_risk() {
    // The rule detects format!() passed to .query()/.execute()/.raw() methods
    let result = scan_temp_project(
        "test-sql-injection",
        r#"
pub struct Db;
impl Db {
    pub fn query(&self, _sql: &str) {}
}
pub fn run_query(db: &Db, user_input: &str) {
    db.query(&format!("SELECT * FROM users WHERE name = '{}'", user_input));
}
"#,
    );

    assert_has_rule(&result, "sql-injection-risk");
}

#[test]
fn test_scan_detects_high_cyclomatic_complexity() {
    let result = scan_temp_project(
        "test-complexity",
        r"
pub fn complex(a: i32, b: i32, c: i32, d: i32) -> i32 {
    if a > 0 {
        if b > 0 {
            if c > 0 { 1 } else if d > 0 { 2 } else { 3 }
        } else if c > 0 {
            if d > 0 { 4 } else { 5 }
        } else if d > 0 {
            6
        } else if a > b {
            if c > d { 7 } else { 8 }
        } else {
            9
        }
    } else if b > 0 {
        if c > 0 {
            if d > 0 { 10 } else { 11 }
        } else if d > 0 {
            12
        } else {
            13
        }
    } else if c > 0 {
        if d > 0 { 14 } else { 15 }
    } else {
        16
    }
}
",
    );

    assert_has_rule(&result, "high-cyclomatic-complexity");
}

#[test]
fn test_scan_detects_unsafe_block_audit() {
    let result = scan_temp_project(
        "test-unsafe",
        r"
pub fn dangerous() {
    unsafe {
        let p = 0x1234 as *const i32;
        let _ = *p;
    }
}
",
    );

    assert_has_rule(&result, "unsafe-block-audit");
}

#[test]
fn test_scan_detects_panic_in_library() {
    let result = scan_temp_project(
        "test-panic",
        r#"
pub fn handle(input: &str) {
    if input.is_empty() {
        panic!("empty input not allowed");
    }
}
"#,
    );

    assert_has_rule(&result, "panic-in-library");
}

#[test]
fn test_scan_detects_excessive_clone() {
    // The rule requires >= 3 clone() calls in a file to trigger.
    // Calls must be outside macros (syn doesn't descend into macro token streams).
    let result = scan_temp_project(
        "test-clone",
        r"
#[derive(Clone)]
pub struct Big {
    data: Vec<u8>,
}
pub fn wasteful(a: &Big, b: &Big, c: &Big) -> (Big, Big, Big) {
    let x = a.clone();
    let y = b.clone();
    let z = c.clone();
    (x, y, z)
}
",
    );

    assert_has_rule(&result, "excessive-clone");
}

#[test]
fn test_scan_clean_project_no_false_positives() {
    let result = scan_temp_project(
        "test-clean-extended",
        r"
/// Add two numbers safely.
pub fn add(a: i32, b: i32) -> Option<i32> {
    a.checked_add(b)
}
",
    );

    assert_no_rules(
        &result,
        &[
            "sql-injection-risk",
            "large-enum-variant",
            "high-cyclomatic-complexity",
            "unsafe-block-audit",
            "panic-in-library",
            "excessive-clone",
        ],
    );
}

// ── Config tests ─────────────────────────────────────────────────────────

#[test]
fn test_scan_respects_ignored_rules() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    write_cargo_toml(dir, "test-ignore");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("src/lib.rs"),
        r#"
pub fn risky() -> String {
    std::env::var("HOME").unwrap()
}
"#,
    )
    .unwrap();

    let cargo_toml = dir.join("Cargo.toml");
    let project_info = discover_project(&cargo_toml, true).unwrap();
    let mut config = fast_config();
    config.ignore_rules = vec!["unwrap-in-production".to_string()];

    let result = scan_project(&project_info, &config, true, &[], true).unwrap();

    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.rule == "unwrap-in-production"),
        "unwrap-in-production should be ignored"
    );
}
