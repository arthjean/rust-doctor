#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! `rust-doctor setup`: the loop for the agents detected, and nothing for the
//! others.

mod support;

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::AtomicUsize;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A git repository holding no agent directory.
fn repository() -> PathBuf {
    let root = support::fresh_workspace("setup-cli", &COUNTER);
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(status.success());
    root
}

/// A `PATH` holding git and, when asked, a fake `codex`, and no other agent.
fn path_with(root: &Path, codex: bool) -> OsString {
    let bin = root.join(".test-bin");
    fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(support::resolve_program("git"), bin.join("git")).unwrap();
    if codex {
        fs::write(bin.join("codex"), "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(bin.join("codex"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin.into_os_string()
}

fn setup(root: &Path, path: &OsString, arguments: &[&str]) -> Output {
    support::rust_doctor()
        .arg("setup")
        .arg(root)
        .args(arguments)
        .env("PATH", path)
        .output()
        .unwrap()
}

/// What the workspace holds, the test's own `PATH` directory aside.
fn tree(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            if relative == ".test-bin" || (relative.starts_with(".git/") && !relative.starts_with(".git/hooks")) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if !relative.ends_with(".sample") {
                found.push(relative);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn a_fake_codex_on_path_gets_its_skill_and_the_git_hook_and_nothing_else() {
    let root = repository();
    let path = path_with(&root, true);

    let planned = setup(&root, &path, &["--dry-run"]);
    assert!(planned.status.success(), "{}", String::from_utf8_lossy(&planned.stderr));
    assert!(tree(&root).is_empty(), "a dry run wrote {:?}", tree(&root));
    let plan = String::from_utf8_lossy(&planned.stdout);
    assert!(plan.contains("Detected: codex."), "{plan}");
    assert!(plan.contains("Would write .agents/skills/rust-doctor/SKILL.md"), "{plan}");

    let output = setup(&root, &path, &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        tree(&root),
        [
            ".agents/skills/rust-doctor/SKILL.md",
            ".agents/skills/rust-doctor/references/expert-review.md",
            ".git/hooks/pre-commit",
        ]
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.lines().last(),
        Some("setup never adds a CI workflow: run `rust-doctor ci install` for GitHub Actions.")
    );

    // A second run refreshes in place rather than refusing.
    assert!(setup(&root, &path, &[]).status.success());
}

#[test]
fn no_agent_detected_exits_two_and_names_what_it_probed() {
    let root = repository();
    let path = path_with(&root, false);
    let output = setup(&root, &path, &[]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    for probed in ["`claude`", "`codex`", "`cursor-agent`", ".claude/", ".agents/", ".codex/", ".cursor/"] {
        assert!(stderr.contains(probed), "{stderr}");
    }
    assert!(tree(&root).is_empty());

    // A project directory is a detection of its own, and `skill install
    // --agent detected` reads the same one.
    fs::create_dir(root.join(".cursor")).unwrap();
    let output = support::rust_doctor()
        .args(["skill", "install"])
        .arg(&root)
        .args(["--agent", "detected"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(root.join(".cursor/skills/rust-doctor/SKILL.md").is_file());
    assert!(!root.join(".claude").exists() && !root.join(".agents").exists());
}
