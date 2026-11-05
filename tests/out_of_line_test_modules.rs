#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

mod support;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/source-kernel/test-gate")
}

fn scan() -> Value {
    let workspace = fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .env("CARGO_TARGET_DIR", support::scan_target(&workspace))
        .args(["inspect", "--json"])
        .arg(&workspace)
        .output()
        .expect("rust-doctor should start");
    assert!(
        output.status.success(),
        "the fixture scan should complete: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout should carry one JSON document")
}

fn members(diagnostic: &Value) -> Vec<String> {
    let mut paths: Vec<String> = diagnostic
        .get("related")
        .and_then(Value::as_array)
        .map(|related| {
            related
                .iter()
                .filter_map(|member| member.get("path")?.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if let Some(path) = diagnostic.get("path").and_then(Value::as_str) {
        paths.push(path.to_owned());
    }
    paths.sort();
    paths
}

fn diagnostics(report: &Value) -> &Vec<Value> {
    report
        .get("diagnostics")
        .and_then(Value::as_array)
        .expect("a report carries a diagnostics array")
}

fn context_of(report: &Value, members_wanted: &[&str]) -> Value {
    let wanted: Vec<String> = {
        let mut wanted: Vec<String> = members_wanted.iter().map(|path| (*path).to_owned()).collect();
        wanted.sort();
        wanted
    };
    let found = diagnostics(report)
        .iter()
        .find(|diagnostic| members(diagnostic) == wanted);
    assert!(found.is_some(), "no diagnostic spans exactly {wanted:?}");
    found
        .and_then(|diagnostic| diagnostic.get("context"))
        .cloned()
        .unwrap_or(Value::Null)
}

#[test]
fn a_family_gated_out_of_line_is_published_as_test_material_and_a_straddling_one_is_not() {
    let report = scan();

    assert_eq!(
        context_of(
            &report,
            &["src/feature/tests/nested.rs", "src/tests/helpers.rs"]
        ),
        Value::String("tests".to_owned())
    );
    assert_eq!(
        context_of(&report, &["src/feature/production.rs", "src/shared.rs"]),
        Value::Null
    );
}

#[test]
fn a_gated_family_is_published_and_charged_nothing() {
    let report = scan();
    let charged: Vec<Vec<String>> = diagnostics(&report)
        .iter()
        .filter(|diagnostic| diagnostic.get("context").is_none_or(Value::is_null))
        .map(members)
        .collect();

    assert_eq!(diagnostics(&report).len(), 2, "both families are published");
    assert_eq!(
        charged,
        vec![vec![
            "src/feature/production.rs".to_owned(),
            "src/shared.rs".to_owned()
        ]],
        "only the family that straddles shipped code is charged"
    );
}

#[test]
fn the_fixture_scans_to_the_same_report_twice() {
    assert_eq!(scan(), scan());
}
