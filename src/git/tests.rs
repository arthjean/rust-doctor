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
    let pairs = filters::neutralizing_pairs(&["p=q".to_owned()]);
    let command = git_command(Path::new("git"), Path::new("/workspace"), &arguments, &pairs);
    assert_eq!(command.get_current_dir(), Some(Path::new("/workspace")));
    assert_eq!(command.get_args().collect::<Vec<_>>(), arguments);
    let environment: BTreeMap<_, _> = command.get_envs().collect();
    assert_eq!(
        environment.get(OsStr::new("GIT_NO_LAZY_FETCH")),
        Some(&Some(OsStr::new("1")))
    );
    assert_eq!(
        environment.get(OsStr::new("GIT_LFS_SKIP_SMUDGE")),
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
        if variable != "GIT_CONFIG_COUNT" {
            assert_eq!(environment.get(OsStr::new(variable)), Some(&None));
        }
    }
    // US-002: the inherited count never survives; the pairs replace it, and
    // they are never spelled as `-c`, which splits `p=q` at its `=`.
    assert_eq!(
        environment.get(OsStr::new("GIT_CONFIG_COUNT")),
        Some(&Some(OsStr::new("8")))
    );
    for (index, (key, value)) in [
        "filter.lfs.clean",
        "filter.lfs.smudge",
        "filter.lfs.process",
        "filter.lfs.required",
        "filter.p=q.clean",
        "filter.p=q.smudge",
        "filter.p=q.process",
        "filter.p=q.required",
    ]
    .into_iter()
    .zip(["", "", "", "false"].into_iter().cycle())
    .enumerate()
    {
        assert_eq!(
            environment.get(OsStr::new(&format!("GIT_CONFIG_KEY_{index}"))),
            Some(&Some(OsStr::new(key)))
        );
        assert_eq!(
            environment.get(OsStr::new(&format!("GIT_CONFIG_VALUE_{index}"))),
            Some(&Some(OsStr::new(value)))
        );
    }
    assert!(
        !arguments
            .iter()
            .any(|argument| argument.to_string_lossy().starts_with("filter.")),
        "{arguments:?}"
    );
    assert!(arguments.contains(&OsString::from("safe.bareRepository=explicit")));
    assert!(arguments.contains(&OsString::from("core.hooksPath=/dev/null")));
}

/// US-002: the enumeration runs first, under the call's own `-C`, carrying
/// the canary alone; the call then carries a blanked key set per name read.
#[cfg(unix)]
#[test]
fn the_enumeration_precedes_the_call_and_feeds_its_pairs() {
    let (workspace, script) = git_script(concat!(
        "#!/bin/sh\n",
        "case \" $* \" in *\" config \"*)\n",
        "  printf '%s=%s|' \"$GIT_CONFIG_KEY_0\" \"$GIT_CONFIG_COUNT\" >> \"$0.log\"\n",
        "  printf 'filter.p=q.process\\0rust-doctor.canary\\0'; exit 0 ;;\n",
        "esac\n",
        "printf '%s:%s:%s' \"$GIT_CONFIG_COUNT\" \"$GIT_CONFIG_KEY_6\" \"$GIT_CONFIG_VALUE_7\"\n",
    ));
    let call = call("scope", git_arguments(&workspace, ["status"]));
    let output = run_git(&script, &workspace, &call, None).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "8:filter.p=q.process:false");
    // A second call of the same location reads no configuration again.
    run_git(&script, &workspace, &call, None).unwrap();
    let log = fs::read_to_string(script.with_extension("log")).unwrap();
    assert_eq!(log, "rust-doctor.canary=1|");
}

/// US-002: a git that ignores the pairs (before 2.31) still runs the calls
/// of a repository that configures no driver, as it did before the
/// enumeration: only the `-c` sentinel matches, and nothing needs blanking.
#[cfg(unix)]
#[test]
fn a_git_without_pairs_still_reads_a_repository_with_no_driver() {
    let (workspace, script) = git_script(concat!(
        "#!/bin/sh\n",
        "case \" $* \" in *\" config \"*)\n",
        "  case \" $* \" in *\" rust-doctor.present=1 \"*) printf 'rust-doctor.present\\0'; exit 0 ;; esac\n",
        "  exit 1 ;;\n",
        "esac\n",
        "touch \"$0.ran\"\n",
    ));
    run_git(&script, &workspace, &call("scope", git_arguments(&workspace, ["diff"])), None).unwrap();
    assert!(script.with_extension("ran").exists());
}

/// US-002: a call whose drivers cannot be read never runs.
#[cfg(unix)]
#[test]
fn an_unreadable_or_oversized_configuration_fails_the_call_closed() {
    for (enumeration, code) in [
        ("exit 1", "git-diff-failed"),
        (
            "i=0; while [ \"$i\" -lt 8193 ]; do printf 12345678; i=$((i + 1)); done; exit 0",
            "git-output-too-large",
        ),
        ("printf 'filter.x.clean\\0'; exit 0", "git-diff-failed"),
    ] {
        let (workspace, script) = git_script(&format!(
            "#!/bin/sh\ncase \" $* \" in *\" config \"*) {enumeration} ;; esac\ntouch \"$0.ran\"\n"
        ));
        let mut failing = call("scope", git_arguments(&workspace, ["diff"]));
        failing.failure = GitFailure::new("git-diff-failed", "Git changed files could not be read.");
        let error = run_git(&script, &workspace, &failing, None).unwrap_err();
        assert_eq!((error.stage, error.code), ("scope", code), "{enumeration}");
        assert!(!script.with_extension("ran").exists(), "{enumeration}");
        assert_eq!(
            run_git_status(&script, &workspace, &failing.arguments, None, None),
            None
        );
        assert!(!script.with_extension("ran").exists(), "{enumeration}");
    }
}

/// EP-002 US-007: git runs in the workspace and is told `-C` of it, so a
/// relative workspace reaches `-C` absolute rather than resolved twice.
#[test]
fn a_relative_workspace_reaches_git_once() {
    let arguments = git_arguments(Path::new("crates/x"), ["status"]);
    let command = git_command(Path::new("git"), Path::new("crates/x"), &arguments, &[]);
    assert_eq!(command.get_current_dir(), Some(Path::new("crates/x")));
    let directory = Path::new(&arguments[10]);
    assert!(directory.is_absolute(), "{}", directory.display());
    assert_eq!(directory, std::env::current_dir().unwrap().join("crates/x"));
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
    assert_eq!(literal[10], OsStr::new("/workspace"));
    assert_eq!(literal[11], OsStr::new("diff"));
}

#[test]
fn missing_git_has_the_single_closed_error() {
    let call = call("scope", git_arguments(Path::new("/workspace"), ["status"]));
    let error = run_git(
        Path::new("/definitely/missing/rust-doctor-git"),
        Path::new("/workspace"),
        &call,
        None,
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
        None,
    )
    .unwrap_err();
    assert_eq!((error.stage, error.code), ("baseline", "git-unavailable"));

    let error = run_git(
        Path::new("/definitely/missing/rust-doctor-git"),
        Path::new("/workspace"),
        &call("repo", git_arguments(Path::new("/workspace"), ["status"])),
        None,
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
    let error = run_git(&oversized, &workspace, &scope_call, None).unwrap_err();
    assert_eq!((error.stage, error.code), ("scope", "git-output-too-large"));
    assert!(!error.message.contains("credential=secret"));

    let baseline_limit = GitCall {
        stage: "baseline",
        overflow: BASELINE_OVERFLOW,
        ..scope_call
    };
    let error = run_git(&oversized, &workspace, &baseline_limit, None).unwrap_err();
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
    let error = run_git(&failing, &workspace, &failing_call, None).unwrap_err();
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
        None,
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
        None,
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
        // The filter driver enumeration that precedes every call answers
        // nothing; the flood is the status call's own.
        "case \" $* \" in *\" config \"*) exit 0 ;; esac\n",
        "i=0\n",
        "while [ \"$i\" -lt 65536 ]; do printf 12345678; printf 12345678 >&2; i=$((i + 1)); done\n",
        "exit 1\n",
    ));
    let arguments = git_arguments(&workspace, ["check-ignore", "--quiet"]);
    assert_eq!(run_git_status(&script, &workspace, &arguments, None, None), Some(1));

    assert_eq!(
        run_git_status(
            Path::new("/definitely/missing/rust-doctor-git"),
            &workspace,
            &arguments,
            None,
            None
        ),
        None
    );
}

/// Every file of the module stays under the bound `oversized_unit` reports at,
/// tests included: the layer that runs the scan has to pass the rule it raises.
#[test]
fn the_git_layer_holds_the_size_bound_it_scans_for() {
    for own in [
        include_str!("../git.rs"),
        include_str!("tests.rs"),
        include_str!("filters.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the git layer is {lines} lines long, over the {} it publishes",
            crate::structure::FILE_LINES
        );
    }
}

/// A git that hangs inside a scan is killed at the run's deadline and answers
/// `deadline-exceeded` at its call's stage, rather than holding the scan past
/// `--max-duration`.
#[cfg(unix)]
#[test]
fn a_git_still_running_at_the_deadline_is_killed_at_its_stage() {
    let (workspace, sleeping) = git_script("#!/bin/sh\nexec sleep 60\n");
    let started = std::time::Instant::now();
    let deadline = RunDeadline::starting_now(std::time::Duration::from_millis(300));
    let error = run_git(
        &sleeping,
        &workspace,
        &call("repo", git_arguments(&workspace, ["ls-files"])),
        Some(deadline),
    )
    .unwrap_err();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!((error.stage, error.code), ("repo", "deadline-exceeded"));
    assert_eq!(
        run_git_status(&sleeping, &workspace, &[], None, Some(deadline)),
        None
    );
}
