#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::string_slice))]

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

/// Every string of a JSON document, keys included.
fn strings_of(value: &Value) -> Vec<&str> {
    match value {
        Value::String(text) => vec![text.as_str()],
        Value::Array(items) => items.iter().flat_map(strings_of).collect(),
        Value::Object(fields) => fields
            .iter()
            .flat_map(|(key, value)| std::iter::once(key.as_str()).chain(strings_of(value)))
            .collect(),
        _ => Vec::new(),
    }
}

/// The places a report could disclose: `roots`, the home directory, and the
/// `RUSTUP_HOME` and `CARGO_HOME` the run resolves, each as written and as
/// the file system resolves it.
fn machine_paths(roots: &[&Path], rustup_home: Option<&Path>) -> Vec<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let resolved = |variable: &str, default: &str| {
        std::env::var_os(variable)
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(default)))
    };
    let rustup = rustup_home.map(Path::to_path_buf).or_else(|| resolved("RUSTUP_HOME", ".rustup"));
    let cargo = resolved("CARGO_HOME", ".cargo");
    roots
        .iter()
        .map(|root| root.to_path_buf())
        .chain([home, rustup, cargo].into_iter().flatten())
        .flat_map(|path| [Some(path.clone()), path.canonicalize().ok()])
        .flatten()
        .map(|path| path.to_string_lossy().into_owned())
        .filter(|path| path.len() > 1)
        .collect()
}

/// EP-001 US-004: no string of the published document holds any of
/// `machine_paths`.
fn assert_publishes_no_machine_path(report: &Value, roots: &[&Path], rustup_home: Option<&Path>) {
    let paths = machine_paths(roots, rustup_home);
    for text in strings_of(report) {
        for path in &paths {
            assert!(!text.contains(path.as_str()), "{path} in {text}");
        }
    }
}

/// Neither the fixture's absolute path nor any home appears anywhere in the
/// published document.
fn assert_no_absolute_path(output: &Output, root: &Path) {
    assert_publishes_no_machine_path(&json(output), &[root], None);
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
    // EP-002 US-009: Cargo stopped before it built anything, so nothing was
    // linted and nothing is scored.
    assert_eq!(report["audit"]["score"], Value::Null, "{}", report["audit"]);
    assert!(error(&report, "clippy", "packages-unlinted")["message"].is_string());
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
    // The member whose build script never returns is named, and no score
    // of its own reads its silence as clean.
    // A parallel test run shares Cargo's home, so the scan may also have
    // waited on another Cargo's package cache lock, which it then names.
    let message = error(&report, "clippy", "deadline-exceeded")["message"]
        .as_str()
        .unwrap();
    let expected = "The scan stopped at the 5 s limit set by --max-duration. Not linted: sleeper.";
    assert!(message.starts_with(expected), "{message}");
    let rest = &message[expected.len()..];
    assert!(
        ["", " Cargo waited on the package cache lock held by another Cargo."].contains(&rest),
        "{message}"
    );
    let sleeper = report["audit"]["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "sleeper")
        .unwrap();
    assert_eq!(sleeper["unlinted"], true, "{sleeper}");
    assert!(sleeper["score"].is_null(), "{sleeper}");
    assert!(reasons(&report).contains(&"deadline-exceeded"));
    // The member without a build script was linted before the deadline.
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "clippy::todo")
    );
    // The native passes ran beside Clippy rather than after it, so the
    // deadline that stopped Clippy left none of them unstarted.
    assert!(
        report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .all(|error| error["stage"] == "clippy"),
        "{}",
        report["errors"]
    );

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

/// A scan of a committed copy of `sleeping-build` with `arguments`, once its
/// build script runs: the child, the build script's pid and the temporary
/// root the run snapshots into. The copy is a repository of its own, so the
/// temporary root under `target/` sits outside the repository it scans, as a
/// baseline requires.
#[cfg(unix)]
fn sleeping_scan(label: &str, arguments: &[&str]) -> (std::process::Child, String, PathBuf) {
    let root = committed_copy("sleeping-build");
    let scratch = support::temporary_target(label, &NEXT_COPY);
    let _ = std::fs::remove_dir_all(&scratch);
    let temporary = scratch.join("tmp");
    std::fs::create_dir_all(&temporary).unwrap();
    let pid_file = scratch.join("sleeper.pid");
    let child = command(&root, arguments)
        .env("RUST_DOCTOR_SLEEPER_PID", &pid_file)
        .env("TMPDIR", &temporary)
        // A target of its own: the deadline test builds the same fixture, and
        // a build directory lock held by its sleeper would stall this one.
        .env("CARGO_TARGET_DIR", scratch.join("target"))
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !pid_file.exists() {
        assert!(started.elapsed() < Duration::from_secs(120), "the build script never ran");
        std::thread::sleep(Duration::from_millis(50));
    }
    // The file exists before its contents are written.
    std::thread::sleep(Duration::from_millis(50));
    let sleeper = std::fs::read_to_string(&pid_file).unwrap();
    (child, sleeper, temporary)
}

/// Whether the process `pid` is gone within two seconds.
#[cfg(unix)]
fn ends(pid: &str) -> bool {
    (0..50).any(|_| {
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
    })
}

/// Sends each of the space-separated `signals` to `pid`, in order. The script is
/// a literal and the signals reach it as positional arguments, so nothing the
/// caller passes is ever read as shell syntax.
#[cfg(unix)]
fn signal(pid: u32, signals: &str) {
    let sent = Command::new("sh")
        .args(["-c", "pid=\"$1\"; shift; for s in \"$@\"; do kill -s \"$s\" \"$pid\"; done", "sh"])
        .arg(pid.to_string())
        .args(signals.split_whitespace())
        .status()
        .unwrap();
    assert!(sent.success());
}

/// One failed report naming the interruption, with no score, exit 130, the
/// build script killed and no snapshot left.
#[cfg(unix)]
fn assert_interrupted(output: &Output, sleeper: &str, temporary: &Path) {
    assert_eq!(output.status.code(), Some(130), "{}", String::from_utf8_lossy(&output.stderr));
    let report = json(output);
    assert_eq!(report["status"], "failed");
    assert_eq!(report["errors"].as_array().unwrap().len(), 1, "{}", report["errors"]);
    assert_eq!(report["errors"][0]["code"], "interrupted");
    // EP-002 US-009: what an interrupted scan found judges nothing.
    assert_eq!(report["audit"]["score"], Value::Null);
    assert!(ends(sleeper), "the build script {sleeper} survived the signal");
    assert_eq!(std::fs::read_dir(temporary).unwrap().count(), 0, "a snapshot was left behind");
}

/// Ctrl-C during the base side of a baseline comparison stops everything the
/// scan started, removes the snapshot, prints one failed report and exits 130.
#[cfg(unix)]
#[test]
fn a_sigint_kills_the_build_and_leaves_one_failed_report_and_no_snapshot() {
    let (child, sleeper, temporary) = sleeping_scan(
        "interrupted",
        &["--json", "--max-duration", "600", "--scope", "baseline", "--base", "HEAD"],
    );
    signal(child.id(), "INT");
    assert_interrupted(&child.wait_with_output().unwrap(), &sleeper, &temporary);
}

/// EP-002 US-010: SIGHUP, a closed terminal tab, ends the scan as SIGTERM
/// does, in full scope and in baseline scope.
#[cfg(unix)]
#[test]
fn a_sighup_ends_the_scan_as_sigterm_does() {
    for arguments in [
        &["--json", "--max-duration", "600"][..],
        &["--json", "--max-duration", "600", "--scope", "baseline", "--base", "HEAD"],
    ] {
        let (child, sleeper, temporary) = sleeping_scan("hung-up", arguments);
        signal(child.id(), "HUP");
        assert_interrupted(&child.wait_with_output().unwrap(), &sleeper, &temporary);
    }
}

/// EP-002 US-010: a terminal's Ctrl-C through a launcher that forwards it
/// arrives twice within a few milliseconds. The second is the same delivery,
/// so the scan still ends with its report.
#[cfg(unix)]
#[test]
fn a_duplicate_signal_still_ends_with_the_report() {
    let (child, sleeper, temporary) = sleeping_scan("interrupted-duplicate", &["--json", "--max-duration", "600"]);
    signal(child.id(), "INT INT TERM");
    assert_interrupted(&child.wait_with_output().unwrap(), &sleeper, &temporary);
}

/// A second signal past the duplicate window does not wait for the report:
/// the handler kills the registered groups itself and exits 130. The report
/// goes to a pipe of one page nobody reads, so the first signal's run is
/// still blocked writing it when the second arrives.
#[cfg(target_os = "linux")]
#[test]
fn a_second_signal_kills_the_build_and_exits_at_once() {
    use std::os::fd::FromRawFd;

    let mut ends_of_pipe = [0; 2];
    // SAFETY: `pipe` writes two descriptors into a live array of two, and
    // `F_SETPIPE_SZ` only shrinks the buffer of the one it is given.
    unsafe {
        assert_eq!(libc::pipe(ends_of_pipe.as_mut_ptr()), 0);
        libc::fcntl(ends_of_pipe[1], libc::F_SETPIPE_SZ, 4096);
    }
    // SAFETY: both descriptors were just created and are owned here alone.
    let (unread, writer) = unsafe {
        (
            std::fs::File::from_raw_fd(ends_of_pipe[0]),
            std::fs::File::from_raw_fd(ends_of_pipe[1]),
        )
    };
    let root = committed_copy("sleeping-build");
    let scratch = support::temporary_target("interrupted-twice", &NEXT_COPY);
    let pid_file = scratch.join("sleeper.pid");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut child = command(&root, &["--json", "--max-duration", "600"])
        .stdout(writer)
        .env("RUST_DOCTOR_SLEEPER_PID", &pid_file)
        .env("CARGO_TARGET_DIR", scratch.join("target"))
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !pid_file.exists() {
        assert!(started.elapsed() < Duration::from_secs(120), "the build script never ran");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(50));
    let sleeper = std::fs::read_to_string(&pid_file).unwrap();
    signal(child.id(), "INT");
    std::thread::sleep(Duration::from_millis(800));
    assert!(child.try_wait().unwrap().is_none(), "the report fit the pipe: nothing is blocked");
    signal(child.id(), "TERM");
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "the second signal did not end the run");
        std::thread::sleep(Duration::from_millis(20));
    };
    drop(unread);
    assert_eq!(status.code(), Some(130));
    assert!(ends(&sleeper), "the build script {sleeper} survived two signals");
}

/// EP-002 US-010: a baseline scan sweeps the snapshots a killed run left,
/// those of this user whose process is gone, and never a live one's. A
/// snapshot it cannot remove leaves the scan as it was.
#[cfg(unix)]
#[test]
fn a_baseline_scan_sweeps_the_snapshots_of_dead_runs_only() {
    use std::os::unix::fs::PermissionsExt;

    let root = committed_copy("lint-flags");
    let temporary = support::temporary_target("sweep", &NEXT_COPY).join("tmp");
    let _ = std::fs::remove_dir_all(&temporary);
    std::fs::create_dir_all(&temporary).unwrap();
    let mut reaped = Command::new("true").spawn().unwrap();
    let dead = reaped.id();
    reaped.wait().unwrap();
    let live = std::process::id();
    let dead_snapshot = temporary.join(format!("rust-doctor-baseline-{dead}-0"));
    let stuck = temporary.join(format!("rust-doctor-baseline-{dead}-1"));
    let live_snapshot = temporary.join(format!("rust-doctor-baseline-{live}-0"));
    let unrelated = temporary.join("rust-doctor-baseline-notes");
    for directory in [&dead_snapshot, &live_snapshot, &unrelated] {
        std::fs::create_dir_all(directory.join("src")).unwrap();
        std::fs::write(directory.join("src/lib.rs"), "").unwrap();
    }
    std::fs::create_dir_all(stuck.join("locked")).unwrap();
    std::fs::write(stuck.join("locked/file"), "").unwrap();
    std::fs::set_permissions(stuck.join("locked"), std::fs::Permissions::from_mode(0o555)).unwrap();

    let output = command(&root, &["--json", "--scope", "baseline", "--base", "HEAD"])
        .env("TMPDIR", &temporary)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    std::fs::set_permissions(stuck.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let report = json(&output);
    assert_eq!(report["status"], "complete", "{}", report["errors"]);
    assert!(!dead_snapshot.exists(), "a dead run's snapshot was kept");
    assert!(live_snapshot.is_dir(), "a live run's snapshot was removed");
    assert!(unrelated.is_dir(), "a directory that is no snapshot was removed");
    // SAFETY: `geteuid` reads the effective user id and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        assert!(stuck.join("locked/file").exists(), "the locked snapshot was not locked");
    }
}

/// A toolchain probe that never answers is killed at the run's deadline, not
/// at its own ten seconds.
#[cfg(unix)]
#[test]
fn a_version_probe_that_sleeps_ends_at_the_run_deadline() {
    use std::os::unix::fs::PermissionsExt;

    let cargo = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v cargo"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let bin = support::temporary_target("sleeping-probe", &std::sync::atomic::AtomicUsize::new(0));
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("cargo");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then sleep 60; fi\nexec {} \"$@\"\n",
            cargo.trim()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let started = Instant::now();
    let output = command(&fixture("lint-flags"), &["--json", "--max-duration", "2"])
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
    let report = json(&output);
    assert_eq!(report["status"], "failed");
    assert_eq!(
        error(&report, "toolchain", "deadline-exceeded")["message"],
        "The scan stopped at the 2 s limit set by --max-duration."
    );
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

/// A `-D warnings` from a `[target.'cfg(all())']` table, or from
/// `CARGO_BUILD_RUSTFLAGS`, is not stripped, and the catalogued warning still
/// stays a warning: the command closes on `-W warnings`, which wins.
#[test]
fn a_target_table_or_cargo_build_rustflags_cannot_deny_warnings() {
    let target_table = run(&fixture("lint-flags-target-config"), &["--json"]);
    let build_variable = command(&fixture("lint-flags"), &["--json"])
        .env("CARGO_BUILD_RUSTFLAGS", "-Dwarnings")
        .output()
        .unwrap();
    for output in [target_table, build_variable] {
        let report = json(&output);
        assert_eq!(report["status"], "complete", "{}", report["errors"]);
        let todo = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|diagnostic| diagnostic["code"] == "clippy::todo")
            .unwrap();
        assert_eq!(todo["base_severity"], "warning");
    }
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

static NEXT_COPY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A git repository holding a copy of `fixture`, committed once, under a
/// scratch directory of its own.
fn committed_copy(name: &str) -> PathBuf {
    let root = support::temporary_target("scan-failures", &NEXT_COPY).join(name);
    let _ = std::fs::remove_dir_all(&root);
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

static NEXT_DISCLOSURE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A one-crate workspace under a fresh temporary root, with `manifest_tail`
/// appended to its manifest.
fn disclosure_workspace(manifest_head: &str) -> (PathBuf, PathBuf) {
    let root = support::temporary_target("scan-failures-disclosure", &NEXT_DISCLOSURE);
    let _ = std::fs::remove_dir_all(&root);
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::write(
        workspace.join("Cargo.toml"),
        format!("{manifest_head}[package]\nname = \"disclosure\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )
    .unwrap();
    std::fs::write(workspace.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    (root, workspace)
}

/// EP-001 US-004: Cargo names the manifest of a member outside the workspace
/// by its absolute path. The report names it by its directory and file name,
/// under `<external>`, in the JSON and in the linear report alike.
#[test]
fn a_member_outside_the_workspace_is_named_without_its_absolute_path() {
    let (root, workspace) = disclosure_workspace("[workspace]\nmembers = [\"../gone\"]\n\n");
    let output = run(&workspace, &["--json"]);
    assert_eq!(output.status.code(), Some(2));
    let report = json(&output);
    assert_publishes_no_machine_path(&report, &[&root], None);
    let message = error(&report, "metadata", "cargo-metadata")["message"].as_str().unwrap();
    assert!(message.contains("<external>/gone/Cargo.toml"), "{message}");

    let linear = run(&workspace, &[]);
    let text = String::from_utf8_lossy(&linear.stdout).split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(text.contains("gone/Cargo.toml"), "{text}");
    for path in machine_paths(&[&root], None) {
        assert!(!text.contains(&path), "{path} in {text}");
    }
}

/// EP-001 US-004: rustup names a toolchain by its path under `RUSTUP_HOME`.
/// With that home outside the home directory, which `<home>` used to be the
/// only cover for, the error names it `$RUSTUP_HOME`.
#[test]
fn a_toolchain_error_under_a_rustup_home_outside_home_names_no_absolute_path() {
    let (root, workspace) = disclosure_workspace("");
    let rustup_home = root.join("rustup-home");
    std::fs::create_dir_all(&rustup_home).unwrap();
    std::fs::write(
        workspace.join("rust-toolchain.toml"),
        format!("[toolchain]\npath = \"{}\"\n", rustup_home.join("toolchains/missing").display()),
    )
    .unwrap();
    let output = command(&workspace, &["--json"])
        .env("RUSTUP_HOME", &rustup_home)
        .env("RUSTUP_AUTO_INSTALL", "0")
        // `cargo test` exports the toolchain it runs, which would outrank the
        // file this workspace names its toolchain in.
        .env_remove("RUSTUP_TOOLCHAIN")
        .output()
        .unwrap();
    let report = json(&output);
    assert_eq!(report["status"], "failed", "{report:#}");
    assert_publishes_no_machine_path(&report, &[&root], Some(&rustup_home));
    assert!(
        strings_of(&report).iter().any(|text| text.contains("$RUSTUP_HOME/toolchains/missing")),
        "{report:#}"
    );
}

/// EP-001 US-001 and US-004: what Cargo prints when a build script fails ends
/// the `clippy-exit` error, so a key the script echoed is masked there and a
/// path outside the workspace it names keeps only its last two components,
/// a quoted one holding a space and an apostrophe included (EP-003 US-012).
/// The script assembles both at run time, so no source file holds either.
#[test]
fn a_failed_build_script_echoes_neither_a_key_nor_an_absolute_path() {
    let (root, workspace) = disclosure_workspace("");
    std::fs::write(
        workspace.join("build.rs"),
        "fn main() {\n    \
         let body = \"a1\".repeat(18);\n    \
         eprintln!(\"using token {}{body}\", \"ghp_\");\n    \
         eprintln!(\"missing {}\", [\"\", \"srv\", \"rust-doctor-probe\", \"lib\", \"libfoo.so\"].join(\"/\"));\n    \
         eprintln!(\"failed to read `{}`\", [\"\", \"srv\", \"Jane Doe's Drive\", \"member\", \"Cargo.toml\"].join(\"/\"));\n    \
         std::process::exit(1);\n}\n",
    )
    .unwrap();
    let output = run(&workspace, &["--json"]);
    let report = json(&output);
    let message = error(&report, "execution", "clippy-exit")["message"].as_str().unwrap();
    let body = "a1".repeat(18);
    assert!(message.contains("ghp_<redacted>"), "{message}");
    assert!(message.contains("<external>/lib/libfoo.so"), "{message}");
    assert!(message.contains("failed to read `<external>/member/Cargo.toml`"), "{message}");
    for text in strings_of(&report) {
        assert!(!text.contains(&body) && !text.contains("/srv/rust-doctor-probe"), "{text}");
        assert!(!text.contains("Jane Doe") && !text.contains("Drive"), "{text}");
    }
    assert_publishes_no_machine_path(&report, &[&root], None);
}

static NEXT_LOCKLESS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// EP-002 US-005: a binary crate with no `Cargo.lock`, under a fresh scratch
/// root, with `build` as its build script when one is given.
fn lockless_binary(build: Option<&str>) -> PathBuf {
    let root = support::temporary_target("scan-failures-lockless", &NEXT_LOCKLESS);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"lockless\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n",
    )
    .unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    if let Some(build) = build {
        std::fs::write(root.join("build.rs"), build).unwrap();
    }
    root
}

fn has_finding(report: &Value, code: &str) -> bool {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == code)
}

/// EP-002 US-005: Clippy writes the lockfile it resolved, and the scan removes
/// it, so a second scan judges the tree the first one judged.
#[test]
fn a_lockfile_the_scan_created_is_removed_and_a_rescan_agrees() {
    let root = lockless_binary(None);
    let before = support::content_states(&root);
    let mut reports = Vec::new();
    for _ in 0..2 {
        let output = run(&root, &["--json", "--advisory-db", "/dev/null"]);
        let mut report = json(&output);
        assert!(
            has_finding(&report, "rust_doctor::cargo::missing_lockfile"),
            "{}",
            report["diagnostics"]
        );
        assert_eq!(support::content_states(&root), before, "the scan changed the tree");
        report.as_object_mut().unwrap().remove("timings");
        reports.push(report);
    }
    assert_eq!(reports[0], reports[1]);
    std::fs::remove_dir_all(root).unwrap();
}

/// EP-002 US-005: a committed lockfile Cargo does not need to change is not
/// written at all, so its modification time survives the scan.
#[test]
fn a_committed_lockfile_cargo_leaves_alone_keeps_its_bytes_and_its_time() {
    let root = support::temporary_target("scan-failures-locked", &NEXT_LOCKLESS);
    let _ = std::fs::remove_dir_all(&root);
    support::copy_tree(&fixture("lint-flags"), &root);
    let before = support::file_states(&root);
    let report = json(&run(&root, &["--json", "--advisory-db", "/dev/null"]));
    assert_eq!(report["status"], "complete", "{}", report["errors"]);
    assert_eq!(support::file_states(&root)["Cargo.lock"], before["Cargo.lock"]);
    std::fs::remove_dir_all(root).unwrap();
}

/// EP-002 US-005: a lockfile that cannot be removed is named on stderr, once,
/// and the report is the one the scan would publish anyway. The build script
/// makes the workspace read-only after Cargo wrote the lockfile into it.
#[cfg(unix)]
#[test]
fn a_lockfile_that_cannot_be_removed_is_named_on_stderr_and_nowhere_else() {
    use std::os::unix::fs::PermissionsExt;

    // SAFETY: `geteuid` reads the effective user id and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        // Root removes from a read-only directory: there is nothing to see.
        return;
    }
    let root = lockless_binary(Some(
        "use std::os::unix::fs::PermissionsExt;\n\
         fn main() {\n    \
         let root = std::env::var(\"CARGO_MANIFEST_DIR\").unwrap_or_default();\n    \
         let _ = std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o555));\n}\n",
    ));
    let output = run(&root, &["--json", "--advisory-db", "/dev/null"]);
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let report = json(&output);
    assert!(has_finding(&report, "rust_doctor::cargo::missing_lockfile"));
    let stderr = String::from_utf8(output.stderr).unwrap();
    let named: Vec<&str> = stderr.lines().filter(|line| line.contains("Cargo.lock")).collect();
    assert_eq!(named.len(), 1, "{stderr}");
    assert!(named[0].contains(&root.join("Cargo.lock").display().to_string()), "{stderr}");
    assert!(!report.to_string().contains("was left as Cargo wrote it"));
    std::fs::remove_dir_all(root).unwrap();
}

/// EP-002 US-009: Cargo refuses a crate whose `rust-version` the toolchain
/// does not reach before it builds anything. Nothing was linted, so nothing
/// is scored: the package is named unlinted and neither it nor the workspace
/// carries a number, in the JSON or on the linear report.
#[test]
fn a_package_cargo_refused_before_building_publishes_no_score() {
    let root = lockless_binary(None);
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"lockless\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.999\"\npublish = false\n",
    )
    .unwrap();
    std::fs::write(root.join("src/main.rs"), "mod extra;\nfn main() {\n    extra::run();\n}\n").unwrap();
    std::fs::write(root.join("src/extra.rs"), "pub fn run() {}\n").unwrap();
    let output = run(&root, &["--json", "--advisory-db", "/dev/null"]);
    let report = json(&output);
    assert_eq!(report["audit"]["score"], Value::Null, "{}", report["audit"]);
    let package = &report["audit"]["packages"][0];
    assert_eq!(package["name"], "lockless");
    assert_eq!(package["unlinted"], true);
    assert_eq!(package["score"], Value::Null);
    // With no artifact the target roots are a floor of one file; the walk read
    // both, and the workspace publishes what its only member does.
    assert_eq!(report["audit"]["source_files"], 2, "{}", report["audit"]);
    assert_eq!(package["source_files"], 2, "{}", report["audit"]);
    assert_eq!(
        error(&report, "clippy", "packages-unlinted")["message"],
        "Not linted because it did not compile: lockless"
    );

    let linear = run(&root, &["--advisory-db", "/dev/null"]);
    let text = String::from_utf8(linear.stdout).unwrap();
    assert!(
        text.contains("Score unavailable: the scan could not score, since no package was linted."),
        "{text}"
    );
    assert!(!text.contains("/ 100"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}
