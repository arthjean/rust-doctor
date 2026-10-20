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

    // Codex has no end-of-turn hook, so the pre-commit hook is its gate, and
    // it blocks on a warning as the Stop hook does.
    let hook = root.join(".git/hooks/pre-commit");
    assert!(
        fs::read_to_string(&hook)
            .unwrap()
            .contains("\nrust-doctor . --yes --staged --scope lines --blocking warning\n")
    );

    // A second run refreshes in place rather than refusing, rewriting the hook
    // an earlier release wrote.
    fs::write(
        &hook,
        "#!/bin/sh\n# rust-doctor pre-commit hook: judges what this commit records.\nif command -v rust-doctor >/dev/null 2>&1; then\n  exec rust-doctor . --yes --staged --scope lines\nfi\n",
    )
    .unwrap();
    let refreshed = setup(&root, &path, &[]);
    assert!(refreshed.status.success(), "{}", String::from_utf8_lossy(&refreshed.stderr));
    assert!(fs::read_to_string(&hook).unwrap().contains("--blocking warning"));
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

/// EP-002 US-007: a repository whose one Cargo package sits at `member`.
fn repository_with_member(member: &str) -> PathBuf {
    let root = repository();
    let crate_root = root.join(member);
    fs::create_dir_all(crate_root.join("src")).unwrap();
    fs::write(
        crate_root.join("Cargo.toml"),
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(crate_root.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    root
}

/// `arguments`, then `path`, then `tail`, run from `directory`.
fn run_in(directory: &Path, path: &OsString, arguments: &[&str], target: &Path, tail: &[&str]) -> Output {
    support::rust_doctor()
        .args(arguments)
        .arg(target)
        .args(tail)
        .current_dir(directory)
        .env("PATH", path)
        .output()
        .unwrap()
}

/// EP-002 US-007: `crates/x` from the repository's root is `<root>/crates/x`.
/// Git used to receive the relative path twice, as its directory and as
/// `-C`, and looked for `crates/x/crates/x`.
#[test]
fn a_relative_member_path_behaves_like_its_absolute_form() {
    let root = repository_with_member("crates/x");
    let path = path_with(&root, true);
    let commands: [(&[&str], &[&str]); 3] = [
        (&["hook", "install", "git"], &["--dry-run"]),
        (&["ci", "install"], &["--dry-run", "--branch", "main"]),
        (&["setup"], &["--dry-run"]),
    ];
    for (arguments, tail) in commands {
        let relative = run_in(&root, &path, arguments, Path::new("crates/x"), tail);
        let absolute = run_in(&root, &path, arguments, &root.join("crates/x"), tail);
        assert!(relative.status.success(), "{arguments:?}: {}", String::from_utf8_lossy(&relative.stderr));
        assert_eq!(relative.status.code(), absolute.status.code(), "{arguments:?}");
        assert_eq!(relative.stdout, absolute.stdout, "{arguments:?}");
        assert_eq!(relative.stderr, absolute.stderr, "{arguments:?}");
    }
    let installed = run_in(&root, &path, &["hook", "install", "git"], Path::new("crates/x"), &[]);
    assert!(installed.status.success(), "{}", String::from_utf8_lossy(&installed.stderr));
    let hook = fs::read_to_string(root.join(".git/hooks/pre-commit")).unwrap();
    assert!(hook.contains("rust-doctor crates/x --yes"), "{hook}");
}

/// EP-002 US-007: a PATH that names nothing is refused before anything is
/// created for it.
#[test]
fn a_missing_path_is_refused_by_every_installer_and_creates_nothing() {
    let root = repository_with_member("crates/x");
    let path = path_with(&root, true);
    let before = support::content_states(&root);
    let commands: [&[&str]; 7] = [
        &["hook", "install", "git"],
        &["hook", "install", "claude"],
        &["hook", "install", "cursor"],
        &["ci", "install"],
        &["setup"],
        &["skill", "install"],
        &["rules", "explain", "todo"],
    ];
    for arguments in commands {
        let output = run_in(&root, &path, arguments, Path::new("crates/y"), &[]);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "rust-doctor: PATH: crates/y does not exist.\n",
            "{arguments:?}"
        );
    }
    assert_eq!(support::content_states(&root), before);
    assert!(!root.join("crates/y").exists());
}

/// EP-002 US-007: a path holding spaces and non-ASCII characters reaches every
/// installer intact. `ci install` is left out: its workflow carries the path
/// as a bare YAML scalar and shell word, so it refuses one by design.
#[test]
fn a_path_with_spaces_and_non_ascii_characters_installs() {
    let member = "crates/caf\u{e9} au lait";
    let root = repository_with_member(member);
    let path = path_with(&root, true);
    for arguments in [
        &["hook", "install", "git"][..],
        &["hook", "install", "claude"],
        &["skill", "install"],
        &["setup"],
    ] {
        let output = run_in(&root, &path, arguments, Path::new(member), &[]);
        assert!(output.status.success(), "{arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
    let hook = fs::read_to_string(root.join(".git/hooks/pre-commit")).unwrap();
    assert!(hook.contains(&format!("rust-doctor '{member}' --yes")), "{hook}");
    assert!(root.join(member).join(".claude/settings.local.json").is_file());
    assert!(root.join(member).join(".claude/skills/rust-doctor/SKILL.md").is_file());
}
