#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

const TLS: &str = "rust_doctor::source::disabled_tls_verification";
const SHELL: &str = "rust_doctor::source::dynamic_shell_command";

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/source-kernel")
        .join(name)
}

fn temporary_workspace(label: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/source-kernel-cli")
        .join(format!(
            "{}-{}-{label}",
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
        if path.file_name().is_some_and(|name| name == "target") {
            continue;
        }
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
                if path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                visit(root, &path, hashes);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                hashes.insert(relative, blake3::hash(&fs::read(path).unwrap()));
            }
        }
    }

    let mut hashes = BTreeMap::new();
    visit(root, root, &mut hashes);
    hashes
}

fn command(path: &Path, json: bool, cargo_home: &Path, target: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rust-doctor"));
    command.arg("inspect");
    if json {
        command.arg("--json");
    }
    command
        .arg(path)
        .env("CARGO_HOME", cargo_home)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", target);
    command
}

fn inspect(path: &Path, json: bool, cargo_home: &Path, target: &Path) -> Output {
    command(path, json, cargo_home, target).output().unwrap()
}

fn report(output: &Output) -> Value {
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn source_findings(report: &Value) -> Vec<&Value> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == TLS || diagnostic["code"] == SHELL)
        .collect()
}

fn non_source_ids(report: &Value) -> BTreeSet<String> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] != TLS && diagnostic["code"] != SHELL)
        .map(|diagnostic| diagnostic["id"].as_str().unwrap().to_owned())
        .collect()
}

fn replace(path: &Path, from: &str, to: &str) {
    let source = fs::read_to_string(path).unwrap();
    assert!(source.contains(from));
    fs::write(path, source.replacen(from, to, 1)).unwrap();
}

fn assert_private(output: &Output, workspace: &Path) {
    let mut rendered = String::from_utf8_lossy(&output.stdout).into_owned();
    rendered.push_str(&String::from_utf8_lossy(&output.stderr));
    for forbidden in [
        "RD_SOURCE_PAYLOAD_8f9c",
        "RD_SOURCE_URL_8f9c",
        "RD_SOURCE_CREDENTIAL_8f9c",
        "RD_SOURCE_PATH_8f9c",
        "RD_SOURCE_ANSI_8f9c",
        "\u{1b}",
        workspace.to_string_lossy().as_ref(),
    ] {
        assert!(!rendered.contains(forbidden), "output leaked {forbidden:?}");
    }
}

#[test]
fn offline_cli_is_deterministic_private_and_correction_preserves_other_ids() {
    let root = temporary_workspace("loop");
    let project = root.join("project");
    copy_tree(&fixture("product-loop"), &project);
    let cargo_home = root.join("cargo-home");
    let target = root.join("target");
    let before_scan = content_hashes(&project);

    let mut expected_json = None;
    let mut initial_report = None;
    for _ in 0..20 {
        let output = inspect(&project, true, &cargo_home, &target);
        assert_eq!(output.status.code(), Some(0));
        assert_private(&output, &project);
        match expected_json.as_ref() {
            Some(expected) => assert_eq!(&output.stdout, expected),
            None => expected_json = Some(output.stdout.clone()),
        }
        initial_report = Some(report(&output));
    }
    assert_eq!(content_hashes(&project), before_scan);

    let initial_report = initial_report.unwrap();
    assert_eq!(initial_report["schema_version"], 14);
    assert_eq!(initial_report["status"], "complete");
    let findings = source_findings(&initial_report);
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().any(|finding| finding["code"] == TLS));
    assert!(findings.iter().any(|finding| finding["code"] == SHELL));
    let targeted_ids: BTreeSet<_> = findings
        .iter()
        .map(|finding| finding["id"].as_str().unwrap().to_owned())
        .collect();
    let untargeted_ids = non_source_ids(&initial_report);
    assert!(!untargeted_ids.is_empty());

    let terminal_a = inspect(&project, false, &cargo_home, &target);
    let terminal_b = inspect(&project, false, &cargo_home, &target);
    assert_eq!(terminal_a.status.code(), Some(0));
    assert_eq!(terminal_a.stdout, terminal_b.stdout);
    let terminal = String::from_utf8_lossy(&terminal_a.stdout);
    // The default rendering exposes a single group, the one with the strongest
    // contribution. Every finding weighs one occurrence, since the scan
    // compiles the library once, so the tie the scale used to break on volume
    // is now broken on tier and the P0 security finding leads. Both P0 rules
    // stay named as the cause of the score cap, including the one left
    // undeveloped.
    assert_eq!(terminal.matches("Rule ID: ").count(), 1, "{terminal}");
    assert!(terminal.contains(&format!("Rule ID: {TLS}")), "{terminal}");
    assert!(
        !terminal.contains(&format!("Rule ID: {SHELL}")),
        "{terminal}"
    );
    assert!(
        terminal.contains("Capped at 40/100 by a P0 finding"),
        "{terminal}"
    );
    assert!(
        terminal.contains(TLS) && terminal.contains(SHELL),
        "{terminal}"
    );
    assert_private(&terminal_a, &project);

    let verbose = command(&project, false, &cargo_home, &target)
        .arg("--verbose")
        .output()
        .unwrap();
    let verbose = String::from_utf8_lossy(&verbose.stdout);
    assert!(verbose.contains(&format!("Rule ID: {TLS}")), "{verbose}");
    assert!(verbose.contains(&format!("Rule ID: {SHELL}")), "{verbose}");

    let source = project.join("app/src/lib.rs");
    replace(
        &source,
        "reqwest::Client::builder().danger_accept_invalid_certs(true)",
        "reqwest::Client::builder().danger_accept_invalid_certs(false)",
    );
    replace(
        &source,
        "std::process::Command::new(\"sh\")\n        .arg(\"-c\")\n        .arg(format!(\"echo {user}\"))",
        "std::process::Command::new(\"printf\")\n        .arg(\"%s\")\n        .arg(user)",
    );
    let corrected_before_scan = content_hashes(&project);
    let corrected = inspect(&project, true, &cargo_home, &target);
    assert_eq!(corrected.status.code(), Some(0));
    assert_private(&corrected, &project);
    assert_eq!(content_hashes(&project), corrected_before_scan);
    let corrected = report(&corrected);
    assert!(source_findings(&corrected).is_empty());
    assert_eq!(non_source_ids(&corrected), untargeted_ids);
    assert!(
        corrected["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| !targeted_ids.contains(diagnostic["id"].as_str().unwrap()))
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_preserves_valid_findings_across_source_failures_and_bounds() {
    let root = temporary_workspace("failures");
    let errors = fixture("errors");
    let errors_before = content_hashes(&errors);
    let output = inspect(
        &errors,
        true,
        &root.join("errors-cargo-home"),
        &root.join("errors-target"),
    );
    assert_eq!(output.status.code(), Some(1));
    assert_private(&output, &errors);
    let error_report = report(&output);
    assert_eq!(error_report["status"], "incomplete");
    assert_eq!(error_report["complete"], false);
    assert_eq!(source_findings(&error_report).len(), 2);
    let source_errors: Vec<_> = error_report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|error| error["stage"] == "source")
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        source_errors,
        [
            "module-ambiguous",
            "module-not-found",
            "parse-error",
            "parse-error",
            "path-outside-workspace",
            "path-outside-workspace",
        ]
    );
    assert_eq!(content_hashes(&errors), errors_before);

    let unreadable = root.join("unreadable");
    fs::create_dir_all(unreadable.join("src")).unwrap();
    fs::write(
        unreadable.join("Cargo.toml"),
        "[package]\nname = \"source-unreadable\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(unreadable.join("src/lib.rs"), [0xff, 0xfe]).unwrap();
    let output = inspect(
        &unreadable,
        true,
        &root.join("unreadable-cargo-home"),
        &root.join("unreadable-target"),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        report(&output)["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["stage"] == "source" && error["code"] == "read-failed")
    );

    let limited = root.join("limited");
    fs::create_dir_all(limited.join("src")).unwrap();
    fs::write(
        limited.join("Cargo.toml"),
        "[package]\nname = \"source-limited\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    let mut oversized = b"pub fn valid() {}\n".to_vec();
    oversized.resize(8_388_609, b' ');
    fs::write(limited.join("src/lib.rs"), oversized).unwrap();
    let output = inspect(
        &limited,
        true,
        &root.join("limited-cargo-home"),
        &root.join("limited-target"),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        report(&output)["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["stage"] == "source" && error["code"] == "limit-exceeded")
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_failures_before_scan_never_emit_source_output() {
    let root = temporary_workspace("pre-scan");

    let discovery = inspect(
        &root.join("missing"),
        true,
        &root.join("discovery-cargo-home"),
        &root.join("discovery-target"),
    );
    assert_eq!(discovery.status.code(), Some(2));

    let malformed = root.join("malformed");
    fs::create_dir_all(&malformed).unwrap();
    fs::write(malformed.join("Cargo.toml"), "not valid TOML = [").unwrap();
    let metadata = inspect(
        &malformed,
        true,
        &root.join("metadata-cargo-home"),
        &root.join("metadata-target"),
    );
    assert_eq!(metadata.status.code(), Some(2));

    let mut spawn = command(
        &fixture("product-loop"),
        true,
        &root.join("spawn-cargo-home"),
        &root.join("spawn-target"),
    );
    spawn.env("PATH", "/definitely/missing/rust-doctor-path");
    let spawn = spawn.output().unwrap();
    assert_eq!(spawn.status.code(), Some(2));

    for output in [&discovery, &metadata, &spawn] {
        let report = report(output);
        assert_eq!(report["status"], "failed");
        assert!(source_findings(&report).is_empty());
        assert!(
            report["errors"]
                .as_array()
                .unwrap()
                .iter()
                .all(|error| error["stage"] != "source")
        );
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_deduplicates_sources_reached_from_multiple_targets() {
    let root = temporary_workspace("shared-target");
    let output = inspect(
        &fixture("precision"),
        true,
        &root.join("cargo-home"),
        &root.join("target"),
    );
    assert_eq!(output.status.code(), Some(0));
    let report = report(&output);
    let shared: Vec<_> = source_findings(&report)
        .into_iter()
        .filter(|finding| finding["path"] == "app/src/shared.rs")
        .collect();
    assert_eq!(shared.len(), 1);
    assert!(shared[0]["target"].is_null());
    fs::remove_dir_all(root).unwrap();
}
