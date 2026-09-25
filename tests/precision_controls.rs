#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Proofs of EP-004: an ignored path, a per-path override, a suppression
//! directive with a reason, a file declared generated, and a scan narrowed to
//! one member, each visible in the report.
//!
//! Every proof starts at the binary a user runs, on a fixture under
//! `tests/fixtures/precision-controls/`, so the wire shape asserted here is
//! the one `--json` publishes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

mod support;

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/precision-controls")
        .join(name)
        .canonicalize()
        .unwrap()
}

fn command(root: &Path, arguments: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rust-doctor"));
    command
        .arg(root)
        .arg("--yes")
        .args(arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    command
}

fn json(root: &Path, arguments: &[&str]) -> Value {
    let output = command(root, &[&["--json"], arguments].concat()).output().unwrap();
    serde_json::from_slice(&output.stdout).expect("stdout should hold the JSON report alone")
}

/// `(rule, path, line, severity)` of every finding.
fn findings(report: &Value) -> Vec<(String, String, u64, String)> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| !diagnostic["span"].is_null())
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap().to_owned(),
                diagnostic["path"].as_str().unwrap().to_owned(),
                diagnostic["span"]["line_start"].as_u64().unwrap(),
                diagnostic["severity"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn finding(rule: &str, path: &str, line: u64, severity: &str) -> (String, String, u64, String) {
    (rule.to_owned(), path.to_owned(), line, severity.to_owned())
}

/// A copy of a fixture in a scratch directory, optionally made a git
/// repository whose index tracks every file.
fn workspace(name: &str) -> PathBuf {
    let root = support::temporary_target("precision-controls", &NEXT_WORKSPACE);
    support::copy_tree(&fixture(name), &root);
    root
}

#[test]
fn an_ignored_path_leaves_the_report_and_an_override_sets_a_level_under_its_path() {
    let root = fixture("paths");
    let report = json(&root, &[]);
    assert_eq!(report["status"], "complete");
    // `src/raised.rs` is raised to error, `src/quiet/**` switches its category
    // off, and nothing under `src/ignored` is left.
    assert_eq!(
        findings(&report),
        [
            finding("clippy::todo", "src/lib.rs", 6, "warning"),
            finding("clippy::todo", "src/raised.rs", 2, "error"),
        ]
    );
    // Clippy still compiled the ignored file, so its `todo!` was produced and
    // then counted out. The walk never read it, so its shell command was never
    // a finding at all, and its lines are not in the denominator.
    assert_eq!(report["scan"]["ignored"], 1);
    assert_eq!(report["audit"]["production_lines"], 13);
    assert_eq!(report["policy"]["ignore"], serde_json::json!(["src/ignored"]));
    assert_eq!(report["policy"]["overrides"][0]["paths"][0], "src/raised.rs");
    assert_eq!(report["policy"]["overrides"][0]["rules"]["clippy::todo"], "error");
    assert_eq!(report["policy"]["overrides"][1]["categories"]["correctness"], "off");

    // A level the request sets is the reader's answer for the run, under
    // every path.
    let requested = json(&root, &["--rule", "clippy::todo=warn"]);
    assert_eq!(
        findings(&requested),
        [
            finding("clippy::todo", "src/lib.rs", 6, "warning"),
            finding("clippy::todo", "src/quiet/mod.rs", 2, "warning"),
            finding("clippy::todo", "src/raised.rs", 2, "warning"),
        ]
    );
}

#[test]
fn a_malformed_glob_or_an_unknown_override_rule_is_refused_by_name_and_index() {
    for (configuration, stage, code, named) in [
        (
            "[ignore]\npaths = [\"src\", \"/private/secret\"]\n",
            "policy",
            "invalid-glob",
            Some("ignore.paths[1]"),
        ),
        (
            "[[overrides]]\npaths = [\"src\"]\n[[overrides]]\npaths = [\"a/{secret,b}\"]\n",
            "policy",
            "invalid-glob",
            Some("overrides[1].paths[0]"),
        ),
        (
            "[[overrides]]\npaths = [\"src\"]\nrules = { \"clippy::no_such_lint\" = \"off\" }\n",
            "configuration",
            "unknown-rule",
            None,
        ),
    ] {
        let root = workspace("members");
        fs::write(root.join("rust-doctor.toml"), configuration).unwrap();
        let output = command(&root, &["--json"]).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{configuration}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["errors"][0]["stage"], stage, "{configuration}");
        assert_eq!(report["errors"][0]["code"], code, "{configuration}");
        let message = report["errors"][0]["message"].as_str().unwrap();
        if let Some(named) = named {
            assert!(message.contains(named), "{message}");
        }
        for secret in ["/private/secret", "{secret", "no_such_lint"] {
            assert!(!message.contains(secret), "{message}");
        }
    }
}

#[test]
fn a_directive_with_a_reason_suppresses_one_native_site_and_every_other_says_why_not() {
    let root = fixture("suppressed");
    let report = json(&root, &[]);
    let suppressions: Vec<(String, u64, String)> = report["suppressions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|suppression| {
            (
                suppression["path"].as_str().unwrap().to_owned(),
                suppression["line"].as_u64().unwrap(),
                suppression["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    // The directive written in a string literal on line 2 is not one.
    assert_eq!(
        suppressions,
        [
            ("Cargo.toml".to_owned(), 8, "applied".to_owned()),
            ("src/lib.rs".to_owned(), 5, "applied".to_owned()),
            ("src/lib.rs".to_owned(), 10, "missing-reason".to_owned()),
            ("src/lib.rs".to_owned(), 15, "use-expect".to_owned()),
            ("src/lib.rs".to_owned(), 20, "unknown-rule".to_owned()),
            ("src/lib.rs".to_owned(), 24, "unused".to_owned()),
        ]
    );
    assert!(
        report["suppressions"][3]["help"]
            .as_str()
            .unwrap()
            .contains("#[expect(clippy::todo, reason = \"...\")]")
    );
    assert_eq!(
        report["suppressions"][4]["hint"],
        "rust_doctor::source::dynamic_shell_command"
    );
    // The applied directives took their finding out of the report; the one
    // without a reason, the one naming a Clippy lint, and the lint table
    // entry's twin in the manifest all left theirs in.
    assert_eq!(
        findings(&report),
        [
            finding("rust_doctor::source::dynamic_shell_command", "src/lib.rs", 11, "warning"),
            finding("clippy::todo", "src/lib.rs", 16, "warning"),
        ]
    );
    assert!(
        !report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "rust_doctor::cargo::permissive_lint_table")
    );

    let terminal = command(&root, &[]).output().unwrap();
    let text = String::from_utf8_lossy(&terminal.stdout);
    // One line, which the eighty-column report may wrap.
    let summary = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        summary.contains(
            "Suppressions: 2 applied, 1 unused, 1 missing-reason, 1 use-expect, 1 unknown-rule"
        ),
        "{text}"
    );
}

#[test]
fn a_file_declared_generated_or_headed_as_one_grades_nothing() {
    let root = workspace("generated");
    fs::rename(root.join("gitattributes"), root.join(".gitattributes")).unwrap();
    support::git_output(&root, &["init", "--quiet"]);
    support::git_output(&root, &["add", "--all"]);
    let report = json(&root, &[]);
    assert_eq!(report["status"], "complete");
    // `set` and `true` exclude; `false` and `unset` keep; the generator header
    // excludes a Clippy finding as it always excluded a structural one.
    assert_eq!(
        findings(&report),
        [
            finding("clippy::todo", "src/kept_false.rs", 2, "warning"),
            finding("clippy::todo", "src/kept_unset.rs", 2, "warning"),
            finding("clippy::todo", "src/lib.rs", 8, "warning"),
        ]
    );
    // Two `todo!` in declared files, one in the headed file, and the lint
    // table entry of the manifest marked generated.
    assert_eq!(report["scan"]["excluded_generated"], 4);
    assert!(report["errors"].as_array().unwrap().is_empty());

    // The base side is a snapshot outside git, and still reads the
    // declaration: nothing a declared file holds reads as fixed.
    support::git_output(
        &root,
        &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "--quiet", "-m", "base"],
    );
    let baseline = json(&root, &["--scope", "baseline", "--base", "HEAD"]);
    assert_eq!(baseline["status"], "complete", "{}", baseline["errors"]);
    // The three kept `todo!` are on both sides, and no declared file's finding
    // was dropped from one side only.
    assert_eq!(baseline["delta"]["summary"]["fixed"], 0);
    assert_eq!(baseline["delta"]["summary"]["pre_existing"], 3);

    // Outside git only the header answers, and no error says git is missing.
    let outside = workspace("generated");
    let output = command(&outside, &["--json"])
        .env("GIT_CEILING_DIRECTORIES", outside.parent().unwrap())
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["scan"]["excluded_generated"], 1);
    assert!(
        findings(&report)
            .iter()
            .any(|(rule, path, _, _)| rule == "clippy::todo" && path == "src/declared.rs")
    );
    assert!(
        report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .all(|error| error["stage"] != "generated"),
        "{}",
        report["errors"]
    );
}

#[test]
fn one_member_is_scanned_and_scored_alone_and_every_member_gets_a_score() {
    let root = fixture("members");
    let whole = json(&root, &[]);
    let members: Vec<(&str, u64)> = whole["audit"]["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| {
            (
                member["name"].as_str().unwrap(),
                member["production_lines"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(members, [("alpha", 3), ("beta", 7)]);
    assert_eq!(whole["audit"]["production_lines"], 10);
    assert_eq!(whole["audit"]["packages"][1]["score"]["value"], 100);
    assert!(whole["audit"]["packages"][0]["score"]["value"].as_u64().unwrap() < 100);

    let alpha = json(&root, &["--package", "alpha"]);
    let command_line: Vec<&str> = alpha["scan"]["command"]
        .as_array()
        .unwrap()
        .iter()
        .map(|argument| argument.as_str().unwrap())
        .collect();
    assert!(command_line.windows(2).any(|pair| pair == ["-p", "alpha"]));
    assert!(!command_line.contains(&"--workspace"));
    assert_eq!(
        findings(&alpha),
        [finding("clippy::todo", "alpha/src/lib.rs", 2, "warning")]
    );
    assert_eq!(alpha["audit"]["packages"].as_array().unwrap().len(), 1);
    assert_eq!(alpha["audit"]["packages"][0]["name"], "alpha");
    assert_eq!(alpha["audit"]["production_lines"], 3);
    // The repository is no member: its rules are named, not silently dropped.
    let repository_rules: Vec<&Value> = alpha["policy"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rule| rule["id"].as_str().unwrap().starts_with("rust_doctor::repo::"))
        .collect();
    assert!(!repository_rules.is_empty());
    assert!(
        repository_rules
            .iter()
            .all(|rule| rule["not_evaluated"] == "package-scoped")
    );
    assert!(
        alpha["audit"]["score"]["reasons"]
            .as_array()
            .unwrap()
            .contains(&Value::from("rules-not-evaluated"))
    );

    let verbose = command(&root, &["--verbose", "--package", "alpha"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&verbose.stdout);
    assert!(text.contains("Member alpha: "), "{text}");
    assert!(text.contains("worst tier"), "{text}");
    assert!(!text.contains("Member beta"), "{text}");

    let unknown = command(&root, &["--json", "--package", "nope"]).output().unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&unknown.stdout).unwrap();
    assert_eq!(report["errors"][0]["stage"], "policy");
    assert_eq!(report["errors"][0]["code"], "unknown-package");
    assert_eq!(
        report["errors"][0]["message"],
        "--package names no workspace member. Members: alpha, beta."
    );
}

#[test]
fn a_manifest_directive_above_a_dependency_key_suppresses_the_finding_it_names() {
    // A dependency finding publishes no line, so the directive reaches it
    // through the key declared on the line below.
    let root = workspace("suppressed");
    fs::create_dir_all(root.join("helper/src")).unwrap();
    fs::write(
        root.join("helper/Cargo.toml"),
        "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n",
    )
    .unwrap();
    fs::write(root.join("helper/src/lib.rs"), "").unwrap();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        format!(
            "{manifest}\n[dependencies]\n# rust-doctor: allow(rust_doctor::cargo::unused_dependency) -- linked for its side effect\nhelper = {{ path = \"helper\" }}\n"
        ),
    )
    .unwrap();
    let report = json(&root, &[]);
    assert_eq!(report["status"], "complete", "{}", report["errors"]);
    assert!(
        !report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "rust_doctor::cargo::unused_dependency"),
        "{}",
        report["diagnostics"]
    );
    assert!(
        report["suppressions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|suppression| suppression["path"] == "Cargo.toml"
                && suppression["line"] == 12
                && suppression["status"] == "applied"),
        "{}",
        report["suppressions"]
    );
}
