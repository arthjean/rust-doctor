use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use cargo_metadata::{Metadata, MetadataCommand};

use crate::execution::InternalError;

#[derive(Debug)]
pub(crate) struct ResolvedScanTarget {
    pub(crate) manifest_path: PathBuf,
    pub(crate) metadata: Metadata,
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

pub(crate) fn resolve(path: &Path, cargo: &Path) -> Result<ResolvedScanTarget, ResolutionFailure> {
    resolve_with(path, |manifest_path, manifest_directory| {
        load_metadata(manifest_path, manifest_directory, cargo, None, None)
    })
}

pub(crate) fn resolve_isolated(
    path: &Path,
    cargo: &Path,
    target_dir: &Path,
    rustup_toolchain: Option<&OsStr>,
) -> Result<ResolvedScanTarget, ResolutionFailure> {
    resolve_with(path, |manifest_path, manifest_directory| {
        load_metadata(
            manifest_path,
            manifest_directory,
            cargo,
            Some(target_dir),
            rustup_toolchain,
        )
    })
}

fn resolve_with(
    path: &Path,
    load: impl FnOnce(&Path, &Path) -> Result<Metadata, InternalError>,
) -> Result<ResolvedScanTarget, ResolutionFailure> {
    let manifest_path = discover_manifest(path).map_err(|error| ResolutionFailure {
        manifest_path: None,
        error,
    })?;
    let Some(manifest_directory) = manifest_path.parent() else {
        return Err(ResolutionFailure {
            manifest_path: Some(manifest_path.clone()),
            error: no_manifest_error(&manifest_path, "manifest has no parent directory"),
        });
    };
    let metadata = load(&manifest_path, manifest_directory).map_err(|error| ResolutionFailure {
        manifest_path: Some(manifest_path.clone()),
        error,
    })?;

    Ok(ResolvedScanTarget {
        manifest_path,
        metadata,
    })
}

fn discover_manifest(path: &Path) -> Result<PathBuf, InternalError> {
    if path.file_name() == Some(OsStr::new("Cargo.toml")) {
        return resolve_manifest_boundary(path);
    }

    if !path.is_dir() {
        return Err(no_manifest_error(path, "path is not a directory"));
    }

    let directory = path.canonicalize().map_err(|error| {
        no_manifest_error(path, format!("could not resolve directory: {error}"))
    })?;
    for ancestor in directory.ancestors() {
        let candidate = ancestor.join("Cargo.toml");
        match candidate.symlink_metadata() {
            Ok(_) => return resolve_manifest_boundary(&candidate),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(invalid_manifest_error(
                    &candidate,
                    format!("could not inspect manifest path: {error}"),
                ));
            }
        }
    }

    Err(no_manifest_error(
        path,
        "no Cargo.toml found in this path or its ancestors",
    ))
}

fn resolve_manifest_boundary(path: &Path) -> Result<PathBuf, InternalError> {
    let resolved = path.canonicalize().map_err(|error| {
        invalid_manifest_error(path, format!("could not resolve manifest path: {error}"))
    })?;
    let metadata = resolved.metadata().map_err(|error| {
        invalid_manifest_error(
            path,
            format!("could not inspect resolved manifest: {error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(invalid_manifest_error(
            path,
            "manifest path does not resolve to a regular file",
        ));
    }

    Ok(resolved)
}

fn no_manifest_error(path: &Path, detail: impl AsRef<str>) -> InternalError {
    InternalError::new(
        "discovery",
        "no-manifest",
        format!("{}: {}", detail.as_ref(), path.display()),
    )
}

fn invalid_manifest_error(path: &Path, detail: impl AsRef<str>) -> InternalError {
    InternalError::new(
        "discovery",
        "invalid-manifest",
        format!("{}: {}", detail.as_ref(), path.display()),
    )
}

fn load_metadata(
    manifest_path: &Path,
    manifest_directory: &Path,
    cargo: &Path,
    target_dir: Option<&Path>,
    rustup_toolchain: Option<&OsStr>,
) -> Result<Metadata, InternalError> {
    metadata_command(
        cargo,
        manifest_path,
        manifest_directory,
        target_dir,
        rustup_toolchain,
    )
    .exec()
    .map_err(|error| {
        if let cargo_metadata::Error::Io(error) = error {
            return InternalError::new(
                "execution",
                "cargo-unavailable",
                format!("Cargo could not be started: {error}"),
            );
        }
        let detail = if matches!(&error, cargo_metadata::Error::CargoMetadata { .. }) {
            "cargo metadata exited with an error".to_owned()
        } else {
            error.to_string()
        };
        InternalError::new(
            "metadata",
            "cargo-metadata",
            format!("cargo metadata failed: {detail}"),
        )
    })
}

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
                load_metadata(manifest, directory, Path::new("cargo"), None, None)
            })
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
        })
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

        let directory = discover_manifest(&root).unwrap_err();
        assert_eq!(
            (directory.stage, directory.code),
            ("discovery", "invalid-manifest")
        );

        fs::remove_dir_all(root.join("Cargo.toml")).unwrap();
        symlink("missing", root.join("Cargo.toml")).unwrap();
        let dangling = discover_manifest(&root).unwrap_err();
        assert_eq!(
            (dangling.stage, dangling.code),
            ("discovery", "invalid-manifest")
        );

        fs::remove_dir_all(root).unwrap();
    }
}
