#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! US-004: `--scope contextual` from the binary's entry point. A rule whose
//! cause can sit above its site is judged by the function around it, so an
//! `.await` an edit adds under a lock already held is the edit's finding,
//! although Clippy reports it on the guard's `let`.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

static NEXT_REPOSITORY: AtomicUsize = AtomicUsize::new(0);

const HELD: &str = "    let value = guarded.lock().unwrap();\n    *value\n";
const AWAITED: &str = "    let value = guarded.lock().unwrap();\n    ready(*value).await\n";

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=Rust Doctor", "-c", "user.email=rust-doctor@example.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(arguments)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The concurrency pack, committed with the guard held but not awaited on.
fn repository() -> PathBuf {
    let root = support::fresh_workspace("contextual-scope", &NEXT_REPOSITORY);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/score-credibility/packs/concurrency");
    fs::create_dir_all(root.join("src")).unwrap();
    for file in ["Cargo.toml", "Cargo.lock", "src/lib.rs", "src/negatives.rs"] {
        fs::copy(fixture.join(file), root.join(file)).unwrap();
    }
    rewrite(&root, AWAITED, HELD);
    fs::write(root.join(".gitignore"), "/target\n").unwrap();
    git(&root, &["init", "--quiet"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "--no-verify", "-m", "held"]);
    root
}

fn rewrite(root: &Path, from: &str, to: &str) {
    let library = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    assert_eq!(library.matches(from).count(), 1, "{from}");
    fs::write(root.join("src/lib.rs"), library.replacen(from, to, 1)).unwrap();
}

fn scan(root: &Path, scope: &str) -> Value {
    let output: Output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(root)
        .args(["--json", "--yes", "--staged", "--scope", scope])
        .args(support::NO_ADVISORY_DB)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("RUSTFLAGS")
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout);
    assert!(
        report.is_ok(),
        "no report: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    report.unwrap()
}

fn holding_lock(report: &Value) -> usize {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "clippy::await_holding_lock")
        .count()
}

#[test]
fn an_await_added_under_a_held_lock_is_the_changes_finding_under_a_contextual_scope() {
    let root = repository();
    rewrite(&root, HELD, AWAITED);
    git(&root, &["add", "src/lib.rs"]);

    let contextual = scan(&root, "contextual");
    assert_eq!(contextual["scope"]["mode"], "contextual", "{contextual}");
    assert_eq!(contextual["schema_version"], 24);
    assert_eq!(holding_lock(&contextual), 1, "{contextual}");
    // The lines scope judges the guard's line alone, which the edit left as is.
    let lines = scan(&root, "lines");
    assert_eq!(holding_lock(&lines), 0, "{lines}");

    // Committed, then a line of another function changes: the finding is
    // legacy, and the contextual scope does not widen past its function.
    git(&root, &["commit", "--quiet", "--no-verify", "-m", "awaited"]);
    rewrite(
        &root,
        "pub fn positive_arc_with_non_send_sync() -> Arc<RefCell<u8>> {\n    Arc::new(RefCell::new(0))",
        "pub fn positive_arc_with_non_send_sync() -> Arc<RefCell<u8>> {\n    Arc::new(RefCell::new(1))",
    );
    git(&root, &["add", "src/lib.rs"]);
    let elsewhere = scan(&root, "contextual");
    assert_eq!(holding_lock(&elsewhere), 0, "{elsewhere}");
    assert!(
        elsewhere["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "clippy::arc_with_non_send_sync"),
        "{elsewhere}"
    );
    fs::remove_dir_all(root).unwrap();
}
