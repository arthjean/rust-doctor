use std::collections::BTreeMap;
use std::ffi::OsStr;
#[cfg(unix)]
use std::fs;
use std::io::Cursor;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

const STDOUT_LIMIT: usize = 4_096;

const SCOPE_FAILURE: GitFailure =
    GitFailure::new("base-unavailable", "Git base commit is unavailable.");
const BASELINE_OVERFLOW: GitFailure = GitFailure::new(
    "baseline-limit-exceeded",
    "Git baseline snapshot exceeds a supported limit.",
);

#[cfg(unix)]
static NEXT_SCRIPT: AtomicUsize = AtomicUsize::new(0);

fn call(stage: &'static str, arguments: Vec<OsString>) -> GitCall {
    GitCall {
        arguments,
        stdout_limit: STDOUT_LIMIT,
        stage,
        failure: SCOPE_FAILURE,
        overflow: OUTPUT_TOO_LARGE,
    }
}

#[cfg(unix)]
fn git_script(contents: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/git-scope-runner")
        .join(format!(
            "{}-{}",
            std::process::id(),
            NEXT_SCRIPT.fetch_add(1, Ordering::Relaxed)
        ));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    fs::create_dir_all(&directory).unwrap();
    let script = directory.join("git");
    fs::write(&script, contents).unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    (directory, script)
}

#[test]
fn git_command_fixes_cwd_stdio_and_hostile_environment() {
    let arguments = git_arguments(Path::new("/workspace"), ["status"]);
    let command = git_command(Path::new("git"), Path::new("/workspace"), &arguments);
    assert_eq!(command.get_current_dir(), Some(Path::new("/workspace")));
    assert_eq!(command.get_args().collect::<Vec<_>>(), arguments);
    let environment: BTreeMap<_, _> = command.get_envs().collect();
    assert_eq!(
        environment.get(OsStr::new("GIT_NO_LAZY_FETCH")),
        Some(&Some(OsStr::new("1")))
    );
    assert_eq!(
        environment.get(OsStr::new("GIT_OPTIONAL_LOCKS")),
        Some(&Some(OsStr::new("0")))
    );
    assert_eq!(
        environment.get(OsStr::new("LC_ALL")),
        Some(&Some(OsStr::new("C")))
    );
    for variable in GIT_ENVIRONMENT_OVERRIDES {
        assert_eq!(environment.get(OsStr::new(variable)), Some(&None));
    }
}

/// The operation is written in whatever string type the call site holds.
#[test]
fn arguments_accept_a_literal_and_an_owned_operation_alike() {
    let literal = git_arguments(Path::new("/workspace"), ["diff", "--name-only"]);
    let owned = git_arguments(
        Path::new("/workspace"),
        [OsString::from("diff"), OsString::from("--name-only")],
    );
    assert_eq!(literal, owned);
    assert_eq!(literal[6], OsStr::new("/workspace"));
    assert_eq!(literal[7], OsStr::new("diff"));
}

#[test]
fn missing_git_has_the_single_closed_error() {
    let call = call("scope", git_arguments(Path::new("/workspace"), ["status"]));
    let error = run_git(
        Path::new("/definitely/missing/rust-doctor-git"),
        Path::new("/workspace"),
        &call,
    )
    .unwrap_err();
    assert_eq!((error.stage, error.code), ("scope", "git-unavailable"));
}

/// Every outcome of a call is reported at the stage the call named.
///
/// The two outcomes the caller does not name, a git that could not start and a
/// stream that overflowed, used to be stamped `scope` whoever ran them, which
/// published a baseline failure at another pass's stage.
#[test]
fn every_outcome_is_reported_at_the_stage_of_its_call() {
    let error = run_git(
        Path::new("/definitely/missing/rust-doctor-git"),
        Path::new("/workspace"),
        &call(
            "baseline",
            git_arguments(Path::new("/workspace"), ["status"]),
        ),
    )
    .unwrap_err();
    assert_eq!((error.stage, error.code), ("baseline", "git-unavailable"));

    let error = run_git(
        Path::new("/definitely/missing/rust-doctor-git"),
        Path::new("/workspace"),
        &call("repo", git_arguments(Path::new("/workspace"), ["status"])),
    )
    .unwrap_err();
    assert_eq!((error.stage, error.code), ("repo", "git-unavailable"));
}

#[test]
fn the_bounded_reader_keeps_the_limit_and_reports_the_byte_past_it() {
    let bounded = collect_bounded(Cursor::new(vec![b'x'; STDOUT_LIMIT + 1]), STDOUT_LIMIT).unwrap();
    assert!(bounded.exceeded);
    assert_eq!(bounded.bytes.len(), STDOUT_LIMIT);

    let exact = collect_bounded(
        Cursor::new(vec![b'x'; STDERR_OUTPUT_LIMIT]),
        STDERR_OUTPUT_LIMIT,
    )
    .unwrap();
    assert!(!exact.exceeded);
    assert_eq!(exact.bytes.len(), STDERR_OUTPUT_LIMIT);

    let oversized = collect_bounded(
        Cursor::new(vec![b'x'; STDERR_OUTPUT_LIMIT + 1]),
        STDERR_OUTPUT_LIMIT,
    )
    .unwrap();
    assert!(oversized.exceeded);
    assert_eq!(oversized.bytes.len(), STDERR_OUTPUT_LIMIT);
}

#[cfg(unix)]
#[test]
fn real_process_output_limits_and_stderr_are_closed() {
    let (workspace, oversized) = git_script(concat!(
        "#!/bin/sh\n",
        "i=0\n",
        "while [ \"$i\" -lt 4097 ]; do printf x; i=$((i + 1)); done\n",
        "printf 'credential=secret\\n' >&2\n",
    ));
    let scope_call = call("scope", git_arguments(&workspace, ["status"]));
    let error = run_git(&oversized, &workspace, &scope_call).unwrap_err();
    assert_eq!((error.stage, error.code), ("scope", "git-output-too-large"));
    assert!(!error.message.contains("credential=secret"));

    let baseline_limit = GitCall {
        stage: "baseline",
        overflow: BASELINE_OVERFLOW,
        ..scope_call
    };
    let error = run_git(&oversized, &workspace, &baseline_limit).unwrap_err();
    assert_eq!(
        (error.stage, error.code),
        ("baseline", "baseline-limit-exceeded")
    );

    let (workspace, failing) = git_script(concat!(
        "#!/bin/sh\n",
        "printf 'https://secret credential=secret\\n' >&2\n",
        "exit 1\n",
    ));
    let mut failing_call = call("scope", git_arguments(&workspace, ["status"]));
    failing_call.failure =
        GitFailure::new("git-diff-failed", "Git changed files could not be read.");
    let error = run_git(&failing, &workspace, &failing_call).unwrap_err();
    assert_eq!(error.code, "git-diff-failed");
    assert!(!error.message.contains("https://secret"));
    assert!(!error.message.contains("credential=secret"));

    let (workspace, exact_stderr) = git_script(concat!(
        "#!/bin/sh\n",
        "i=0\n",
        "while [ \"$i\" -lt 8192 ]; do printf 12345678 >&2; i=$((i + 1)); done\n",
    ));
    let output = run_git(
        &exact_stderr,
        &workspace,
        &call("scope", git_arguments(&workspace, ["status"])),
    )
    .unwrap();
    assert!(output.is_empty());

    // The stderr bound answers at the stage of its own call, which is the
    // regression `every_outcome_is_reported_at_the_stage_of_its_call` states
    // for a git that never started and this one states through a real process.
    let (workspace, oversized_stderr) = git_script(concat!(
        "#!/bin/sh\n",
        "i=0\n",
        "while [ \"$i\" -lt 8192 ]; do printf 12345678 >&2; i=$((i + 1)); done\n",
        "printf x >&2\n",
    ));
    let error = run_git(
        &oversized_stderr,
        &workspace,
        &GitCall {
            stage: "baseline",
            overflow: BASELINE_OVERFLOW,
            ..call("baseline", git_arguments(&workspace, ["status"]))
        },
    )
    .unwrap_err();
    assert_eq!(
        (error.stage, error.code),
        ("baseline", "baseline-limit-exceeded")
    );
}

/// A call that answers with its exit code buys no unbounded read.
#[cfg(unix)]
#[test]
fn a_status_call_answers_its_code_and_discards_both_streams() {
    let (workspace, script) = git_script(concat!(
        "#!/bin/sh\n",
        "i=0\n",
        "while [ \"$i\" -lt 65536 ]; do printf 12345678; printf 12345678 >&2; i=$((i + 1)); done\n",
        "exit 1\n",
    ));
    let arguments = git_arguments(&workspace, ["check-ignore", "--quiet"]);
    assert_eq!(run_git_status(&script, &workspace, &arguments), Some(1));

    assert_eq!(
        run_git_status(
            Path::new("/definitely/missing/rust-doctor-git"),
            &workspace,
            &arguments
        ),
        None
    );
}

/// Every file of the module stays under the bound `oversized_unit` reports at,
/// tests included: the layer that runs the scan has to pass the rule it raises.
#[test]
fn the_git_layer_holds_the_size_bound_it_scans_for() {
    for own in [include_str!("../git.rs"), include_str!("tests.rs")] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the git layer is {lines} lines long, over the {} it publishes",
            crate::structure::FILE_LINES
        );
    }
}
