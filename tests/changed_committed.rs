#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

#[test]
fn committed_changed_scope_is_introduced_only_but_dirty_scope_is_file_complete() {
    let repository = tempfile::tempdir().unwrap();
    fs::create_dir(repository.path().join("src")).unwrap();
    fs::write(
        repository.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.97\"\n",
    )
    .unwrap();
    fs::write(
        repository.path().join("rust-doctor.toml"),
        "dependencies = false\n",
    )
    .unwrap();
    fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn existing(value: Option<u8>) -> u8 { value.unwrap() }\n",
    )
    .unwrap();
    init_repository(repository.path());

    fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn existing(value: Option<u8>) -> u8 { value.unwrap() }\npub fn committed_marker() {}\n",
    )
    .unwrap();
    git(repository.path(), &["add", "src/lib.rs"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "committed change"],
    );

    let committed = scan(repository.path(), "HEAD~1");
    assert_success(&committed);
    let committed = parse_report(&committed);
    assert_eq!(committed["reporting_scope"], "changed");
    assert!(
        committed["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["rule"] != "unwrap-in-production")
    );

    fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn existing(value: Option<u8>) -> u8 { value.unwrap() }\npub fn committed_marker() {}\npub fn dirty_marker() {}\n",
    )
    .unwrap();
    let dirty = scan(repository.path(), "HEAD~1");
    assert_success(&dirty);
    let dirty = parse_report(&dirty);
    assert!(
        dirty["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["rule"] == "unwrap-in-production")
    );
}

fn init_repository(root: &Path) {
    git(root, &["init", "--quiet", "--initial-branch", "main"]);
    git(root, &["config", "user.email", "scope@example.com"]);
    git(root, &["config", "user.name", "Scope Fixture"]);
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "initial"]);
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn scan(root: &Path, base: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(root)
        .args(["--json", "--offline", "--scope", "changed", "--base", base])
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "scan failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON report ({error}): {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
