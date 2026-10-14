use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use cargo_metadata::{Metadata, MetadataCommand};

use crate::bounded_read::collect_bounded;
use crate::cargo_stderr;
use crate::execution::{RunDeadline, run_bounded};
use crate::internal_error::InternalError;

mod discovery;

#[derive(Debug)]
pub(crate) struct ResolvedScanTarget {
    pub(crate) manifest_path: PathBuf,
    pub(crate) metadata: Metadata,
    /// The directory discovery found the workspace in below the requested
    /// path, relative to it, when none was above it.
    pub(crate) discovered: Option<String>,
}

impl ResolvedScanTarget {
    pub(crate) fn workspace_root(&self) -> &Path {
        self.metadata.workspace_root.as_std_path()
    }
}

#[derive(Debug)]
pub(crate) struct ResolutionFailure {
    pub(crate) manifest_path: Option<PathBuf>,
    pub(crate) error: InternalError,
}

/// Finds the workspace `path` names and asks Cargo for its metadata, killed
/// at `deadline` when one is set.
pub(crate) fn resolve(
    path: &Path,
    cargo: &Path,
    deadline: Option<RunDeadline>,
) -> Result<ResolvedScanTarget, ResolutionFailure> {
    resolve_with(path, |manifest_path, manifest_directory| {
        load_metadata(manifest_path, manifest_directory, cargo, None, None, deadline)
    }, deadline)
}

pub(crate) fn resolve_isolated(
    path: &Path,
    cargo: &Path,
    target_dir: &Path,
    rustup_toolchain: Option<&OsStr>,
    deadline: Option<RunDeadline>,
) -> Result<ResolvedScanTarget, ResolutionFailure> {
    resolve_with(path, |manifest_path, manifest_directory| {
        load_metadata(
            manifest_path,
            manifest_directory,
            cargo,
            Some(target_dir),
            rustup_toolchain,
            deadline,
        )
    }, deadline)
}

fn resolve_with(
    path: &Path,
    load: impl FnOnce(&Path, &Path) -> Result<Metadata, InternalError>,
    deadline: Option<RunDeadline>,
) -> Result<ResolvedScanTarget, ResolutionFailure> {
    let (manifest_path, discovered) =
        discover_manifest(path, deadline).map_err(|error| ResolutionFailure {
            manifest_path: None,
            error,
        })?;
    let Some(manifest_directory) = manifest_path.parent() else {
        return Err(ResolutionFailure {
            manifest_path: Some(manifest_path.clone()),
            error: no_manifest_error("manifest has no parent directory"),
        });
    };
    let metadata = load(&manifest_path, manifest_directory).map_err(|error| ResolutionFailure {
        manifest_path: Some(manifest_path.clone()),
        error,
    })?;

    Ok(ResolvedScanTarget {
        manifest_path,
        metadata,
        discovered,
    })
}

/// The manifest a path names, found at it or above it, or else the one
/// workspace below it, with the directory it was found in.
///
/// The upward walk stops at the first directory holding `.git`, a file in a
/// worktree or a submodule and a directory otherwise: a manifest above the
/// repository belongs to another project, and scanning it would judge code
/// the reader never pointed at.
fn discover_manifest(
    path: &Path,
    deadline: Option<RunDeadline>,
) -> Result<(PathBuf, Option<String>), InternalError> {
    if path.file_name() == Some(OsStr::new("Cargo.toml")) {
        return resolve_manifest_boundary(path).map(|manifest| (manifest, None));
    }

    if !path.is_dir() {
        return Err(no_manifest_error("path is not a directory"));
    }

    let directory = path
        .canonicalize()
        .map_err(|error| no_manifest_error(format!("could not resolve directory: {error}")))?;
    for ancestor in directory.ancestors() {
        let candidate = ancestor.join("Cargo.toml");
        match candidate.symlink_metadata() {
            Ok(_) => return resolve_manifest_boundary(&candidate).map(|manifest| (manifest, None)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(invalid_manifest_error(format!(
                    "could not inspect manifest path: {error}"
                )));
            }
        }
        if ancestor.join(".git").symlink_metadata().is_ok() {
            break;
        }
    }

    match discovery::workspaces_below(&directory, deadline).as_slice() {
        [] => Err(no_manifest_error(NO_MANIFEST)),
        // The candidate is resolved, then held to the requested directory: a
        // manifest reached through a link out of it never qualifies.
        [only] => resolve_manifest_boundary(&directory.join(only).join("Cargo.toml"))
            .and_then(|manifest| {
                manifest
                    .starts_with(&directory)
                    .then(|| (manifest, Some(only.clone())))
                    .ok_or_else(|| no_manifest_error(NO_MANIFEST))
            }),
        several => Err(discovery::ambiguous(several)),
    }
}

fn resolve_manifest_boundary(path: &Path) -> Result<PathBuf, InternalError> {
    let resolved = path.canonicalize().map_err(|error| {
        invalid_manifest_error(format!("could not resolve manifest path: {error}"))
    })?;
    let metadata = resolved.metadata().map_err(|error| {
        invalid_manifest_error(format!("could not inspect resolved manifest: {error}"))
    })?;
    if !metadata.is_file() {
        return Err(invalid_manifest_error(
            "manifest path does not resolve to a regular file",
        ));
    }

    Ok(resolved)
}

const NO_MANIFEST: &str = "no Cargo.toml found in this path or its ancestors";

/// The path is the reader's own, so the message does not repeat it: a report
/// names no absolute path, and a relative one would only echo the argument.
fn no_manifest_error(detail: impl Into<String>) -> InternalError {
    InternalError::new("discovery", "no-manifest", detail)
}

fn invalid_manifest_error(detail: impl Into<String>) -> InternalError {
    InternalError::new("discovery", "invalid-manifest", detail)
}

fn load_metadata(
    manifest_path: &Path,
    manifest_directory: &Path,
    cargo: &Path,
    target_dir: Option<&Path>,
    rustup_toolchain: Option<&OsStr>,
    deadline: Option<RunDeadline>,
) -> Result<Metadata, InternalError> {
    let command = metadata_command(
        cargo,
        manifest_path,
        manifest_directory,
        target_dir,
        rustup_toolchain,
    )
    .cargo_command();
    let finished = run_bounded(command, deadline, |stdout| {
        collect_bounded(stdout, METADATA_OUTPUT_LIMIT)
    })
    .map_err(|error| {
        InternalError::new(
            "execution",
            "cargo-unavailable",
            format!("Cargo could not be started: {error}"),
        )
    })?;
    let failed = |detail: String| {
        InternalError::new(
            "metadata",
            "cargo-metadata",
            format!("cargo metadata failed: {detail}"),
        )
    };
    if let Some(deadline) = deadline.filter(|_| finished.expired) {
        return Err(InternalError::new(
            "metadata",
            crate::report::DEADLINE_EXCEEDED,
            deadline.message(),
        ));
    }
    let status = finished
        .status
        .map_err(|error| failed(format!("its exit status could not be read: {error}")))?;
    if !status.success() {
        // Cargo says why on stderr, a manifest it could not parse or a member
        // it could not find, and that sentence is the whole remedy.
        return Err(failed(
            cargo_stderr::excerpt(&finished.stderr, manifest_directory, target_dir).map_or_else(
                || "cargo metadata exited with an error".to_owned(),
                |cause| format!("Cargo reported: {cause}"),
            ),
        ));
    }
    let output = finished
        .output
        .transpose()
        .map_err(|error| failed(format!("its output could not be read: {error}")))?
        .ok_or_else(|| failed("it printed nothing".to_owned()))?;
    if output.exceeded {
        return Err(failed(format!(
            "its output exceeded {METADATA_OUTPUT_LIMIT} bytes"
        )));
    }
    let text = String::from_utf8(output.bytes)
        .map_err(|error| failed(format!("its output is not UTF-8: {error}")))?;
    MetadataCommand::parse(text).map_err(|error| failed(error.to_string()))
}

/// What `cargo metadata --no-deps` may print. It is a few kilobytes per
/// member, so this holds workspaces far past the two hundred members the scan
/// is sized for.
const METADATA_OUTPUT_LIMIT: usize = 64 * 1024 * 1024;

fn metadata_command(
    cargo: &Path,
    manifest_path: &Path,
    manifest_directory: &Path,
    target_dir: Option<&Path>,
    rustup_toolchain: Option<&OsStr>,
) -> MetadataCommand {
    let mut command = MetadataCommand::new();
    command
        .cargo_path(cargo)
        .manifest_path(manifest_path)
        .current_dir(manifest_directory)
        .no_deps();
    if let Some(target_dir) = target_dir {
        command.env("CARGO_TARGET_DIR", target_dir);
    }
    if let Some(toolchain) = rustup_toolchain {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    command
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/configuration-kernel/workspace")
    }

    #[test]
    fn root_member_manifest_and_subdirectory_share_one_metadata_workspace() {
        let workspace = fixture().canonicalize().unwrap();
        let workspace_manifest = workspace.join("Cargo.toml");
        let member_manifest = workspace.join("member/Cargo.toml");
        let cases = [
            (workspace.clone(), workspace_manifest),
            (workspace.join("member"), member_manifest.clone()),
            (workspace.join("member/src/nested"), member_manifest.clone()),
            (member_manifest.clone(), member_manifest),
        ];

        for (path, expected_manifest) in cases {
            let calls = Cell::new(0);
            let target = resolve_with(&path, |manifest, directory| {
                calls.set(calls.get() + 1);
                load_metadata(manifest, directory, Path::new("cargo"), None, None, None)
            }, None)
            .unwrap();

            assert_eq!(calls.get(), 1);
            assert_eq!(target.manifest_path, expected_manifest);
            assert_eq!(target.workspace_root(), workspace);
        }
    }

    #[test]
    fn metadata_command_uses_the_versioned_no_deps_contract() {
        let manifest = fixture().join("Cargo.toml");
        let directory = manifest.parent().unwrap();
        let command =
            metadata_command(Path::new("cargo"), &manifest, directory, None, None).cargo_command();
        let arguments: Vec<_> = command.get_args().collect();

        assert_eq!(command.get_program(), OsStr::new("cargo"));
        assert_eq!(
            arguments,
            [
                OsStr::new("metadata"),
                OsStr::new("--format-version"),
                OsStr::new("1"),
                OsStr::new("--no-deps"),
                OsStr::new("--manifest-path"),
                manifest.as_os_str(),
            ]
        );
        assert_eq!(command.get_current_dir(), Some(directory));
    }

    #[test]
    fn path_failures_stop_before_metadata() {
        let missing_calls = Cell::new(0);
        let missing = resolve_with(
            Path::new("/definitely/missing/rust-doctor-fixture"),
            |_, _| {
                missing_calls.set(missing_calls.get() + 1);
                unreachable!("missing paths must not reach metadata")
            },
            None,
        )
        .unwrap_err();
        assert_eq!(
            (missing.error.stage, missing.error.code),
            ("discovery", "no-manifest")
        );
        assert_eq!(missing_calls.get(), 0);

        let ordinary_file = fixture().join("member/src/lib.rs");
        let file_calls = Cell::new(0);
        let ordinary = resolve_with(&ordinary_file, |_, _| {
            file_calls.set(file_calls.get() + 1);
            unreachable!("ordinary files must not reach metadata")
        }, None)
        .unwrap_err();
        assert_eq!(
            (ordinary.error.stage, ordinary.error.code),
            ("discovery", "no-manifest")
        );
        assert_eq!(file_calls.get(), 0);
    }

    #[test]
    fn missing_cargo_preserves_the_v4_unavailable_error_contract() {
        let failure = resolve(
            &fixture(),
            Path::new("/definitely/missing/rust-doctor-cargo"),
            None,
        )
        .unwrap_err();

        assert_eq!(
            (failure.error.stage, failure.error.code),
            ("execution", "cargo-unavailable")
        );
        assert!(
            failure
                .error
                .message
                .starts_with("Cargo could not be started")
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_regular_and_dangling_manifest_boundaries_are_closed() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/configuration-kernel-target-boundary")
            .join(std::process::id().to_string());
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(root.join("Cargo.toml")).unwrap();

        let directory = discover_manifest(&root, None).unwrap_err();
        assert_eq!(
            (directory.stage, directory.code),
            ("discovery", "invalid-manifest")
        );

        fs::remove_dir_all(root.join("Cargo.toml")).unwrap();
        symlink("missing", root.join("Cargo.toml")).unwrap();
        let dangling = discover_manifest(&root, None).unwrap_err();
        assert_eq!(
            (dangling.stage, dangling.code),
            ("discovery", "invalid-manifest")
        );

        fs::remove_dir_all(root).unwrap();
    }
}
