//! Tests of the orchestration, in a file of their own so that every file of the
//! module stays under the size bound `oversized_unit` reports at.

use std::ffi::OsStr;

use super::*;

fn execute(path: &Path) -> ExecutionResult {
    let prepared = match super::prepare(path) {
        Ok(prepared) => prepared,
        Err(result) => return *result,
    };
    super::execute(prepared, &PolicyPlan::default())
}

fn execute_with(path: &Path, programs: &Programs) -> ExecutionResult {
    let prepared = match prepare_with(path, programs) {
        Ok(prepared) => prepared,
        Err(result) => return *result,
    };
    execute_with_plan(prepared, programs, &PolicyPlan::default())
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

#[test]
fn nonzero_clippy_preflight_is_classified() {
    let error = tool_version(
        Path::new("/bin/false"),
        &["clippy", "--version"],
        &fixture("clean"),
        "clippy-unavailable",
        "Clippy",
        &CommandEnvironment::default(),
    )
    .unwrap_err();

    assert_eq!(
        (error.stage, error.code),
        ("execution", "clippy-unavailable")
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
