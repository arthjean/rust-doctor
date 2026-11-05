#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::AtomicUsize;
use std::thread;
use std::time::Instant;

use serde_json::Value;

mod support;

static NEXT: AtomicUsize = AtomicUsize::new(0);

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

const CASES: u32 = 40;

#[test]
fn overlapping_scans_of_one_workspace_leave_it_as_they_found_it() {
    let root = lockless_library();
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
