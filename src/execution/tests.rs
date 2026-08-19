//! Tests of the orchestration, in a file of their own so that every file of the
//! module stays under the size bound `oversized_unit` reports at.

use std::ffi::OsStr;

use super::*;
use crate::cargo_health::{CargoHealthError, CargoHealthScan};
use crate::repo_hygiene::{RepoError, RepoScan};
use crate::source_kernel::{SourceError, SourceScan};
use crate::structure::{StructureError, StructureScan};

/// One artifact cache per scanned fixture, keyed by its path.
///
/// Every scan here really runs `cargo clippy`. Two fixtures never share a
/// cache, one fixture scanned twice always does, and none of them inherits the
/// harness's own `CARGO_TARGET_DIR`, under which an already compiled fixture
/// replays with no diagnostic at all.
fn scan_target_dir(path: &Path) -> PathBuf {
    let key = blake3::hash(path.as_os_str().as_encoded_bytes()).to_hex();
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/execution-tests")
        .join(&key[..16])
}

fn execute(path: &Path) -> ExecutionResult {
    execute_with(path, &Programs::default())
}

fn execute_with(path: &Path, programs: &Programs) -> ExecutionResult {
    let prepared = match prepare_with(path, programs) {
        Ok(prepared) => prepared,
        Err(result) => return *result,
    };
    execute_into(
        prepared,
        programs,
        &PolicyPlan::default(),
        Some(&scan_target_dir(path)),
    )
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

fn compiler_message_count(scan: &ScanExecution) -> usize {
    scan.messages
        .iter()
        .filter(|message| matches!(message, CapturedMessage::Compiler(_)))
        .count()
}

/// A result whose Clippy pass went perfectly, so that the only thing a test can
/// make incomplete is the producer it puts an error in.
fn clean_result() -> ExecutionResult {
    ExecutionResult {
        manifest_path: None,
        metadata: None,
        toolchain: None,
        scan: ClippyExecution::Finished(ScanExecution {
            command: vec!["cargo".to_owned(), "clippy".to_owned()],
            exit_code: Some(0),
            exit_success: Some(true),
            build_finished: Some(true),
            noise_lines: 0,
            malformed_messages: 0,
            messages: Vec::new(),
            errors: Vec::new(),
        }),
        source: None,
        structure: None,
        cargo_health: None,
        repo: None,
        error: None,
    }
}

/// Every producer that degrades rather than aborts costs the scan its
/// authoritative flag, and publishes at its own stage.
///
/// `cargo_health` used to be in `report::errors` and out of `is_complete`, so a
/// workspace whose `.cargo/config.toml` could not be read published a
/// `dependencies` error under `"status": "complete"`. The four are asserted
/// together because the defect was never the missing clause, it was that the
/// list of producers was written twice.
#[test]
fn every_producer_error_drops_the_authoritative_flag_at_its_own_stage() {
    assert!(clean_result().is_complete());

    /// One way of degrading a clean result, and the stage it must publish at.
    type Degradation = (&'static str, fn(&mut ExecutionResult));

    let cases: [Degradation; 4] = [
        ("source", |result| {
            result.source = Some(SourceScan {
                errors: vec![SourceError {
                    code: "unit-unreadable",
                    message: "a unit could not be read".to_owned(),
                }],
                ..SourceScan::default()
            });
        }),
        ("structure", |result| {
            result.structure = Some(StructureScan {
                errors: vec![StructureError {
                    code: "budget-exhausted",
                    message: "the pass stopped at its budget".to_owned(),
                }],
                ..StructureScan::default()
            });
        }),
        ("dependencies", |result| {
            result.cargo_health = Some(CargoHealthScan {
                errors: vec![CargoHealthError {
                    code: "cargo-config-unreadable",
                    message: "a file is not a readable regular file",
                }],
                ..CargoHealthScan::default()
            });
        }),
        ("repo", |result| {
            result.repo = Some(RepoScan {
                errors: vec![RepoError {
                    code: "git-unavailable",
                    message: "git was not available",
                }],
                ..RepoScan::default()
            });
        }),
    ];

    for (stage, degrade) in cases {
        let mut result = clean_result();
        degrade(&mut result);

        assert!(
            !result.is_complete(),
            "a {stage} error left the scan calling itself complete"
        );
        assert_eq!(
            result
                .producer_errors()
                .map(|error| error.stage)
                .collect::<Vec<_>>(),
            [stage]
        );
    }
}

/// A producer that raised nothing publishes nothing, whether the plan ran it or
/// left it out.
#[test]
fn a_producer_that_ran_clean_adds_no_error() {
    let mut result = clean_result();
    result.cargo_health = Some(CargoHealthScan::default());
    result.repo = Some(RepoScan::default());

    assert_eq!(result.producer_errors().count(), 0);
    assert!(result.is_complete());
}

#[test]
fn missing_manifest_returns_structured_failure_without_scan() {
    let result = execute(Path::new("/definitely/missing/rust-doctor-fixture"));

    assert_eq!(
        result.error.as_ref().map(|error| (error.stage, error.code)),
        Some(("discovery", "no-manifest"))
    );
    assert!(result.metadata.is_none());
    assert!(result.scan.finished().is_none());
}

#[test]
fn version_command_uses_the_requested_working_directory() {
    let workspace = fixture("clean").canonicalize().unwrap();
    let command = version_command(
        Path::new("cargo"),
        &["--version"],
        &workspace,
        &CommandEnvironment::default(),
    );

    assert_eq!(command.get_program(), OsStr::new("cargo"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [OsStr::new("--version")]
    );
    assert_eq!(command.get_current_dir(), Some(workspace.as_path()));
}

#[test]
fn cargo_spawn_failure_is_classified_before_versions_or_scan() {
    let programs = Programs {
        cargo: PathBuf::from("/definitely/missing/rust-doctor-cargo"),
        rustc: PathBuf::from("rustc"),
    };
    let result = execute_with(&fixture("clean"), &programs);

    assert_eq!(
        result.error.as_ref().map(|error| (error.stage, error.code)),
        Some(("execution", "cargo-unavailable"))
    );
    assert!(result.metadata.is_none());
    assert!(result.scan.finished().is_none());
}

/// A preflight failure says what the toolchain said and what to do about it.
///
/// The message was `Clippy exited with status exit status: 101` and nothing
/// else, with cargo's own line, the one naming the missing component, sent to
/// `/dev/null` by the probe itself.
#[test]
fn a_failed_preflight_quotes_the_toolchain_and_names_the_remedy() {
    let error = tool_version(
        Path::new("/bin/sh"),
        &["-c", "echo 'error: no such command: `clippy`' >&2; exit 101"],
        &fixture("clean"),
        CLIPPY_PROBE,
        &CommandEnvironment::default(),
    )
    .unwrap_err();

    assert_eq!(
        (error.stage, error.code),
        ("execution", "clippy-unavailable")
    );
    assert!(
        error.message.contains("no such command: `clippy`"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("rustup component add clippy"),
        "{}",
        error.message
    );
}

/// A probe that says nothing still names its remedy, so the sentence closes
/// either way.
#[test]
fn a_silent_preflight_failure_still_names_its_remedy() {
    let error = tool_version(
        Path::new("/bin/false"),
        &["clippy", "--version"],
        &fixture("clean"),
        CLIPPY_PROBE,
        &CommandEnvironment::default(),
    )
    .unwrap_err();

    assert_eq!(
        error.message,
        "Clippy could not report a version (exit status: 1). Clippy is required: \
         install the component with `rustup component add clippy`, or add \
         `components: clippy` to the toolchain step in CI."
    );
}

#[test]
fn virtual_workspace_metadata_is_preserved() {
    let result = execute(&fixture("virtual-workspace"));

    assert!(result.error.is_none(), "{:?}", result.error);
    let metadata = result.metadata.unwrap();
    assert!(metadata.resolve.is_none());
    assert_eq!(metadata.workspace_members.len(), 2);
    assert_eq!(metadata.packages.len(), 2);
    let member = metadata
        .packages
        .iter()
        .find(|package| package.name == "workspace-member")
        .unwrap();
    assert_eq!(member.targets[0].name, "workspace_member");
    let external = metadata
        .packages
        .iter()
        .find(|package| package.name == "shared")
        .unwrap();
    assert_eq!(external.targets[0].name, "shared");
    assert!(!external.manifest_path.starts_with(&metadata.workspace_root));
    assert!(
        metadata
            .workspace_root
            .as_std_path()
            .ends_with("virtual-workspace")
    );
}

#[test]
fn valid_scan_collects_provenance_and_clippy_diagnostics() {
    let result = execute(&fixture("clippy-warning"));

    assert!(result.error.is_none(), "{:?}", result.error);
    let toolchain = result.toolchain.as_ref().unwrap();
    assert!(toolchain.cargo.starts_with("cargo "));
    assert!(toolchain.rustc.starts_with("rustc "));
    assert!(toolchain.clippy.starts_with("clippy "));
    let scan = result.scan.into_finished().unwrap();
    // The catalog is the only source of the list, here as in the command test:
    // a frozen count would have to be moved by every rule admission.
    assert_eq!(
        scan.command,
        std::iter::once("cargo".to_owned())
            .chain(
                clippy::arguments_for_plan(&PolicyPlan::default())
                    .into_iter()
                    .map(str::to_owned)
            )
            .collect::<Vec<_>>()
    );
    assert_eq!(scan.exit_code, Some(0));
    assert_eq!(scan.exit_success, Some(true));
    assert_eq!(scan.build_finished, Some(true));
    assert_eq!(scan.noise_lines, 0);
    assert_eq!(scan.malformed_messages, 0);
    assert!(scan.errors.is_empty());
    assert!(compiler_message_count(&scan) >= 1);
}

#[test]
fn nonzero_scan_preserves_prior_diagnostics() {
    let result = execute(&fixture("compile-error"));

    assert!(result.error.is_none(), "{:?}", result.error);
    let scan = result.scan.into_finished().unwrap();
    assert_eq!(scan.exit_code, Some(101));
    assert_eq!(scan.exit_success, Some(false));
    assert_eq!(scan.build_finished, Some(false));
    assert!(compiler_message_count(&scan) >= 1);
}

/// Every file of the module stays under the bound `oversized_unit` reports at,
/// tests included: the layer that starts the scan has to pass the rule it
/// raises. The module used to be one file of 977 lines with no such test, which
/// is the only module of the crate that was both near the bound and unguarded.
#[test]
fn the_execution_holds_the_size_bound_it_scans_for() {
    for own in [
        include_str!("../execution.rs"),
        include_str!("baseline.rs"),
        include_str!("clippy.rs"),
        include_str!("clippy/tests.rs"),
        include_str!("messages.rs"),
        include_str!("messages/tests.rs"),
        include_str!("tests.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the execution layer is {lines} lines long, over the {} it publishes",
            crate::structure::FILE_LINES
        );
    }
}
