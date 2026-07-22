#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const MANIFEST: &str = "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.85\"\n";
const FAST_CONFIG: &str = "lint = false\ndependencies = false\n";

#[test]
fn full_files_lines_and_untracked_scopes_report_exact_coverage() {
    let repository = tempfile::tempdir().unwrap();
    write_package(repository.path(), "fixture");
    init_repository(repository.path());

    let full = scan(repository.path(), &["--json", "--offline"]);
    assert_success(&full);
    let full = parse_report(&full);
    assert_eq!(full["mode"], "full");
    assert_eq!(full["execution_scope"], "full_packages");
    assert_eq!(full["completeness"]["planned_files"], 1);
    assert_eq!(full["completeness"]["analyzed_files"], 1);

    let files = scan(
        repository.path(),
        &[
            "--json",
            "--offline",
            "--scope",
            "files",
            "--files",
            "src/lib.rs",
        ],
    );
    assert_success(&files);
    let files = parse_report(&files);
    assert_eq!(files["mode"], "files");
    assert_eq!(files["reporting_scope"], "files");
    assert_eq!(files["completeness"]["planned_files"], 1);

    write(
        &repository.path().join("src/lib.rs"),
        "pub fn value() -> u8 {\n    2\n}\n",
    );
    write(
        &repository.path().join("src/untracked.rs"),
        "pub fn untracked() {}\n",
    );
    let lines = scan(
        repository.path(),
        &["--json", "--offline", "--scope", "lines", "--base", "HEAD"],
    );
    assert_success(&lines);
    let lines = parse_report(&lines);
    assert_eq!(lines["mode"], "lines");
    assert_eq!(lines["completeness"]["planned_files"], 1);

    let untracked = scan(
        repository.path(),
        &[
            "--json",
            "--offline",
            "--scope",
            "changed",
            "--base",
            "HEAD",
            "--include-untracked",
        ],
    );
    assert_success(&untracked);
    assert_eq!(parse_report(&untracked)["completeness"]["planned_files"], 2);
}

#[test]
fn staged_scope_scans_index_content_instead_of_the_worktree() {
    let repository = tempfile::tempdir().unwrap();
    write_package(repository.path(), "fixture");
    write(
        &repository.path().join("rust-doctor.toml"),
        "dependencies = false\n",
    );
    init_repository(repository.path());

    write(
        &repository.path().join("src/lib.rs"),
        "pub fn staged(value: Option<u8>) -> u8 { value.unwrap() }\n",
    );
    git(repository.path(), &["add", "src/lib.rs"]);
    let worktree = "pub fn working(value: Option<u8>) -> u8 { value.unwrap_or_default() }\n";
    write(&repository.path().join("src/lib.rs"), worktree);

    let output = scan(repository.path(), &["--json", "--offline", "--staged"]);
    assert_success(&output);
    let report = parse_report(&output);
    assert_eq!(report["mode"], "staged");
    assert_eq!(report["execution_scope"], "isolated_snapshot");
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| {
                finding["rule"] == "unwrap-in-production"
                    && finding["location"]["path"] == "src/lib.rs"
            })
    );
    assert_eq!(
        fs::read_to_string(repository.path().join("src/lib.rs")).unwrap(),
        worktree
    );
}

#[test]
fn workspace_defaults_star_and_changed_ownership_follow_cargo_metadata() {
    let repository = tempfile::tempdir().unwrap();
    write(
        &repository.path().join("Cargo.toml"),
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/a\", \"crates/b\"]\ndefault-members = [\"crates/a\"]\n",
    );
    write(&repository.path().join("rust-doctor.toml"), FAST_CONFIG);
    write_package(&repository.path().join("crates/a"), "a");
    write_package(&repository.path().join("crates/b"), "b");
    init_repository(repository.path());

    let default = scan(repository.path(), &["--json", "--offline"]);
    assert_success(&default);
    let default = parse_report(&default);
    assert_eq!(default["projects"].as_array().unwrap().len(), 1);
    assert_eq!(default["projects"][0]["package_root"], "crates/a");

    let all = scan(
        repository.path(),
        &["--json", "--offline", "--project", "*"],
    );
    assert_success(&all);
    assert_eq!(parse_report(&all)["projects"].as_array().unwrap().len(), 2);

    write(
        &repository.path().join("crates/b/src/lib.rs"),
        "pub fn b_changed() {}\n",
    );
    let changed = scan(
        repository.path(),
        &[
            "--json",
            "--offline",
            "--project",
            "*",
            "--scope",
            "changed",
            "--base",
            "HEAD",
        ],
    );
    assert_success(&changed);
    let changed = parse_report(&changed);
    assert_eq!(changed["projects"].as_array().unwrap().len(), 1);
    assert_eq!(changed["projects"][0]["package_root"], "crates/b");

    let unknown = scan(
        repository.path(),
        &["--json", "--offline", "--project", "missing"],
    );
    assert_eq!(unknown.status.code(), Some(2));
    let unknown = parse_report(&unknown);
    assert_eq!(unknown["outcome"], "failed");
    assert!(
        unknown["error"]["message"]
            .as_str()
            .unwrap()
            .contains("crates/a")
    );
}

#[test]
fn root_packages_exclusions_and_nested_path_members_follow_cargo_metadata() {
    let repository = tempfile::tempdir().unwrap();
    write_package(repository.path(), "root");
    write(
        &repository.path().join("Cargo.toml"),
        "[package]\nname = \"root\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.85\"\n\n[workspace]\nresolver = \"3\"\nmembers = [\"crates/a\"]\ndefault-members = [\".\"]\nexclude = [\"excluded\"]\n",
    );
    write_package(&repository.path().join("crates/a"), "a");
    write(
        &repository.path().join("crates/a/Cargo.toml"),
        "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.85\"\n\n[dependencies]\nshared = { path = \"../../shared\" }\n",
    );
    write_package(&repository.path().join("shared"), "shared");
    write_package(&repository.path().join("excluded"), "excluded");
    init_repository(repository.path());

    let default = scan(repository.path(), &["--json", "--offline"]);
    assert_success(&default);
    let default = parse_report(&default);
    assert_eq!(default["projects"].as_array().unwrap().len(), 1);
    assert_eq!(default["projects"][0]["package_root"], ".");

    let all = scan(
        repository.path(),
        &["--json", "--offline", "--project", "*"],
    );
    assert_success(&all);
    let roots: Vec<_> = parse_report(&all)["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|project| project["package_root"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        roots,
        [
            ".".to_string(),
            "crates/a".to_string(),
            "shared".to_string()
        ]
    );

    write(
        &repository.path().join("excluded/src/lib.rs"),
        "pub fn excluded_changed() {}\n",
    );
    write(
        &repository.path().join("shared/src/lib.rs"),
        "pub fn shared_changed() {}\n",
    );
    let changed = scan(
        repository.path(),
        &[
            "--json",
            "--offline",
            "--project",
            "*",
            "--scope",
            "changed",
            "--base",
            "HEAD",
        ],
    );
    assert_success(&changed);
    let changed = parse_report(&changed);
    assert_eq!(changed["projects"].as_array().unwrap().len(), 1);
    assert_eq!(changed["projects"][0]["package_root"], "shared");
}

#[test]
fn baseline_renames_are_isolated_and_shallow_history_fails_complete_gate() {
    let repository = tempfile::tempdir().unwrap();
    write_package(repository.path(), "fixture");
    init_repository(repository.path());
    git(repository.path(), &["mv", "src/lib.rs", "src/renamed.rs"]);
    write(
        &repository.path().join("Cargo.toml"),
        &format!("{MANIFEST}\n[lib]\npath = \"src/renamed.rs\"\n"),
    );
    let renamed = scan(
        repository.path(),
        &["--json", "--offline", "--baseline", "--base", "HEAD"],
    );
    assert_success(&renamed);
    let renamed = parse_report(&renamed);
    assert_eq!(renamed["mode"], "baseline");
    assert_eq!(renamed["baseline"]["baseline_degraded"], false);

    git(repository.path(), &["add", "."]);
    git(repository.path(), &["commit", "--quiet", "-m", "rename"]);
    let shallow = tempfile::tempdir().unwrap();
    let source_url = format!("file://{}", repository.path().display());
    let clone = Command::new("git")
        .args(["clone", "--quiet", "--depth", "1"])
        .arg(source_url)
        .arg(shallow.path())
        .output()
        .unwrap();
    assert!(
        clone.status.success(),
        "{}",
        String::from_utf8_lossy(&clone.stderr)
    );
    write(
        &shallow.path().join("src/renamed.rs"),
        "pub fn shallow_change() {}\n",
    );
    let degraded = scan(
        shallow.path(),
        &[
            "--json",
            "--offline",
            "--baseline",
            "--base",
            "HEAD~1",
            "--require-complete",
        ],
    );
    assert_eq!(degraded.status.code(), Some(3));
    let degraded = parse_report(&degraded);
    assert_eq!(degraded["baseline"]["baseline_degraded"], true);
    assert_eq!(degraded["summary"]["score_authoritative"], false);
}

#[test]
fn staged_scope_rejects_an_unresolved_index() {
    let repository = tempfile::tempdir().unwrap();
    write_package(repository.path(), "fixture");
    init_repository(repository.path());
    git(
        repository.path(),
        &["checkout", "--quiet", "-b", "conflict"],
    );
    write(
        &repository.path().join("src/lib.rs"),
        "pub fn side() -> u8 { 1 }\n",
    );
    git(repository.path(), &["add", "src/lib.rs"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "conflict side"],
    );
    git(repository.path(), &["checkout", "--quiet", "main"]);
    write(
        &repository.path().join("src/lib.rs"),
        "pub fn side() -> u8 { 2 }\n",
    );
    git(repository.path(), &["add", "src/lib.rs"]);
    git(repository.path(), &["commit", "--quiet", "-m", "main side"]);
    let merge = Command::new("git")
        .current_dir(repository.path())
        .args(["merge", "--quiet", "conflict"])
        .output()
        .unwrap();
    assert!(!merge.status.success());

    let output = scan(repository.path(), &["--json", "--offline", "--staged"]);
    assert_eq!(output.status.code(), Some(2));
    let report = parse_report(&output);
    assert_eq!(report["outcome"], "failed");
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unresolved")
    );
}

fn write_package(root: &Path, name: &str) {
    fs::create_dir_all(root.join("src")).unwrap();
    write(
        &root.join("Cargo.toml"),
        &MANIFEST.replace("name = \"fixture\"", &format!("name = \"{name}\"")),
    );
    write(
        &root.join("src/lib.rs"),
        &format!("pub fn {name}_value() -> u8 {{ 1 }}\n"),
    );
    if !root.join("rust-doctor.toml").exists() {
        write(&root.join("rust-doctor.toml"), FAST_CONFIG);
    }
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn init_repository(root: &Path) {
    git(root, &["init", "--quiet", "--initial-branch", "main"]);
    git(root, &["config", "user.email", "scope@example.com"]);
    git(root, &["config", "user.name", "Scope Fixture"]);
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "initial"]);
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn scan(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(root)
        .args(arguments)
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "scan failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON report ({error}): {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
