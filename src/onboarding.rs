//! Persistent state for the once-per-user terminal onboarding.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MARKER_NAME: &str = "onboarding-complete";

/// Returns whether the interactive onboarding has already been shown.
///
/// State access fails safe to completed so a broken user configuration never
/// makes the full onboarding replay on every invocation.
pub fn has_completed() -> bool {
    crate::telemetry::config_root().map_or(true, |root| has_completed_at(&root))
}

/// Records the first completed interactive onboarding, best-effort.
///
/// Callers decide whether the current run qualifies. In particular, forced
/// demo runs must not call this function.
pub fn mark_completed() {
    if let Ok(root) = crate::telemetry::config_root() {
        let _ = mark_completed_at(&root);
    }
}

fn marker_path(root: &Path) -> PathBuf {
    root.join(MARKER_NAME)
}

fn has_completed_at(root: &Path) -> bool {
    match std::fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Ok(_) | Err(_) => return true,
    }

    match std::fs::symlink_metadata(marker_path(root)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Ok(_) | Err(_) => true,
    }
}

fn mark_completed_at(root: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(root)?;
            let metadata = std::fs::symlink_metadata(root)?;
            if !metadata.file_type().is_dir() {
                return Ok(());
            }
        }
        Err(error) => return Err(error),
    }

    let destination = marker_path(root);
    match std::fs::symlink_metadata(&destination) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    let completed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    writeln!(temporary, "{completed_at}")?;
    temporary.flush()?;

    match temporary.persist_noclobber(destination) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_root_or_marker_is_not_completed() {
        let directory = tempfile::tempdir().unwrap();
        let missing_root = directory.path().join("config");

        assert!(!has_completed_at(&missing_root));

        std::fs::create_dir(&missing_root).unwrap();
        assert!(!has_completed_at(&missing_root));
    }

    #[test]
    fn mark_is_atomic_and_preserves_the_first_marker() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("config");

        mark_completed_at(&root).unwrap();
        assert!(has_completed_at(&root));
        let marker = marker_path(&root);
        assert!(
            std::fs::symlink_metadata(&marker)
                .unwrap()
                .file_type()
                .is_file()
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);

        std::fs::write(&marker, b"first marker\n").unwrap();
        mark_completed_at(&root).unwrap();
        assert_eq!(std::fs::read(&marker).unwrap(), b"first marker\n");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    }

    #[test]
    fn non_directory_root_fails_safe_without_being_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("config");
        std::fs::write(&root, b"unrelated state").unwrap();

        assert!(has_completed_at(&root));
        mark_completed_at(&root).unwrap();
        assert_eq!(std::fs::read(&root).unwrap(), b"unrelated state");
    }

    #[test]
    fn non_regular_marker_fails_safe_without_being_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let marker = marker_path(directory.path());
        std::fs::create_dir(&marker).unwrap();

        assert!(has_completed_at(directory.path()));
        mark_completed_at(directory.path()).unwrap();
        assert!(
            std::fs::symlink_metadata(marker)
                .unwrap()
                .file_type()
                .is_dir()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_state_fails_safe_without_following_it() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        symlink(&target, marker_path(directory.path())).unwrap();

        assert!(has_completed_at(directory.path()));
        mark_completed_at(directory.path()).unwrap();
        assert!(!target.exists());
    }
}
