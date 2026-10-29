#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::AtomicUsize;

use rust_doctor::{
    BlockingLevel, BlockingLevelSource, CategoryOverride, GateStatus, InspectRequest, RuleLevel,
    RuleLevelSource, RuleOverride, Status, inspect,
};
use serde_json::Value;

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/configuration-kernel/workspace")
}

fn temporary_workspace() -> PathBuf {
    let root = support::temporary_target("configuration-kernel-integration", &NEXT_WORKSPACE);
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    support::copy_tree(&fixture(), &root);
    root
}

fn cli(path: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--json")
        .args(support::NO_ADVISORY_DB)
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

fn terminal(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .args(support::NO_ADVISORY_DB)
        .arg(path)
        .env("CARGO_NET_OFFLINE", "true")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn root_member_manifest_and_subdirectory_keep_one_workspace_and_selected_manifest() {
    let workspace = temporary_workspace();
    let cases = [
        (workspace.clone(), "Cargo.toml"),
        (workspace.join("Cargo.toml"), "Cargo.toml"),
        (workspace.join("member"), "member/Cargo.toml"),
        (workspace.join("member/src/nested"), "member/Cargo.toml"),
    ];

    for (path, expected_manifest) in cases {
        let report = inspect(InspectRequest::new(path));
        assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
        let policy = report.policy.as_ref().unwrap();
        assert!(policy.config_file.is_none());
        assert_eq!(policy.rules.len(), 142);
        assert!(
            policy
                .rules
                .iter()
                .all(|rule| rule.source == RuleLevelSource::Default)
        );
        let project = report.project.unwrap();
        assert_eq!(project.workspace_root, ".");
        assert_eq!(project.manifest_path, expected_manifest);
        assert_eq!(
            report.scan.command.unwrap()[..7],
            [
                "cargo",
                "clippy",
                "--workspace",
                "--no-deps",
                "--keep-going",
                "--message-format=json",
                "--",
            ]
        );
    }
}

#[test]
fn only_the_workspace_root_configuration_is_consulted() {
    let workspace = temporary_workspace();
    fs::write(
        workspace.join("member/rust-doctor.toml"),
        "hostile-member-field = \"must be ignored\"\n",
    )
    .unwrap();
    let report = inspect(InspectRequest::new(workspace.join("member")));

    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    assert!(report.errors.is_empty());
    assert!(report.policy.unwrap().config_file.is_none());
}

#[test]
fn v7_policy_precedence_and_blocking_are_shared_by_cli_and_api() {
    let workspace = temporary_workspace();
    fs::write(
        workspace.join("rust-doctor.toml"),
        concat!(
            "blocking = \"warning\"\n\n",
            "[categories]\nsecurity = \"off\"\ncorrectness = \"off\"\n\n",
            "[rules]\n",
            "\"clippy::todo\" = \"error\"\n",
            "\"rust_doctor::source::dynamic_shell_command\" = \"error\"\n",
        ),
    )
    .unwrap();
    let request = InspectRequest::new(&workspace)
        .with_category_override(CategoryOverride::new("correctness", RuleLevel::Warn))
        .with_rule_override(RuleOverride::new(
            "rust_doctor::source::dynamic_shell_command",
            RuleLevel::Warn,
        ));
    let api = inspect(request);
    let policy = api.policy.as_ref().unwrap();
    assert_eq!(api.schema_version, 24);
    assert_eq!(policy.config_file.as_deref(), Some("rust-doctor.toml"));
    assert_eq!(policy.blocking.level, BlockingLevel::Warning);
    assert_eq!(policy.blocking.source, BlockingLevelSource::Config);
    assert_eq!(policy.rules.len(), 142);
    assert_eq!(policy.rules[0].id, "clippy::absurd_extreme_comparisons");
    assert_eq!(
        policy.rules[141].id,
        "rust_doctor::structure::unreferenced_feature"
    );

    let levels: std::collections::BTreeMap<_, _> = policy
        .rules
        .iter()
        .map(|rule| (rule.id.as_str(), (rule.level, rule.source)))
        .collect();
    assert_eq!(
        levels["clippy::todo"],
        (RuleLevel::Warn, RuleLevelSource::RequestCategory)
    );
    assert_eq!(
        levels["rust_doctor::source::disabled_tls_verification"],
        (RuleLevel::Off, RuleLevelSource::ConfigCategory)
    );
    assert_eq!(
        levels["rust_doctor::source::dynamic_shell_command"],
        (RuleLevel::Warn, RuleLevelSource::RequestRule)
    );

    let cli_output = cli(
        &workspace,
        &[
            "--category",
            "correctness=warn",
            "--rule",
            "rust_doctor::source::dynamic_shell_command=warn",
        ],
    );
    assert_eq!(cli_output.status.code(), Some(api.exit_code().into()));
    let cli_report = json(&cli_output);
    assert_eq!(cli_report["policy"], serde_json::to_value(policy).unwrap());
    assert_eq!(cli_report["gate"], serde_json::to_value(&api.gate).unwrap());

    let explicit = cli(&workspace, &["--blocking", "none"]);
    let explicit = json(&explicit);
    assert_eq!(explicit["policy"]["blocking"]["level"], "none");
    assert_eq!(explicit["policy"]["blocking"]["source"], "request");
}

#[test]
fn terminal_exposes_config_state_and_blocking_source_compactly() {
    let absent = temporary_workspace();
    let absent = terminal(&absent);
    assert!(absent.status.success());
    let absent = String::from_utf8(absent.stdout).unwrap();
    assert!(absent.contains("Configuration: none loaded; blocking error (default)"));
    assert_eq!(absent.matches("Configuration:").count(), 1);

    let configured = temporary_workspace();
    fs::write(
        configured.join("rust-doctor.toml"),
        "blocking = \"warning\"\n",
    )
    .unwrap();
    let configured = terminal(&configured);
    assert!(configured.status.success());
    let configured = String::from_utf8(configured.stdout).unwrap();
    assert!(
        configured.contains("Configuration: rust-doctor.toml loaded; blocking warning (config)")
    );
    assert_eq!(configured.matches("Configuration:").count(), 1);
}

#[cfg(unix)]
#[test]
fn all_configuration_error_families_are_failed_private_v7_reports_for_api_and_cli() {
    enum Surface {
        Directory,
        DanglingSymlink,
        Bytes(Vec<u8>),
    }

    let cases = [
        ("config-not-file", Surface::Directory),
        ("config-unreadable", Surface::DanglingSymlink),
        ("config-too-large", Surface::Bytes(vec![b' '; 65_537])),
        ("config-invalid-utf8", Surface::Bytes(vec![0xff])),
        (
            "config-invalid",
            Surface::Bytes(b"unknown = \"warn\"\n".to_vec()),
        ),
        (
            "invalid-rule-selector",
            Surface::Bytes(b"[rules]\n\"BAD\" = \"warn\"\n".to_vec()),
        ),
        (
            "unknown-rule",
            Surface::Bytes(b"[rules]\n\"clippy::unknown\" = \"warn\"\n".to_vec()),
        ),
        (
            "invalid-category-selector",
            Surface::Bytes(b"[categories]\n\"bad-key\" = \"warn\"\n".to_vec()),
        ),
        (
            "unknown-category",
            Surface::Bytes(b"[categories]\nunknown = \"warn\"\n".to_vec()),
        ),
    ];

    for (expected_code, surface) in cases {
        let workspace = temporary_workspace();
        let config = workspace.join("rust-doctor.toml");
        match surface {
            Surface::Directory => fs::create_dir(&config).unwrap(),
            Surface::DanglingSymlink => symlink("missing", &config).unwrap(),
            Surface::Bytes(bytes) => fs::write(&config, bytes).unwrap(),
        }

        let api = inspect(InspectRequest::new(&workspace));
        assert_eq!(api.schema_version, 24, "{expected_code}");
        assert_eq!(api.status, Status::Failed, "{expected_code}");
        assert_eq!(api.exit_code(), 2, "{expected_code}");
        assert!(api.policy.is_none(), "{expected_code}");
        assert!(api.project.is_some(), "{expected_code}");
        assert!(api.toolchain.cargo.is_none(), "{expected_code}");
        assert!(api.toolchain.rustc.is_none(), "{expected_code}");
        assert!(api.toolchain.clippy.is_none(), "{expected_code}");
        assert!(api.scan.command.is_none(), "{expected_code}");
        assert_eq!(api.gate.status, GateStatus::NotEvaluated, "{expected_code}");
        assert_eq!(api.errors.len(), 1, "{expected_code}");
        assert_eq!(api.errors[0].stage, "configuration", "{expected_code}");
        assert_eq!(api.errors[0].code, expected_code, "{expected_code}");

        let cli_output = cli(&workspace, &[]);
        assert_eq!(cli_output.status.code(), Some(2), "{expected_code}");
        let cli_report = json(&cli_output);
        assert_eq!(cli_report["schema_version"], 24, "{expected_code}");
        assert_eq!(cli_report["policy"], Value::Null, "{expected_code}");
        assert_eq!(
            cli_report["gate"]["status"], "not-evaluated",
            "{expected_code}"
        );
        assert_eq!(
            cli_report["errors"][0]["code"], expected_code,
            "{expected_code}"
        );
        assert!(cli_report["scan"]["command"].is_null(), "{expected_code}");
    }
}

#[test]
fn invalid_workspace_configuration_fails_after_metadata_and_before_all_analysis() {
    let workspace = temporary_workspace();
    let config = workspace.join("rust-doctor.toml");
    fs::write(&config, "unknown = \"warn\"\n").unwrap();
    let before = fs::read(&config).unwrap();
    let before_metadata = fs::metadata(&config).unwrap();

    let report = inspect(InspectRequest::new(&workspace));

    assert_eq!(report.status, Status::Failed);
    assert_eq!(report.schema_version, 24);
    assert!(report.policy.is_none());
    assert_eq!(report.exit_code(), 2);
    assert!(report.project.is_some());
    assert!(report.toolchain.cargo.is_none());
    assert!(report.toolchain.rustc.is_none());
    assert!(report.toolchain.clippy.is_none());
    assert!(report.scan.command.is_none());
    assert!(report.diagnostics.is_empty());
    assert_eq!(report.gate.status, GateStatus::NotEvaluated);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].stage, "configuration");
    assert_eq!(report.errors[0].code, "config-invalid");
    assert_eq!(fs::read(&config).unwrap(), before);
    let after_metadata = fs::metadata(&config).unwrap();
    assert_eq!(after_metadata.len(), before_metadata.len());
    assert_eq!(
        after_metadata.modified().unwrap(),
        before_metadata.modified().unwrap()
    );
}

#[test]
fn hostile_configuration_content_never_reaches_json_or_terminal_reports() {
    let workspace = temporary_workspace();
    let hostile = "/home/private https://secret credential=token \u{1b}[31m";
    fs::write(
        workspace.join("rust-doctor.toml"),
        format!("[rules]\n\"{hostile}\" = \"warn\"\n"),
    )
    .unwrap();
    let report = inspect(InspectRequest::new(&workspace));
    let mut json = Vec::new();
    let mut terminal = Vec::new();
    rust_doctor::render::render_json(&report, &mut json).unwrap();
    rust_doctor::render::render_terminal(&report, &mut terminal).unwrap();

    let rendered = format!(
        "{}{}",
        String::from_utf8(json).unwrap(),
        String::from_utf8(terminal).unwrap()
    );
    for sentinel in [
        "/home/private",
        "https://secret",
        "credential=token",
        "\u{1b}",
    ] {
        assert!(!rendered.contains(sentinel));
    }
    assert!(rendered.contains("config-invalid"));
}

#[test]
fn default_v7_report_matches_the_frozen_v4_contract_outside_policy_scope_and_delta() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kernel-contract/todo");
    let output = cli(&fixture, &[]);
    assert!(output.status.success());
    let mut current = json(&output);
    let mut baseline: Value = serde_json::from_str(include_str!(
        "fixtures/configuration-kernel/v4-default-report.json"
    ))
    .unwrap();

    assert_eq!(current["schema_version"], 24);
    assert_eq!(baseline["schema_version"], 4);
    current.as_object_mut().unwrap().remove("schema_version");
    current.as_object_mut().unwrap().remove("audit");
    current.as_object_mut().unwrap().remove("policy");
    current.as_object_mut().unwrap().remove("scope");
    current.as_object_mut().unwrap().remove("delta");
    // Both named quantities are added by schema v9, the context of every
    // diagnostic by v10. The five flat fields of v4 keep their name, their type
    // and their value.
    for added in ["distinct", "occurrences"] {
        assert!(
            current["summary"]
                .as_object_mut()
                .unwrap()
                .remove(added)
                .is_some()
        );
    }
    // The context is only present outside production, so the projection removes
    // it when it is there and never requires it.
    // v18 marks a compiler note `unscored` and names the binary in `toolchain`.
    for diagnostic in current["diagnostics"].as_array_mut().unwrap() {
        diagnostic.as_object_mut().unwrap().remove("context");
        diagnostic.as_object_mut().unwrap().remove("unscored");
    }
    for added in ["rust_doctor", "removed_lint_flags"] {
        current["toolchain"].as_object_mut().unwrap().remove(added);
    }
    // v19 counts what left the report and lists the suppression directives.
    for added in ["ignored", "excluded_generated"] {
        current["scan"].as_object_mut().unwrap().remove(added);
    }
    current.as_object_mut().unwrap().remove("suppressions");
    baseline.as_object_mut().unwrap().remove("schema_version");
    assert_ne!(current["scan"]["command"], baseline["scan"]["command"]);
    current["scan"].as_object_mut().unwrap().remove("command");
    baseline["scan"].as_object_mut().unwrap().remove("command");
    assert_eq!(current, baseline);
}

/// A directory tree written from `(path, contents)` pairs, under a repository
/// of its own so the upward walk stops there and never reaches this crate.
fn discovery_tree(files: &[(&str, &str)]) -> PathBuf {
    let root = support::fresh_workspace("configuration-kernel-discovery", &NEXT_WORKSPACE);
    for (path, contents) in files {
        let file = root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, contents).unwrap();
    }
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&root)
        .env_remove("GIT_DIR")
        .status()
        .unwrap();
    assert!(status.success());
    root
}

fn package(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n")
}

fn discover(root: &Path) -> (Output, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--json")
        .args(support::NO_ADVISORY_DB)
        .arg(root)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .output()
        .unwrap();
    let report = json(&output);
    (output, report)
}

/// US-005: at the root of a Tauri-shaped repository the one workspace below
/// is scanned and named on stderr; a manifest nested inside it is its member,
/// not a second candidate, and one in an ignored or skipped tree never counts.
#[test]
fn the_one_workspace_below_a_repository_root_is_scanned_and_named() {
    let root = discovery_tree(&[
        (".gitignore", "/src-tauri/target\n/ignored\n"),
        ("package.json", "{\"name\": \"app\"}\n"),
        ("src-tauri/Cargo.toml", &package("tauri-app")),
        ("src-tauri/src/lib.rs", "pub fn run() {}\n"),
        ("src-tauri/crates/helper/Cargo.toml", &package("helper")),
        ("src-tauri/target/debug/build/x/Cargo.toml", &package("built")),
        ("node_modules/native/Cargo.toml", &package("native")),
        ("ignored/Cargo.toml", &package("ignored")),
    ]);
    let (output, report) = discover(&root);
    assert_eq!(report["status"], "complete", "{report}");
    assert_eq!(report["project"]["packages"][0]["name"], "tauri-app", "{report}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No Cargo workspace at this path: scanning the one in src-tauri/."),
        "{stderr}"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(root.to_str().unwrap()));
}

/// US-005: several workspaces below are listed, sorted, and the run refuses
/// to pick one; manifests only in build or ignored trees keep the old error;
/// and a manifest above the repository is never selected.
#[test]
fn several_workspaces_are_refused_and_nothing_qualifies_from_outside_the_repository() {
    let root = discovery_tree(&[
        ("src-tauri/Cargo.toml", &package("tauri-app")),
        ("src-tauri/src/lib.rs", ""),
        ("backend/Cargo.toml", &package("backend")),
        ("backend/src/lib.rs", ""),
    ]);
    let (output, report) = discover(&root);
    assert_eq!(output.status.code(), Some(2), "{report}");
    assert_eq!(report["errors"][0]["stage"], "discovery", "{report}");
    assert_eq!(report["errors"][0]["code"], "ambiguous-workspace", "{report}");
    assert_eq!(
        report["errors"][0]["message"],
        "Several Cargo workspaces under this path: backend, src-tauri. Pass one of them."
    );
    assert!(!report.to_string().contains(root.to_str().unwrap()), "{report}");

    let built = discovery_tree(&[
        (".gitignore", "/vendored\n"),
        ("target/debug/build/x/Cargo.toml", &package("built")),
        ("vendored/Cargo.toml", &package("vendored")),
    ]);
    let (output, report) = discover(&built);
    assert_eq!(output.status.code(), Some(2), "{report}");
    assert_eq!(report["errors"][0]["code"], "no-manifest", "{report}");

    // An outer workspace with an unrelated repository below it: the walk up
    // from inside that repository stops at its root.
    let outer = discovery_tree(&[("Cargo.toml", &package("outer")), ("src/lib.rs", "")]);
    fs::create_dir_all(outer.join("inner/docs")).unwrap();
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(outer.join("inner"))
        .env_remove("GIT_DIR")
        .status()
        .unwrap();
    assert!(status.success());
    let (output, report) = discover(&outer.join("inner/docs"));
    assert_eq!(output.status.code(), Some(2), "{report}");
    assert_eq!(report["errors"][0]["code"], "no-manifest", "{report}");
    assert_eq!(
        report["errors"][0]["message"],
        "no Cargo.toml found in this path or its ancestors"
    );
    assert!(!report.to_string().contains(outer.to_str().unwrap()), "{report}");
}
