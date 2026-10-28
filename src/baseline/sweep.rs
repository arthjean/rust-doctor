//! Snapshots a killed run left behind.
//!
//! A snapshot is removed by its `Drop`, and a run a handled signal stops
//! drops it too. SIGKILL runs nothing, so every baseline or staged scan a
//! killed process started left one more `rust-doctor-baseline-<pid>-<n>`
//! directory in the temporary root, a full copy of the base tree each time.
//! The first snapshot a run creates sweeps the ones whose process is gone.

use std::fs;
use std::path::Path;
use std::sync::Once;

/// What every snapshot directory is named, followed by `<pid>-<sequence>`.
pub(super) const PREFIX: &str = "rust-doctor-baseline-";

static SWEPT: Once = Once::new();

/// Removes, once per process, every snapshot under `temporary_root` that this
/// user owns and whose process no longer runs. A directory it cannot remove
/// is left where it is: the sweep never fails the scan that runs it.
pub(super) fn dead_snapshots(temporary_root: &Path) {
    SWEPT.call_once(|| sweep(temporary_root));
}

#[cfg(unix)]
fn sweep(temporary_root: &Path) {
    use std::os::unix::fs::MetadataExt;

    let Ok(entries) = fs::read_dir(temporary_root) else {
        return;
    };
    let user = crate::execution::effective_user();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(owner) else {
            continue;
        };
        if pid == std::process::id() || crate::execution::is_running(pid) {
            continue;
        }
        // Not followed: a link planted under this name is not a snapshot.
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if metadata.is_dir() && metadata.uid() == user {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(not(unix))]
fn sweep(_temporary_root: &Path) {}

/// Whether `path` is the tree of a snapshot: the `tree` directory of a
/// `rust-doctor-baseline-<pid>-<n>` root.
pub(super) fn is_snapshot_tree(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "tree")
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .and_then(owner)
            .is_some()
}

/// The process a snapshot directory's name says created it.
fn owner(name: &str) -> Option<u32> {
    let (pid, sequence) = name.strip_prefix(PREFIX)?.split_once('-')?;
    sequence.parse::<u64>().ok()?;
    pid.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_snapshot_name_names_its_owner() {
        assert_eq!(owner("rust-doctor-baseline-4242-7"), Some(4242));
        for name in [
            "rust-doctor-baseline-4242",
            "rust-doctor-baseline-x-7",
            "rust-doctor-baseline-4242-x",
            "rust-doctor-temp-boundary-4242",
        ] {
            assert_eq!(owner(name), None, "{name}");
        }
    }

    #[test]
    fn only_the_tree_of_a_snapshot_root_is_a_snapshot_tree() {
        assert!(is_snapshot_tree(Path::new("/p/rust-doctor-baseline-4242-7/tree")));
        for path in [
            "/p/rust-doctor-baseline-4242-7",
            "/p/rust-doctor-baseline-4242-7/target",
            "/p/rust-doctor-baseline-4242-7/tree/src",
            "/p/rust-doctor-baseline-notes/tree",
            "/tree",
        ] {
            assert!(!is_snapshot_tree(Path::new(path)), "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn this_process_is_alive_and_a_reaped_child_is_not() {
        use crate::execution::is_running as alive;

        assert!(alive(std::process::id()));
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!alive(pid));
    }
}
