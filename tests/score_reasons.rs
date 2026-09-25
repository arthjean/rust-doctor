#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Proofs of EP-001 US-004 and US-005: an uncatalogued warning is published and
//! weighs nothing, and a score that is not authoritative says why.
//!
//! Every proof starts at the binary a user runs, on a fixture under
//! `tests/fixtures/score-reasons/`, so the wire shape asserted here is the one
//! `--json` publishes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rust_doctor::ScoreReason;
use serde_json::Value;

mod support;

/// Every scored rule the fixtures can raise, switched off where a proof is
/// about the notes alone. The repository rule depends on how the checkout
/// tracks the fixture, so it is off everywhere.
const SCORED_OFF: [&str; 4] = [
    "--rule",
    "rust_doctor::repo::unignored_build_output=off",
    "--rule",
    "clippy::eq_op=off",
];
const REPO_RULE_OFF: [&str; 2] = ["--rule", "rust_doctor::repo::unignored_build_output=off"];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/score-reasons")
        .join(name)
}

fn run(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(root)
        .arg("--yes")
        .args(arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .expect("the binary should start")
}

fn json(root: &Path, arguments: &[&str]) -> Value {
    let mut arguments = arguments.to_vec();
    arguments.push("--json");
    serde_json::from_slice(&run(root, &arguments).stdout).expect("a JSON report")
}

fn diagnostic<'a>(report: &'a Value, code: &str) -> &'a Value {
    let found = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == code);
    assert!(
        found.is_some(),
        "{code} is missing from {}",
        report["diagnostics"]
    );
    found.expect("the diagnostic was just asserted")
}

/// US-004 AC-1 and AC-2: rustc's `unused_imports` and a Clippy lint the
/// workspace's own `[lints.clippy]` table enabled are both published, marked
/// `unscored: "uncatalogued"`, and leave the score whole and authoritative.
#[test]
fn an_uncatalogued_warning_is_published_unscored_and_voids_nothing() {
    let root = fixture("uncatalogued");
    let report = json(&root, &SCORED_OFF);

    assert_eq!(report["status"], "complete", "{}", report["errors"]);
    for code in ["unused_imports", "clippy::must_use_candidate"] {
        let note = diagnostic(&report, code);
        assert_eq!(note["unscored"], "uncatalogued", "{code}");
        assert_eq!(note["severity"], "warning", "{code}");
        assert_eq!(note["path"], "src/lib.rs", "{code}");
        assert!(note["category"].is_null(), "{code}");
    }
    let score = &report["audit"]["score"];
    assert_eq!(score["authoritative"], true);
    assert_eq!(score["reasons"], serde_json::json!([]));
    assert_eq!(score["value"], 100, "a note weighed on the score");
    assert!(
        score["dimensions"]
            .as_object()
            .unwrap()
            .values()
            .all(|value| value == 100),
        "{}",
        score["dimensions"]
    );
    assert_eq!(
        report["gate"]["status"], "passed",
        "a note blocked the gate"
    );

    // A scored finding carries no such member: absence is what says it weighs.
    let scored = json(&root, &REPO_RULE_OFF);
    let eq_op = diagnostic(&scored, "clippy::eq_op");
    assert!(eq_op.get("unscored").is_none());
    assert!(scored["audit"]["score"]["value"].as_u64().unwrap() < 100);
}

/// US-004 AC-5: the linear report prints the notes after every scored finding,
/// under their own heading, and counts them in the summary line.
#[test]
fn the_linear_report_prints_compiler_notes_last_under_their_heading() {
    let root = fixture("uncatalogued");
    let output = run(&root, &["--verbose", REPO_RULE_OFF[0], REPO_RULE_OFF[1]]);
    let stdout = String::from_utf8(output.stdout).unwrap();

    let heading = stdout.find("Compiler notes (not scored)");
    assert!(heading.is_some(), "no compiler notes heading in:\n{stdout}");
    let heading = heading.expect("the heading was just asserted");
    let last_scored = stdout
        .rfind("Rule ID:")
        .expect("the eq_op finding is scored and printed in full");
    assert!(last_scored < heading, "a scored finding follows the notes");
    assert!(stdout[heading..].contains("clippy::must_use_candidate"));
    assert!(stdout[heading..].contains("unused_imports"));
    assert!(
        stdout.contains("across 3 findings, 2 of them compiler notes (not scored)"),
        "{stdout}"
    );
}

/// US-004 AC-4 and US-005 AC-3: an uncatalogued error is the compilation
/// failing, not a note. The stage error stays in `errors[]`, and the score says
/// `stage-failed`, in the report and in the linear output.
#[test]
fn a_compile_error_fails_the_stage_and_is_the_reason_given() {
    let root = fixture("compile-error");
    let report = json(&root, &REPO_RULE_OFF);

    let error = diagnostic(&report, "E0308");
    assert_eq!(error["severity"], "error");
    assert!(error.get("unscored").is_none(), "an error became a note");
    let errors = report["errors"].as_array().unwrap();
    assert!(
        errors
            .iter()
            .any(|error| error["stage"] == "execution" && error["code"] == "build-failed"),
        "{errors:?}"
    );
    let score = &report["audit"]["score"];
    assert_eq!(score["authoritative"], false);
    assert_eq!(score["reasons"], serde_json::json!(["stage-failed"]));

    let output = run(&root, &REPO_RULE_OFF);
    let stdout = String::from_utf8(output.stdout).unwrap();
    // The terminal wraps the line to its width, so the words are compared.
    let words = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        words.contains(ScoreReason::StageFailed.explanation()),
        "{stdout}"
    );
    assert!(!stdout.contains("Score is partial because"), "{stdout}");
}

/// US-005 AC-6: the report names the binary that produced it.
#[test]
fn the_toolchain_block_names_the_rust_doctor_version() {
    let report = json(&fixture("uncatalogued"), &REPO_RULE_OFF);
    assert_eq!(
        report["toolchain"]["rust_doctor"],
        env!("CARGO_PKG_VERSION")
    );
}
