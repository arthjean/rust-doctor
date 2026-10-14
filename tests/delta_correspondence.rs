#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! What a `--scope baseline` run calls introduced once the pairing reads the
//! diff: a refactor inside a flagged line, a file moved with `git mv`, a
//! duplicate inserted above an old finding, a diff too large to map, and a
//! build script that prints.
//!
//! Every case is a real repository scanned twice by the binary, its base at
//! `HEAD` and its working tree as the current side.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(arguments)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
}

/// A one-package repository whose `HEAD` holds `files`.
fn repository(files: &[(&str, &str)]) -> PathBuf {
    let root = support::fresh_workspace("delta-correspondence", &COUNTER);
    write(
        &root,
        "Cargo.toml",
        "[package]\nname = \"pairing\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&root, ".gitignore", "/target\n");
    for (path, contents) in files {
        write(&root, path, contents);
    }
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    root
}

fn write(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// The baseline report of the working tree against `HEAD`.
fn compare(root: &Path, blocking: &str) -> Value {
    let output = support::rust_doctor()
        .arg(root)
        .args(["--json", "--yes", "--scope", "baseline", "--base", "HEAD"])
        .args(["--blocking", blocking])
        .args(support::NO_ADVISORY_DB)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    let report = serde_json::from_slice::<Value>(&output.stdout);
    assert!(report.is_ok(), "{}", String::from_utf8_lossy(&output.stderr));
    let report = report.unwrap();
    assert_eq!(report["status"], "complete", "{}", report["errors"]);
    report
}

fn summary(report: &Value) -> (u64, u64, u64, u64) {
    let summary = &report["delta"]["summary"];
    (
        summary["introduced"].as_u64().unwrap(),
        summary["fixed"].as_u64().unwrap(),
        summary["cross_file_matches"].as_u64().unwrap(),
        summary["rule_count_matches"].as_u64().unwrap(),
    )
}

/// The introduced diagnostics, as `(rule, path, line)`.
fn introduced(report: &Value) -> Vec<(String, String, u64)> {
    let ids = report["delta"]["introduced"].as_array().unwrap();
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| ids.contains(&diagnostic["id"]))
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap_or_default().to_owned(),
                diagnostic["path"].as_str().unwrap_or_default().to_owned(),
                diagnostic["span"]["line_start"].as_u64().unwrap_or_default(),
            )
        })
        .collect()
}

#[test]
fn a_variable_renamed_inside_a_flagged_line_introduces_and_fixes_nothing() {
    let root = repository(&[(
        "src/lib.rs",
        "pub fn read(cfg: &std::collections::HashMap<&str, u8>) -> u8 {\n    *cfg.get(\"k\").unwrap()\n}\n",
    )]);
    write(
        &root,
        "src/lib.rs",
        "pub fn read(config: &std::collections::HashMap<&str, u8>) -> u8 {\n    *config.get(\"k\").unwrap()\n}\n",
    );
    let report = compare(&root, "warning");
    assert_eq!(summary(&report), (0, 0, 0, 1), "{}", report["delta"]);
    assert_eq!(report["gate"]["status"], "passed");
}

#[test]
fn a_finding_carried_to_another_file_pairs_on_evidence_or_along_a_rename() {
    // Fixed in one file and added, identical, in another: evidence follows
    // the code across files, the blind spot the pairing accepts.
    let root = repository(&[
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub fn a() -> u8 {\n    todo!()\n}\n"),
        ("src/b.rs", "pub fn b() -> u8 {\n    1\n}\n"),
    ]);
    write(&root, "src/a.rs", "pub fn a() -> u8 {\n    1\n}\n");
    write(&root, "src/b.rs", "pub fn b() -> u8 {\n    todo!()\n}\n");
    let report = compare(&root, "warning");
    assert_eq!(summary(&report), (0, 0, 1, 0), "{}", report["delta"]);

    // Moved with `git mv` and edited inside the flagged span: no evidence
    // matches, and the rename alone places the base finding in the new file.
    let root = repository(&[
        ("src/lib.rs", "pub mod a;\n"),
        ("src/a.rs", "pub fn a() -> u8 {\n    todo!(\"before\")\n}\n"),
    ]);
    git(&root, &["mv", "src/a.rs", "src/b.rs"]);
    write(&root, "src/lib.rs", "pub mod b;\n");
    write(&root, "src/b.rs", "pub fn a() -> u8 {\n    todo!(\"after\")\n}\n");
    let report = compare(&root, "warning");
    assert_eq!(summary(&report), (0, 0, 1, 1), "{}", report["delta"]);
}

#[test]
fn a_duplicate_inserted_above_an_old_finding_is_the_one_introduced() {
    let root = repository(&[("src/lib.rs", "pub fn old() -> u8 {\n    todo!()\n}\n")]);
    write(
        &root,
        "src/lib.rs",
        "pub fn new() -> u8 {\n    todo!()\n}\n\npub fn old() -> u8 {\n    todo!()\n}\n",
    );
    let report = compare(&root, "warning");
    assert_eq!(
        introduced(&report),
        [("clippy::todo".to_owned(), "src/lib.rs".to_owned(), 2)],
        "{}",
        report["delta"]
    );
}

#[test]
fn a_diff_too_large_to_map_still_compares_and_says_so() {
    let root = repository(&[
        ("src/lib.rs", "pub fn old() -> u8 {\n    todo!()\n}\n"),
        ("data.txt", "seed\n"),
    ]);
    write(&root, "data.txt", &"0123456789abcdef\n".repeat(80_000));
    write(
        &root,
        "src/lib.rs",
        "pub fn new() -> u8 {\n    todo!()\n}\n\npub fn old() -> u8 {\n    todo!()\n}\n",
    );
    let report = compare(&root, "warning");
    assert_eq!(summary(&report).0, 1, "{}", report["delta"]);
    let notice = report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|error| error["code"] == "line-map-unavailable");
    assert!(notice.is_some(), "{}", report["errors"]);
    let notice = notice.unwrap();
    assert_eq!(notice["stage"], "delta");
    assert_eq!(report["audit"]["score"]["authoritative"], true, "{}", report["audit"]);
}

/// The PRD's example is a `println!`, but Clippy exempts build scripts from
/// `print_stdout`; an `unwrap` is the same case, a finding of the channel
/// Cargo imposes, marked `build-script` and weighing nothing.
#[test]
fn a_finding_added_to_a_build_script_is_introduced_and_does_not_block() {
    let root = repository(&[
        ("src/lib.rs", "pub fn one() -> u8 {\n    1\n}\n"),
        ("build.rs", "fn main() {}\n"),
    ]);
    write(
        &root,
        "build.rs",
        "fn main() {\n    let _ = std::env::var(\"OUT_DIR\").unwrap();\n}\n",
    );
    let report = compare(&root, "warning");
    assert_eq!(
        introduced(&report),
        [("clippy::unwrap_used".to_owned(), "build.rs".to_owned(), 2)],
        "{}",
        report["delta"]
    );
    let introduced_id = &report["delta"]["introduced"][0];
    let diagnostic = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| &diagnostic["id"] == introduced_id)
        .unwrap();
    assert_eq!(diagnostic["severity"], "warning", "{diagnostic}");
    assert!(diagnostic["context"].is_string(), "{diagnostic}");
    assert_eq!(report["gate"]["status"], "passed", "{}", report["gate"]);
}
