// Integration test crates are not covered by clippy.toml allow-in-tests settings.
#![allow(clippy::unwrap_used)]

use serde_json::Value;
use std::fs;
use std::process::{Command, Stdio};

fn project() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/nested")).unwrap();
    fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname='workflow-fixture'\nversion='0.1.0'\nedition='2024'\nrust-version='1.97'\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("rust-doctor.toml"),
        "dependencies = false\n[rules.unwrap-in-production]\nseverity = 'error'\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("src/lib.rs"),
        "pub fn value(input: Option<u8>) -> u8 {\n    input.unwrap()\n}\n",
    )
    .unwrap();
    directory
}

const fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_rust-doctor")
}

#[test]
fn nested_rule_and_why_workflows_share_project_discovery() {
    let directory = project();
    let nested = directory.path().join("src/nested");
    let list = Command::new(binary())
        .args(["rules", "list"])
        .arg(&nested)
        .args(["--configured-only", "--json"])
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let entries: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert!(entries.as_array().unwrap().iter().any(|entry| {
        entry["canonical_id"] == "unwrap-in-production"
            && entry["effective_policy"]["level"] == "error"
    }));

    let explanation = Command::new(binary())
        .args(["why", "lib.rs:2", "--directory"])
        .arg(directory.path().join("src"))
        .args(["--offline", "--max-duration", "60", "--json"])
        .output()
        .unwrap();
    assert!(
        explanation.status.success(),
        "{}",
        String::from_utf8_lossy(&explanation.stderr)
    );
    let report: Value = serde_json::from_slice(&explanation.stdout).unwrap();
    assert_eq!(report["location"], "src/lib.rs:2");
    assert!(report["findings"].as_array().is_some_and(|values| {
        values
            .iter()
            .any(|finding| finding["diagnostic"]["rule"] == "unwrap-in-production")
    }));
}

#[test]
fn explicit_output_directory_writes_bounded_handoff_without_prompting() {
    let directory = project();
    let output_dir = directory.path().join("doctor-output");
    let scan = Command::new(binary())
        .arg(directory.path())
        .args([
            "--offline",
            "--disable-adapter",
            "compiler-lint,supply-chain,quality,network-dependent",
            "--output-dir",
        ])
        .arg(&output_dir)
        .args(["--handoff", "none", "--blocking", "none", "--no-color"])
        .output()
        .unwrap();
    assert!(
        scan.status.success(),
        "{}",
        String::from_utf8_lossy(&scan.stderr)
    );
    let dump: Value =
        serde_json::from_slice(&fs::read(output_dir.join("diagnostics.json")).unwrap()).unwrap();
    assert_eq!(dump["schema_version"], "1.0");
    assert!(dump["included_diagnostics"].as_u64().unwrap() > 0);
    assert!(output_dir.join("handoff.md").is_file());
    assert!(
        fs::read_dir(output_dir.join("rules"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "txt"))
    );
}

#[test]
fn closed_stdout_pipe_terminates_without_a_crash_exit() {
    let directory = project();
    let mut child = Command::new(binary())
        .args(["rules", "list"])
        .arg(directory.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    assert!(child.wait().unwrap().success());
}

#[test]
fn scan_exit_codes_follow_the_react_doctor_contract() {
    let directory = project();
    let scan = |extra: &[&str]| {
        Command::new(binary())
            .arg(directory.path())
            .args([
                "--offline",
                "--disable-adapter",
                "compiler-lint,supply-chain,quality,network-dependent",
                "--no-color",
            ])
            .args(extra)
            .output()
            .unwrap()
    };

    let blocked = scan(&[]);
    assert_eq!(blocked.status.code(), Some(1));

    let advisory = scan(&["--blocking", "none"]);
    assert!(advisory.status.success());

    let score = scan(&["--score"]);
    assert!(score.status.success());
    assert!(
        String::from_utf8(score.stdout)
            .unwrap()
            .trim()
            .parse::<u32>()
            .is_ok()
    );

    let unknown_flag = scan(&["--not-a-real-option", "--blocking", "none"]);
    assert!(
        unknown_flag.status.success(),
        "React Doctor-compatible parsing should ignore unknown flags: {}",
        String::from_utf8_lossy(&unknown_flag.stderr)
    );
}
