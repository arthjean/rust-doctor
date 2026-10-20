#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! `rules explain` and `why`, run from the binary: what a rule is and how a
//! workspace sets it, without a process; and why a line is reported, from a
//! scan of its member or from a saved report.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A two-member workspace whose `a` member leaves a `todo!()` on line 2.
fn workspace() -> PathBuf {
    let root = support::fresh_workspace("explain-cli", &COUNTER);
    let write = |path: &str, contents: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    };
    write("Cargo.toml", "[workspace]\nmembers = [\"a\", \"b\"]\nresolver = \"2\"\n");
    for member in ["a", "b"] {
        write(
            &format!("{member}/Cargo.toml"),
            &format!("[package]\nname = \"{member}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        );
    }
    write("a/src/lib.rs", "pub fn a() -> u8 {\n    todo!()\n}\n\npub fn clean() -> u8 {\n    1\n}\n");
    write("b/src/lib.rs", "pub fn b() -> u8 {\n    2\n}\n");
    write(
        "rust-doctor.toml",
        "[rules]\n\"clippy::todo\" = \"error\"\n\n[[overrides]]\npaths = [\"b/**\"]\nrules = { \"clippy::todo\" = \"off\" }\n",
    );
    root
}

/// Every run below starts with an empty `PATH`: given the workspace root
/// itself, an answer needs neither Cargo, git nor any other process.
#[test]
fn rules_explain_resolves_a_suffix_and_reads_the_workspace_without_a_process() {
    let root = workspace();
    let output = support::rust_doctor()
        .args(["rules", "explain", "todo"])
        .arg(&root)
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let explained = text(&output);
    for line in [
        "clippy::todo\n",
        "Producer: clippy",
        "Category: correctness",
        "Tier: P2",
        "Default level: warn",
        "Help: ",
        "Rule: https://rust-doctor.com/rules/clippy%3A%3Atodo",
        "Corpus noise: ",
        "Effective level: error (config-rule)",
        "Override: off under b/**",
    ] {
        assert!(explained.contains(line), "{line:?} missing from {explained}");
    }

    let output = support::rust_doctor()
        .args(["rules", "explain", "tood"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("`clippy::todo`"));

    // `rules list` takes no path and reads none: it answers from an empty
    // directory as from anywhere.
    let empty = support::fresh_workspace("explain-cli", &COUNTER);
    let output = support::rust_doctor()
        .args(["rules", "list", "--json"])
        .current_dir(&empty)
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(serde_json::from_slice::<Value>(&output.stdout).unwrap().is_array());
}

fn why(root: &Path, arguments: &[&str]) -> Output {
    support::rust_doctor()
        .arg("why")
        .args(arguments)
        .current_dir(root)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap()
}

#[test]
fn why_explains_a_reported_line_live_and_from_a_saved_report() {
    let root = workspace();
    let live = why(&root, &["a/src/lib.rs:2", "--json"]);
    assert!(live.status.success(), "{}", String::from_utf8_lossy(&live.stderr));
    let site: Value = serde_json::from_slice(&live.stdout).unwrap();
    let hit = &site["hits"][0];
    assert_eq!(hit["rule_id"], "clippy::todo", "{site}");
    assert_eq!(hit["severity"], "error", "{site}");
    assert_eq!(hit["level_source"], "config-rule", "{site}");
    assert_eq!(hit["path"], "a/src/lib.rs", "{site}");
    assert!(hit["directive"].as_str().unwrap().starts_with("#[expect(clippy::todo"), "{site}");

    let printed = text(&why(&root, &["a/src/lib.rs:2"]));
    assert!(printed.contains("> 2 |     todo!()"), "{printed}");
    assert!(printed.contains("To suppress this site: #[expect(clippy::todo"), "{printed}");

    // A report saved earlier answers the same, and nothing is spawned: the
    // run below has no `PATH` at all.
    let scan = support::rust_doctor()
        .arg(&root)
        .args(["--json", "--yes"])
        .args(support::NO_ADVISORY_DB)
        .env("CARGO_TARGET_DIR", support::scan_target(&root))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    let saved = root.join("report.json");
    fs::write(&saved, &scan.stdout).unwrap();
    let output = support::rust_doctor()
        .args(["why", "a/src/lib.rs:2:5", "--json", "--report"])
        .arg(&saved)
        .current_dir(&root)
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let from_report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(from_report["hits"][0]["rule_id"], "clippy::todo", "{from_report}");
}

#[test]
fn why_says_why_a_line_has_nothing_and_refuses_a_non_positive_line() {
    let root = workspace();
    let output = why(&root, &["a/src/lib.rs:6"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(text(&output), "Nothing is reported at a/src/lib.rs:6.\n");

    for refused in ["a/src/lib.rs:0", "a/src/lib.rs:-1", "a/src/lib.rs"] {
        let output = why(&root, &[refused]);
        assert_eq!(output.status.code(), Some(2), "{refused}");
        assert!(output.stdout.is_empty(), "{refused}");
    }
}

/// EP-002 US-008: a member's PATH explains with the configuration a scan of
/// it reads, the workspace root's. The member's own `rust-doctor.toml`, which
/// scans ignore, is not read.
#[test]
fn rules_explain_on_a_member_reads_the_workspace_roots_configuration() {
    let root = workspace();
    fs::write(root.join("a/rust-doctor.toml"), "[rules]\n\"clippy::todo\" = \"off\"\n").unwrap();
    let output = support::rust_doctor()
        .args(["rules", "explain", "todo", "--json"])
        .arg(root.join("a"))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let explained: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(explained["effective"]["level"], "error", "{explained}");
    assert_eq!(explained["effective"]["source"], "config-rule", "{explained}");

    let scan = support::rust_doctor()
        .arg(root.join("a"))
        .args(["--json", "--yes"])
        .args(support::NO_ADVISORY_DB)
        .env("CARGO_TARGET_DIR", support::scan_target(&root))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    let scanned: Value = serde_json::from_slice(&scan.stdout).unwrap();
    let rule = scanned["policy"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rule| rule["id"] == "clippy::todo")
        .unwrap();
    assert_eq!(rule["level"], explained["effective"]["level"], "{rule}");
}
