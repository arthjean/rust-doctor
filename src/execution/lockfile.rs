use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use super::{RunDeadline, interrupt};

const POLL: Duration = Duration::from_millis(25);

#[derive(Debug)]
pub(super) enum Turn {
    Held { _lock: fs::File },
    Missed,
    Unavailable,
}

impl Turn {
    pub(super) fn take(workspace_root: &Path, deadline: Option<RunDeadline>) -> Self {
        let Some(file) = open(workspace_root) else {
            return Self::Unavailable;
        };
        loop {
            match file.try_lock() {
                Ok(()) => return Self::Held { _lock: file },
                Err(fs::TryLockError::WouldBlock) => {}
                Err(fs::TryLockError::Error(_)) => return Self::Unavailable,
            }
            if interrupt::requested() || deadline.is_some_and(RunDeadline::passed) {
                return Self::Missed;
            }
            thread::sleep(deadline.map_or(POLL, |deadline| deadline.remaining().min(POLL)));
        }
    }
}

#[cfg(unix)]
fn open(workspace_root: &Path) -> Option<fs::File> {
    fs::File::open(workspace_root).ok()
}

#[cfg(not(unix))]
fn open(workspace_root: &Path) -> Option<fs::File> {
    fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_path(workspace_root)?)
        .ok()
}

#[cfg(not(unix))]
fn lock_path(workspace_root: &Path) -> Option<PathBuf> {
    let canonical = workspace_root.canonicalize().ok()?;
    let digest = blake3::hash(canonical.as_os_str().as_encoded_bytes());
    Some(std::env::temp_dir().join(format!("rust-doctor-lockfile-{}.lock", digest.to_hex())))
}

#[derive(Debug)]
pub(super) struct LockfileGuard {
    path: PathBuf,
    before: Option<Option<Vec<u8>>>,
}

impl LockfileGuard {
    pub(super) fn capture(workspace_root: &Path, turn: &Turn) -> Self {
        let path = workspace_root.join("Cargo.lock");
        let before = match (turn, fs::read(&path)) {
            (Turn::Missed, _) => None,
            (_, Ok(bytes)) => Some(Some(bytes)),
            (_, Err(error)) if error.kind() == io::ErrorKind::NotFound => Some(None),
            (_, Err(_)) => None,
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
            Some(Some(bytes)) => match fs::read(&self.path) {
                Ok(now) if now == *bytes => Ok(()),
                _ => fs::write(&self.path, bytes),
            },
        }
    }
}

impl Drop for LockfileGuard {
    fn drop(&mut self) {
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
    use std::sync::mpsc;
    use std::time::Instant;

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

    fn release(workspace: &Path) {
        #[cfg(not(unix))]
        if let Some(path) = lock_path(workspace) {
            let _ = fs::remove_file(path);
        }
        #[cfg(unix)]
        let _ = workspace;
    }

    #[test]
    fn a_lockfile_the_pass_created_is_removed() {
        let root = scratch("created");
        let guard = LockfileGuard::capture(&root, &Turn::Unavailable);
        fs::write(root.join("Cargo.lock"), "version = 4\n").unwrap();
        drop(guard);
        assert!(!root.join("Cargo.lock").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_lockfile_cargo_rewrote_gets_its_bytes_back() {
        let root = scratch("rewritten");
        fs::write(root.join("Cargo.lock"), "version = 3\n").unwrap();
        let guard = LockfileGuard::capture(&root, &Turn::Unavailable);
        fs::write(root.join("Cargo.lock"), "version = 4\n").unwrap();
        drop(guard);
        assert_eq!(fs::read_to_string(root.join("Cargo.lock")).unwrap(), "version = 3\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_second_scan_waits_and_captures_what_the_first_put_back() {
        let root = scratch("waits");
        let first = Turn::take(&root, None);
        assert!(matches!(first, Turn::Held { .. }), "{first:?}");
        let guard = LockfileGuard::capture(&root, &first);
        fs::write(root.join("Cargo.lock"), "version = 4\n").unwrap();

        let (taken, turn) = mpsc::channel();
        let second = {
            let root = root.clone();
            thread::spawn(move || {
                let deadline = RunDeadline::starting_now(Duration::from_secs(60));
                let turn = Turn::take(&root, Some(deadline));
                let guard = LockfileGuard::capture(&root, &turn);
                taken.send(()).unwrap();
                (turn, guard.before.clone())
            })
        };
        assert!(turn.recv_timeout(Duration::from_millis(300)).is_err(), "the second scan did not wait");
        drop(guard);
        drop(first);
        let (turn, before) = second.join().unwrap();
        assert!(matches!(turn, Turn::Held { .. }), "{turn:?}");
        assert_eq!(before, Some(None));
        assert!(!root.join("Cargo.lock").exists());
        drop(turn);
        release(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_scan_that_misses_its_turn_leaves_the_lockfile_alone() {
        let root = scratch("missed");
        let first = Turn::take(&root, None);
        fs::write(root.join("Cargo.lock"), "version = 4\n").unwrap();

        let started = Instant::now();
        let deadline = RunDeadline::starting_now(Duration::from_millis(100));
        let second = Turn::take(&root, Some(deadline));
        assert!(matches!(second, Turn::Missed), "{second:?}");
        assert!(started.elapsed() >= Duration::from_millis(100));
        drop(LockfileGuard::capture(&root, &second));
        assert!(root.join("Cargo.lock").exists());

        drop(first);
        release(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_workspace_with_nothing_to_lock_is_scanned_unguarded() {
        let root = scratch("unguarded");
        let turn = Turn::take(&root.join("missing"), None);
        assert!(matches!(turn, Turn::Unavailable), "{turn:?}");
        fs::remove_dir_all(root).unwrap();
    }
}
