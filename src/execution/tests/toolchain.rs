//! Tests of what the run asks of the installed toolchain before it lints, and
//! of what a baseline comparison keeps between two runs.

use std::fs;
use std::process::Command;

use super::*;
use crate::audit::ScoreReason;
use crate::git_scope::ScopeReport;
use crate::report::{NotEvaluated, from_execution_scoped};

/// A `clippy-driver` whose lint table lacks `clippy::todo`, which is what an
/// older toolchain that never shipped a catalogued lint looks like.
#[cfg(unix)]
fn driver_without_todo() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let root = crate::test_scratch::scratch("execution", "driver-without-todo");
    let driver = root.join("clippy-driver");
    fs::write(
        &driver,
        "#!/bin/sh\nclippy-driver \"$@\" | grep -v 'clippy::todo '\n",
    )
    .unwrap();
    fs::set_permissions(&driver, fs::Permissions::from_mode(0o755)).unwrap();
    driver
}

#[cfg(unix)]
#[test]
fn a_rule_the_installed_clippy_does_not_list_is_published_as_not_evaluated() {
    let todo = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kernel-contract/todo");
    let programs = Programs {
        clippy_driver: driver_without_todo(),
        ..Programs::default()
    };
    let result = execute_with(&todo, &programs);
    let scan = result.scan.finished().unwrap();
    assert_eq!(scan.not_evaluated, ["clippy::todo"]);
    assert!(
        !scan
            .command
            .iter()
            .any(|argument| argument == "clippy::todo")
    );
    assert!(
        !scan
            .command
            .iter()
            .any(|argument| argument == "unknown_lints")
    );

    let report = from_execution_scoped(result, &PolicyPlan::default(), ScopeReport::full());
    let rule = report
        .policy
        .as_ref()
        .unwrap()
        .rules
        .iter()
        .find(|rule| rule.id == "clippy::todo")
        .unwrap();
    assert_eq!(rule.not_evaluated, Some(NotEvaluated::UnknownToToolchain));
    let score = report.audit.score.as_ref().unwrap();
    assert_eq!(score.reasons, [ScoreReason::RulesNotEvaluated]);
    assert!(!score.authoritative);
}

#[test]
fn a_lint_list_that_cannot_be_read_tolerates_unknown_lints_without_voiding_the_score() {
    let programs = Programs {
        clippy_driver: PathBuf::from("/definitely/missing/clippy-driver"),
        ..Programs::default()
    };
    let result = execute_with(&fixture("clean"), &programs);
    let scan = result.scan.finished().unwrap();
    assert!(
        scan.command
            .ends_with(&["-A".to_owned(), "unknown_lints".to_owned()])
    );
    assert!(scan.not_evaluated.is_empty());
    assert!(result.is_complete());

    let report = from_execution_scoped(result, &PolicyPlan::default(), ScopeReport::full());
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.stage == "toolchain" && error.code == "lint-list-unavailable")
    );
    let score = report.audit.score.as_ref().unwrap();
    assert!(score.reasons.is_empty(), "{:?}", score.reasons);
    assert!(score.authoritative);
}

#[cfg(unix)]
#[test]
fn a_version_probe_that_never_answers_times_out() {
    let started = std::time::Instant::now();
    let error = tool_version(
        Path::new("/bin/sh"),
        &["-c", "sleep 60"],
        &fixture("clean"),
        CLIPPY_PROBE,
        &CommandEnvironment::default(),
        None,
    )
    .unwrap_err();
    assert!(started.elapsed() < std::time::Duration::from_secs(15));
    assert_eq!((error.stage, error.code), ("toolchain", "probe-timeout"));
}

fn git(root: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(arguments)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?}");
}

/// A repository whose one dependency comes from a registry, served from a
/// directory source outside the repository so that no test reaches the
/// network and the dependency's path never moves, as `$CARGO_HOME` does not.
fn registry_repository() -> PathBuf {
    let root = crate::test_scratch::scratch("execution", "baseline-fresh");
    let vendor = root.join("vendor/tinydep");
    fs::create_dir_all(vendor.join("src")).unwrap();
    fs::write(
        vendor.join("Cargo.toml"),
        "[package]\nname = \"tinydep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        vendor.join("src/lib.rs"),
        "pub fn one() -> u8 {\n    1\n}\n",
    )
    .unwrap();
    fs::write(
        vendor.join(".cargo-checksum.json"),
        "{\"files\":{},\"package\":null}",
    )
    .unwrap();

    let repository = root.join("repository");
    fs::create_dir_all(repository.join("src")).unwrap();
    fs::create_dir_all(repository.join(".cargo")).unwrap();
    fs::write(
        repository.join("Cargo.toml"),
        "[package]\nname = \"fresh\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\ntinydep = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        repository.join("src/lib.rs"),
        "pub fn two() -> u8 {\n    tinydep::one() + 1\n}\n",
    )
    .unwrap();
    fs::write(
        repository.join(".cargo/config.toml"),
        format!(
            "[source.crates-io]\nreplace-with = \"vendored\"\n\n[source.vendored]\n\
             directory = \"{}\"\n",
            root.join("vendor").display()
        ),
    )
    .unwrap();
    fs::write(repository.join(".gitignore"), "/target\n").unwrap();
    git(&repository, &["init", "-q"]);
    git(&repository, &["add", "-A"]);
    git(&repository, &["commit", "-q", "-m", "base"]);
    repository
}

/// Every registry dependency the base side compiled, and whether Cargo found
/// it fresh.
fn baseline_registry_freshness(repository: &Path) -> Vec<bool> {
    let prepared = prepare(repository, &RunOptions::default(), true, false).unwrap();
    let target =
        crate::baseline::persistent_target(prepared.target_directory(), "baseline").unwrap();
    assert!(target.ends_with("rust-doctor/baseline"));
    let snapshot = crate::baseline::materialize(prepared.workspace_root(), "HEAD", None).unwrap();
    let plan = PolicyPlan::default();
    let options = RunOptions::default();
    let setup = crate::execution::setup_baseline(&prepared, "HEAD", &plan, &options).unwrap();
    let execution = execute_baseline(
        prepared,
        setup,
        Some(crate::execution::Side {
            workspace: snapshot.workspace(),
            target_dir: &target,
            view: snapshot.view(),
        }),
        None,
        &plan,
        &options,
    )
    .unwrap();
    snapshot.cleanup().unwrap();
    let baseline = match execution.into_sides().0 {
        crate::execution::BaseSide::Ran(baseline) => Some(baseline),
        crate::execution::BaseSide::Cached(_) => None,
    }
    .expect("the base side was scanned");
    baseline
        .scan
        .finished()
        .unwrap()
        .messages
        .iter()
        .filter_map(|message| match message {
            CapturedMessage::Known(message) => match message.as_ref() {
                cargo_metadata::Message::CompilerArtifact(artifact)
                    if artifact.package_id.repr.starts_with("registry+") =>
                {
                    Some(artifact.fresh)
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_second_baseline_run_recompiles_no_registry_dependency() {
    let repository = registry_repository();
    let first = baseline_registry_freshness(&repository);
    assert!(!first.is_empty());
    let second = baseline_registry_freshness(&repository);
    assert!(!second.is_empty());
    assert!(second.iter().all(|fresh| *fresh), "{second:?}");
}

#[test]
fn a_target_directory_that_cannot_be_created_falls_back_to_the_snapshot() {
    let root = crate::test_scratch::scratch("execution", "baseline-unwritable");
    let file = root.join("not-a-directory");
    fs::write(&file, "").unwrap();
    assert_eq!(crate::baseline::persistent_target(&file, "baseline"), None);
}

/// A committed target directory cannot steer the base side's build: a link
/// at any component under it is refused, and the snapshot's target serves.
#[cfg(unix)]
#[test]
fn a_symlinked_target_component_falls_back_to_the_snapshot() {
    for (component, side) in [
        ("rust-doctor", "baseline"),
        ("rust-doctor/baseline", "baseline"),
        ("rust-doctor/staged", "staged"),
    ] {
        let root = crate::test_scratch::scratch("execution", "baseline-component-link");
        let target = root.join("target");
        let elsewhere = root.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        let link = target.join(component);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        assert_eq!(crate::baseline::persistent_target(&target, side), None, "{component}");
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0, "{component}");
    }
}

/// The write probe is created where nothing sits: a link planted at its
/// PID-named path leaves the file it points at as it was.
#[cfg(unix)]
#[test]
fn a_link_at_the_probe_path_is_never_written_through() {
    let root = crate::test_scratch::scratch("execution", "baseline-probe-link");
    let outside = root.join("outside.txt");
    fs::write(&outside, b"not the probe's to touch").unwrap();
    let modified = fs::metadata(&outside).unwrap().modified().unwrap();
    let side = root.join("target/rust-doctor/baseline");
    fs::create_dir_all(&side).unwrap();
    let probe = side.join(format!(".rust-doctor-write-{}", std::process::id()));
    std::os::unix::fs::symlink(&outside, &probe).unwrap();
    assert_eq!(crate::baseline::persistent_target(&root.join("target"), "baseline"), None);
    assert_eq!(fs::read(&outside).unwrap(), b"not the probe's to touch");
    assert_eq!(fs::metadata(&outside).unwrap().modified().unwrap(), modified);
}

/// A Clippy pass the deadline ends while Cargo waits on the build directory's
/// lock says so: the other Cargo, or rust-analyzer, is what to stop.
#[cfg(unix)]
#[test]
fn a_deadline_spent_waiting_on_the_build_lock_names_the_lock() {
    use std::os::unix::fs::PermissionsExt;
    let root = crate::test_scratch::scratch("execution", "locked-cargo");
    let cargo = root.join("cargo");
    fs::write(
        &cargo,
        "#!/bin/sh\nif [ \"$1\" = clippy ] && [ \"$2\" != --version ]; then\n\
         echo '    Blocking waiting for file lock on build directory' >&2\nexec sleep 60\nfi\n\
         exec cargo \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let programs = Programs {
        cargo,
        ..Programs::default()
    };
    let path = fixture("clean");
    let prepared = prepare_with(&path, &programs, None, true, false).unwrap();
    let options = RunOptions {
        deadline: Some(RunDeadline::starting_now(std::time::Duration::from_secs(3))),
        ..RunOptions::default()
    };
    let result = execute_into(prepared, &programs, &PolicyPlan::default(), None, &options);
    let scan = result.scan.finished().unwrap();
    let error = scan
        .errors
        .iter()
        .find(|error| error.code == "deadline-exceeded")
        .unwrap();
    assert!(
        error.message.ends_with(
            "Cargo waited on the build directory lock held by another Cargo or rust-analyzer."
        ),
        "{}",
        error.message
    );
}

/// Clippy failing to start ends the run where it always did, whatever the
/// native passes running beside it found: one `execution` error, the
/// toolchain kept, nothing scanned.
#[cfg(unix)]
#[test]
fn a_clippy_that_cannot_start_returns_early_with_its_stage_error() {
    use std::os::unix::fs::PermissionsExt;
    let root = crate::test_scratch::scratch("execution", "unstartable-clippy");
    let cargo = root.join("cargo");
    // The second version probe takes the execute bit away, so the probes
    // answer and the Clippy spawn that follows them cannot start.
    fs::write(
        &cargo,
        format!(
            "#!/bin/sh\ncase \"$*\" in *--version) mkdir '{}/once' 2>/dev/null || chmod -x \"$0\";; esac\n\
             exec cargo \"$@\"\n",
            root.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let programs = Programs {
        cargo,
        ..Programs::default()
    };
    let path = fixture("clean");
    let prepared = prepare_with(&path, &Programs::default(), None, true, false).unwrap();
    let result = execute_into(
        prepared,
        &programs,
        &PolicyPlan::default(),
        Some(&scan_target_dir(&path)),
        &RunOptions::default(),
    );
    assert_eq!(
        result.error.as_ref().map(|error| (error.stage, error.code)),
        Some(("execution", "clippy-start-failed"))
    );
    assert!(result.toolchain.is_some());
    assert!(!result.scan.has_outcome());
    assert!(result.source.is_none() && result.structure.is_none());
}
