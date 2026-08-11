#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use support::rule_scaling::{clippy_command_without_rules, oracle};

const RULES: [(&str, &str); 7] = [
    ("clippy::dbg_macro", "maintainability"),
    ("clippy::todo", "correctness"),
    ("clippy::unimplemented", "correctness"),
    (
        "rust_doctor::cargo::unbounded_registry_dependency",
        "reliability",
    ),
    ("rust_doctor::cargo::unpinned_git_dependency", "security"),
    ("rust_doctor::source::disabled_tls_verification", "security"),
    ("rust_doctor::source::dynamic_shell_command", "security"),
];

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/policy-gate/product-loop")
}

fn temporary_workspace() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/policy-gate-product-proof")
        .join(format!(
            "{}-{}",
            std::process::id(),
            NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed)
        ));
    if path.exists() {
        fs::remove_dir_all(&path).unwrap();
    }
    fs::create_dir_all(&path).unwrap();
    path
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    let mut entries: Vec<_> = fs::read_dir(source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let target = destination.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(path, target).unwrap();
        }
    }
}

fn content_hashes(root: &Path) -> BTreeMap<String, blake3::Hash> {
    fn visit(root: &Path, directory: &Path, hashes: &mut BTreeMap<String, blake3::Hash>) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, hashes);
            } else {
                hashes.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    blake3::hash(&fs::read(path).unwrap()),
                );
            }
        }
    }

    let mut hashes = BTreeMap::new();
    visit(root, root, &mut hashes);
    hashes
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

fn init_git_dependency(root: &Path) {
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

struct PreparedFixture {
    root: PathBuf,
    project: PathBuf,
    cargo_home: PathBuf,
    target: PathBuf,
    before: BTreeMap<String, blake3::Hash>,
}

fn prepare_fixture() -> PreparedFixture {
    let root = temporary_workspace();
    let project = root.join("project");
    copy_tree(&fixture(), &project);
    let dependency = root.join("git-dependency");
    init_git_dependency(&dependency);
    let manifest = project.join("app/Cargo.toml");
    let source = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        source.replace("POLICY_GATE_GIT_DEPENDENCY", &dependency.to_string_lossy()),
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
    let before = content_hashes(&project);
    PreparedFixture {
        root,
        project,
        cargo_home,
        target,
        before,
    }
}

fn inspect(fixture: &PreparedFixture, policy_arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--json")
        .args(policy_arguments)
        .arg(&fixture.project)
        .env("CARGO_HOME", &fixture.cargo_home)
        .env("CARGO_TARGET_DIR", &fixture.target)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap()
}

fn report(output: &Output) -> Value {
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn diagnostics(report: &Value) -> &[Value] {
    report["diagnostics"].as_array().unwrap()
}

fn findings(report: &Value) -> BTreeMap<String, &Value> {
    diagnostics(report)
        .iter()
        .map(|diagnostic| (diagnostic["code"].as_str().unwrap().to_owned(), diagnostic))
        .collect()
}

fn id_map(report: &Value) -> BTreeMap<String, String> {
    findings(report)
        .into_iter()
        .map(|(code, diagnostic)| (code, diagnostic["id"].as_str().unwrap().to_owned()))
        .collect()
}

fn id_hash(report: &Value) -> String {
    let ids: Vec<_> = id_map(report).into_values().collect();
    blake3::hash(ids.join("\n").as_bytes()).to_hex().to_string()
}

fn legacy_v3_id(diagnostic: &Value) -> String {
    let span = diagnostic["span"].as_object().map_or_else(
        || "null".to_owned(),
        |span| {
            format!(
                concat!(
                    "{{\"line_start\":{},\"column_start\":{},",
                    "\"line_end\":{},\"column_end\":{}}}"
                ),
                span["line_start"], span["column_start"], span["line_end"], span["column_end"],
            )
        },
    );
    let tuple = format!(
        "[{},{},{},{},{},{}]",
        diagnostic["source"],
        diagnostic["code"],
        diagnostic["path"],
        span,
        diagnostic["base_severity"],
        diagnostic["message"],
    );
    blake3::hash(tuple.as_bytes()).to_hex().to_string()
}

fn assert_private(output: &Output, fixture: &PreparedFixture) {
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

fn deterministic_policy(fixture: &PreparedFixture, arguments: &[&str]) -> (Output, Value) {
    let first = inspect(fixture, arguments);
    let expected_stdout = first.stdout.clone();
    let expected_stderr = first.stderr.clone();
    let expected_exit = first.status.code();
    assert_private(&first, fixture);
    for _ in 1..20 {
        let output = inspect(fixture, arguments);
        assert_eq!(output.status.code(), expected_exit, "{arguments:?}");
        assert_eq!(output.stdout, expected_stdout, "{arguments:?}");
        assert_eq!(output.stderr, expected_stderr, "{arguments:?}");
        assert_private(&output, fixture);
    }
    let value = report(&first);
    (first, value)
}

fn normalized_policy(arguments: &[&str], output: &Output, report: &Value) -> Value {
    json!({
        "arguments": arguments,
        "scan_command": report["scan"]["command"],
        "summary": report["summary"],
        "status": report["status"],
        "gate": report["gate"],
        "exit_code": output.status.code(),
        "id_hash": id_hash(report),
    })
}

#[test]
fn seven_rule_policy_matrix_is_deterministic_private_and_non_mutating() {
    let fixture = prepare_fixture();
    let policies: [(&str, &[&str]); 6] = [
        ("default", &[]),
        ("security-off", &["--category", "security=off"]),
        (
            "shell-reactivated",
            &[
                "--category",
                "security=off",
                "--rule",
                "rust_doctor::source::dynamic_shell_command=error",
            ],
        ),
        ("correctness-error", &["--category", "correctness=error"]),
        (
            "correctness-error-none",
            &["--category", "correctness=error", "--blocking", "none"],
        ),
        (
            "correctness-error-warning",
            &["--category", "correctness=error", "--blocking", "warning"],
        ),
    ];
    let mut reports = BTreeMap::new();
    let mut evaluation_policies = Vec::new();
    for (name, arguments) in policies {
        let (output, report) = deterministic_policy(&fixture, arguments);
        evaluation_policies.push(json!({
            "name": name,
            "result": normalized_policy(arguments, &output, &report),
        }));
        reports.insert(name, (output.status.code().unwrap(), report));
    }

    let (default_exit, default) = &reports["default"];
    assert_eq!(*default_exit, 0);
    assert_eq!(default["schema_version"], 14);
    assert_eq!(default["status"], "complete");
    // Nine warnings since EP-002 of the suppression PRD: the two decoy
    // dependencies this fixture declares and never references, the unbounded
    // registry alias and the local git alias, are now `unused_dependency`
    // findings on top of the historical seven.
    assert_eq!(default["summary"]["warnings"], 9);
    assert_eq!(default["summary"]["total"], 9);
    assert_eq!(default["gate"]["status"], "passed");
    assert_eq!(default["gate"]["blocking_diagnostics"], 0);
    let default_findings = findings(default);
    assert_eq!(default_findings.len(), RULES.len() + 1);
    assert!(default_findings.values().all(|diagnostic| {
        diagnostic["base_severity"] == "warning" && diagnostic["severity"] == "warning"
    }));
    let baseline_ids = id_map(default);
    let legacy_v3_ids: BTreeMap<_, _> = default_findings
        .iter()
        .map(|(code, diagnostic)| (code.clone(), legacy_v3_id(diagnostic)))
        .collect();
    assert_eq!(baseline_ids, legacy_v3_ids);

    let security_off = &reports["security-off"].1;
    let security_codes: BTreeSet<_> = RULES
        .iter()
        .filter_map(|(id, category)| (*category == "security").then_some(*id))
        .collect();
    assert_eq!(reports["security-off"].0, 0);
    assert_eq!(security_off["summary"]["warnings"], 6);
    assert!(
        findings(security_off)
            .keys()
            .all(|code| !security_codes.contains(code.as_str()))
    );
    for (code, id) in id_map(security_off) {
        assert_eq!(baseline_ids[&code], id);
    }

    let shell = &reports["shell-reactivated"].1;
    assert_eq!(reports["shell-reactivated"].0, 1);
    assert_eq!(shell["summary"]["total"], 7);
    assert_eq!(shell["summary"]["errors"], 1);
    assert_eq!(shell["gate"]["status"], "failed");
    assert_eq!(shell["gate"]["blocking_diagnostics"], 1);
    let shell_findings = findings(shell);
    assert_eq!(
        shell_findings["rust_doctor::source::dynamic_shell_command"]["severity"],
        "error"
    );
    for absent in [
        "rust_doctor::cargo::unpinned_git_dependency",
        "rust_doctor::source::disabled_tls_verification",
    ] {
        assert!(!shell_findings.contains_key(absent));
    }

    let correctness = &reports["correctness-error"].1;
    assert_eq!(reports["correctness-error"].0, 1);
    assert_eq!(correctness["status"], "complete");
    assert_eq!(correctness["summary"]["errors"], 2);
    for code in ["clippy::todo", "clippy::unimplemented"] {
        assert_eq!(findings(correctness)[code]["severity"], "error");
        let command = correctness["scan"]["command"].as_array().unwrap();
        assert!(
            command
                .windows(2)
                .any(|pair| pair[0] == "-W" && pair[1] == code)
        );
        assert!(!command.iter().any(|argument| argument == "-D"));
    }

    let none = &reports["correctness-error-none"].1;
    let warning = &reports["correctness-error-warning"].1;
    let mut correctness_without_gate = correctness.clone();
    let mut none_without_gate = none.clone();
    let mut warning_without_gate = warning.clone();
    correctness_without_gate
        .as_object_mut()
        .unwrap()
        .remove("gate");
    correctness_without_gate
        .as_object_mut()
        .unwrap()
        .remove("policy");
    none_without_gate.as_object_mut().unwrap().remove("gate");
    none_without_gate.as_object_mut().unwrap().remove("policy");
    warning_without_gate.as_object_mut().unwrap().remove("gate");
    warning_without_gate
        .as_object_mut()
        .unwrap()
        .remove("policy");
    assert_eq!(correctness_without_gate, none_without_gate);
    assert_eq!(correctness_without_gate, warning_without_gate);
    assert_eq!(correctness["policy"]["blocking"]["source"], "default");
    assert_eq!(none["policy"]["blocking"]["source"], "request");
    assert_eq!(warning["policy"]["blocking"]["source"], "request");
    assert_eq!(reports["correctness-error-none"].0, 0);
    assert_eq!(none["gate"]["status"], "passed");
    assert_eq!(none["gate"]["blocking_diagnostics"], 0);
    assert_eq!(reports["correctness-error-warning"].0, 1);
    assert_eq!(warning["gate"]["status"], "failed");
    assert_eq!(warning["gate"]["blocking_diagnostics"], 9);

    for (rule, _) in RULES {
        let error_value = format!("{rule}=error");
        let error = inspect(&fixture, &["--rule", &error_value]);
        assert_eq!(error.status.code(), Some(1));
        let error = report(&error);
        let error_findings = findings(&error);
        assert_eq!(error_findings[rule]["id"], baseline_ids[rule]);
        assert_eq!(error_findings[rule]["base_severity"], "warning");
        assert_eq!(error_findings[rule]["severity"], "error");

        let off_value = format!("{rule}=off");
        let off = inspect(&fixture, &["--rule", &off_value]);
        assert_eq!(off.status.code(), Some(0));
        let off = report(&off);
        let off_ids = id_map(&off);
        assert!(!off_ids.contains_key(rule));
        for (survivor, id) in off_ids {
            assert_eq!(baseline_ids[&survivor], id);
        }
    }

    let producer_off_policies = [
        (
            "clippy",
            vec![
                "--rule",
                "clippy::arc_with_non_send_sync=off",
                "--rule",
                "clippy::await_holding_lock=off",
                "--rule",
                "clippy::await_holding_refcell_ref=off",
                "--rule",
                "clippy::dbg_macro=off",
                "--rule",
                "clippy::exit=off",
                "--rule",
                "clippy::expect_used=off",
                "--rule",
                "clippy::format_collect=off",
                "--rule",
                "clippy::indexing_slicing=off",
                "--rule",
                "clippy::large_types_passed_by_value=off",
                "--rule",
                "clippy::manual_memcpy=off",
                "--rule",
                "clippy::mem_forget=off",
                "--rule",
                "clippy::missing_safety_doc=off",
                "--rule",
                "clippy::mut_mutex_lock=off",
                "--rule",
                "clippy::non_send_fields_in_send_ty=off",
                "--rule",
                "clippy::panic=off",
                "--rule",
                "clippy::panic_in_result_fn=off",
                "--rule",
                "clippy::permissions_set_readonly_false=off",
                "--rule",
                "clippy::print_stderr=off",
                "--rule",
                "clippy::print_stdout=off",
                "--rule",
                "clippy::ptr_arg=off",
                "--rule",
                "clippy::rc_buffer=off",
                "--rule",
                "clippy::rc_mutex=off",
                "--rule",
                "clippy::redundant_allocation=off",
                "--rule",
                "clippy::stable_sort_primitive=off",
                "--rule",
                "clippy::string_slice=off",
                "--rule",
                "clippy::suspicious_command_arg_space=off",
                "--rule",
                "clippy::todo=off",
                "--rule",
                "clippy::too_many_arguments=off",
                "--rule",
                "clippy::type_complexity=off",
                "--rule",
                "clippy::unimplemented=off",
                "--rule",
                "clippy::unnecessary_to_owned=off",
                "--rule",
                "clippy::unreachable=off",
                "--rule",
                "clippy::unused_async=off",
                "--rule",
                "clippy::unwrap_used=off",
                "--rule",
                "clippy::useless_vec=off",
                "--rule",
                "clippy::vec_init_then_push=off",
                "--rule",
                "clippy::zombie_processes=off",
            ],
        ),
        (
            "cargo-health",
            vec![
                "--rule",
                "rust_doctor::cargo::unbounded_registry_dependency=off",
                "--rule",
                "rust_doctor::cargo::unpinned_git_dependency=off",
            ],
        ),
        (
            "source-kernel",
            vec![
                "--rule",
                "rust_doctor::source::disabled_tls_verification=off",
                "--rule",
                "rust_doctor::source::dynamic_shell_command=off",
            ],
        ),
    ];
    let mut execution_pruning = BTreeMap::new();
    for (producer, arguments) in producer_off_policies {
        let output = inspect(&fixture, &arguments);
        assert_eq!(output.status.code(), Some(0));
        assert_private(&output, &fixture);
        let report = report(&output);
        let active = findings(&report);
        let inactive_rules: Vec<_> = arguments
            .iter()
            .skip(1)
            .step_by(2)
            .map(|value| value.trim_end_matches("=off"))
            .collect();
        for rule in &inactive_rules {
            assert!(!active.contains_key(*rule));
        }

        let proof = match producer {
            "clippy" => {
                assert!(report["scan"]["command"].is_null());
                json!({
                    "inactive_rules": inactive_rules,
                    "scan_command": null,
                    "process_count": 0,
                    "counter_test": "baseline_kernel::disabled_clippy_producer_starts_no_scan_on_either_side",
                })
            }
            "cargo-health" => json!({
                "inactive_rules": inactive_rules,
                "dependencies_evaluated": 0,
                "unbounded_registry_predicates": 0,
                "unpinned_git_predicates": 0,
                "counter_test": "cargo_health::tests::policy_prunes_the_producer_and_each_inactive_predicate",
            }),
            "source-kernel" => json!({
                "inactive_rules": inactive_rules,
                "files_read": 0,
                "files_parsed": 0,
                "bytes_read": 0,
                "disabled_tls_predicates": 0,
                "dynamic_shell_predicates": 0,
                "counter_test": "source_kernel::tests::policy_prunes_source_io_and_each_inactive_predicate",
            }),
            _ => unreachable!("producer proof matrix is closed"),
        };
        execution_pruning.insert(producer, proof);
    }

    let invalid = [
        (vec!["--rule", "bad/path=warn"], "invalid-rule-selector"),
        (vec!["--rule", "unknown::rule=warn"], "unknown-rule"),
        (
            vec![
                "--rule",
                "clippy::todo=warn",
                "--rule",
                "clippy::todo=error",
            ],
            "duplicate-rule-override",
        ),
        (
            vec!["--category", "bad_category=warn"],
            "invalid-category-selector",
        ),
        (vec!["--category", "unknown=warn"], "unknown-category"),
        (
            vec!["--category", "security=warn", "--category", "security=off"],
            "duplicate-category-override",
        ),
    ];
    for (arguments, code) in invalid {
        let output = inspect(&fixture, &arguments);
        assert_eq!(output.status.code(), Some(2));
        let report = report(&output);
        assert_eq!(report["status"], "failed");
        assert_eq!(report["errors"][0]["code"], code);
        assert!(report["scan"]["command"].is_null());
        assert_private(&output, &fixture);
    }

    assert_eq!(content_hashes(&fixture.project), fixture.before);
    let evaluation = json!({
        "schema_version": 1,
        "toolchain": default["toolchain"],
        "inventory": RULES.map(|(id, category)| json!({
            "id": id,
            "category": category,
            "default_level": "warn",
        })),
        "v3_diagnostic_ids": legacy_v3_ids,
        "execution_pruning": execution_pruning,
        "policies": evaluation_policies,
    });
    let rule_scaling = oracle();
    let scan_commands: BTreeMap<_, _> = evaluation["policies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|policy| {
            (
                policy["name"].as_str().unwrap().to_owned(),
                policy["result"]["scan_command"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|argument| argument.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    // The command of the default policy is the living reference: every other
    // policy must be exactly that command minus the rules it turns off.
    let default_command = scan_commands
        .get("default")
        .expect("the default policy should publish its command")
        .clone();
    let expected_scan_commands: BTreeMap<_, _> = rule_scaling
        .compatibility
        .policy_disabled_clippy_rules
        .iter()
        .map(|(name, disabled)| {
            (
                name.clone(),
                clippy_command_without_rules(&default_command, disabled),
            )
        })
        .collect();
    assert_eq!(
        scan_commands,
        expected_scan_commands,
        "EP-017 scan commands differ:\n{}",
        serde_json::to_string_pretty(&evaluation).unwrap()
    );
    assert_eq!(
        evaluation["execution_pruning"]["clippy"],
        serde_json::to_value(&rule_scaling.compatibility.policy_clippy_pruning).unwrap()
    );

    fs::remove_dir_all(&fixture.root).unwrap();
}
