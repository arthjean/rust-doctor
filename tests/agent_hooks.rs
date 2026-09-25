#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! The installers and the hooks they install, from the binary's entry point:
//! a git pre-commit hook that really refuses a commit, an agent's settings
//! file merged rather than overwritten, and an end-of-turn rescan that blocks
//! on what the turn introduced and never on a broken toolchain.

mod support;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::AtomicUsize;

use serde_json::{Value, json};

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rust-doctor"));
    command
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("RUSTFLAGS");
    command
}

fn git(root: &Path, arguments: &[&str]) -> Output {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// The environment a commit runs its hook under: the binary on `PATH`, and a
/// Cargo target of the test's own.
fn commit(root: &Path, message: &str) -> Output {
    let binary_directory = Path::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .parent()
        .unwrap();
    let path = std::env::join_paths(
        std::iter::once(binary_directory.to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    Command::new("git")
        .args([
            "-c",
            "user.name=Rust Doctor",
            "-c",
            "user.email=rust-doctor@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            message,
        ])
        .current_dir(root)
        .env("PATH", path)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("RUSTFLAGS")
        .output()
        .unwrap()
}

/// A one-crate repository whose gate blocks on a warning, committed clean.
fn repository(name: &str) -> PathBuf {
    let root = support::temporary_target(&format!("agent-hooks-{name}"), &NEXT_WORKSPACE);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"agent-hooks\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "/target\n").unwrap();
    fs::write(root.join("rust-doctor.toml"), "blocking = \"warning\"\n").unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    git(&root, &["init", "--quiet", "--initial-branch=master"]);
    git(&root, &["add", "."]);
    let initial = commit(&root, "initial");
    assert!(initial.status.success());
    root
}

fn workspace(name: &str) -> PathBuf {
    let root = support::temporary_target(&format!("agent-hooks-{name}"), &NEXT_WORKSPACE);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A directory outside every git repository. `target/` sits inside this one,
/// so a workspace there is always in a working tree.
fn outside_git(name: &str) -> PathBuf {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "rust-doctor-agent-hooks-{name}-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    let inside = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        !inside.status.success(),
        "the temporary directory is inside a repository"
    );
    root
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn install(root: &Path, arguments: &[&str]) -> Output {
    binary()
        .args(["hook", "install"])
        .args(arguments)
        .arg(root)
        .output()
        .unwrap()
}

/// The pre-commit hook judges what the commit records: a staged defect is
/// refused even once the working tree fixed it, and a clean commit passes.
#[test]
fn the_git_hook_refuses_a_commit_that_records_a_finding() {
    let root = repository("git-commit");
    let dry = install(&root, &["git", "--dry-run"]);
    assert_eq!(dry.status.code(), Some(0), "{}", stderr(&dry));
    assert!(stdout(&dry).contains("Would write .git/hooks/pre-commit"));
    assert!(!root.join(".git/hooks/pre-commit").exists());

    let installed = install(&root, &["git"]);
    assert_eq!(installed.status.code(), Some(0), "{}", stderr(&installed));
    let hook = fs::read_to_string(root.join(".git/hooks/pre-commit")).unwrap();
    assert!(
        hook.ends_with("exec rust-doctor . --yes --staged --scope lines\n"),
        "{hook}"
    );

    fs::write(
        root.join("src/lib.rs"),
        "pub fn value() -> u8 { todo!() }\n",
    )
    .unwrap();
    git(&root, &["add", "src/lib.rs"]);
    fs::write(root.join("src/lib.rs"), "pub fn value() -> u8 { 2 }\n").unwrap();
    let refused = commit(&root, "staged defect");
    assert!(
        !refused.status.success(),
        "the hook let a staged defect through"
    );
    // Git sends a hook's output to its own stderr.
    assert!(
        stderr(&refused).contains("clippy::todo"),
        "{}",
        stderr(&refused)
    );

    git(&root, &["add", "src/lib.rs"]);
    let accepted = commit(&root, "clean");
    assert!(
        accepted.status.success(),
        "{}{}",
        stdout(&accepted),
        stderr(&accepted)
    );

    let again = install(&root, &["git"]);
    assert_eq!(again.status.code(), Some(0));
    assert!(
        stdout(&again).contains("already installed"),
        "{}",
        stdout(&again)
    );
    fs::remove_dir_all(root).unwrap();
}

/// Each way a repository already runs hooks decides where the hook goes, and
/// nothing rust-doctor did not write is ever written over.
#[test]
fn the_git_hook_follows_the_repository_hook_setup() {
    // A foreign hook is left alone, and the line to add is printed.
    let root = repository("git-foreign");
    fs::write(root.join(".git/hooks/pre-commit"), "#!/bin/sh\nmake lint\n").unwrap();
    let foreign = install(&root, &["git"]);
    assert_eq!(foreign.status.code(), Some(1));
    assert!(stdout(&foreign).contains("rust-doctor . --yes --staged --scope lines"));
    assert_eq!(
        fs::read_to_string(root.join(".git/hooks/pre-commit")).unwrap(),
        "#!/bin/sh\nmake lint\n"
    );
    fs::remove_dir_all(&root).unwrap();

    // core.hooksPath inside the repository.
    let root = repository("git-hooks-path");
    git(&root, &["config", "core.hooksPath", ".githooks"]);
    let written = install(&root, &["git"]);
    assert_eq!(written.status.code(), Some(0), "{}", stderr(&written));
    assert!(root.join(".githooks/pre-commit").is_file());
    assert!(!root.join(".git/hooks/pre-commit").exists());
    fs::remove_dir_all(&root).unwrap();

    // A hooks directory shared beyond this working tree is never written.
    let root = repository("git-hooks-outside");
    let shared = outside_git("git-shared-hooks");
    git(&root, &["config", "core.hooksPath", shared.to_str().unwrap()]);
    let refused = install(&root, &["git"]);
    assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
    assert!(stdout(&refused).contains("outside this working tree"));
    assert_eq!(fs::read_dir(&shared).unwrap().count(), 0);
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(shared).unwrap();

    // cargo-husky.
    let root = repository("git-husky");
    fs::create_dir_all(root.join(".cargo-husky/hooks")).unwrap();
    assert_eq!(install(&root, &["git"]).status.code(), Some(0));
    assert!(root.join(".cargo-husky/hooks/pre-commit").is_file());
    fs::remove_dir_all(&root).unwrap();

    // A hook manager: the snippet is printed and nothing is written.
    for config in [".pre-commit-config.yaml", "lefthook.yml"] {
        let root = repository("git-manager");
        fs::write(root.join(config), "# managed\n").unwrap();
        let printed = install(&root, &["git"]);
        assert_eq!(printed.status.code(), Some(0));
        assert!(stdout(&printed).contains(config), "{}", stdout(&printed));
        assert!(stdout(&printed).contains("--staged --scope lines"));
        assert!(!root.join(".git/hooks/pre-commit").exists());
        assert_eq!(
            fs::read_to_string(root.join(config)).unwrap(),
            "# managed\n"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    // Not a repository.
    let outside = outside_git("git-outside");
    let refused = install(&outside, &["git"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(stderr(&refused).contains("not a git repository"));
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn the_claude_hook_merges_one_entry_into_the_settings() {
    let root = workspace("claude-merge");
    fs::create_dir_all(root.join(".claude")).unwrap();
    let settings = root.join(".claude/settings.local.json");
    fs::write(
        &settings,
        r#"{"permissions":{"allow":["Bash(ls)"]},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}],"PreToolUse":[]}}"#,
    )
    .unwrap();

    let dry = install(&root, &["claude", "--dry-run"]);
    assert_eq!(dry.status.code(), Some(0));
    assert!(stdout(&dry).starts_with("Would add a Stop hook to .claude/settings.local.json: "));
    assert!(
        !fs::read_to_string(&settings)
            .unwrap()
            .contains("rust-doctor")
    );

    let added = install(&root, &["claude"]);
    assert_eq!(added.status.code(), Some(0), "{}", stderr(&added));
    assert!(
        stdout(&added).starts_with("Added a Stop hook to .claude/settings.local.json: "),
        "{}",
        stdout(&added)
    );
    let merged: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(merged["permissions"]["allow"], json!(["Bash(ls)"]));
    assert_eq!(merged["hooks"]["PreToolUse"], json!([]));
    assert_eq!(merged["hooks"]["Stop"][0]["hooks"][0]["command"], "other");
    assert_eq!(
        merged["hooks"]["Stop"][1],
        json!({"hooks": [{"type": "command", "command": "rust-doctor hook run claude", "timeout": 600}]})
    );

    let again = install(&root, &["claude"]);
    assert_eq!(again.status.code(), Some(0));
    assert!(stdout(&again).contains("already in"));
    let unchanged: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(unchanged, merged);

    let shared = install(&root, &["claude", "--shared"]);
    assert_eq!(shared.status.code(), Some(0));
    assert!(root.join(".claude/settings.json").is_file());

    let cursor = install(&root, &["cursor"]);
    assert_eq!(cursor.status.code(), Some(0));
    let hooks: Value =
        serde_json::from_str(&fs::read_to_string(root.join(".cursor/hooks.json")).unwrap())
            .unwrap();
    assert_eq!(
        hooks,
        json!({"version": 1, "hooks": {"stop": [{"command": "rust-doctor hook run cursor"}]}})
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_settings_file_that_does_not_parse_is_left_alone() {
    let root = workspace("claude-invalid");
    fs::create_dir_all(root.join(".claude")).unwrap();
    let settings = root.join(".claude/settings.local.json");
    fs::write(&settings, "{\n  \"hooks\": {,\n}\n").unwrap();
    let refused = install(&root, &["claude"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(
        stderr(&refused).contains(
            "Could not parse .claude/settings.local.json at line 2, column 13: nothing was written."
        ),
        "{}",
        stderr(&refused)
    );
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        "{\n  \"hooks\": {,\n}\n"
    );
    fs::remove_dir_all(&root).unwrap();

    // A symlinked settings directory is never written through.
    let root = workspace("claude-symlink");
    let elsewhere = workspace("claude-symlink-target");
    std::os::unix::fs::symlink(&elsewhere, root.join(".claude")).unwrap();
    let refused = install(&root, &["claude"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(
        stderr(&refused).contains(".claude is a symlink"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(elsewhere).unwrap();
}

fn run_hook(agent: &str, root: &Path, input: &str) -> Output {
    let mut child = binary()
        .args(["hook", "run", agent])
        .current_dir(root)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// The end-of-turn hook blocks on a finding in the lines the turn changed,
/// names it with the rescan command, and lets a clean turn end.
#[test]
fn the_stop_hook_blocks_on_what_the_turn_introduced() {
    let root = repository("stop");
    let stop = r#"{"hook_event_name":"Stop","stop_hook_active":false}"#;
    let clean = run_hook("claude", &root, stop);
    assert_eq!(clean.status.code(), Some(0), "{}", stderr(&clean));

    fs::write(
        root.join("src/lib.rs"),
        "pub fn value() -> u8 { 1 }\npub fn pending() { todo!() }\n",
    )
    .unwrap();
    let blocked = run_hook("claude", &root, stop);
    assert_eq!(blocked.status.code(), Some(2), "{}", stderr(&blocked));
    let message = stderr(&blocked);
    assert!(
        message.contains("- clippy::todo src/lib.rs:2 "),
        "{message}"
    );
    assert!(
        message.ends_with(
            "Fix them, then rescan: rust-doctor . --scope lines --base HEAD --include-untracked --blocking warning\n"
        ),
        "{message}"
    );

    // An untracked file the turn created is judged whole.
    git(&root, &["checkout", "--", "src/lib.rs"]);
    fs::write(root.join("src/extra.rs"), "pub fn extra() { todo!() }\n").unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub mod extra;\npub fn value() -> u8 { 1 }\n",
    )
    .unwrap();
    let untracked = run_hook("claude", &root, stop);
    assert_eq!(untracked.status.code(), Some(2), "{}", stderr(&untracked));
    assert!(
        stderr(&untracked).contains("src/extra.rs:1"),
        "{}",
        stderr(&untracked)
    );

    // The loop guard: a turn already continuing because of this hook ends.
    let guarded = run_hook("claude", &root, r#"{"stop_hook_active":true}"#);
    assert_eq!(guarded.status.code(), Some(0));
    assert!(guarded.stderr.is_empty());

    // Cursor cannot block, so the same findings become a follow-up message.
    let cursor = run_hook("cursor", &root, r#"{"status":"completed","loop_count":0}"#);
    assert_eq!(cursor.status.code(), Some(0));
    let followup: Value = serde_json::from_slice(&cursor.stdout).unwrap();
    assert!(
        followup["followup_message"]
            .as_str()
            .unwrap()
            .contains("src/extra.rs:1")
    );
    fs::remove_dir_all(root).unwrap();
}

/// A scan that cannot run is the tool's failure, not the agent's, and never
/// traps it in its turn.
#[test]
fn the_stop_hook_never_traps_the_agent_on_a_failed_scan() {
    let root = outside_git("stop-unscannable");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"outside-git\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn pending() { todo!() }\n").unwrap();
    let output = run_hook("claude", &root, "{}");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("rust-doctor could not scan (scope: "),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output)
            .trim_end()
            .ends_with("not blocking this turn.")
    );
    fs::remove_dir_all(root).unwrap();
}
