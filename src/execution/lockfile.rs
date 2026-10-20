//! The `Cargo.lock` a Clippy pass leaves behind.
//!
//! `cargo clippy` resolves the workspace before it lints, and writes the
//! lockfile it resolved: a library or a fresh clone without one gained an
//! untracked file, and the next scan read it and stopped reporting
//! `missing_lockfile`, so two scans of one tree disagreed. Passing `--locked`
//! refuses a missing lockfile and `--offline` resolves differently, so the
//! file is put back as it was instead: removed when the scan created it,
//! rewritten with its original bytes when Cargo changed it, untouched when
//! Cargo left it alone.

use std::fs;
use std::io;
use std::path::PathBuf;

/// What the workspace root held before Clippy ran, put back when this drops,
/// whatever ended the pass: success, failure, the deadline or a signal.
#[derive(Debug)]
pub(super) struct LockfileGuard {
    path: PathBuf,
    /// `None` when there was no lockfile, and nothing at all when the file
    /// could not be read, since a file this pass cannot read is not one it
    /// can restore.
    before: Option<Option<Vec<u8>>>,
}

impl LockfileGuard {
    pub(super) fn capture(workspace_root: &std::path::Path) -> Self {
        let path = workspace_root.join("Cargo.lock");
        let before = match fs::read(&path) {
            Ok(bytes) => Some(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Some(None),
            Err(_) => None,
        };
        Self { path, before }
    }

    fn restore(&self) -> io::Result<()> {
        match &self.before {
            None => Ok(()),
            Some(None) => match fs::remove_file(&self.path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            },
            // Written back only when Cargo changed it, so a lockfile Cargo
            // did not need to touch keeps its modification time.
            Some(Some(bytes)) => match fs::read(&self.path) {
                Ok(now) if now == *bytes => Ok(()),
                _ => fs::write(&self.path, bytes),
            },
        }
    }
}

impl Drop for LockfileGuard {
    fn drop(&mut self) {
        // The report is not the place for it: it judges the workspace, and a
        // file the scan could not remove is a fact about the scan.
        if let Err(error) = self.restore() {
            eprintln!(
                "rust-doctor: {} was left as Cargo wrote it: {error}.",
                self.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rust-doctor-lockfile-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_lockfile_the_pass_created_is_removed() {
        let root = scratch("created");
        let guard = LockfileGuard::capture(&root);
        fs::write(root.join("Cargo.lock"), "version = 4\n").unwrap();
        drop(guard);
        assert!(!root.join("Cargo.lock").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_lockfile_cargo_rewrote_gets_its_bytes_back() {
        let root = scratch("rewritten");
        fs::write(root.join("Cargo.lock"), "version = 3\n").unwrap();
        let guard = LockfileGuard::capture(&root);
        fs::write(root.join("Cargo.lock"), "version = 4\n").unwrap();
        drop(guard);
        assert_eq!(fs::read_to_string(root.join("Cargo.lock")).unwrap(), "version = 3\n");
        fs::remove_dir_all(root).unwrap();
    }
}
