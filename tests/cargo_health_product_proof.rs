#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

mod support;

const REGISTRY_CODE: &str = "rust_doctor::cargo::unbounded_registry_dependency";
const GIT_CODE: &str = "rust_doctor::cargo::unpinned_git_dependency";

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cargo-health")
        .join(name)
}

fn temporary_workspace(label: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("cargo-health-product-proof")
        .join(format!(
            "{}-{}-{label}",
            std::process::id(),
            NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed)
        ));
    if path.exists() {
        fs::remove_dir_all(&path).expect("stale product proof workspace should be removable");
    }
    fs::create_dir_all(&path).expect("product proof workspace should be creatable");
    path
}

fn run(command: &mut Command, description: &str) -> Output {
    let output = command.output().expect("proof command should start");
    assert!(
        output.status.success(),
        "{description} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn inspect(path: &Path, json: bool, cargo_home: &Path, target: &Path) -> Output {
    inspect_with(path, json, false, cargo_home, target)
}

fn inspect_with(
    path: &Path,
    json: bool,
    verbose: bool,
    cargo_home: &Path,
    target: &Path,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rust-doctor"));
    command.arg("inspect");
    if json {
        command.arg("--json");
    }
    if verbose {
        command.arg("--verbose");
    }
    command
        .arg(path)
        .env("CARGO_HOME", cargo_home)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", target);
    command.output().expect("rust-doctor should start")
}

fn report(output: &Output) -> Value {
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    serde_json::from_slice(&output.stdout).expect("stdout should contain one JSON report")
}

fn native_findings(report: &Value) -> Vec<&Value> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter(|diagnostic| diagnostic["source"] == "rust-doctor")
        .collect()
}

fn non_native_ids(report: &Value) -> BTreeSet<&str> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter(|diagnostic| diagnostic["source"] != "rust-doctor")
        .map(|diagnostic| {
            diagnostic["id"]
                .as_str()
                .expect("diagnostic ID should be a string")
        })
        .collect()
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("fixture destination should be creatable");
    let mut entries: Vec<_> = fs::read_dir(source)
        .expect("fixture directory should be readable")
        .map(|entry| entry.expect("fixture entry should be readable").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.file_name().is_some_and(|name| name == "target") {
            continue;
        }
        let target = destination.join(
            path.file_name()
                .expect("fixture entry should have a file name"),
        );
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, target).expect("fixture file should copy");
        }
    }
}

fn content_hashes(root: &Path) -> BTreeMap<String, blake3::Hash> {
    fn visit(root: &Path, directory: &Path, hashes: &mut BTreeMap<String, blake3::Hash>) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .expect("fixture directory should be readable")
            .map(|entry| entry.expect("fixture entry should be readable").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                if path
                    .file_name()
                    .is_some_and(|name| name == "target" || name == ".git")
                {
                    continue;
                }
                visit(root, &path, hashes);
            } else if path.file_name().is_none_or(|name| name != "Cargo.lock") {
                let relative = path
                    .strip_prefix(root)
                    .expect("fixture file should be below its root")
                    .to_string_lossy()
                    .into_owned();
                hashes.insert(
                    relative,
                    blake3::hash(&fs::read(path).expect("fixture file should be readable")),
                );
            }
        }
    }

    let mut hashes = BTreeMap::new();
    visit(root, root, &mut hashes);
    hashes
}

fn replace(path: &Path, from: &str, to: &str) {
    let source = fs::read_to_string(path).expect("fixture file should be readable");
    assert!(source.contains(from), "replacement source should exist");
    fs::write(path, source.replace(from, to)).expect("fixture file should be writable");
}

fn init_git_dependency(root: &Path) -> String {
    fs::create_dir_all(root.join("src")).expect("Git dependency source directory should exist");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"local-git-dependency\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("Git dependency manifest should be writable");
    fs::write(root.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n")
        .expect("Git dependency source should be writable");

    run(
        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(root),
        "git init",
    );
    run(
        Command::new("git")
            .args(["config", "user.name", "Rust Doctor"])
            .current_dir(root),
        "git config user.name",
    );
    run(
        Command::new("git")
            .args(["config", "user.email", "rust-doctor@example.invalid"])
            .current_dir(root),
        "git config user.email",
    );
    run(
        Command::new("git").args(["add", "."]).current_dir(root),
        "git add",
    );
    run(
        Command::new("git")
            .args(["commit", "-m", "fixture"])
            .current_dir(root),
        "git commit",
    );
    let output = run(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root),
        "git rev-parse",
    );
    String::from_utf8(output.stdout)
        .expect("commit should be UTF-8")
        .trim()
        .to_owned()
}

struct GitFixture {
    project: PathBuf,
    dependency: PathBuf,
    revision: String,
    cargo_home: PathBuf,
    target: PathBuf,
}

fn local_git_fixture(root: &Path) -> GitFixture {
    let dependency = root.join("dependency");
    let revision = init_git_dependency(&dependency);
    let project = root.join("project");
    fs::create_dir_all(project.join("src")).expect("Git fixture source directory should exist");
    let dependency_url = format!("file://{}", dependency.display());
    fs::write(
        project.join("Cargo.toml"),
        format!(
            "[package]\nname = \"cargo-health-local-git\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nlocal_git_alias = {{ package = \"local-git-dependency\", git = \"{dependency_url}\", branch = \"main\" }}\n"
        ),
    )
    .expect("Git fixture manifest should be writable");
    fs::write(
        project.join("src/lib.rs"),
        "pub fn git_fixture() -> u8 { local_git_alias::value() }\n",
    )
    .expect("Git fixture source should be writable");

    let cargo_home = root.join("cargo-home");
    let target = root.join("target");
    run(
        Command::new(env!("CARGO"))
            .args(["fetch", "--manifest-path"])
            .arg(project.join("Cargo.toml"))
            .env("CARGO_HOME", &cargo_home)
            .env("CARGO_TARGET_DIR", &target),
        "local Git fixture fetch",
    );

    GitFixture {
        project,
        dependency,
        revision,
        cargo_home,
        target,
    }
}

fn assert_private_output(bytes: &[u8], forbidden_path: &Path) {
    let output = String::from_utf8_lossy(bytes);
    for secret in [
        "file://",
        "git+",
        "?branch=",
        "#",
        "\u{1b}",
        forbidden_path.to_string_lossy().as_ref(),
    ] {
        assert!(!output.contains(secret), "output leaked {secret:?}");
    }
}

#[test]
fn offline_registry_cli_and_renderers_share_the_normative_finding() {
    let fixture = fixture("offline-registry");
    let before = content_hashes(&fixture);
    let root = temporary_workspace("registry");
    let output = inspect(
        &fixture,
        true,
        &root.join("cargo-home"),
        &root.join("target"),
    );
    assert_eq!(output.status.code(), Some(0));
    let report = report(&output);
    assert_eq!(report["schema_version"], 13);
    assert_eq!(report["status"], "complete");
    assert_eq!(report["complete"], true);
    let findings = native_findings(&report);
    assert_eq!(findings.len(), 1);
    let finding = findings[0];
    assert_eq!(finding["source"], "rust-doctor");
    assert_eq!(finding["code"], REGISTRY_CODE);
    assert_eq!(finding["severity"], "warning");
    assert_eq!(finding["category"], "reliability");
    assert_eq!(
        finding["message"],
        "Registry dependency \"fixture_registry_alias\" uses an unbounded \"*\" version requirement."
    );
    assert_eq!(
        finding["help"],
        "Replace the unbounded version requirement with the minimum compatible version intended by the project."
    );
    assert_eq!(finding["package"], "cargo-health-offline-registry");
    assert_eq!(finding["path"], "Cargo.toml");
    assert!(finding["target"].is_null());
    assert!(finding["span"].is_null());
    assert_eq!(
        report["scan"]["command"]
            .as_array()
            .expect("a scan should publish its command")
            .iter()
            .map(|argument| argument.as_str().unwrap_or_default().to_owned())
            .collect::<Vec<_>>(),
        support::expected_clippy_command(&report["policy"])
    );

    // `--verbose` lists every group: the default rendering shows only one, and
    // the ordering follows the contribution, now weighted by the occurrence
    // count.
    let terminal = inspect_with(
        &fixture,
        false,
        true,
        &root.join("cargo-home"),
        &root.join("target"),
    );
    assert_eq!(terminal.status.code(), Some(0));
    let terminal = String::from_utf8(terminal.stdout).expect("terminal output should be UTF-8");
    assert!(
        terminal.contains(&format!("Rule ID: {REGISTRY_CODE}")),
        "{terminal}"
    );
    assert!(terminal.contains("Help: Replace the unbounded version requirement with the minimum"));
    assert!(terminal.contains("version intended by the project."));
    assert_eq!(content_hashes(&fixture), before);
    fs::remove_dir_all(root).expect("registry proof workspace should be removable");
}

#[test]
fn local_git_cli_is_offline_private_and_full_revision_clears_only_its_id() {
    let root = temporary_workspace("git");
    let fixture = local_git_fixture(&root);
    let dependency_before = run(
        Command::new("git")
            .args(["status", "--short", "--untracked-files=no"])
            .current_dir(&fixture.dependency),
        "dependency status before inspection",
    )
    .stdout;
    let before = inspect(&fixture.project, true, &fixture.cargo_home, &fixture.target);
    assert_eq!(before.status.code(), Some(0));
    let before_report = report(&before);
    let findings = native_findings(&before_report);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["code"], GIT_CODE);
    assert_eq!(
        findings[0]["message"],
        "Git dependency \"local_git_alias\" is not pinned to a full commit revision."
    );
    assert_private_output(&before.stdout, &fixture.dependency);

    replace(
        &fixture.project.join("Cargo.toml"),
        "branch = \"main\"",
        &format!("rev = \"{}\"", fixture.revision),
    );
    let corrected = inspect(&fixture.project, true, &fixture.cargo_home, &fixture.target);
    assert_eq!(corrected.status.code(), Some(0));
    let corrected_report = report(&corrected);
    assert!(native_findings(&corrected_report).is_empty());
    assert_eq!(
        non_native_ids(&corrected_report),
        non_native_ids(&before_report)
    );
    assert_private_output(&corrected.stdout, &fixture.dependency);

    let dependency_after = run(
        Command::new("git")
            .args(["status", "--short", "--untracked-files=no"])
            .current_dir(&fixture.dependency),
        "dependency status after inspection",
    )
    .stdout;
    assert_eq!(dependency_after, dependency_before);
    fs::remove_dir_all(root).expect("Git proof workspace should be removable");
}

#[test]
fn registry_correction_rescan_removes_only_the_targeted_id() {
    let root = temporary_workspace("registry-correction");
    let project = root.join("project");
    copy_tree(&fixture("offline-registry"), &project);
    let cargo_home = root.join("cargo-home");
    let target = root.join("target");

    let before = inspect(&project, true, &cargo_home, &target);
    assert_eq!(before.status.code(), Some(0));
    let before_report = report(&before);
    let finding = native_findings(&before_report)
        .into_iter()
        .find(|finding| finding["code"] == REGISTRY_CODE)
        .expect("registry finding should exist");
    let targeted_id = finding["id"]
        .as_str()
        .expect("registry finding ID should be a string")
        .to_owned();
    let untargeted_ids = non_native_ids(&before_report);
    assert!(!untargeted_ids.is_empty());

    replace(
        &project.join("Cargo.toml"),
        "version = \"*\"",
        "version = \"1\"",
    );
    let corrected = inspect(&project, true, &cargo_home, &target);
    assert_eq!(corrected.status.code(), Some(0));
    let corrected_report = report(&corrected);
    assert!(
        corrected_report["diagnostics"]
            .as_array()
            .expect("diagnostics should be an array")
            .iter()
            .all(|diagnostic| diagnostic["id"] != targeted_id)
    );
    assert!(native_findings(&corrected_report).is_empty());
    assert_eq!(non_native_ids(&corrected_report), untargeted_ids);
    fs::remove_dir_all(root).expect("registry correction workspace should be removable");
}

#[test]
fn compilation_failure_after_metadata_retains_native_finding_and_unique_causes() {
    let root = temporary_workspace("incomplete");
    let project = root.join("project");
    copy_tree(&fixture("offline-registry"), &project);
    fs::write(
        project.join("src/lib.rs"),
        "pub fn registry_fixture() -> u8 { let value: u8 = \"wrong\"; value }\n",
    )
    .expect("failing source should be writable");

    let output = inspect(
        &project,
        true,
        &root.join("cargo-home"),
        &root.join("target"),
    );
    assert_eq!(output.status.code(), Some(1));
    let report = report(&output);
    assert_eq!(report["status"], "incomplete");
    assert_eq!(report["complete"], false);
    assert!(
        native_findings(&report)
            .iter()
            .any(|finding| finding["code"] == REGISTRY_CODE)
    );
    let errors = report["errors"]
        .as_array()
        .expect("errors should be an array");
    let unique: BTreeSet<_> = errors
        .iter()
        .map(|error| {
            (
                error["stage"].as_str().expect("error stage"),
                error["code"].as_str().expect("error code"),
                error["message"].as_str().expect("error message"),
            )
        })
        .collect();
    assert_eq!(unique.len(), errors.len());
    assert!(
        unique
            .iter()
            .any(|(stage, code, _)| *stage == "execution" && *code == "clippy-exit")
    );
    fs::remove_dir_all(root).expect("incomplete proof workspace should be removable");
}
