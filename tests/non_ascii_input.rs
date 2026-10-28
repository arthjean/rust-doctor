#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! EP-001 US-001: a typographic quote or any other multibyte character in
//! front of `://` once made the credential scanner slice inside it, and the
//! scan died with exit 101 on the three paths that read such a line: the
//! repository pass over a tracked README, the code frame `--verbose` prints,
//! and the excerpt of what a failed build script wrote to stderr.
//!
//! Every proof runs the binary a user runs, inside a fresh git repository.
//! The credential each workspace carries is spelled with its `://` split, by
//! `@@` in the README fixture and by a format argument here, so no file of
//! this repository holds a value the rule matches.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

mod support;

const CREDENTIAL: &str = "rust_doctor::repo::hardcoded_credential";

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scan-failures")
        .join(name)
}

/// A fresh one-crate workspace named `name`, holding `files`.
fn workspace(name: &str, files: &[(&str, String)]) -> PathBuf {
    let root = support::fresh_workspace("non-ascii-input", &NEXT_WORKSPACE);
    fs::write(
        root.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[workspace]\n"),
    )
    .unwrap();
    for (path, contents) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    root
}

fn commit(root: &Path) {
    for arguments in [
        &["init", "--quiet"][..],
        &["add", "-A"],
        &[
            "-c",
            "user.name=Rust Doctor",
            "-c",
            "user.email=rust-doctor@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "base",
        ],
    ] {
        let output = support::git_output(root, arguments);
        assert!(output.status.success(), "git {arguments:?}: {output:?}");
    }
}

fn run(root: &Path, arguments: &[&str]) -> Output {
    support::rust_doctor()
        .arg(root)
        .arg("--yes")
        .args(support::NO_ADVISORY_DB)
        .args(arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .output()
        .unwrap()
}

/// The run did not panic: no exit 101, no panic message on stderr, and not
/// the exit 2 `main` turns a panic no pass contained into.
fn assert_survived(output: &Output) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(output.status.code(), Some(101), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(!stderr.contains("internal error"), "{stderr}");
}

fn strings_of(value: &Value) -> Vec<&str> {
    match value {
        Value::String(text) => vec![text.as_str()],
        Value::Array(items) => items.iter().flat_map(strings_of).collect(),
        Value::Object(fields) => fields.values().flat_map(strings_of).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn a_tracked_readme_with_a_quote_before_the_scheme_is_reported_not_crashed_on() {
    let root = support::fresh_workspace("non-ascii-input", &NEXT_WORKSPACE);
    support::copy_tree(&fixture("multibyte-scheme"), &root);
    let readme = root.join("README.md");
    fs::write(&readme, fs::read_to_string(&readme).unwrap().replace("@@", "")).unwrap();
    commit(&root);

    let output = run(&root, &["--json"]);
    assert_survived(&output);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "complete", "{}", report["errors"]);
    // The exit status is the gate's: a warning passes the default gate.
    let gate = if report["gate"]["status"] == "passed" { 0 } else { 1 };
    assert_eq!(output.status.code(), Some(gate), "{}", report["gate"]);
    let found: Vec<&Value> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == CREDENTIAL)
        .collect();
    assert_eq!(found.len(), 1, "{report}");
    assert_eq!(found[0]["path"], "README.md");
    assert_eq!(found[0]["span"]["line_start"], 3, "{}", found[0]);
}

#[test]
fn a_verbose_code_frame_over_a_quoted_url_renders() {
    let comment = format!("// docs: “postgres:/{}/u:p@h.example.com/x”", "");
    let root = workspace(
        "multibyte-frame",
        &[(
            "src/main.rs",
            format!("fn main() {{\n    {comment}\n    println!(\"hello\");\n}}\n"),
        )],
    );
    commit(&root);

    let output = run(&root, &["--verbose", "--package", "multibyte-frame"]);
    assert_survived(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("println!(\"hello\")"), "{stdout}");
    // The frame escapes the quotes it prints and masks the password.
    let masked = format!("// docs: \\u{{201C}}postgres:/{}/u:<redacted>@h.example.com/x", "");
    assert!(stdout.contains(&masked), "{stdout}");
}

#[test]
fn a_build_script_printing_a_quoted_url_leaves_a_masked_report() {
    let root = workspace(
        "multibyte-build",
        &[
            ("src/lib.rs", "pub fn fixture() {}\n".to_owned()),
            (
                "build.rs",
                "fn main() {\n    \
                 eprintln!(\"“postgres:/{}/u:secretpw@db.example.com/x\", \"\");\n    \
                 std::process::exit(1);\n}\n"
                    .to_owned(),
            ),
        ],
    );
    commit(&root);

    let output = run(&root, &["--json"]);
    assert_survived(&output);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let errors = report["errors"].as_array().unwrap();
    assert!(
        errors.iter().all(|error| error["code"] != "clippy-thread-failed"),
        "{errors:?}"
    );
    let echoed = errors
        .iter()
        .filter_map(|error| error["message"].as_str())
        .find(|message| message.contains("postgres://"));
    assert!(
        echoed.is_some_and(|message| message.contains(&format!("“postgres:/{}/u:secr<redacted>@db.example.com/x", ""))),
        "{errors:?}"
    );
    for text in strings_of(&report) {
        assert!(!text.contains("secretpw"), "{text}");
    }
}
