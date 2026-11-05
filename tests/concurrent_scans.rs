#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Two scans of one workspace that overlap.
//!
//! A scan leaves `Cargo.lock` as it found it: Clippy writes the lockfile it
//! resolved, and the scan removes one that was not there before
//! (`docs/subsystems/execution.md`). Each scan used to do that on its own, so
//! two that overlapped undid each other. One that started while the other's
//! Cargo had just written the file took it for the workspace's own and wrote
//! it back at its end, leaving a lockfile no one committed. One whose Cargo
//! found the file and opened it a moment after the other removed it failed
//! with `failed to open: ./Cargo.lock` and published an incomplete report,
//! which is how `audit_contract`'s twenty constructions of one fixture
//! disagreed on CI. A git pre-commit hook and an agent's end-of-turn hook
//! scanning one library overlap the same way.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::AtomicUsize;
use std::thread;
use std::time::Instant;

use serde_json::Value;

mod support;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A library with no `Cargo.lock`, under a fresh scratch root.
fn lockless_library() -> PathBuf {
    let root = support::temporary_target("concurrent-scans", &NEXT);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"overlap\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n",
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn later() -> u8 {\n    todo!()\n}\n").unwrap();
    root
}

/// One scan of `root` in its own process, the way two hooks run.
fn scan(root: &Path) -> Output {
    support::rust_doctor()
        .arg(root)
        .args(["--yes", "--json"])
        .args(support::NO_ADVISORY_DB)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap()
}

fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should hold the JSON report alone")
}

/// The pairs one sweep runs, the second scan of each starting one step later.
const CASES: u32 = 40;

/// Pairs of scans of one lockless workspace, the second starting a little
/// later at each case so the pairs sweep the length of a warm scan, measured
/// here rather than assumed, since a slower machine stretches the window
/// where the two overlap: every report is complete and every pair leaves the
/// workspace without a lockfile. Before the scans took turns, every sweep left
/// one behind, around a third of the way through.
#[test]
fn overlapping_scans_of_one_workspace_leave_it_as_they_found_it() {
    let root = lockless_library();
    // The first scan compiles what the pairs replay; the second measures how
    // long a scan that replays it runs.
    assert_eq!(report(&scan(&root))["status"], "complete");
    let started = Instant::now();
    assert_eq!(report(&scan(&root))["status"], "complete");
    let warm = started.elapsed();
    assert!(!root.join("Cargo.lock").exists());

    for case in 0..CASES {
        let first = {
            let root = root.clone();
            thread::spawn(move || scan(&root))
        };
        thread::sleep(warm * case / CASES);
        let second = scan(&root);
        let first = first.join().unwrap();
        for (which, output) in [("first", &first), ("second", &second)] {
            let report = report(output);
            assert_eq!(
                report["status"], "complete",
                "case {case}: the {which} scan was cut short: {}",
                report["errors"]
            );
        }
        assert!(
            !root.join("Cargo.lock").exists(),
            "case {case}: the pair left a Cargo.lock neither scan found"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
