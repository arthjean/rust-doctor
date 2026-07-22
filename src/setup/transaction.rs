//! Multi-file transaction support for setup mutations.

use std::collections::BTreeSet;
use std::fs::{self, File, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Clone, Debug)]
pub(super) struct FileState {
    pub(super) contents: Vec<u8>,
    mode: u32,
}

#[derive(Clone, Debug)]
pub(super) enum DesiredState {
    Write { contents: Vec<u8>, executable: bool },
    Delete,
}

#[derive(Clone, Debug)]
pub(super) struct Mutation {
    pub(super) path: PathBuf,
    pub(super) before: Option<FileState>,
    pub(super) desired: DesiredState,
}

#[derive(Debug)]
pub(super) struct TransactionError {
    pub(super) path: Option<PathBuf>,
    pub(super) message: String,
}

impl TransactionError {
    fn at(path: &Path, message: impl Into<String>) -> Self {
        Self {
            path: Some(path.to_path_buf()),
            message: message.into(),
        }
    }
}

pub(super) fn read_state(path: &Path) -> Result<Option<FileState>, TransactionError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(TransactionError::at(
                path,
                format!("failed to inspect destination: {error}"),
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(TransactionError::at(
            path,
            "refusing to mutate a symbolic link",
        ));
    }
    if !metadata.is_file() {
        return Err(TransactionError::at(
            path,
            "destination exists but is not a regular file",
        ));
    }
    let contents = fs::read(path).map_err(|error| {
        TransactionError::at(path, format!("failed to read destination: {error}"))
    })?;
    Ok(Some(FileState {
        contents,
        mode: file_mode(&metadata),
    }))
}

/// Apply every mutation or restore all earlier targets if any mutation fails.
pub(super) fn apply(mutations: &[Mutation]) -> Result<Vec<PathBuf>, TransactionError> {
    apply_inner(mutations, None)
}

fn apply_inner(
    mutations: &[Mutation],
    injected_failure: Option<usize>,
) -> Result<Vec<PathBuf>, TransactionError> {
    preflight(mutations)?;

    let mut backups = Vec::new();
    for mutation in mutations {
        let Some(before) = &mutation.before else {
            continue;
        };
        let backup = next_backup_path(&mutation.path)?;
        if let Err(error) = atomic_create(&backup, before) {
            let cleanup_errors = remove_backups(&backups);
            if cleanup_errors.is_empty() {
                return Err(error);
            }
            return Err(TransactionError {
                path: error.path,
                message: format!(
                    "{}; backup cleanup also failed: {}",
                    error.message,
                    cleanup_errors.join("; ")
                ),
            });
        }
        backups.push(backup);
    }

    let mut applied = Vec::new();
    let mut created_directories = Vec::new();
    for (index, mutation) in mutations.iter().enumerate() {
        if injected_failure == Some(index) {
            let error = TransactionError::at(&mutation.path, "injected transaction failure");
            return rollback_failure(mutations, &applied, &created_directories, &backups, error);
        }
        if let Err(error) = verify_unchanged(mutation) {
            return rollback_failure(mutations, &applied, &created_directories, &backups, error);
        }
        if let Err(error) = ensure_parent(&mutation.path, &mut created_directories) {
            return rollback_failure(mutations, &applied, &created_directories, &backups, error);
        }
        if let Err(error) = apply_one(mutation) {
            return rollback_failure(mutations, &applied, &created_directories, &backups, error);
        }
        applied.push(index);
    }

    Ok(backups)
}

fn preflight(mutations: &[Mutation]) -> Result<(), TransactionError> {
    let mut paths = BTreeSet::new();
    for mutation in mutations {
        if !paths.insert(&mutation.path) {
            return Err(TransactionError::at(
                &mutation.path,
                "duplicate mutation target",
            ));
        }
        verify_unchanged(mutation)?;
        verify_writable_destination(&mutation.path, mutation.before.as_ref())?;
    }
    Ok(())
}

fn verify_writable_destination(
    path: &Path,
    before: Option<&FileState>,
) -> Result<(), TransactionError> {
    if before.is_some() {
        let metadata = fs::metadata(path).map_err(|error| {
            TransactionError::at(path, format!("failed to inspect permissions: {error}"))
        })?;
        if !metadata_writable(&metadata) {
            return Err(TransactionError::at(path, "destination is read-only"));
        }
    }

    let mut ancestor = path.parent();
    while let Some(candidate) = ancestor {
        match fs::metadata(candidate) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(TransactionError::at(
                        path,
                        format!(
                            "destination parent `{}` is not a directory",
                            candidate.display()
                        ),
                    ));
                }
                if !metadata_writable(&metadata) {
                    return Err(TransactionError::at(
                        path,
                        format!(
                            "destination parent `{}` is not writable",
                            candidate.display()
                        ),
                    ));
                }
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                ancestor = candidate.parent();
            }
            Err(error) => {
                return Err(TransactionError::at(
                    path,
                    format!("failed to inspect destination parent: {error}"),
                ));
            }
        }
    }
    Err(TransactionError::at(
        path,
        "destination has no existing writable ancestor",
    ))
}

fn verify_unchanged(mutation: &Mutation) -> Result<(), TransactionError> {
    let current = read_state(&mutation.path)?;
    let unchanged = match (&current, &mutation.before) {
        (None, None) => true,
        (Some(current), Some(before)) => {
            current.contents == before.contents && current.mode == before.mode
        }
        _ => false,
    };
    if unchanged {
        Ok(())
    } else {
        Err(TransactionError::at(
            &mutation.path,
            "destination changed after setup planning; refusing to overwrite it",
        ))
    }
}

fn ensure_parent(
    path: &Path,
    created_directories: &mut Vec<PathBuf>,
) -> Result<(), TransactionError> {
    let Some(parent) = path.parent() else {
        return Err(TransactionError::at(path, "destination has no parent"));
    };
    if parent.exists() {
        return Ok(());
    }

    let mut missing = Vec::new();
    let mut cursor = parent;
    while !cursor.exists() {
        missing.push(cursor.to_path_buf());
        let Some(next) = cursor.parent() else {
            return Err(TransactionError::at(
                path,
                "destination has no existing ancestor",
            ));
        };
        cursor = next;
    }
    for directory in missing.iter().rev() {
        fs::create_dir(directory).map_err(|error| {
            TransactionError::at(
                directory,
                format!("failed to create destination directory: {error}"),
            )
        })?;
        created_directories.push(directory.clone());
    }
    Ok(())
}

fn apply_one(mutation: &Mutation) -> Result<(), TransactionError> {
    match &mutation.desired {
        DesiredState::Write {
            contents,
            executable,
        } => {
            let mode = desired_mode(mutation.before.as_ref(), *executable);
            atomic_replace(&mutation.path, contents, mode)
        }
        DesiredState::Delete => fs::remove_file(&mutation.path).map_err(|error| {
            TransactionError::at(
                &mutation.path,
                format!("failed to remove managed file: {error}"),
            )
        }),
    }
}

fn rollback_failure(
    mutations: &[Mutation],
    applied: &[usize],
    created_directories: &[PathBuf],
    backups: &[PathBuf],
    original: TransactionError,
) -> Result<Vec<PathBuf>, TransactionError> {
    let mut rollback_errors = Vec::new();
    for index in applied.iter().rev() {
        let mutation = &mutations[*index];
        let result = mutation.before.as_ref().map_or_else(
            || match fs::remove_file(&mutation.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(TransactionError::at(
                    &mutation.path,
                    format!("failed to remove newly created file during rollback: {error}"),
                )),
            },
            |before| atomic_replace(&mutation.path, &before.contents, before.mode),
        );
        if let Err(error) = result {
            rollback_errors.push(format_error(&error));
        }
    }
    for directory in created_directories.iter().rev() {
        if let Err(error) = fs::remove_dir(directory) {
            if error.kind() != io::ErrorKind::NotFound
                && error.kind() != io::ErrorKind::DirectoryNotEmpty
            {
                rollback_errors.push(format!(
                    "failed to remove directory `{}` during rollback: {error}",
                    directory.display()
                ));
            }
        }
    }
    if rollback_errors.is_empty() {
        for backup in backups {
            if let Err(error) = fs::remove_file(backup) {
                if error.kind() != io::ErrorKind::NotFound {
                    rollback_errors.push(format!(
                        "failed to remove backup `{}` after rollback: {error}",
                        backup.display()
                    ));
                }
            }
        }
    } else if !backups.is_empty() {
        rollback_errors.push(format!(
            "recovery backups retained at: {}",
            backups
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if rollback_errors.is_empty() {
        Err(original)
    } else {
        Err(TransactionError {
            path: original.path,
            message: format!(
                "{}; rollback also failed: {}",
                original.message,
                rollback_errors.join("; ")
            ),
        })
    }
}

fn next_backup_path(path: &Path) -> Result<PathBuf, TransactionError> {
    let Some(parent) = path.parent() else {
        return Err(TransactionError::at(path, "destination has no parent"));
    };
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(TransactionError::at(
            path,
            "destination filename is not valid UTF-8",
        ));
    };

    for suffix in 0..10_000_u32 {
        let candidate_name = if suffix == 0 {
            format!("{file_name}.rust-doctor.bak")
        } else {
            format!("{file_name}.rust-doctor.bak.{suffix}")
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(TransactionError::at(
        path,
        "could not allocate a unique backup filename",
    ))
}

fn atomic_create(path: &Path, state: &FileState) -> Result<(), TransactionError> {
    let Some(parent) = path.parent() else {
        return Err(TransactionError::at(path, "backup path has no parent"));
    };
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        TransactionError::at(path, format!("failed to create backup temp file: {error}"))
    })?;
    write_temporary(temporary.as_file_mut(), &state.contents, state.mode, path)?;
    temporary.persist_noclobber(path).map_err(|error| {
        TransactionError::at(path, format!("failed to persist backup: {}", error.error))
    })?;
    Ok(())
}

fn atomic_replace(path: &Path, contents: &[u8], mode: u32) -> Result<(), TransactionError> {
    let Some(parent) = path.parent() else {
        return Err(TransactionError::at(path, "destination has no parent"));
    };
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        TransactionError::at(path, format!("failed to create atomic temp file: {error}"))
    })?;
    write_temporary(temporary.as_file_mut(), contents, mode, path)?;
    temporary.persist(path).map_err(|error| {
        TransactionError::at(
            path,
            format!("failed to atomically replace destination: {}", error.error),
        )
    })?;
    Ok(())
}

fn write_temporary(
    file: &mut File,
    contents: &[u8],
    mode: u32,
    path: &Path,
) -> Result<(), TransactionError> {
    file.write_all(contents)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            TransactionError::at(path, format!("failed to write atomic temp file: {error}"))
        })?;
    set_file_mode(file, mode).map_err(|error| {
        TransactionError::at(path, format!("failed to set file permissions: {error}"))
    })
}

fn remove_backups(backups: &[PathBuf]) -> Vec<String> {
    let mut errors = Vec::new();
    for backup in backups {
        if let Err(error) = fs::remove_file(backup) {
            if error.kind() != io::ErrorKind::NotFound {
                errors.push(format!("{}: {error}", backup.display()));
            }
        }
    }
    errors
}

fn format_error(error: &TransactionError) -> String {
    error.path.as_ref().map_or_else(
        || error.message.clone(),
        |path| format!("{}: {}", path.display(), error.message),
    )
}

#[cfg(unix)]
fn file_mode(metadata: &Metadata) -> u32 {
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn file_mode(metadata: &Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

#[cfg(unix)]
fn metadata_writable(metadata: &Metadata) -> bool {
    metadata.permissions().mode() & 0o222 != 0
}

#[cfg(not(unix))]
fn metadata_writable(metadata: &Metadata) -> bool {
    !metadata.permissions().readonly()
}

#[cfg(unix)]
const fn desired_mode(before: Option<&FileState>, executable: bool) -> u32 {
    match (before, executable) {
        (Some(state), true) => state.mode | 0o111,
        (Some(state), false) => state.mode,
        (None, true) => 0o755,
        (None, false) => 0o600,
    }
}

#[cfg(not(unix))]
fn desired_mode(before: Option<&FileState>, _executable: bool) -> u32 {
    before.map_or(0, |state| state.mode)
}

#[cfg(unix)]
fn set_file_mode(file: &File, mode: u32) -> io::Result<()> {
    file.set_permissions(fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_file_mode(file: &File, mode: u32) -> io::Result<()> {
    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(mode != 0);
    file.set_permissions(permissions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_mutation(path: &Path, value: &[u8]) -> Mutation {
        Mutation {
            path: path.to_path_buf(),
            before: read_state(path).expect("file state"),
            desired: DesiredState::Write {
                contents: value.to_vec(),
                executable: false,
            },
        }
    }

    #[test]
    fn successful_transaction_keeps_pre_mutation_backup() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config.json");
        fs::write(&path, b"before").expect("fixture");

        let backups = apply(&[write_mutation(&path, b"after")]).expect("transaction");
        assert_eq!(fs::read(&path).expect("result"), b"after");
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).expect("backup"), b"before");
    }

    #[test]
    fn failure_restores_prior_files_and_removes_transaction_backups() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        fs::write(&first, b"first-before").expect("first fixture");
        fs::write(&second, b"second-before").expect("second fixture");
        let mutations = [
            write_mutation(&first, b"first-after"),
            write_mutation(&second, b"second-after"),
        ];

        let error = apply_inner(&mutations, Some(1)).expect_err("injected failure");
        assert!(error.message.contains("injected"));
        assert_eq!(fs::read(&first).expect("restored first"), b"first-before");
        assert_eq!(
            fs::read(&second).expect("unchanged second"),
            b"second-before"
        );
        let backup_count = fs::read_dir(directory.path())
            .expect("directory listing")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("rust-doctor.bak")
            })
            .count();
        assert_eq!(backup_count, 0);
    }

    #[test]
    fn duplicate_targets_are_refused_before_mutation() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config");
        fs::write(&path, b"before").expect("fixture");
        let first = write_mutation(&path, b"one");
        let second = write_mutation(&path, b"two");

        let error = apply(&[first, second]).expect_err("duplicate target");
        assert!(error.message.contains("duplicate"));
        assert_eq!(fs::read(path).expect("unchanged file"), b"before");
    }
}
