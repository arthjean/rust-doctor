#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! What a scan publishes for a module the compiler only builds under
//! `cfg(test)`.
//!
//! An out-of-line `#[cfg(test)] mod tests;` is no Cargo target, so the target
//! kind cannot name it, and it sits wherever its declaration points, so a path
//! convention only guesses at it. Before the walk carried the gate, every file
//! below such a declaration was published as production and charged to the
//! score. The `test-gate` fixture holds one file per form of the grammar, and
//! this is the whole product path over it: the binary, `cargo clippy`, the
//! structural pass, and the JSON the report publishes.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

mod support;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/source-kernel/test-gate")
}

/// One scan of the fixture, through the binary and with an artifact cache keyed
/// on the scanned path, so the second scan of the same workspace is the cache
/// hit a rescan assertion needs rather than a fresh compile of a different tree.
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

/// Every path a diagnostic speaks for: its own, and every member its `related`
/// array names. A structural family is one diagnostic over several sites, and
/// the context it publishes is a claim about all of them.
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

/// The two families the fixture publishes, and the two different answers they
/// deserve.
///
/// One duplication sits between `src/tests/helpers.rs` and
/// `src/feature/tests/nested.rs`. Neither is a Cargo target and neither is under
/// a `tests` directory the convention would recognize: both are reached only
/// through a `#[cfg(test)] mod tests;`, one resolving to `src/tests/mod.rs` and
/// the other to `src/feature/tests.rs`, and the second is a further level below
/// its gate. Every member is test material, so the family is published as such
/// and stops weighing.
///
/// The other sits between `src/shared.rs` and `src/feature/production.rs`.
/// `src/shared.rs` is reached both by an ungated declaration in the crate root
/// and by a gated one under `#[path]`, so it abstains, and the family straddles
/// it and shipped code. It keeps a production context and is charged: the
/// duplication genuinely involves code that ships.
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

/// A test-context diagnostic leaves the score and stays in the report.
///
/// The two are one requirement: a finding the reader can still act on, costing
/// nothing. The gated family is one of the two the fixture publishes, and the
/// score is what the other one alone costs.
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

/// Two scans of one workspace publish one report. The classification is read
/// out of the source, so it cannot depend on what Cargo had already built.
#[test]
fn the_fixture_scans_to_the_same_report_twice() {
    assert_eq!(scan(), scan());
}
