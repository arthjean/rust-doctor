#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::AtomicUsize;

use rust_doctor::{
    BlockingLevel, ExecutionScope, GateStatus, InspectRequest, ScopeMode, Status, inspect,
};
use serde_json::Value;

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/configuration-kernel/workspace")
}

fn successful(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git(root: &Path, arguments: &[&str]) -> Output {
    successful(Command::new("git").args(arguments).current_dir(root))
}

fn repository(scope: &str) -> PathBuf {
    let root = support::temporary_target(scope, &NEXT_WORKSPACE);
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    support::copy_tree(&fixture(), &root);
    fs::write(root.join(".gitignore"), "/target\n").unwrap();
    successful(
        Command::new(env!("CARGO"))
            .args(["generate-lockfile", "--offline"])
            .current_dir(&root),
    );
    git(&root, &["init", "--quiet"]);
    git(&root, &["config", "user.name", "Rust Doctor"]);
    git(
        &root,
        &["config", "user.email", "rust-doctor@example.invalid"],
    );
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    git(&root, &["branch", "baseline"]);
    fs::write(
        root.join("member/src/lib.rs"),
        "pub fn configuration_kernel_fixture() -> bool {\n    false\n}\n",
    )
    .unwrap();
    root
}

fn cli(path: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--json")
        .args(arguments)
        .arg(path)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn snapshot(root: &Path) -> Vec<Vec<u8>> {
    [
        vec!["rev-parse", "HEAD"],
        vec!["show-ref"],
        vec!["hash-object", ".git/index"],
        vec!["status", "--porcelain=v1", "-z"],
    ]
    .into_iter()
    .map(|arguments| git(root, &arguments).stdout)
    .collect()
}

#[cfg(unix)]
fn instrumented_cli(root: &Path, arguments: &[&str], scope: &str) -> (Output, Vec<String>) {
    let harness_root = support::temporary_target(scope, &NEXT_WORKSPACE);
    let processes = support::ProcessHarness::install_with_git(&harness_root);
    let real_git = support::resolve_program("git");
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--json")
        .args(arguments)
        .arg(root)
        .env("PATH", processes.command_path())
        .env("RUST_DOCTOR_REAL_GIT", real_git)
        .env("RUST_DOCTOR_REAL_CARGO", env!("CARGO"))
        .env("RUST_DOCTOR_REAL_RUSTC", support::resolve_program("rustc"))
        .env("RUST_DOCTOR_PROCESS_LOG", processes.log_path())
        .env("CARGO_NET_OFFLINE", "true")
        .env("GIT_DIR", "/private/hostile/repository")
        .env("GIT_WORK_TREE", "/private/hostile/worktree")
        .env("GIT_INDEX_FILE", "/private/hostile/index")
        .env("GIT_OBJECT_DIRECTORY", "/private/hostile/objects")
        .env(
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "/private/hostile/alternates",
        )
        .env("GIT_COMMON_DIR", "/private/hostile/common")
        .env("GIT_CONFIG", "/private/hostile/config")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.pager")
        .env("GIT_CONFIG_VALUE_0", "/private/hostile/pager")
        .env("GIT_EXTERNAL_DIFF", "/private/hostile/diff")
        .env("GIT_PAGER", "/private/hostile/pager")
        .output()
        .unwrap();
    let git_processes = processes
        .events()
        .into_iter()
        .filter_map(|event| event.strip_prefix("git-").map(str::to_owned))
        .collect();
    (output, git_processes)
}

#[test]
fn api_full_and_files_resolve_one_workspace_without_mutating_git() {
    let root = repository("git-scope-api");
    let before = snapshot(&root);
    let full = inspect(InspectRequest::new(&root));
    assert_eq!(full.status, Status::Complete, "{:?}", full.errors);
    assert_eq!(full.schema_version, 6);
    let full_scope = full.scope.unwrap();
    assert_eq!(full_scope.mode(), ScopeMode::Full);
    assert_eq!(full_scope.execution_scope(), ExecutionScope::Workspace);
    assert_eq!(full_scope.comparison_base(), None);
    assert_eq!(full_scope.files(), None);

    let entries = [
        root.clone(),
        root.join("member/Cargo.toml"),
        root.join("member/src/nested"),
    ];
    let mut expected_scope = None;
    for entry in entries {
        let report = inspect(InspectRequest::new(entry).with_files_scope("baseline"));
        assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
        let scope = report.scope.unwrap();
        assert_eq!(scope.mode(), ScopeMode::Files);
        assert_eq!(scope.execution_scope(), ExecutionScope::Workspace);
        assert_eq!(scope.files(), Some(&["member/src/lib.rs".to_owned()][..]));
        assert_eq!(scope.comparison_base().map(str::len), Some(40));
        if let Some(expected) = &expected_scope {
            assert_eq!(&scope, expected);
        } else {
            expected_scope = Some(scope);
        }
    }
    assert_eq!(snapshot(&root), before);
}

#[test]
fn full_v6_is_the_frozen_v5_fixture_plus_version_and_scope() {
    let report = inspect(InspectRequest::new(fixture()));
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    let scope = report.scope.as_ref().unwrap();
    assert_eq!(scope.mode(), ScopeMode::Full);
    assert_eq!(scope.execution_scope(), ExecutionScope::Workspace);
    assert!(scope.comparison_base().is_none());
    assert!(scope.files().is_none());

    let mut compatible = serde_json::to_value(report).unwrap();
    compatible["schema_version"] = Value::from(5);
    compatible.as_object_mut().unwrap().remove("scope");
    let frozen: Value =
        serde_json::from_str(include_str!("fixtures/git-scope/v5-full-report.json")).unwrap();

    assert_eq!(compatible, frozen);
}

#[test]
fn files_scope_projects_diagnostics_through_the_canonical_path_representation() {
    let root = repository("git-scope-canonical-path");
    let manifest = root.join("member/Cargo.toml");
    let source = root.join("member/src/100%.rs");
    git(&root, &["mv", "member/src/lib.rs", "member/src/100%.rs"]);
    fs::write(
        &manifest,
        format!(
            "{}\n[lib]\npath = \"src/100%.rs\"\n",
            fs::read_to_string(&manifest).unwrap()
        ),
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(
        &root,
        &["commit", "--quiet", "-m", "canonical path baseline"],
    );
    git(&root, &["branch", "-f", "baseline", "HEAD"]);
    fs::write(
        &source,
        "pub fn canonical_path_fixture() -> bool { todo!() }\n",
    )
    .unwrap();

    let report = inspect(InspectRequest::new(&root).with_files_scope("baseline"));

    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    assert_eq!(
        report.scope.as_ref().and_then(|scope| scope.files()),
        Some(&["member/src/100%25.rs".to_owned()][..])
    );
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_deref() == Some("clippy::todo")
            && diagnostic.path.as_deref() == Some("member/src/100%25.rs")
    }));
}

#[cfg(unix)]
#[test]
fn cli_and_api_share_scope_while_full_runs_zero_git_and_files_runs_three() {
    let root = repository("git-scope-cli");
    let api = inspect(
        InspectRequest::new(&root)
            .with_files_scope("baseline")
            .with_blocking(BlockingLevel::Warning),
    );
    let expected_report = serde_json::to_value(&api).unwrap();
    let expected_scope = api.scope.unwrap();

    let (full, full_processes) = instrumented_cli(&root, &[], "git-scope-full-processes");
    assert!(
        full.status.success(),
        "{}",
        String::from_utf8_lossy(&full.stderr)
    );
    assert!(full_processes.is_empty());

    let (files, files_processes) = instrumented_cli(
        &root,
        &[
            "--scope",
            "files",
            "--base",
            "baseline",
            "--blocking",
            "warning",
        ],
        "git-scope-files-processes",
    );
    assert!(
        files.status.success(),
        "{}",
        String::from_utf8_lossy(&files.stderr)
    );
    assert_eq!(files_processes, ["rev-parse", "merge-base", "diff"]);
    let report = json(&files);
    assert_eq!(
        report["scope"],
        serde_json::to_value(expected_scope).unwrap()
    );
    for field in [
        "status",
        "complete",
        "policy",
        "scope",
        "project",
        "toolchain",
        "scan",
        "diagnostics",
        "errors",
        "summary",
        "gate",
    ] {
        assert_eq!(report[field], expected_report[field], "{field}");
    }
    let rendered = String::from_utf8(files.stdout).unwrap();
    for hostile in [
        "/private/hostile",
        "credential=secret",
        "https://secret",
        "\u{1b}",
    ] {
        assert!(!rendered.contains(hostile));
    }
}

#[cfg(unix)]
#[test]
fn policy_discovery_and_metadata_failures_precede_every_git_process() {
    let missing = support::temporary_target("git-scope-missing-entry", &NEXT_WORKSPACE);
    if missing.exists() {
        fs::remove_dir_all(&missing).unwrap();
    }
    let (policy, policy_processes) = instrumented_cli(
        &missing,
        &[
            "--scope",
            "files",
            "--base",
            "main",
            "--rule",
            "unknown::rule=warn",
        ],
        "git-scope-policy-order",
    );
    assert_eq!(policy.status.code(), Some(2));
    assert!(policy_processes.is_empty());
    let policy = json(&policy);
    assert_eq!(policy["errors"][0]["stage"], "policy");
    assert_eq!(policy["scope"], Value::Null);

    let (discovery, discovery_processes) = instrumented_cli(
        &missing,
        &["--scope", "files", "--base", "main"],
        "git-scope-discovery-order",
    );
    assert_eq!(discovery.status.code(), Some(2));
    assert!(discovery_processes.is_empty());
    let discovery = json(&discovery);
    assert_eq!(discovery["errors"][0]["stage"], "discovery");
    assert_eq!(discovery["scope"], Value::Null);

    let invalid_metadata = support::temporary_target("git-scope-metadata-entry", &NEXT_WORKSPACE);
    if invalid_metadata.exists() {
        fs::remove_dir_all(&invalid_metadata).unwrap();
    }
    fs::create_dir_all(&invalid_metadata).unwrap();
    fs::write(
        invalid_metadata.join("Cargo.toml"),
        "[package]\nname = \"invalid-metadata\"\n",
    )
    .unwrap();
    let (metadata, metadata_processes) = instrumented_cli(
        &invalid_metadata,
        &["--scope", "files", "--base", "main"],
        "git-scope-metadata-order",
    );
    assert_eq!(metadata.status.code(), Some(2));
    assert!(metadata_processes.is_empty());
    let metadata = json(&metadata);
    assert_eq!(metadata["errors"][0]["stage"], "metadata");
    assert_eq!(metadata["scope"], Value::Null);
}

#[test]
fn invalid_api_base_stops_before_discovery_without_disclosing_input() {
    let hostile = "--secret^{commit}";
    let request = InspectRequest::new("/path/that/must/not/be/inspected").with_files_scope(hostile);
    assert!(!format!("{request:?}").contains(hostile));
    let report = inspect(request);

    assert_eq!(report.schema_version, 6);
    assert_eq!(report.status, Status::Failed);
    assert!(report.project.is_none());
    assert!(report.policy.is_none());
    assert!(report.scope.is_none());
    assert!(report.toolchain.cargo.is_none());
    assert!(report.toolchain.rustc.is_none());
    assert!(report.toolchain.clippy.is_none());
    assert!(report.scan.command.is_none());
    assert_eq!(report.gate.status, GateStatus::NotEvaluated);
    assert_eq!(report.exit_code(), 2);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].stage, "scope");
    assert_eq!(report.errors[0].code, "invalid-base");
    assert!(!format!("{report:?}").contains(hostile));
}

#[test]
fn git_failure_after_configuration_keeps_only_project_and_closed_scope_error() {
    let root = support::temporary_target("git-scope-no-repository", &NEXT_WORKSPACE);
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    support::copy_tree(&fixture(), &root);
    fs::write(root.join(".git"), "gitdir: missing\n").unwrap();

    let report = inspect(InspectRequest::new(&root).with_files_scope("main"));

    assert_eq!(report.status, Status::Failed);
    assert!(report.project.is_some());
    assert!(report.policy.is_none());
    assert!(report.scope.is_none());
    assert!(report.toolchain.cargo.is_none());
    assert!(report.toolchain.rustc.is_none());
    assert!(report.toolchain.clippy.is_none());
    assert!(report.scan.command.is_none());
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].stage, "scope");
    assert_eq!(report.errors[0].code, "base-unavailable");
    assert!(!format!("{report:?}").contains(&root.display().to_string()));
}

#[test]
fn missing_base_is_closed_and_never_reaches_tool_execution() {
    let root = repository("git-scope-missing-base");
    let selector = "missing-private-base";
    let report = inspect(InspectRequest::new(&root).with_files_scope(selector));
    assert_eq!(report.status, Status::Failed);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].code, "base-unavailable");
    assert!(report.scope.is_none());
    assert!(report.toolchain.cargo.is_none());
    assert!(report.scan.command.is_none());
    assert!(!format!("{report:?}").contains(selector));
}

#[test]
fn terminal_scope_failure_exposes_only_the_closed_code() {
    let root = repository("git-scope-terminal-error");
    let selector = "missing-private-base";
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .args(["--scope", "files", "--base", selector])
        .arg(&root)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let mut rendered = String::from_utf8_lossy(&output.stdout).into_owned();
    rendered.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(rendered.contains("scope/base-unavailable"));
    for forbidden in [
        selector,
        root.to_string_lossy().as_ref(),
        "credential=secret",
        "https://",
        "\u{1b}",
    ] {
        assert!(!rendered.contains(forbidden), "leaked {forbidden:?}");
    }
}

#[test]
fn clap_rejects_invalid_scope_combinations_without_a_report_or_inspection() {
    for arguments in [
        vec!["--scope", "files"],
        vec!["--scope", "full", "--base", "main"],
        vec!["--base", "main"],
        vec!["--scope", "unknown"],
    ] {
        let output = cli(Path::new("/path/that/must/not/be/inspected"), &arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            !stderr.contains("Inspecting Cargo workspace"),
            "{arguments:?}"
        );
    }
}

#[test]
fn closed_configuration_rejects_scope_fields_before_git() {
    for document in ["scope = \"files\"\n", "base = \"main\"\n"] {
        let root = support::temporary_target("git-scope-closed-configuration", &NEXT_WORKSPACE);
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        support::copy_tree(&fixture(), &root);
        fs::write(root.join("rust-doctor.toml"), document).unwrap();
        fs::write(root.join(".git"), "gitdir: missing\n").unwrap();

        let report = inspect(InspectRequest::new(&root).with_files_scope("main"));
        assert_eq!(report.status, Status::Failed);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].stage, "configuration");
        assert_eq!(report.errors[0].code, "config-invalid");
        assert!(report.scope.is_none());
    }
}

#[test]
fn request_debug_never_contains_a_files_base() {
    let request = InspectRequest::new(".").with_files_scope("credential-secret-ref");
    let debug = format!("{request:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("credential-secret-ref"));
}
