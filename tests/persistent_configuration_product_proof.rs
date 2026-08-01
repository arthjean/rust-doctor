#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::AtomicUsize;
use std::time::SystemTime;

use serde_json::{Value, json};

const SHELL_RULE: &str = "rust_doctor::source::dynamic_shell_command";
const RULES: [&str; 7] = [
    "clippy::dbg_macro",
    "clippy::todo",
    "clippy::unimplemented",
    "rust_doctor::cargo::unbounded_registry_dependency",
    "rust_doctor::cargo::unpinned_git_dependency",
    "rust_doctor::source::disabled_tls_verification",
    SHELL_RULE,
];

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, PartialEq, Eq)]
struct FileState {
    hash: blake3::Hash,
    length: u64,
    modified: Option<SystemTime>,
}

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    cargo_home: PathBuf,
    target: PathBuf,
    wrapper_bin: PathBuf,
    process_log: PathBuf,
    real_rustc: PathBuf,
}

#[derive(Clone, Copy)]
struct PolicyCase {
    name: &'static str,
    configuration: Option<&'static str>,
    arguments: &'static [&'static str],
}

fn source_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/policy-gate/product-loop")
}

fn temporary_workspace() -> PathBuf {
    support::temporary_target("persistent-configuration-product-proof", &NEXT_WORKSPACE)
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

fn initialize_git_dependency(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"local-git-dependency\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
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
}

fn resolve_program(name: &str) -> PathBuf {
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
        .unwrap()
}

fn install_process_wrappers(root: &Path) -> (PathBuf, PathBuf) {
    let wrapper_bin = root.join("wrapper-bin");
    let process_log = root.join("process.log");
    fs::create_dir_all(&wrapper_bin).unwrap();
    let wrapper = wrapper_bin.join("cargo");
    fs::write(
        &wrapper,
        concat!(
            "#!/bin/sh\n",
            "case \"$1\" in\n",
            "  metadata) stage=metadata ;;\n",
            "  --version) stage=cargo-version ;;\n",
            "  clippy)\n",
            "    if [ \"$2\" = \"--version\" ]; then stage=clippy-version; else stage=clippy; fi ;;\n",
            "  *) stage=other ;;\n",
            "esac\n",
            "printf '%s\\n' \"$stage\" >> \"$RUST_DOCTOR_PROCESS_LOG\"\n",
            "exec \"$RUST_DOCTOR_REAL_CARGO\" \"$@\"\n",
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&wrapper).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper, permissions).unwrap();
    let rustc_wrapper = wrapper_bin.join("rustc");
    fs::write(
        &rustc_wrapper,
        concat!(
            "#!/bin/sh\n",
            "if [ \"$1\" = \"--version\" ]; then\n",
            "  printf 'rustc-version\\n' >> \"$RUST_DOCTOR_PROCESS_LOG\"\n",
            "fi\n",
            "exec \"$RUST_DOCTOR_REAL_RUSTC\" \"$@\"\n",
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&rustc_wrapper).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&rustc_wrapper, permissions).unwrap();
    fs::write(&process_log, []).unwrap();
    (wrapper_bin, process_log)
}

fn prepare_fixture() -> Fixture {
    let root = temporary_workspace();
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    let project = root.join("project");
    support::copy_tree(&source_fixture(), &project);
    let dependency = root.join("git-dependency");
    initialize_git_dependency(&dependency);
    let manifest = project.join("app/Cargo.toml");
    let manifest_source = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        manifest_source.replace("POLICY_GATE_GIT_DEPENDENCY", &dependency.to_string_lossy()),
    )
    .unwrap();
    fs::write(
        project.join("app/rust-doctor.toml"),
        "[categories]\nsecurity = \"error\"\n",
    )
    .unwrap();
    let cargo_home = root.join("cargo-home");
    let target = root.join("target");
    run(
        Command::new(env!("CARGO"))
            .arg("fetch")
            .current_dir(&project)
            .env("CARGO_HOME", &cargo_home)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_NET_OFFLINE", "false"),
        "local fixture fetch",
    );
    let real_rustc = resolve_program("rustc");
    let (wrapper_bin, process_log) = install_process_wrappers(&root);
    Fixture {
        root,
        project,
        cargo_home,
        target,
        wrapper_bin,
        process_log,
        real_rustc,
    }
}

fn file_states(root: &Path) -> BTreeMap<String, FileState> {
    fn visit(root: &Path, directory: &Path, states: &mut BTreeMap<String, FileState>) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, states);
            } else {
                let metadata = fs::symlink_metadata(&path).unwrap();
                let contents = if metadata.file_type().is_symlink() {
                    fs::read_link(&path)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                        .into_bytes()
                } else {
                    fs::read(&path).unwrap()
                };
                states.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    FileState {
                        hash: blake3::hash(&contents),
                        length: metadata.len(),
                        modified: metadata.modified().ok(),
                    },
                );
            }
        }
    }

    let mut states = BTreeMap::new();
    visit(root, root, &mut states);
    states
}

fn remove_config_surface(path: &Path) {
    let Ok(metadata) = path.symlink_metadata() else {
        return;
    };
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path).unwrap();
    } else {
        fs::remove_file(path).unwrap();
    }
}

fn set_configuration(project: &Path, contents: Option<&str>) {
    let path = project.join("rust-doctor.toml");
    remove_config_surface(&path);
    if let Some(contents) = contents {
        fs::write(path, contents).unwrap();
    }
}

fn command_path(wrapper_bin: &Path) -> OsString {
    let mut paths = vec![wrapper_bin.to_path_buf()];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    env::join_paths(paths).unwrap()
}

fn inspect(
    fixture: &Fixture,
    entry: &Path,
    arguments: &[&str],
) -> (Output, BTreeMap<String, usize>) {
    fs::write(&fixture.process_log, []).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--json")
        .args(arguments)
        .arg(entry)
        .env("PATH", command_path(&fixture.wrapper_bin))
        .env("RUST_DOCTOR_PROCESS_LOG", &fixture.process_log)
        .env("RUST_DOCTOR_REAL_CARGO", env!("CARGO"))
        .env("RUST_DOCTOR_REAL_RUSTC", &fixture.real_rustc)
        .env("CARGO_HOME", &fixture.cargo_home)
        .env("CARGO_TARGET_DIR", &fixture.target)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    let mut counters = BTreeMap::new();
    for stage in fs::read_to_string(&fixture.process_log).unwrap().lines() {
        *counters.entry(stage.to_owned()).or_insert(0) += 1;
    }
    (output, counters)
}

fn report(output: &Output) -> Value {
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_private(output: &Output, fixture: &Fixture) {
    let mut rendered = String::from_utf8_lossy(&output.stdout).into_owned();
    rendered.push_str(&String::from_utf8_lossy(&output.stderr));
    for forbidden in [
        "file://",
        "POLICY_GATE_GIT_DEPENDENCY",
        fixture.root.to_string_lossy().as_ref(),
        "\u{1b}",
    ] {
        assert!(!rendered.contains(forbidden), "output leaked {forbidden:?}");
    }
}

fn assert_complete_counters(counters: &BTreeMap<String, usize>) {
    assert_eq!(counters.get("metadata"), Some(&1));
    assert_eq!(counters.get("cargo-version"), Some(&1));
    assert_eq!(counters.get("rustc-version"), Some(&1));
    assert_eq!(counters.get("clippy-version"), Some(&1));
    assert_eq!(counters.get("clippy"), Some(&1));
    assert_eq!(counters.len(), 5);
}

fn normalized_for_entry(report: &Value) -> Value {
    let mut normalized = report.clone();
    normalized["project"]
        .as_object_mut()
        .unwrap()
        .remove("manifest_path");
    normalized
}

fn diagnostic_ids(report: &Value) -> BTreeMap<String, String> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap().to_owned(),
                diagnostic["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn id_hash(report: &Value) -> String {
    let ids: Vec<_> = diagnostic_ids(report).into_values().collect();
    blake3::hash(ids.join("\n").as_bytes()).to_hex().to_string()
}

fn policy_rule<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["policy"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rule| rule["id"] == id)
        .unwrap()
}

fn set_error_surface(project: &Path, code: &str) {
    let path = project.join("rust-doctor.toml");
    remove_config_surface(&path);
    match code {
        "config-not-file" => fs::create_dir(&path).unwrap(),
        "config-unreadable" => symlink("missing", &path).unwrap(),
        "config-too-large" => fs::write(&path, vec![b' '; 65_537]).unwrap(),
        "config-invalid-utf8" => fs::write(&path, [0xff]).unwrap(),
        "config-invalid" => fs::write(&path, "unknown = \"warn\"\n").unwrap(),
        "invalid-rule-selector" => fs::write(&path, "[rules]\n\"BAD\" = \"warn\"\n").unwrap(),
        "unknown-rule" => fs::write(&path, "[rules]\n\"clippy::unknown\" = \"warn\"\n").unwrap(),
        "invalid-category-selector" => {
            fs::write(&path, "[categories]\n\"bad-key\" = \"warn\"\n").unwrap()
        }
        "unknown-category" => fs::write(&path, "[categories]\nunknown = \"warn\"\n").unwrap(),
        _ => unreachable!("configuration error matrix is closed"),
    }
}

#[test]
fn persistent_configuration_matrix_is_deterministic_private_and_non_mutating() {
    const CONFIG_RULE: &str = concat!(
        "[categories]\nsecurity = \"off\"\n\n",
        "[rules]\n",
        "\"rust_doctor::source::dynamic_shell_command\" = \"error\"\n",
    );
    const CONFIG_SECURITY_OFF: &str = "[categories]\nsecurity = \"off\"\n";
    const CONFIG_TODO_ERROR: &str = "[rules]\n\"clippy::todo\" = \"error\"\n";
    const CONFIG_BLOCKING: &str = "blocking = \"warning\"\n";
    const CASES: [PolicyCase; 6] = [
        PolicyCase {
            name: "absent",
            configuration: None,
            arguments: &[],
        },
        PolicyCase {
            name: "empty",
            configuration: Some(""),
            arguments: &[],
        },
        PolicyCase {
            name: "config-rule",
            configuration: Some(CONFIG_RULE),
            arguments: &[],
        },
        PolicyCase {
            name: "shell-reactivated",
            configuration: Some(CONFIG_SECURITY_OFF),
            arguments: &["--rule", "rust_doctor::source::dynamic_shell_command=error"],
        },
        PolicyCase {
            name: "request-category-off",
            configuration: Some(CONFIG_TODO_ERROR),
            arguments: &["--category", "correctness=off"],
        },
        PolicyCase {
            name: "request-blocking",
            configuration: Some(CONFIG_BLOCKING),
            arguments: &["--blocking", "none"],
        },
    ];
    const ENTRIES: [(&str, &str, &str); 3] = [
        ("workspace", ".", "Cargo.toml"),
        ("member-manifest", "app/Cargo.toml", "app/Cargo.toml"),
        ("member-subdirectory", "app/src", "app/Cargo.toml"),
    ];

    let fixture = prepare_fixture();
    let mut reports = BTreeMap::new();
    let mut evaluation_policies = Vec::new();
    let mut toolchain = Value::Null;

    for case in CASES {
        set_configuration(&fixture.project, case.configuration);
        let before = file_states(&fixture.project);
        let config_hash = case
            .configuration
            .map(|contents| blake3::hash(contents.as_bytes()).to_hex().to_string());
        let mut normalized = None;
        let mut output_hashes = BTreeMap::new();
        let mut representative = None;

        for (entry_name, relative_entry, expected_manifest) in ENTRIES {
            let entry = fixture.project.join(relative_entry);
            let mut expected_output = None;
            let mut first_report = None;
            for _ in 0..20 {
                let (output, counters) = inspect(&fixture, &entry, case.arguments);
                assert_complete_counters(&counters);
                assert_private(&output, &fixture);
                match expected_output.as_ref() {
                    Some((status, stdout, stderr)) => {
                        assert_eq!(output.status.code(), *status, "{} {entry_name}", case.name);
                        assert_eq!(&output.stdout, stdout, "{} {entry_name}", case.name);
                        assert_eq!(&output.stderr, stderr, "{} {entry_name}", case.name);
                    }
                    None => {
                        expected_output = Some((
                            output.status.code(),
                            output.stdout.clone(),
                            output.stderr.clone(),
                        ));
                        first_report = Some(report(&output));
                    }
                }
            }
            let report = first_report.unwrap();
            assert_eq!(report["schema_version"], 5);
            assert_eq!(report["project"]["manifest_path"], expected_manifest);
            assert_eq!(report["policy"]["rules"].as_array().unwrap().len(), 7);
            let rule_ids: Vec<_> = report["policy"]["rules"]
                .as_array()
                .unwrap()
                .iter()
                .map(|rule| rule["id"].as_str().unwrap())
                .collect();
            assert_eq!(rule_ids, RULES);
            assert_eq!(
                report["policy"]["config_file"],
                if case.configuration.is_some() {
                    Value::String("rust-doctor.toml".to_owned())
                } else {
                    Value::Null
                }
            );
            match normalized.as_ref() {
                Some(expected) => assert_eq!(
                    normalized_for_entry(&report),
                    *expected,
                    "normalized entry mismatch for {} {entry_name}",
                    case.name
                ),
                None => normalized = Some(normalized_for_entry(&report)),
            }
            output_hashes.insert(
                entry_name,
                blake3::hash(&expected_output.as_ref().unwrap().1)
                    .to_hex()
                    .to_string(),
            );
            if representative.is_none() {
                representative = Some(report);
            }
        }

        assert_eq!(file_states(&fixture.project), before, "{}", case.name);
        let report = representative.unwrap();
        if case.name == "absent" {
            toolchain = report["toolchain"].clone();
        }
        for (code, id) in diagnostic_ids(&report) {
            if let Some(baseline) = reports
                .get("absent")
                .map(diagnostic_ids)
                .and_then(|ids| ids.get(&code).cloned())
            {
                assert_eq!(id, baseline, "diagnostic ID changed for {code}");
            }
        }
        evaluation_policies.push(json!({
            "name": case.name,
            "config_hash": config_hash,
            "arguments": case.arguments,
            "policy": report["policy"],
            "gate": report["gate"],
            "diagnostic_id_hash": id_hash(&report),
            "diagnostics": report["diagnostics"].as_array().unwrap().len(),
            "entry_output_hashes": output_hashes,
            "runs_per_entry": 20,
            "processes_per_run": {
                "metadata": 1,
                "cargo_version": 1,
                "rustc_version": 1,
                "clippy_version": 1,
                "clippy": 1,
            },
        }));
        reports.insert(case.name, report);
    }

    assert_eq!(
        reports["absent"]["diagnostics"],
        reports["empty"]["diagnostics"]
    );
    assert_eq!(reports["absent"]["gate"], reports["empty"]["gate"]);
    assert_eq!(
        reports["absent"]["policy"]["rules"],
        reports["empty"]["policy"]["rules"]
    );
    assert_eq!(diagnostic_ids(&reports["absent"]).len(), 7);
    let historical: Value = serde_json::from_str(include_str!(
        "../tasks/rust-doctor-rule-policy-quality-gate-evaluation.json"
    ))
    .unwrap();
    let historical = &historical["policies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|policy| policy["name"] == "default")
        .unwrap()["result"];
    let absent = &reports["absent"];
    assert_eq!(absent["status"], historical["status"]);
    assert_eq!(absent["scan"]["command"], historical["scan_command"]);
    assert_eq!(absent["summary"], historical["summary"]);
    assert_eq!(absent["gate"], historical["gate"]);
    assert_eq!(id_hash(absent), historical["id_hash"]);
    assert_eq!(
        policy_rule(&reports["config-rule"], SHELL_RULE)["source"],
        "config-rule"
    );
    assert_eq!(
        policy_rule(&reports["shell-reactivated"], SHELL_RULE)["source"],
        "request-rule"
    );
    for inactive in [
        "rust_doctor::cargo::unpinned_git_dependency",
        "rust_doctor::source::disabled_tls_verification",
    ] {
        let rule = policy_rule(&reports["shell-reactivated"], inactive);
        assert_eq!(rule["level"], "off");
        assert_eq!(rule["source"], "config-category");
        assert!(!diagnostic_ids(&reports["shell-reactivated"]).contains_key(inactive));
    }
    for correctness in ["clippy::todo", "clippy::unimplemented"] {
        let rule = policy_rule(&reports["request-category-off"], correctness);
        assert_eq!(rule["level"], "off");
        assert_eq!(rule["source"], "request-category");
    }
    assert_eq!(
        reports["request-blocking"]["policy"]["blocking"],
        json!({"level": "none", "source": "request"})
    );

    let error_codes = [
        "config-not-file",
        "config-unreadable",
        "config-too-large",
        "config-invalid-utf8",
        "config-invalid",
        "invalid-rule-selector",
        "unknown-rule",
        "invalid-category-selector",
        "unknown-category",
    ];
    let mut configuration_errors = Vec::new();
    for code in error_codes {
        set_error_surface(&fixture.project, code);
        let before = file_states(&fixture.project);
        let (output, counters) = inspect(&fixture, &fixture.project, &[]);
        assert_eq!(output.status.code(), Some(2), "{code}");
        assert_private(&output, &fixture);
        assert_eq!(counters, BTreeMap::from([("metadata".to_owned(), 1)]));
        let metadata = counters.get("metadata").copied().unwrap_or(0);
        let tool_versions = ["cargo-version", "rustc-version", "clippy-version"]
            .into_iter()
            .map(|stage| counters.get(stage).copied().unwrap_or(0))
            .sum::<usize>();
        let clippy = counters.get("clippy").copied().unwrap_or(0);
        let execution_started = tool_versions > 0 || clippy > 0;
        assert_eq!((metadata, tool_versions, clippy), (1, 0, 0));
        assert!(!execution_started);
        let report = report(&output);
        assert_eq!(report["schema_version"], 5);
        assert_eq!(report["status"], "failed");
        assert_eq!(report["policy"], Value::Null);
        assert_eq!(report["gate"]["status"], "not-evaluated");
        assert_eq!(report["errors"][0]["stage"], "configuration");
        assert_eq!(report["errors"][0]["code"], code);
        assert!(report["toolchain"]["cargo"].is_null());
        assert!(report["toolchain"]["rustc"].is_null());
        assert!(report["toolchain"]["clippy"].is_null());
        assert!(report["scan"]["command"].is_null());
        assert_eq!(file_states(&fixture.project), before, "{code}");
        configuration_errors.push(json!({
            "code": code,
            "processes": {
                "metadata": metadata,
                "tool_versions": tool_versions,
                "clippy": clippy,
            },
            "analysis": {"execution_started": execution_started},
            "gate": "not-evaluated",
            "exit_code": 2,
        }));
    }

    let evaluation = json!({
        "schema_version": 1,
        "epic": "EP-012",
        "stories": ["US-033", "US-034", "US-035"],
        "toolchain": toolchain,
        "targets": ENTRIES.map(|(name, path, manifest)| json!({
            "name": name,
            "path": path,
            "selected_manifest": manifest,
            "workspace_root": ".",
            "metadata_invocations": 1,
        })),
        "member_config_ignored": "app/rust-doctor.toml",
        "policies": evaluation_policies,
        "configuration_errors": configuration_errors,
        "determinism": {
            "policies": 6,
            "entry_points": 3,
            "runs_per_combination": 20,
            "total_reports": 360,
        },
        "privacy": "pass",
        "non_mutation": "content-size-mtime",
        "verdict": "pass",
    });
    let artifact_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tasks/rust-doctor-scan-target-persistent-configuration-kernel-evaluation.json");
    if env::var_os("RUST_DOCTOR_UPDATE_CONFIGURATION_EVALUATION").is_some() {
        fs::write(
            &artifact_path,
            format!("{}\n", serde_json::to_string_pretty(&evaluation).unwrap()),
        )
        .unwrap();
    }
    let expected: Value =
        serde_json::from_str(&fs::read_to_string(&artifact_path).unwrap()).unwrap();
    remove_config_surface(&fixture.project.join("rust-doctor.toml"));
    fs::remove_dir_all(&fixture.root).unwrap();
    assert_eq!(
        evaluation,
        expected,
        "evaluation artifact differs:\n{}",
        serde_json::to_string_pretty(&evaluation).unwrap()
    );
}
