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

/// EP-005 US-021: an end-of-turn hook installed through `setup` says what it
/// runs and with whose authority, once per hook, and a dry run says nothing.
#[test]
fn setup_states_the_compile_authority_of_the_hooks_it_installs() {
    let root = repository();
    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::create_dir_all(root.join(".cursor")).unwrap();
    let path = path_with(&root, false);
    let authority = "Note: this hook runs `cargo clippy`, which executes the build scripts and \
                     procedural macros of the tree the agent wrote, at the end of every turn and \
                     with your account's authority. `--no-clippy` installs a hook that compiles nothing.";

    let planned = setup(&root, &path, &["--dry-run"]);
    assert!(planned.status.success(), "{}", String::from_utf8_lossy(&planned.stderr));
    assert!(!String::from_utf8_lossy(&planned.stderr).contains(authority));

    let output = setup(&root, &path, &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.matches(authority).count(), 2, "{stderr}");
    assert!(root.join(".claude/settings.local.json").is_file());
    assert!(root.join(".cursor/hooks.json").is_file());
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

/// Every byte of `output` a terminal could act on, `\n` aside.
fn controls(output: &Output) -> Vec<char> {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.chars()
        .filter(|character| character.is_control() && *character != '\n')
        .collect()
}

fn git(root: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?}");
}

/// A clone of a repository that commits a terminal escape everywhere an
/// installer prints from: the directory `.cargo-husky/hooks` links to, the
/// `rust-doctor` line of the marker hook in it, a workflow's file name, and C1
/// controls in the Claude settings' hook.
fn hostile_clone() -> PathBuf {
    let origin = repository();
    let directory = "\u{1b}[2Jhooks\u{1b}]0;title\u{7}";
    fs::create_dir_all(origin.join(directory)).unwrap();
    fs::write(
        origin.join(directory).join("pre-commit"),
        "#!/bin/sh\n# rust-doctor pre-commit hook\nrust-doctor \u{1b}]52;c;cHduZWQ=\u{7} . --yes\n",
    )
    .unwrap();
    fs::create_dir_all(origin.join(".cargo-husky")).unwrap();
    std::os::unix::fs::symlink(format!("../{directory}"), origin.join(".cargo-husky/hooks")).unwrap();
    fs::create_dir_all(origin.join(".github/workflows")).unwrap();
    fs::write(
        origin.join(".github/workflows/\u{1b}[31mscan.yml"),
        "jobs:\n  scan:\n    steps:\n      - uses: arthjean/rust-doctor@v0\n",
    )
    .unwrap();
    fs::create_dir_all(origin.join(".claude")).unwrap();
    fs::write(
        origin.join(".claude/settings.local.json"),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"rust-doctor hook run claude","timeout":7777,"note":"\u009b2J\u009d0;title\u009c"}]}]}}"#,
    )
    .unwrap();
    // Forced: a global excludes file commonly ignores `settings.local.json`.
    git(&origin, &["add", "-A", "-f"]);
    git(
        &origin,
        &["-c", "user.name=t", "-c", "user.email=t@t.invalid", "commit", "-qm", "hostile"],
    );
    let clone = origin.with_file_name(format!(
        "{}-clone",
        origin.file_name().unwrap().to_string_lossy()
    ));
    let _ = fs::remove_dir_all(&clone);
    let status = Command::new("git")
        .args(["clone", "-q", "--no-hardlinks"])
        .arg(&origin)
        .arg(&clone)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(status.success());
    clone
}

/// No name, line or settings value a cloned repository commits reaches the
/// terminal as anything but text, through any installer that prints it.
#[test]
fn the_installers_print_no_control_character_a_clone_commits() {
    let clone = hostile_clone();
    let path = path_with(&clone, false);
    let run = |arguments: &[&str]| {
        support::rust_doctor()
            .args(arguments)
            .arg(&clone)
            .env("PATH", &path)
            .output()
            .unwrap()
    };
    let setup = run(&["setup", "--dry-run"]);
    let git_hook = run(&["hook", "install", "git", "--update", "--dry-run"]);
    let ci = run(&["ci", "install", "--branch", "main", "--dry-run"]);
    let claude = run(&["hook", "install", "claude", "--update", "--dry-run"]);
    let text = |output: &Output| {
        format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
    };
    // Each sink is reached, so the absence below is the gate's doing.
    assert!(text(&setup).contains("becomes"), "{}", text(&setup));
    // The gate drops each escape sequence whole, so the names print bare.
    assert!(
        text(&git_hook).contains("Would update hooks/pre-commit: `rust-doctor  . --yes` becomes"),
        "{}",
        text(&git_hook)
    );
    assert!(text(&ci).contains(".github/workflows/scan.yml already runs rust-doctor"), "{}", text(&ci));
    assert!(text(&claude).contains("7777"), "{}", text(&claude));
    for (name, output) in [("setup", &setup), ("hook git", &git_hook), ("ci", &ci), ("hook claude", &claude)] {
        assert_eq!(controls(output), Vec::<char>::new(), "{name}: {}", text(output));
    }
}

/// A workspace whose path from the repository's top holds a control
/// character is refused before the hook that would carry it is written.
#[test]
fn a_workspace_path_holding_a_control_character_installs_no_hook() {
    let root = repository();
    let workspace = root.join("crates/\u{1b}[31mx");
    fs::create_dir_all(&workspace).unwrap();
    let path = path_with(&root, false);
    let output = support::rust_doctor()
        .args(["hook", "install", "git"])
        .arg(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("control character"));
    assert!(!root.join(".git/hooks/pre-commit").exists());
    assert_eq!(controls(&output), Vec::<char>::new());
}
