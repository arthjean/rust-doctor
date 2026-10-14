#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! `--warning-exit-code`: a passing run that carries a warning exits with the
//! code the caller chose, and nothing else moves.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicUsize;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Three `todo!()` sites, each a warning.
fn warning_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kernel-contract/todo")
}

fn exit(workspace: &Path, arguments: &[&str]) -> Option<i32> {
    support::rust_doctor()
        .arg(workspace)
        .args(["--json", "--yes"])
        .args(support::NO_ADVISORY_DB)
        .args(arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(workspace))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap()
        .status
        .code()
}

#[test]
fn a_passing_run_with_warnings_exits_with_the_chosen_code_and_nothing_else_does() {
    let fixture = warning_fixture();
    assert_eq!(exit(&fixture, &[]), Some(0));
    assert_eq!(exit(&fixture, &["--warning-exit-code", "78"]), Some(78));
    assert_eq!(
        exit(&fixture, &["--warning-exit-code", "78", "--rule", "clippy::todo=error"]),
        Some(1),
        "a gate failure keeps 1"
    );
    assert_eq!(exit(&fixture, &["--warning-exit-code", "78", "--blocking", "none"]), Some(0));
    for refused in ["2", "126", "0"] {
        assert_eq!(exit(&fixture, &["--warning-exit-code", refused]), Some(2), "{refused}");
    }
}

fn git(root: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(arguments)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?}");
}

/// In a baseline scope only what the change introduced counts: a warning the
/// base already carried keeps the run at 0.
#[test]
fn a_baseline_run_with_only_pre_existing_warnings_exits_zero() {
    let root = support::fresh_workspace("warning-exit-code", &COUNTER);
    for file in ["Cargo.toml", "Cargo.lock"] {
        std::fs::copy(warning_fixture().join(file), root.join(file)).unwrap();
    }
    support::copy_tree(&warning_fixture().join("src"), &root.join("src"));
    std::fs::write(root.join(".gitignore"), "/target\n").unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    let baseline = ["--scope", "baseline", "--base", "HEAD", "--warning-exit-code", "78"];
    assert_eq!(exit(&root, &baseline), Some(0));

    std::fs::write(
        root.join("src/added.rs"),
        "pub fn added() -> u8 {\n    todo!()\n}\n",
    )
    .unwrap();
    let lib = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    std::fs::write(root.join("src/lib.rs"), format!("pub mod added;\n{lib}")).unwrap();
    assert_eq!(exit(&root, &baseline), Some(78), "an introduced warning takes the code");
}
