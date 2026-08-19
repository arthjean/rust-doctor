//! One scratch directory per test, for every unit test of this repository that
//! writes to disk.
//!
//! It was written six times, once in `handoff`, `skill`, `tui`, `tui::workflow`
//! and `code_frame`, and once more in an integration test, and the copies had
//! already drifted: one of them swallowed a failed `create_dir_all` where the
//! others refused. The file is reached from both crate roots with `#[path]`
//! rather than copied into each, so the enumeration sees one function and the
//! duplicate detector has nothing to name.
//!
//! The name carries the process id and a counter, so two tests of the same
//! binary, and two binaries running at once, never share a directory.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A fresh directory under `target/<area>/`, created.
pub(crate) fn scratch(area: &str, name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(area)
        .join(format!(
            "{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&root).expect("a scratch directory should be creatable");
    root
}
