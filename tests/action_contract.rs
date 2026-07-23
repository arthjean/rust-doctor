// Integration test crates are outside Clippy's allow-unwrap-in-tests handling.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn prepare(directory: &Path, runner_temp: &Path, scope: &str, base: &str) -> String {
    let output_file = runner_temp.join("github-output");
    let _ = fs::remove_file(&output_file);
    let status = Command::new("bash")
        .arg(format!(
            "{}/scripts/action/prepare.sh",
            env!("CARGO_MANIFEST_DIR")
        ))
        .env("INPUT_SCOPE", scope)
        .env("INPUT_DIRECTORY", directory)
        .env("INPUT_VERSION", env!("CARGO_PKG_VERSION"))
        .env("EVENT_NAME", "pull_request")
        .env("PR_BASE_SHA", base)
        .env("PR_NUMBER", "1")
        .env("REPOSITORY", "fork-owner/repository")
        .env("GH_TOKEN", "")
        .env("RUNNER_TEMP", runner_temp)
        .env("GITHUB_OUTPUT", &output_file)
        .status()
        .unwrap();
    assert!(status.success());
    fs::read_to_string(output_file).unwrap()
}

#[test]
fn action_declares_complete_contract_and_pins_third_party_actions() {
    let action = fs::read_to_string("action.yml").unwrap();
    for input in [
        "directory:",
        "project:",
        "scope:",
        "blocking:",
        "require-complete:",
        "comment:",
        "review-comments:",
        "commit-status:",
        "sarif:",
        "version:",
    ] {
        assert!(action.contains(input), "missing action input {input}");
    }
    for line in action
        .lines()
        .filter(|line| line.trim().starts_with("uses:"))
    {
        let reference = line.split('#').next().unwrap().trim();
        let revision = reference.rsplit_once('@').unwrap().1.trim();
        assert_eq!(revision.len(), 40, "action is not commit-pinned: {line}");
        assert!(
            revision
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        );
    }
    assert!(action.contains("scripts/action/prepare.sh"));
    assert!(action.contains("scripts/action/report.sh"));
    assert!(action.contains("scripts/action/sarif.sh"));
}

#[test]
fn changed_path_resolution_preserves_unusual_rust_filenames() {
    let repository = tempfile::tempdir().unwrap();
    let root = repository.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "tests@rust-doctor.local"]);
    git(root, &["config", "user.name", "Rust Doctor Tests"]);
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn baseline() {}\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "baseline"]);
    let base = git(root, &["rev-parse", "HEAD"]);

    let unusual = ["src/space name.rs", "src/tab\tname.rs", "src/new\nline.rs"];
    for path in unusual {
        fs::write(root.join(path), "pub fn changed() {}\n").unwrap();
    }
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "head"]);

    let output_file = root.join("github-output");
    let status = Command::new("bash")
        .arg(format!(
            "{}/scripts/action/prepare.sh",
            env!("CARGO_MANIFEST_DIR")
        ))
        .env("INPUT_SCOPE", "baseline")
        .env("INPUT_DIRECTORY", root)
        .env("INPUT_VERSION", env!("CARGO_PKG_VERSION"))
        .env("EVENT_NAME", "pull_request")
        .env("PR_BASE_SHA", base)
        .env("PR_NUMBER", "1")
        .env("REPOSITORY", "example/repository")
        .env("GH_TOKEN", "")
        .env("RUNNER_TEMP", root)
        .env("GITHUB_OUTPUT", &output_file)
        .status()
        .unwrap();
    assert!(status.success());

    let outputs = fs::read_to_string(output_file).unwrap();
    let changed_file = outputs
        .lines()
        .find_map(|line| line.strip_prefix("changed-paths-file="))
        .unwrap();
    let changed = fs::read(changed_file).unwrap();
    let paths: Vec<_> = changed
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8(path.to_vec()).unwrap())
        .collect();
    for path in unusual {
        assert!(paths.iter().any(|candidate| candidate == path));
    }
    assert!(outputs.contains("scope=baseline"));
    assert!(outputs.contains("skip=false"));
}

#[test]
fn explicit_and_degraded_full_scopes_never_skip_a_pull_request() {
    let repository = tempfile::tempdir().unwrap();
    let root = repository.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "tests@rust-doctor.local"]);
    git(root, &["config", "user.name", "Rust Doctor Tests"]);
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "baseline"]);

    let explicit = prepare(root, root, "full", "");
    assert!(explicit.contains("scope=full"));
    assert!(explicit.contains("skip=false"));

    let degraded = prepare(
        root,
        root,
        "baseline",
        "0000000000000000000000000000000000000000",
    );
    assert!(degraded.contains("scope=full"));
    assert!(degraded.contains("skip=false"));
    assert!(degraded.contains("base history unavailable; running a full scan"));
}

#[test]
fn absolute_subdirectory_in_nested_detached_checkout_resolves_inner_repository() {
    let outer = tempfile::tempdir().unwrap();
    git(outer.path(), &["init", "-q"]);
    let inner = outer.path().join("nested");
    fs::create_dir(&inner).unwrap();
    git(&inner, &["init", "-q"]);
    git(&inner, &["config", "user.email", "tests@rust-doctor.local"]);
    git(&inner, &["config", "user.name", "Rust Doctor Tests"]);
    fs::create_dir(inner.join("src")).unwrap();
    fs::write(
        inner.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    fs::write(inner.join("src/lib.rs"), "pub fn nested() {}\n").unwrap();
    git(&inner, &["add", "."]);
    git(&inner, &["commit", "-qm", "baseline"]);
    git(&inner, &["checkout", "--detach", "-q"]);

    let outputs = prepare(&inner.join("src"), &inner, "full", "");
    assert!(outputs.contains(&format!("scan-root={}", inner.join("src").display())));
    assert!(outputs.contains(&format!("git-root={}", inner.display())));
    assert!(outputs.contains("skip=false"));
}
