#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Proofs of EP-002: a scan that fails says what Cargo said, a scan bounded by
//! `--max-duration` finishes on time and kills what it started, one broken
//! member does not hide the others, and the caller's lint flags cannot turn a
//! catalogued warning into a build failure.
//!
//! Every proof starts at the binary a user runs, on a fixture under
//! `tests/fixtures/scan-failures/`, so the wire shape asserted here is the one
//! `--json` publishes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde_json::Value;

mod support;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scan-failures")
        .join(name)
        .canonicalize()
        .unwrap()
}

/// The binary on `root`, with the caller's rustflags cleared unless the test
/// sets its own: the harness may run under a `RUSTFLAGS` of its own.
fn command(root: &Path, arguments: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rust-doctor"));
    command.stdout(std::process::Stdio::piped());
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

fn run(root: &Path, arguments: &[&str]) -> Output {
    command(root, arguments).output().unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should hold the JSON report alone")
}

fn error<'a>(report: &'a Value, stage: &str, code: &str) -> &'a Value {
    let found = report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|error| error["stage"] == stage && error["code"] == code);
    assert!(
        found.is_some(),
        "no {stage}/{code} error in {}",
        report["errors"]
    );
    found.unwrap()
}

fn reasons(report: &Value) -> Vec<&str> {
    report["audit"]["score"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reason| reason.as_str().unwrap())
        .collect()
}

/// Neither the fixture's absolute path nor the home directory appears
/// anywhere in the published document.
fn assert_no_absolute_path(output: &Output, root: &Path) {
    let wire = String::from_utf8_lossy(&output.stdout);
    assert!(!wire.contains(&*root.to_string_lossy()), "{wire}");
    if let Some(home) = std::env::var_os("HOME") {
        assert!(
            !wire.contains(&*Path::new(&home).to_string_lossy()),
            "{wire}"
        );
    }
}

/// The report a fixture's scan publishes and the message of its `stage`/`code`
/// error, once the document is checked to name no absolute path.
fn published_cause(name: &str, stage: &str, code: &str) -> (Value, String) {
    let root = fixture(name);
    let output = run(&root, &["--json"]);
    assert_no_absolute_path(&output, &root);
    let report = json(&output);
    let message = error(&report, stage, code)["message"]
        .as_str()
        .unwrap()
        .to_owned();
    (report, message)
}

#[test]
fn a_lockfile_cargo_cannot_parse_publishes_cargos_own_sentence() {
    let (report, message) = published_cause("bad-lockfile", "execution", "clippy-exit");
    assert!(
        message.contains("Cargo reported: error: failed to parse lock file at: ./Cargo.lock"),
        "{message}"
    );
    assert!(message.len() <= 1_020 + "Clippy exited with status 101. Cargo reported: ".len());
    assert!(reasons(&report).contains(&"stage-failed"));
}

#[test]
fn a_manifest_cargo_metadata_refuses_publishes_its_cause() {
    let (report, message) = published_cause("bad-manifest", "metadata", "cargo-metadata");
    assert_eq!(report["status"], "failed");
    assert!(
        message.starts_with("cargo metadata failed: Cargo reported: error:"),
        "{message}"
    );
    assert!(message.contains("Cargo.toml:8"), "{message}");
}

#[test]
fn a_successful_run_publishes_nothing_from_stderr() {
    let root = fixture("lint-flags");
    let report = json(&run(&root, &["--json"]));
    assert_eq!(report["status"], "complete");
    assert!(
        report["errors"].as_array().unwrap().is_empty(),
        "{}",
        report["errors"]
    );
    assert!(!report.to_string().contains("Cargo reported"));
}

#[test]
fn a_broken_member_leaves_the_others_linted_and_is_named() {
    let root = fixture("broken-member");
    let output = run(&root, &["--json"]);
    let report = json(&output);
    let command = report["scan"]["command"].as_array().unwrap();
    assert!(command.iter().any(|argument| argument == "--keep-going"));
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "clippy::todo"
                && diagnostic["path"] == "b/src/lib.rs"),
        "the healthy member's finding is missing"
    );
    // `dependent-c` compiles on its own and fails only through `broken-a`. Its
    // build script still compiles, and that record is not a linted member.
    assert_eq!(
        error(&report, "clippy", "packages-unlinted")["message"],
        "Not linted because it did not compile: broken-a, dependent-c"
    );
    assert!(reasons(&report).contains(&"packages-unlinted"));
    assert_no_absolute_path(&output, &root);
}

#[test]
fn a_workspace_that_compiles_names_no_unlinted_package() {
    let report = json(&run(&fixture("lint-flags"), &["--json"]));
    assert!(
        !report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["code"] == "packages-unlinted")
    );
}

#[cfg(unix)]
#[test]
fn the_deadline_ends_the_scan_kills_the_build_script_and_keeps_what_was_found() {
    let root = fixture("sleeping-build");
    let pid_file =
        support::temporary_target("scan-failures", &std::sync::atomic::AtomicUsize::new(0));
    std::fs::create_dir_all(&pid_file).unwrap();
    let pid_file = pid_file.join("sleeper.pid");
    let started = Instant::now();
    let output = command(&root, &["--json", "--max-duration", "5"])
        .env("RUST_DOCTOR_SLEEPER_PID", &pid_file)
        .output()
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(output.status.code(), Some(2));

    let report = json(&output);
    assert_eq!(report["status"], "incomplete");
    assert_eq!(
        error(&report, "clippy", "deadline-exceeded")["message"],
        "The scan stopped at the 5 s limit set by --max-duration."
    );
    assert!(reasons(&report).contains(&"deadline-exceeded"));
    // The member without a build script was linted before the deadline.
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "clippy::todo")
    );
    // The native passes that had not started are named at their own stage.
    error(&report, "structure", "deadline-exceeded");

    // The build script ran in Clippy's process group, which was killed whole.
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    let gone = (0..50).any(|_| {
        let alive = Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        if alive {
            std::thread::sleep(Duration::from_millis(40));
        }
        !alive
    });
    assert!(gone, "the build script {pid} survived the deadline");
}

#[test]
fn a_max_duration_outside_its_range_is_refused_by_clap() {
    for value in ["0", "86401", "soon"] {
        let output = run(&fixture("lint-flags"), &["--max-duration", value]);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("--max-duration"));
    }
}

#[test]
fn a_caller_exporting_deny_warnings_keeps_the_warning_a_warning() {
    let root = fixture("lint-flags");
    for (variable, value) in [
        ("RUSTFLAGS", "-Dwarnings"),
        ("CARGO_ENCODED_RUSTFLAGS", "--deny\u{1f}warnings"),
    ] {
        let output = command(&root, &["--json"])
            .env(variable, value)
            .output()
            .unwrap();
        let report = json(&output);
        assert_eq!(
            report["status"], "complete",
            "{variable}: {}",
            report["errors"]
        );
        assert_eq!(report["toolchain"]["removed_lint_flags"][0], "-D warnings");
        let todo = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|diagnostic| diagnostic["code"] == "clippy::todo")
            .unwrap();
        assert_eq!(todo["severity"], "warning");
        // The gate decides the exit: the default blocks on errors only.
        assert_eq!(output.status.code(), Some(0));
    }
}

#[test]
fn the_workspace_configuration_cannot_deny_warnings_either() {
    let report = json(&run(&fixture("lint-flags-config"), &["--json"]));
    assert_eq!(report["status"], "complete", "{}", report["errors"]);
    assert_eq!(report["toolchain"]["removed_lint_flags"][0], "-D warnings");
}

#[test]
fn rustflags_without_a_lint_level_are_left_alone() {
    let report = json(
        &command(&fixture("lint-flags"), &["--json"])
            .env("RUSTFLAGS", "-Cdebuginfo=0")
            .output()
            .unwrap(),
    );
    assert_eq!(report["status"], "complete");
    assert_eq!(
        report["toolchain"]["removed_lint_flags"],
        serde_json::json!([])
    );
}

#[test]
fn off_a_terminal_each_phase_prints_one_line() {
    let output = run(&fixture("broken-member"), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let phases: Vec<_> = stderr
        .lines()
        .filter(|line| {
            line.starts_with("Compiling dependencies")
                || line.starts_with("Linting ")
                || line.starts_with("Running native passes")
        })
        .collect();
    assert!(!stderr.contains('\r'), "{stderr}");
    assert!(!stderr.contains("Scanning Rust files"), "{stderr}");
    assert!(phases.len() <= 5, "{phases:?}");
    assert!(phases.contains(&"Compiling dependencies..."), "{stderr}");
    assert!(phases.contains(&"Running native passes..."), "{stderr}");
    assert!(
        phases
            .iter()
            .any(|line| line.starts_with("Linting healthy-b (")),
        "{stderr}"
    );
}

#[test]
fn a_json_run_prints_no_progress() {
    let output = run(&fixture("lint-flags"), &["--json"]);
    json(&output);
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("Compiling dependencies"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A git repository holding a copy of `fixture`, committed once, under a
/// scratch directory of its own.
fn committed_copy(name: &str) -> PathBuf {
    let root = support::temporary_target("scan-failures", &std::sync::atomic::AtomicUsize::new(0))
        .join(name);
    support::copy_tree(&fixture(name), &root);
    for arguments in [
        &["init", "-q"][..],
        &["add", "-A"],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "base",
        ],
    ] {
        support::git_output(&root, arguments);
    }
    root
}

#[test]
fn two_concurrent_baseline_runs_on_one_workspace_both_complete() {
    let root = committed_copy("lint-flags");
    let spawn = || {
        command(&root, &["--json", "--scope", "baseline", "--base", "HEAD"])
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .spawn()
            .unwrap()
    };
    let (first, second) = (spawn(), spawn());
    for child in [first, second] {
        let output = child.wait_with_output().unwrap();
        let report = json(&output);
        assert_eq!(report["status"], "complete", "{}", report["errors"]);
    }
    // Both built the base side where Cargo builds the workspace, never under
    // the workspace's sources.
    assert!(
        support::scan_target(&root)
            .join("rust-doctor/baseline")
            .is_dir()
    );
}
