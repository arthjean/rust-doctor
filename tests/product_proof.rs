#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

fn kernel_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/kernel-contract")
        .join(name)
}

fn inspect_json(path: &Path) -> Output {
    binary()
        .args(["inspect", "--json"])
        .arg(path)
        .output()
        .expect("rust-doctor should start")
}

fn parse_report(output: &Output) -> Value {
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    serde_json::from_slice(&output.stdout).expect("stdout should contain one JSON document")
}

fn source_hash(root: &Path) -> blake3::Hash {
    let mut files = vec![root.join("Cargo.toml")];
    collect_files(&root.join("src"), &mut files);
    files.sort();

    let mut hasher = blake3::Hasher::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .expect("fixture file should be under fixture root");
        hasher.update(relative.as_os_str().as_encoded_bytes());
        hasher.update(&[0]);
        hasher.update(&fs::read(path).expect("fixture file should be readable"));
        hasher.update(&[0]);
    }
    hasher.finalize()
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .expect("fixture source directory should be readable")
        .map(|entry| entry.expect("fixture entry should be readable").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(&path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

fn copy_fixture(source: &Path, destination: &Path) {
    if destination.exists() {
        fs::remove_dir_all(destination).expect("stale test workspace should be removable");
    }
    fs::create_dir_all(destination.join("src"))
        .expect("temporary fixture directory should be creatable");
    fs::copy(source.join("Cargo.toml"), destination.join("Cargo.toml"))
        .expect("fixture manifest should copy");
    fs::copy(source.join("clippy.toml"), destination.join("clippy.toml"))
        .expect("fixture Clippy policy should copy");
    fs::copy(source.join("src/lib.rs"), destination.join("src/lib.rs"))
        .expect("fixture source should copy");
}

#[test]
fn fixtures_prove_complete_incomplete_and_source_preservation() {
    let cases = [
        ("clean", "complete", 0, 0),
        ("clippy-warning", "complete", 0, 1),
        ("compile-error", "incomplete", 1, 1),
    ];
    let before: Vec<_> = cases
        .iter()
        .map(|(name, _, _, _)| source_hash(&fixture(name)))
        .collect();

    for (name, status, exit_code, minimum_diagnostics) in cases {
        let output = inspect_json(&fixture(name));
        assert_eq!(output.status.code(), Some(exit_code), "{name}");
        let report = parse_report(&output);
        assert_eq!(report["schema_version"], 8, "{name}");
        assert!(report["diagnostics"].as_array().is_some_and(|diagnostics| {
            diagnostics.iter().all(|diagnostic| {
                diagnostic.get("category").is_some() && diagnostic.get("help").is_some()
            })
        }));
        assert_eq!(report["status"], status, "{name}");
        assert_eq!(
            report["complete"],
            Value::Bool(status == "complete"),
            "{name}"
        );
        let diagnostic_count = report["diagnostics"]
            .as_array()
            .expect("diagnostics should be an array")
            .len();
        if name == "clean" {
            assert_eq!(diagnostic_count, 0, "{name}");
        } else {
            assert!(diagnostic_count >= minimum_diagnostics, "{name}");
        }
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Inspecting Cargo workspace"),
            "{name}"
        );
    }

    let compile_error = parse_report(&inspect_json(&fixture("compile-error")));
    let captured_occurrences: u64 = compile_error["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .map(|diagnostic| {
            diagnostic["occurrences"]
                .as_u64()
                .expect("occurrences should be numeric")
        })
        .sum();
    assert_eq!(captured_occurrences, 4);
    assert_eq!(compile_error["scan"]["exit_code"], 101);
    assert_eq!(compile_error["scan"]["build_finished"], false);
    assert!(compile_error["errors"].as_array().is_some_and(|errors| {
        errors.iter().any(|error| {
            error["stage"] == "execution"
                && error["code"] == "clippy-exit"
                && error["message"] == "Clippy exited with status 101"
        })
    }));

    let compile_terminal = binary()
        .arg("inspect")
        .arg(fixture("compile-error"))
        .output()
        .expect("rust-doctor should start");
    assert_eq!(compile_terminal.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&compile_terminal.stdout)
            .contains("Scan incomplete: Clippy exited with status 101")
    );

    for ((name, _, _, _), expected) in cases.iter().zip(before) {
        assert_eq!(source_hash(&fixture(name)), expected, "{name}");
    }
}

#[test]
fn failed_json_report_is_valid_and_returns_two() {
    let missing =
        std::env::temp_dir().join(format!("rust-doctor-no-manifest-{}", std::process::id()));
    if missing.exists() {
        fs::remove_dir_all(&missing).expect("test path should be removable");
    }
    fs::create_dir_all(&missing).expect("test path should be creatable");

    let output = inspect_json(&missing);
    let report = parse_report(&output);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(report["status"], "failed");
    assert_eq!(report["complete"], false);
    assert!(report["project"].is_null());
    assert_eq!(report["diagnostics"], Value::Array(Vec::new()));
    assert!(
        report["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
    );
    fs::remove_dir_all(missing).expect("test path should be removable");
}

#[test]
fn default_path_terminal_help_and_clap_errors_follow_cli_contract() {
    let default_path = binary()
        .current_dir(fixture("clean"))
        .args(["inspect", "--json"])
        .output()
        .expect("rust-doctor should start");
    assert_eq!(default_path.status.code(), Some(0));
    assert_eq!(
        parse_report(&default_path)["project"]["manifest_path"],
        "Cargo.toml"
    );

    let terminal = binary()
        .args(["inspect"])
        .arg(fixture("clippy-warning"))
        .output()
        .expect("rust-doctor should start");
    let terminal_stdout = String::from_utf8_lossy(&terminal.stdout);
    assert_eq!(terminal.status.code(), Some(0));
    assert!(terminal_stdout.contains("src/lib.rs:4:5 warning [clippy::needless_return]"));
    assert!(terminal_stdout.contains("status complete"));

    let help = binary()
        .arg("--help")
        .output()
        .expect("rust-doctor should start");
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("build.rs"));
    assert!(help.contains("procedural macros"));
    assert!(help.contains("trusted local repositories"));

    let syntax_error = binary()
        .args(["inspect", "--not-a-real-option"])
        .output()
        .expect("rust-doctor should start");
    assert_eq!(syntax_error.status.code(), Some(2));
    assert!(syntax_error.stdout.is_empty());
    assert!(String::from_utf8_lossy(&syntax_error.stderr).contains("error:"));
}

#[test]
fn curated_kernel_drives_command_json_terminal_and_effective_severity() {
    let output = inspect_json(&kernel_fixture("dbg-macro"));
    let report = parse_report(&output);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(report["schema_version"], 8);
    assert_eq!(report["status"], "complete");
    assert_eq!(
        report["scan"]["command"],
        serde_json::json!([
            "cargo",
            "clippy",
            "--workspace",
            "--all-targets",
            "--no-deps",
            "--message-format=json",
            "--",
            "-W",
            "clippy::dbg_macro",
            "-W",
            "clippy::mem_forget",
            "-W",
            "clippy::non_send_fields_in_send_ty",
            "-W",
            "clippy::permissions_set_readonly_false",
            "-W",
            "clippy::suspicious_command_arg_space",
            "-W",
            "clippy::todo",
            "-W",
            "clippy::unimplemented",
            "-W",
            "clippy::zombie_processes"
        ])
    );
    let dbg_findings: Vec<_> = report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "clippy::dbg_macro")
        .collect();
    assert_eq!(dbg_findings.len(), 3);
    assert!(dbg_findings.iter().all(|diagnostic| {
        diagnostic["severity"] == "warning"
            && diagnostic["category"] == "maintainability"
            && diagnostic["help"] == "Remove dbg! or replace it with intentional logging."
    }));

    let terminal = binary()
        .arg("inspect")
        .arg(kernel_fixture("dbg-macro"))
        .output()
        .expect("rust-doctor should start");
    let terminal = String::from_utf8_lossy(&terminal.stdout);
    assert_eq!(terminal.matches("Help (maintainability):").count(), 3);

    let denied_output = inspect_json(&kernel_fixture("denied"));
    let denied = parse_report(&denied_output);
    assert_eq!(denied_output.status.code(), Some(1));
    assert_eq!(denied["status"], "incomplete");
    let finding = denied["diagnostics"]
        .as_array()
        .and_then(|diagnostics| {
            diagnostics
                .iter()
                .find(|diagnostic| diagnostic["code"] == "clippy::todo")
        })
        .expect("denied curated finding should be retained");
    assert_eq!(finding["severity"], "error");
    assert_eq!(finding["category"], "correctness");
    assert_eq!(
        finding["help"],
        "Replace todo! with the intended implementation or remove the reachable placeholder."
    );
    assert!(denied["errors"].as_array().is_some_and(|errors| {
        errors
            .iter()
            .any(|error| error["stage"] == "execution" && error["code"] == "clippy-exit")
    }));
}

#[test]
fn non_curated_clippy_and_rustc_diagnostics_remain_null_enriched() {
    for name in ["clippy-warning", "compile-error"] {
        let report = parse_report(&inspect_json(&fixture(name)));
        assert!(report["diagnostics"].as_array().is_some_and(|diagnostics| {
            !diagnostics.is_empty()
                && diagnostics.iter().all(|diagnostic| {
                    diagnostic["category"].is_null() && diagnostic["help"].is_null()
                })
        }));
    }
}

#[test]
fn correcting_a_copied_finding_removes_its_id_on_rescan() {
    let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("product-loop-{}", std::process::id()));
    copy_fixture(&fixture("clippy-warning"), &destination);

    let first_output = inspect_json(&destination);
    let first = parse_report(&first_output);
    assert_eq!(first_output.status.code(), Some(0));
    assert_eq!(first["status"], "complete");
    let initial_id = first["diagnostics"][0]["id"]
        .as_str()
        .expect("seeded finding should have an id")
        .to_owned();

    fs::write(
        destination.join("src/lib.rs"),
        "pub fn answer() -> u8 {\n    42\n}\n",
    )
    .expect("copied fixture should be writable");
    let second_output = inspect_json(&destination);
    let second = parse_report(&second_output);

    assert_eq!(second_output.status.code(), Some(0));
    assert_eq!(second["status"], "complete");
    assert!(
        second["diagnostics"]
            .as_array()
            .expect("diagnostics should be an array")
            .iter()
            .all(|diagnostic| diagnostic["id"] != initial_id)
    );

    fs::remove_dir_all(destination).expect("temporary fixture should be removable");
}

#[test]
fn repository_json_has_exact_top_level_contract_without_absolute_private_paths() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = inspect_json(root);
    let report = parse_report(&output);
    let keys: Vec<_> = report
        .as_object()
        .expect("report should be an object")
        .keys()
        .map(String::as_str)
        .collect();

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        keys,
        [
            "audit",
            "complete",
            "delta",
            "diagnostics",
            "errors",
            "gate",
            "policy",
            "project",
            "scan",
            "schema_version",
            "scope",
            "status",
            "summary",
            "toolchain",
        ]
    );
    assert_eq!(report["project"]["workspace_root"], ".");
    let json = String::from_utf8(output.stdout).expect("JSON should be UTF-8");
    assert!(!json.contains(&root.display().to_string()));
    if let Some(home) = std::env::var_os("HOME").as_deref().and_then(OsStr::to_str) {
        assert!(!json.contains(home));
    }
    assert!(!json.contains("\u{1b}["));
}

#[test]
fn virtual_workspace_retains_external_members_without_exposing_their_manifests() {
    let report = parse_report(&inspect_json(&fixture("virtual-workspace")));
    let packages = report["project"]["packages"]
        .as_array()
        .expect("packages should be an array");

    assert_eq!(packages.len(), 2);
    let member = packages
        .iter()
        .find(|package| package["name"] == "workspace-member")
        .expect("internal member should be present");
    assert_eq!(member["manifest_path"], "member/Cargo.toml");
    assert_eq!(member["targets"], serde_json::json!(["workspace_member"]));

    let external = packages
        .iter()
        .find(|package| package["name"] == "shared")
        .expect("external member should be present");
    assert!(external["manifest_path"].is_null());
    assert_eq!(external["targets"], serde_json::json!(["shared"]));
}

#[test]
fn invalid_manifest_boundary_does_not_fall_back_to_the_repository_manifest() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("product-invalid-manifest-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).expect("stale invalid-manifest fixture should be removable");
    }
    let nested = root.join("nested");
    fs::create_dir_all(nested.join("Cargo.toml"))
        .expect("invalid manifest directory should be creatable");

    let output = inspect_json(&nested);
    let report = parse_report(&output);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(report["status"], "failed");
    assert!(report["project"].is_null());
    assert!(report["toolchain"]["cargo"].is_null());
    assert!(report["scan"]["command"].is_null());
    assert!(report["errors"].as_array().is_some_and(|errors| {
        errors
            .iter()
            .any(|error| error["stage"] == "discovery" && error["code"] == "invalid-manifest")
    }));

    fs::remove_dir_all(root).expect("invalid-manifest fixture should be removable");
}

#[test]
fn same_named_library_and_binary_targets_are_both_preserved() {
    let output = inspect_json(&fixture("same_name_targets"));
    let report = parse_report(&output);
    let packages = report["project"]["packages"]
        .as_array()
        .expect("packages should be an array");
    let package = packages
        .iter()
        .find(|package| package["name"] == "same_name_targets")
        .expect("fixture package should be present");

    assert_eq!(
        package["targets"],
        serde_json::json!(["same_name_targets", "same_name_targets"])
    );
}
