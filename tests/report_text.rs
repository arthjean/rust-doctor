#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn workspace() -> PathBuf {
    let root = support::fresh_workspace("report-text", &COUNTER);
    support::copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/report-text"),
        &root,
    );
    root
}

fn scan(root: &Path, arguments: &[&str], environment: &[(&str, &str)]) -> Output {
    let mut command = support::rust_doctor();
    command
        .arg(root)
        .arg("--yes")
        .args(support::NO_ADVISORY_DB)
        .args(arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("RUST_DOCTOR_LAUNCHER");
    for (name, value) in environment {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    output
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn position(text: &str, needle: &str) -> usize {
    let found = text.find(needle);
    assert!(found.is_some(), "{needle:?} is missing from:\n{text}");
    found.unwrap()
}

const KEEP_WARNING: [&str; 2] = ["--rule", "clippy::unwrap_used=warn"];

#[test]
fn a_distrusted_rule_publishes_at_info_unless_the_request_keeps_its_warning() {
    let root = workspace();
    let text = stdout(&scan(&root, &[], &[]));
    position(&text, "P3 info  Unwrap used  x0 (+1 info)");
    assert!(!text.contains("Reported, but found wrong"), "{text}");
    assert!(!text.contains("Left out of what to fix first"), "{text}");
}

#[test]
fn every_rule_header_names_its_tier_severity_and_what_each_count_counts() {
    let root = workspace();
    let verbose = [KEEP_WARNING[0], KEEP_WARNING[1], "--verbose"];
    for arguments in [&KEEP_WARNING[..], &verbose[..]] {
        let text = stdout(&scan(&root, arguments, &[]));
        let measured = position(&text, "P3 warning  Rc buffer  x1");
        let heading = position(&text, "Reported, but found wrong on most reviewed sites\n");
        let withheld = position(&text, "P3 warning  Unwrap used  x1");
        assert!(
            measured < heading && heading < withheld,
            "a distrusted rule closes its tier under one sub-heading: {text}"
        );
        assert_eq!(text.matches("Reported, but found wrong").count(), 1, "{text}");
        assert!(!text.contains("more)"), "{text}");
        assert!(!text.contains("Top warning"), "{text}");
    }
}

#[test]
fn the_tallies_use_the_dimension_names_the_score_line_uses() {
    let root = workspace();
    let text = stdout(&scan(&root, &KEEP_WARNING, &[]));
    let reliability = position(&text, "\nReliability: 1 warning (1 from Unwrap used)\n");
    let performance = position(&text, "\nPerformance: 1 warning (1 from Rc buffer)\n");
    assert!(reliability < performance, "{text}");
    assert!(!text.contains("Bugs"), "{text}");

    let report: Value = serde_json::from_slice(&scan(&root, &["--json"], &[]).stdout).unwrap();
    let names: Vec<&str> = report["audit"]["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| category["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Bugs"), "the wire keeps its value: {names:?}");
}

#[test]
fn the_footer_says_what_to_fix_what_the_corpus_distrusts_and_what_its_terms_mean() {
    let root = workspace();
    let text = stdout(&scan(&root, &KEEP_WARNING, &[]));
    let json = [KEEP_WARNING[0], KEEP_WARNING[1], "--json"];
    let report: Value = serde_json::from_slice(&scan(&root, &json, &[]).stdout).unwrap();
    let production = |severity: &str| -> u64 {
        report["audit"]["categories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|category| category["production"][severity].as_u64().unwrap())
            .sum()
    };
    let warnings = production("warnings");
    let info = production("info");
    assert!(
        text.contains(&format!("; {warnings} scored warnings, {info} info\n")),
        "{text}"
    );
    let share = text
        .split_whitespace()
        .find(|word| word.starts_with("https://rust-doctor.com/share?"))
        .unwrap();
    assert!(share.contains(&format!("&w={warnings}")), "{share}");
    let flat = text.replace('\n', " ");
    assert!(
        text.contains("\n\nLeft out of what to fix first: clippy::unwrap_used"),
        "the distrusted rules open a paragraph of their own: {text}"
    );
    assert!(
        flat.contains(
            "Left out of what to fix first: clippy::unwrap_used (86% noise on 5 sites). The \
             corpus found it wrong on most of the sites it reviewed."
        ),
        "{text}"
    );
    assert_eq!(
        flat.matches(
            "P0 to P3 rank a rule's severity. Noise is the share of reviewed sites where the \
             corpus (pinned public repositories) found the rule wrong; unmeasured means no site \
             was reviewed, and the score charges such a rule at half."
        )
        .count(),
        1,
        "{text}"
    );
    assert!(!text.contains("already hold it lower"), "{text}");
    assert!(!text.contains("ceiling:"), "{text}");
}

#[test]
fn the_gate_configuration_and_scope_lines_each_say_one_thing() {
    let root = workspace();
    let text = stdout(&scan(&root, &[], &[]));
    assert!(
        text.contains("\nExit gate (--blocking error): passed, no error-level finding.\n"),
        "{text}"
    );
    assert!(
        text.contains("\nConfiguration: no rust-doctor.toml; defaults apply.\n"),
        "{text}"
    );
    let flat = text.replace('\n', " ");
    let scope = flat
        .split_once("Scored 1 production files")
        .and_then(|(_, rest)| rest.split_once(" P3 "))
        .map(|(scope, _)| scope);
    assert!(scope.is_some(), "{text}");
    let scope = scope.unwrap();
    let triple = scope
        .split_once("Clippy linted library and binary targets for ")
        .and_then(|(_, rest)| rest.split_once("; "))
        .map(|(triple, _)| triple);
    assert!(triple.is_some(), "{scope}");
    let triple = triple.unwrap();
    assert!(triple.split('-').count() >= 3, "{scope}");
    assert!(
        scope.ends_with(
            "; code gated to other platforms and test code were not linted. Native rules also \
             read tests."
        ),
        "{scope}"
    );
}

#[test]
fn off_a_terminal_progress_closes_on_one_line_and_the_skill_hint_opens_stderr() {
    let root = workspace();
    let version = env!("CARGO_PKG_VERSION");
    let direct = stderr(&scan(&root, &[], &[("CLAUDECODE", "1")]));
    let lines: Vec<&str> = direct.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some(
            "rust-doctor: no rust-doctor skill is installed for claude; run `rust-doctor skill \
             install --agent claude`."
        ),
        "{direct}"
    );
    assert_eq!(lines.last().copied(), Some("Linted 1 member."), "{direct}");
    assert!(!direct.contains("Linting "), "{direct}");

    for runner in ["bunx", "npx"] {
        let launched = stderr(&scan(
            &root,
            &[],
            &[("CLAUDECODE", "1"), ("RUST_DOCTOR_LAUNCHER", runner)],
        ));
        assert!(
            launched.starts_with(&format!(
                "rust-doctor: no rust-doctor skill is installed for claude; run `{runner} \
                 rust-doctor@{version} skill install --agent claude`.\n"
            )),
            "{launched}"
        );
    }
}

#[test]
fn a_replacement_is_printed_under_the_text_it_replaces() {
    let root = workspace();
    let text = stdout(&scan(&root, &["--verbose"], &[]));
    let rows: Vec<&str> = text.lines().collect();
    let replace = rows
        .iter()
        .position(|row| *row == "Replace `Vec<u8>` with: [u8] (unspecified)");
    assert!(replace.is_some(), "{text}");
    let replace = replace.unwrap();
    assert_eq!(rows.get(replace + 1).copied(), Some("src/lib.rs:4:13"), "{text}");
    let source = rows
        .iter()
        .skip(replace)
        .find(|row| row.contains("values: Rc<Vec<u8>>,"))
        .unwrap();
    let carets = rows
        .iter()
        .skip_while(|row| *row != source)
        .nth(1)
        .unwrap();
    assert_eq!(carets.matches('^').count(), "Vec<u8>".len(), "{text}");
    assert_eq!(carets.find('^'), source.find("Vec<u8>"), "{text}");

    let report: Value = serde_json::from_slice(&scan(&root, &["--json"], &[]).stdout).unwrap();
    let suggestion = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "clippy::rc_buffer")
        .map(|diagnostic| &diagnostic["suggestion"])
        .unwrap();
    assert_eq!(
        suggestion,
        &serde_json::json!({ "replacement": "[u8]", "applicability": "unspecified" }),
        "the JSON suggestion keeps its schema 24 shape"
    );
}
