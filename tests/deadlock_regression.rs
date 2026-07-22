//! Workspace deadlock regression (EP-001 / US-002).
//!
//! This test scans a synthetic workspace whose member count exceeds the core
//! count with a cold cache — the exact condition that deadlocked pre-US-001.
//! It lives in its own integration binary on purpose: it spawns one `cargo
//! clippy` per member (up to `available_parallelism` concurrently), so running
//! it alongside the other `cargo`-spawning integration tests in a shared libtest
//! thread pool oversubscribes the machine (fd/process pressure) and flakes. Cargo
//! runs test binaries sequentially, so isolating it here gives it the machine to
//! itself while still parallelizing within `integration.rs`.

// Integration test crates aren't covered by clippy.toml `allow-*-in-tests`
// (clippy #13981); unwrap/expect are fine in test assertions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rust_doctor::cli::FailOn;
use rust_doctor::config::ResolvedConfig;
use rust_doctor::discovery::discover_project;
use rust_doctor::scan::scan_project;
use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

/// A `ResolvedConfig` that enables custom rules (the rule engine carries the
/// inner rayon `par_iter` that deadlocked) but skips dependency-tool passes.
fn fast_config() -> ResolvedConfig {
    ResolvedConfig {
        verbose: false,
        diff: None,
        fail_on: FailOn::None,
        ignore_rules: vec![],
        ignore_files: vec![],
        lint: true,
        dependencies: false,
        rules_config: std::collections::HashMap::new(),
        category_config: std::collections::HashMap::new(),
        tag_config: std::collections::HashMap::new(),
        path_overrides: vec![],
        enable_rules: vec![],
        score_fail_below: None,
    }
}

/// Regression test for the latent rayon deadlock on workspaces (EP-001).
///
/// Pre-US-001, `run_passes` iterated scan roots with a rayon `par_iter`. Each
/// root blocked a rayon worker on `std::thread::scope.join()` while the rule
/// engine fanned out files via an inner rayon `par_iter` on the *same* global
/// pool. With workspace members ≥ cores and a cold cache, every worker parked on
/// `join` and the inner `par_iter` starved → permanent hang. After US-001 the
/// roots are scanned with bounded OS threads (never rayon), so a pool worker is
/// never the thread that blocks on a join and the nesting can no longer deadlock.
///
/// The member count is derived from `available_parallelism` (= rayon's default
/// pool size) so the test is deterministic across CI machines: strictly greater
/// than the pool guarantees every worker could park on `join` at once under the
/// old code. The scan runs on a worker thread with a hard wall-clock bound — on
/// pre-US-001 code it never completes and `recv_timeout` trips the assertion.
#[test]
fn test_workspace_scan_no_deadlock_members_exceed_cores() {
    // Many stale files per member keep the rule engine's inner `par_iter` pending
    // for a wide window, so the parked-worker overlap actually materializes (a
    // single trivial file completes too fast to trigger the starvation).
    let files_per_member = 24usize;

    let cores = thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
    // Members ≥ 2× the rayon pool size (= available parallelism) so that every
    // worker can park on `thread::scope.join()` simultaneously under the old code.
    let members = cores * 2;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Virtual workspace manifest listing every member.
    let member_list = (0..members)
        .map(|i| format!("    \"m{i}\","))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        root.join("Cargo.toml"),
        format!("[workspace]\nresolver = \"2\"\nmembers = [\n{member_list}\n]\n"),
    )
    .unwrap();

    // Heavy-ish source: one function with many violation-bearing statements so the
    // rule visitors do real work while the worker that spawned them is parked.
    let body =
        "    let v = std::env::var(\"HOME\").unwrap();\n    let _s = v.clone();\n".repeat(12);
    let file_src = format!("pub fn risky() -> String {{\n{body}    v\n}}\n");

    // Each member is a crate with many stale source files (cold cache: no
    // `.rust-doctor-cache.json` in a fresh temp dir → every file is stale). Extra
    // `.rs` files need not be wired as modules — the scanner walks the filesystem.
    for i in 0..members {
        let member = root.join(format!("m{i}"));
        fs::create_dir_all(member.join("src")).unwrap();
        fs::write(
            member.join("Cargo.toml"),
            format!("[package]\nname = \"m{i}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        )
        .unwrap();
        fs::write(member.join("src/lib.rs"), &file_src).unwrap();
        for j in 1..files_per_member {
            fs::write(member.join(format!("src/f{j}.rs")), &file_src).unwrap();
        }
    }

    let project_info = discover_project(&root.join("Cargo.toml"), true).unwrap();
    assert!(project_info.is_workspace, "fixture must be a workspace");
    assert_eq!(project_info.member_count, members);

    let config = fast_config();

    // Run the scan on a worker thread with a hard wall-clock bound. A deadlock
    // (pre-US-001) never sends, so `recv_timeout` trips and fails the test via the
    // `expect` below — the rayon deadlock regression for EP-001.
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let _ = tx.send(scan_project(&project_info, &config, true, &[], true));
    });

    let scan = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("workspace scan did not terminate within 60s — rayon deadlock regression (EP-001)")
        .expect("workspace scan should succeed");

    // Sanity: the rule engine ran across members (inner par_iter exercised).
    assert!(
        scan.diagnostics
            .iter()
            .any(|d| d.rule == "unwrap-in-production"),
        "expected rule-engine diagnostics from the workspace members",
    );
    handle.join().unwrap();
}
