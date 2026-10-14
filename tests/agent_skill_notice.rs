#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! The line an agent reads on stderr when its rust-doctor skill is missing or
//! was written by another version, and the silence when it is current.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::AtomicUsize;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn workspace() -> PathBuf {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kernel-contract/todo");
    let root = support::fresh_workspace("agent-skill-notice", &COUNTER);
    for file in ["Cargo.toml", "Cargo.lock"] {
        fs::copy(fixture.join(file), root.join(file)).unwrap();
    }
    support::copy_tree(&fixture.join("src"), &root.join("src"));
    root
}

fn scan(root: &Path, json: bool) -> Output {
    let mut command = support::rust_doctor();
    command
        .arg(root)
        .args(["--scope", "full"])
        .args(support::NO_ADVISORY_DB)
        .env("CLAUDECODE", "1")
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CARGO_NET_OFFLINE", "true");
    if json {
        command.arg("--json");
    }
    command.output().unwrap()
}

fn skill_lines(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| line.contains("skill"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn an_agent_hears_once_that_its_skill_is_missing_stale_or_nothing_when_current() {
    let root = workspace();
    let missing = scan(&root, false);
    assert_eq!(missing.status.code(), Some(0));
    assert_eq!(
        skill_lines(&missing),
        ["rust-doctor: no rust-doctor skill is installed for claude; run `rust-doctor skill install --agent claude`."]
    );
    assert!(!root.join(".claude").exists(), "the notice writes nothing");
    assert!(skill_lines(&scan(&root, true)).is_empty(), "--json prints nothing");

    let installed = support::rust_doctor()
        .args(["skill", "install"])
        .arg(&root)
        .args(["--agent", "claude"])
        .output()
        .unwrap();
    assert!(installed.status.success());
    assert!(skill_lines(&scan(&root, false)).is_empty(), "a current skill is silent");

    let skill = root.join(".claude/skills/rust-doctor/SKILL.md");
    let current = fs::read_to_string(&skill).unwrap();
    let version = env!("CARGO_PKG_VERSION");
    fs::write(&skill, current.replacen(&format!("rust-doctor {version}."), "rust-doctor 0.0.1.", 1)).unwrap();
    assert_eq!(
        skill_lines(&scan(&root, false)),
        [format!(
            "rust-doctor: the claude skill was written by rust-doctor 0.0.1, and this is {version}; run `rust-doctor skill install --agent claude --update`."
        )]
    );
}
