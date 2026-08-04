#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::AtomicUsize;

use rust_doctor::{InspectRequest, Status, inspect};
use serde_json::{Value, json};
use support::rule_scaling::oracle as rule_scaling_oracle;

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);
const FILES_ARGUMENTS: &[&str] = &["--scope", "files", "--base", "baseline"];

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    cargo_home: PathBuf,
    target: PathBuf,
    processes: support::ProcessHarness,
    real_cargo: PathBuf,
    real_git: PathBuf,
    real_rustc: PathBuf,
}

fn source_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/policy-gate/product-loop")
}

fn run(command: &mut Command, description: &str) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{description} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git(root: &Path, arguments: &[&str]) -> Output {
    run(
        Command::new("git").args(arguments).current_dir(root),
        "git command",
    )
}

fn configure_repository(root: &Path) {
    git(root, &["init", "--initial-branch=main", "--quiet"]);
    git(root, &["config", "user.name", "Rust Doctor"]);
    git(
        root,
        &["config", "user.email", "rust-doctor@example.invalid"],
    );
    git(root, &["config", "commit.gpgsign", "false"]);
}

fn commit(root: &Path, message: &str, timestamp: &str) {
    run(
        Command::new("git")
            .args(["commit", "--quiet", "-m", message])
            .current_dir(root)
            .env("GIT_AUTHOR_DATE", timestamp)
            .env("GIT_COMMITTER_DATE", timestamp),
        "git commit",
    );
}

fn initialize_git_dependency(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"local-git-dependency\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    configure_repository(root);
    git(root, &["add", "."]);
    commit(root, "fixture", "2000-01-01T00:00:00Z");
}

fn prepare_fixture() -> Fixture {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/git-change-scope-product-proof/matrix");
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    let project = root.join("project");
    support::copy_tree(&source_fixture(), &project);
    let dependency = root.join("git-dependency");
    initialize_git_dependency(&dependency);
    let manifest = project.join("app/Cargo.toml");
    let source = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        source.replace("POLICY_GATE_GIT_DEPENDENCY", &dependency.to_string_lossy()),
    )
    .unwrap();
    let cargo_home = root.join("cargo-home");
    let target = root.join("target");
    let real_cargo = support::resolve_program("cargo");
    let real_git = support::resolve_program("git");
    let real_rustc = support::resolve_program("rustc");
    run(
        Command::new(&real_cargo)
            .arg("fetch")
            .current_dir(&project)
            .env("CARGO_HOME", &cargo_home)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_NET_OFFLINE", "false"),
        "fixture fetch",
    );
    fs::write(project.join(".gitignore"), "/target\n").unwrap();
    configure_repository(&project);
    git(&project, &["add", "."]);
    commit(&project, "baseline", "2000-01-02T00:00:00Z");
    git(&project, &["branch", "baseline"]);
    let processes = support::ProcessHarness::install_with_git(&root);
    Fixture {
        root,
        project,
        cargo_home,
        target,
        processes,
        real_cargo,
        real_git,
        real_rustc,
    }
}

fn inspect_cli(
    fixture: &Fixture,
    entry: &Path,
    arguments: &[&str],
) -> (Output, BTreeMap<String, usize>) {
    fixture.processes.reset();
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--json")
        .args(arguments)
        .arg(entry)
        .env("PATH", fixture.processes.command_path())
        .env("RUST_DOCTOR_PROCESS_LOG", fixture.processes.log_path())
        .env("RUST_DOCTOR_REAL_CARGO", &fixture.real_cargo)
        .env("RUST_DOCTOR_REAL_GIT", &fixture.real_git)
        .env("RUST_DOCTOR_REAL_RUSTC", &fixture.real_rustc)
        .env("CARGO_HOME", &fixture.cargo_home)
        .env("CARGO_TARGET_DIR", &fixture.target)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    (output, fixture.processes.counts())
}

fn assert_processes(counters: &BTreeMap<String, usize>, files: bool) {
    for stage in [
        "metadata",
        "cargo-version",
        "rustc-version",
        "clippy-version",
        "clippy",
    ] {
        assert_eq!(counters.get(stage), Some(&1), "missing {stage}");
    }
    for stage in ["git-rev-parse", "git-merge-base", "git-diff"] {
        assert_eq!(
            counters.get(stage).copied().unwrap_or(0),
            usize::from(files)
        );
    }
    assert_eq!(counters.len(), if files { 8 } else { 5 });
}

fn assert_private(output: &Output, fixture: &Fixture) {
    let mut rendered = String::from_utf8_lossy(&output.stdout).into_owned();
    rendered.push_str(&String::from_utf8_lossy(&output.stderr));
    for forbidden in [
        "file://",
        "POLICY_GATE_GIT_DEPENDENCY",
        fixture.root.to_string_lossy().as_ref(),
        "credential=secret",
        "\u{1b}",
    ] {
        assert!(!rendered.contains(forbidden), "output leaked {forbidden:?}");
    }
}

fn normalized_for_entry(report: &Value) -> Value {
    let mut normalized = report.clone();
    normalized["project"]
        .as_object_mut()
        .unwrap()
        .remove("manifest_path");
    normalized
}

fn diagnostic_ids(report: &Value) -> BTreeSet<String> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["id"].as_str().unwrap().to_owned())
        .collect()
}

fn diagnostic_id_hash(report: &Value) -> String {
    blake3::hash(
        diagnostic_ids(report)
            .into_iter()
            .collect::<Vec<_>>()
            .join("\n")
            .as_bytes(),
    )
    .to_hex()
    .to_string()
}

fn v6_compatible_output(output: &[u8]) -> Vec<u8> {
    let projected = support::project_v10_wire_to_v7(output);
    std::str::from_utf8(&projected)
        .unwrap()
        .replacen("\"schema_version\":7", "\"schema_version\":6", 1)
        .replacen(",\"delta\":null", "", 1)
        .into_bytes()
}

fn run_case(
    fixture: &Fixture,
    name: &str,
    arguments: &[&str],
    expected_files: Option<&[&str]>,
    expected_diagnostics: usize,
) -> (Value, Value) {
    const ENTRIES: [(&str, &str, &str); 3] = [
        ("workspace", ".", "Cargo.toml"),
        ("member-manifest", "app/Cargo.toml", "app/Cargo.toml"),
        ("member-subdirectory", "app/src", "app/Cargo.toml"),
    ];
    let before = support::file_states(&fixture.project);
    let mut normalized = None;
    let mut representative = None;
    let mut output_hashes = BTreeMap::new();
    let mut observed_processes = None;

    for (entry_name, entry_path, expected_manifest) in ENTRIES {
        let entry = fixture.project.join(entry_path);
        let mut expected_output = None;
        for _ in 0..20 {
            let (output, counters) = inspect_cli(fixture, &entry, arguments);
            assert_eq!(output.status.code(), Some(0), "{name} {entry_name}");
            assert_private(&output, fixture);
            assert_processes(&counters, expected_files.is_some());
            if observed_processes.is_none() {
                observed_processes = Some(counters);
            }
            if let Some(expected) = &expected_output {
                assert_eq!(&output.stdout, expected, "{name} {entry_name}");
            } else {
                expected_output = Some(output.stdout.clone());
            }
        }
        let output = expected_output.unwrap();
        output_hashes.insert(
            entry_name,
            blake3::hash(&v6_compatible_output(&output))
                .to_hex()
                .to_string(),
        );
        let report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(report["schema_version"], 10);
        assert_eq!(report["project"]["workspace_root"], ".");
        assert_eq!(report["project"]["manifest_path"], expected_manifest);
        let diagnostics = report["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), expected_diagnostics);
        // Les deux grandeurs du rapport se recalculent depuis les diagnostics
        // publiés: un finding remonté par deux cibles reste un diagnostic
        // distinct et deux occurrences.
        assert_eq!(
            report["summary"]["distinct"]["total"].as_u64().unwrap(),
            expected_diagnostics as u64
        );
        assert_eq!(
            report["summary"]["occurrences"]["total"].as_u64().unwrap(),
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic["occurrences"].as_u64().unwrap())
                .sum::<u64>()
        );
        match expected_files {
            None => {
                assert_eq!(report["scope"]["mode"], "full");
                assert!(report["scope"]["comparison_base"].is_null());
                assert!(report["scope"]["files"].is_null());
            }
            Some(files) => {
                assert_eq!(report["scope"]["mode"], "files");
                let comparison_base = report["scope"]["comparison_base"].as_str().unwrap();
                assert_eq!(comparison_base.len(), 40);
                assert!(comparison_base.bytes().all(|byte| byte.is_ascii_hexdigit()));
                assert_eq!(report["scope"]["files"], json!(files));
                assert!(
                    report["diagnostics"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|diagnostic| files
                            .contains(&diagnostic["path"].as_str().unwrap_or_default()))
                );
            }
        }
        assert_eq!(report["scope"]["execution_scope"], "workspace");
        match &normalized {
            Some(expected) => assert_eq!(normalized_for_entry(&report), *expected),
            None => normalized = Some(normalized_for_entry(&report)),
        }
        if representative.is_none() {
            representative = Some(report);
        }
    }

    assert_eq!(
        support::file_states(&fixture.project),
        before,
        "{name} mutated fixture"
    );
    let report = representative.unwrap();
    let evaluation = json!({
        "name": name,
        "scope": report["scope"],
        "diagnostics": expected_diagnostics,
        "diagnostic_id_hash": diagnostic_id_hash(&report),
        "summary": report["summary"],
        "gate": report["gate"],
        "exit_code": 0,
        "entry_output_hashes": output_hashes,
        "processes_per_run": observed_processes.unwrap(),
        "runs_per_entry": 20,
    });
    (report, evaluation)
}

#[test]
fn git_change_scope_matrix_is_deterministic_private_and_non_mutating() {
    let fixture = prepare_fixture();
    let mut reports = BTreeMap::new();
    let mut cases = Vec::new();

    let (report, evaluation) = run_case(&fixture, "full", &[], None, 7);
    reports.insert("full", report);
    cases.push(evaluation);

    let (report, evaluation) = run_case(&fixture, "files-empty", FILES_ARGUMENTS, Some(&[]), 0);
    let empty = json!({ "errors": 0, "warnings": 0, "info": 0, "unknown": 0, "total": 0 });
    assert_eq!(
        report["summary"],
        json!({
            "errors": 0,
            "warnings": 0,
            "info": 0,
            "unknown": 0,
            "total": 0,
            "distinct": empty,
            "occurrences": empty,
        })
    );
    assert_eq!(report["gate"]["status"], "passed");
    assert_eq!(report["gate"]["blocking_diagnostics"], 0);
    reports.insert("files-empty", report);
    cases.push(evaluation);

    fs::write(
        fixture.project.join("app/src/lib.rs"),
        format!(
            "{}\n// tracked source change\n",
            fs::read_to_string(fixture.project.join("app/src/lib.rs")).unwrap()
        ),
    )
    .unwrap();
    let (report, evaluation) = run_case(
        &fixture,
        "files-source",
        FILES_ARGUMENTS,
        Some(&["app/src/lib.rs"]),
        5,
    );
    reports.insert("files-source", report);
    cases.push(evaluation);

    git(&fixture.project, &["add", "app/src/lib.rs"]);
    commit(
        &fixture.project,
        "source checkpoint",
        "2000-01-03T00:00:00Z",
    );
    git(&fixture.project, &["branch", "-f", "baseline", "HEAD"]);
    let manifest = fixture.project.join("app/Cargo.toml");
    fs::write(
        &manifest,
        format!(
            "{}\n# tracked manifest change\n",
            fs::read_to_string(&manifest).unwrap()
        ),
    )
    .unwrap();
    let (report, evaluation) = run_case(
        &fixture,
        "files-manifest",
        FILES_ARGUMENTS,
        Some(&["app/Cargo.toml"]),
        2,
    );
    reports.insert("files-manifest", report);
    cases.push(evaluation);

    let full_ids = diagnostic_ids(&reports["full"]);
    let source_ids = diagnostic_ids(&reports["files-source"]);
    let manifest_ids = diagnostic_ids(&reports["files-manifest"]);
    assert!(source_ids.is_disjoint(&manifest_ids));
    assert_eq!(
        source_ids
            .union(&manifest_ids)
            .cloned()
            .collect::<BTreeSet<_>>(),
        full_ids
    );
    let full_diagnostics: BTreeMap<_, _> = reports["full"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| (diagnostic["id"].as_str().unwrap(), diagnostic))
        .collect();
    for report in [&reports["files-source"], &reports["files-manifest"]] {
        for diagnostic in report["diagnostics"].as_array().unwrap() {
            assert_eq!(
                diagnostic,
                full_diagnostics[diagnostic["id"].as_str().unwrap()]
            );
        }
    }

    let before_missing = support::file_states(&fixture.project);
    let missing = inspect_cli(
        &fixture,
        &fixture.project,
        &["--scope", "files", "--base", "missing-private-base"],
    );
    assert_eq!(missing.0.status.code(), Some(2));
    assert_private(&missing.0, &fixture);
    assert_eq!(
        missing.1,
        BTreeMap::from([("git-rev-parse".to_owned(), 1), ("metadata".to_owned(), 1),])
    );
    let missing_report: Value = serde_json::from_slice(&missing.0.stdout).unwrap();
    assert_eq!(missing_report["errors"][0]["code"], "base-unavailable");
    assert!(missing_report["scope"].is_null());
    assert!(missing_report["toolchain"]["cargo"].is_null());
    assert!(missing_report["scan"]["command"].is_null());
    assert_eq!(support::file_states(&fixture.project), before_missing);

    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/git-scope/oracle.json")).unwrap();
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 24);
    let git_version = String::from_utf8(
        run(
            Command::new(&fixture.real_git).arg("--version"),
            "git version",
        )
        .stdout,
    )
    .unwrap();
    let evaluation = json!({
        "schema_version": 1,
        "epic": "EP-014",
        "stories": ["US-039", "US-040", "US-041"],
        "toolchain": reports["full"]["toolchain"],
        "git_version": git_version.trim(),
        "targets": [
            {"name": "workspace", "path": ".", "manifest": "Cargo.toml"},
            {"name": "member-manifest", "path": "app/Cargo.toml", "manifest": "app/Cargo.toml"},
            {"name": "member-subdirectory", "path": "app/src", "manifest": "app/Cargo.toml"},
        ],
        "cases": cases,
        "scope_error": {
            "code": "base-unavailable",
            "processes": missing.1,
            "execution_started": false,
            "exit_code": 2,
        },
        "edge_case_evidence": {
            "oracle_cases": 24,
            "oracle_artifact": "tests/fixtures/git-scope/oracle.json",
            "bounded_kernel": "git_scope::tests::all_output_and_path_boundaries_fail_atomically",
            "stderr_bound": "git_scope::tests::real_process_output_limits_and_stderr_are_closed",
            "scope_output_bound": "git_scope::tests::serialized_scope_limit_accepts_the_last_byte_and_rejects_the_boundary",
            "scope_expansion_bound": "git_scope::tests::normalized_scope_cannot_expand_beyond_the_report_limit",
            "canonical_paths": "workspace_path::tests::changed_paths_use_the_same_safe_representation_as_diagnostics",
            "external_symlink": "workspace_path::tests::paths_crossing_symlinks_outside_the_workspace_are_null",
        },
        "determinism": {
            "combinations": 4,
            "entry_points": 3,
            "runs_per_combination": 20,
            "total_reports": 240,
        },
        "privacy": "pass",
        "non_mutation": "content-size-mtime-and-git-state",
        "verdict": "pass",
    });
    let artifact_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tasks/rust-doctor-git-change-scope-kernel-evaluation.json");
    let expected: Value =
        serde_json::from_str(&fs::read_to_string(&artifact_path).unwrap()).unwrap();
    fs::remove_dir_all(&fixture.root).unwrap();

    let rule_scaling = rule_scaling_oracle();
    let output_hashes: BTreeMap<_, _> = evaluation["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            (
                case["name"].as_str().unwrap().to_owned(),
                case["entry_output_hashes"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(entry, hash)| (entry.clone(), hash.as_str().unwrap().to_owned()))
                    .collect::<BTreeMap<_, _>>(),
            )
        })
        .collect();
    assert_eq!(
        output_hashes,
        rule_scaling.compatibility.git_change_scope_output_hashes,
        "EP-017 output hashes differ:\n{}",
        serde_json::to_string_pretty(&evaluation).unwrap()
    );

    let mut historical_evaluation = evaluation.clone();
    let mut historical_expected = expected;
    for value in [&mut historical_evaluation, &mut historical_expected] {
        for case in value["cases"].as_array_mut().unwrap() {
            case.as_object_mut().unwrap().remove("entry_output_hashes");
        }
    }
    assert_eq!(
        historical_evaluation,
        historical_expected,
        "evaluation artifact differs:\n{}",
        serde_json::to_string_pretty(&evaluation).unwrap()
    );
}

#[test]
fn files_projection_keeps_workspace_execution_failure_observable() {
    let root = support::temporary_target("git-change-scope-compile-error", &NEXT_WORKSPACE);
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    support::copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/projects/compile-error"),
        &root,
    );
    git(&root, &["init", "--initial-branch=main", "--quiet"]);
    git(&root, &["config", "user.name", "Rust Doctor"]);
    git(
        &root,
        &["config", "user.email", "rust-doctor@example.invalid"],
    );
    git(&root, &["add", "."]);
    commit(&root, "baseline", "2000-01-01T00:00:00Z");
    git(&root, &["branch", "baseline"]);
    let manifest = root.join("Cargo.toml");
    fs::write(
        &manifest,
        format!(
            "{}\n# selected manifest\n",
            fs::read_to_string(&manifest).unwrap()
        ),
    )
    .unwrap();

    let report = inspect(InspectRequest::new(&root).with_files_scope("baseline"));

    assert_eq!(report.status, Status::Incomplete);
    assert!(!report.complete);
    assert_eq!(
        report.scope.as_ref().unwrap().files(),
        Some(&["Cargo.toml".to_owned()][..])
    );
    assert!(report.diagnostics.is_empty());
    assert!(report.scan.command.is_some());
    assert!(report.scan.exit_code.is_some_and(|code| code != 0));
    assert!(!report.errors.is_empty());
    assert_eq!(report.exit_code(), 1);
}
