#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! US-031: known-vulnerable and unmaintained crates, read from a RustSec
//! database the machine already holds and never fetched.
//!
//! Every run here points `CARGO_HOME` at a scratch directory of its own, so no
//! test reads the database of the machine it runs on, and the one database
//! any of them reads is `tests/fixtures/advisory-db`, copied where discovery
//! looks or named with `--advisory-db`. The workspace is the offline-registry
//! fixture, whose one crates.io package, `fixture-registry-dep 1.0.0`, is
//! vendored, so Cargo reaches no network either.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

const VULNERABLE: &str = "rust_doctor::cargo::vulnerable_dependency";
const UNMAINTAINED: &str = "rust_doctor::cargo::unmaintained_dependency";

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn database() -> PathBuf {
    repository().join("tests/fixtures/advisory-db")
}

/// A scratch root holding a copy of the offline-registry workspace, an empty
/// `CARGO_HOME` and a target directory.
fn scratch(label: &str) -> PathBuf {
    let root = repository().join("target/advisory-database").join(format!(
        "{}-{}-{label}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    copy_tree(&repository().join("tests/fixtures/cargo-health/offline-registry"), &root.join("project"));
    fs::create_dir_all(root.join("cargo-home")).unwrap();
    root
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|name| name == "target") {
            continue;
        }
        let target = destination.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, target).unwrap();
        }
    }
}

/// A copy of the fixture database at `at`, made a clone of `remote` the way
/// cargo audit and cargo deny leave one: discovery reads its `.git/config`.
fn clone_at(at: &Path, remote: &str) {
    copy_tree(&database(), at);
    fs::create_dir_all(at.join(".git")).unwrap();
    fs::write(at.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(
        at.join(".git/config"),
        format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {remote}\n\tfetch = +HEAD:refs/remotes/origin/HEAD\n"),
    )
    .unwrap();
}

const RUSTSEC: &str = "https://github.com/RustSec/advisory-db";

fn scan(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(root.join("project"))
        .arg("--yes")
        .args(arguments)
        .env("CARGO_HOME", root.join("cargo-home"))
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .unwrap()
}

fn json(root: &Path, arguments: &[&str]) -> Value {
    let mut arguments = arguments.to_vec();
    arguments.push("--json");
    let output = scan(root, &arguments);
    let report = serde_json::from_slice(&output.stdout);
    assert!(report.is_ok(), "{}", String::from_utf8_lossy(&output.stderr));
    report.unwrap()
}

fn findings<'a>(report: &'a Value, code: &str) -> Vec<&'a Value> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == code)
        .collect()
}

fn rule<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["policy"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rule| rule["id"] == id)
        .unwrap()
}

#[test]
fn a_named_database_reports_the_affected_package_at_its_lockfile_entry() {
    let root = scratch("explicit");
    let named = database();
    let report = json(&root, &["--advisory-db", named.to_str().unwrap()]);
    assert_eq!(report["status"], "complete", "{}", report["errors"]);

    // 9001 names a patched release the resolved 1.0.0 predates; 9003 is
    // unsound, which feeds the vulnerable rule. 9006 is patched against
    // 1.0.0, 9004 is a notice and 9005 was withdrawn: none of them reports.
    let vulnerable = findings(&report, VULNERABLE);
    let mut messages: Vec<&str> = vulnerable.iter().map(|finding| finding["message"].as_str().unwrap()).collect();
    messages.sort_unstable();
    assert_eq!(
        messages,
        [
            "Cargo.lock resolves fixture-registry-dep 1.0.0, which RUSTSEC-2020-9001 affects; patched in >= 1.0.1.",
            "Cargo.lock resolves fixture-registry-dep 1.0.0, which RUSTSEC-2020-9003 affects; patched in >= 2.0.0.",
        ]
    );
    let unmaintained = findings(&report, UNMAINTAINED);
    assert_eq!(unmaintained.len(), 1);
    assert_eq!(
        unmaintained[0]["message"],
        "Cargo.lock resolves fixture-registry-dep 1.0.0, which RUSTSEC-2020-9002 reports unmaintained."
    );

    // Each finding points at the package's entry in the lockfile.
    let lockfile = fs::read_to_string(root.join("project/Cargo.lock")).unwrap();
    let entry = lockfile
        .lines()
        .position(|line| line == "name = \"fixture-registry-dep\"")
        .unwrap()
        + 1;
    for finding in vulnerable.iter().chain(&unmaintained) {
        assert_eq!(finding["path"], "Cargo.lock");
        assert_eq!(finding["span"]["line_start"], entry);
        assert_eq!(finding["package"], "cargo-health-offline-registry");
    }
    assert_eq!(rule(&report, VULNERABLE)["category"], "security");
    assert_eq!(rule(&report, VULNERABLE)["tier"], "P1");
    assert_eq!(rule(&report, UNMAINTAINED)["category"], "dependencies");
    assert_eq!(rule(&report, UNMAINTAINED)["tier"], "P2");

    // The database is published by the tool that left it and its date, and
    // the malformed advisory 9007 is skipped and counted. Its path is not.
    assert_eq!(
        report["scan"]["advisory_db"],
        serde_json::json!({"source": "explicit", "date": "2020-01-20", "advisories": 7, "skipped": 1})
    );
    let wire = serde_json::to_string(&report).unwrap();
    assert!(!wire.contains(named.to_str().unwrap()));
    assert!(!wire.contains("advisory-db"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovery_reads_what_cargo_audit_or_cargo_deny_left_under_cargo_home() {
    let root = scratch("cargo-audit");
    clone_at(&root.join("cargo-home/advisory-db"), RUSTSEC);
    let report = json(&root, &[]);
    assert_eq!(report["scan"]["advisory_db"]["source"], "cargo-audit");
    assert_eq!(findings(&report, VULNERABLE).len(), 2);
    fs::remove_dir_all(root).unwrap();

    // cargo-deny keeps one directory per database, and the newest is read.
    let root = scratch("cargo-deny");
    let stale = root.join("cargo-home/advisory-dbs/advisory-db-0000000000000000");
    fs::create_dir_all(stale.join("crates")).unwrap();
    fs::create_dir_all(stale.join(".git")).unwrap();
    fs::write(stale.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(stale.join(".git/config"), format!("[remote \"origin\"]\n\turl = {RUSTSEC}\n")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    clone_at(&root.join("cargo-home/advisory-dbs/advisory-db-1111111111111111"), RUSTSEC);
    let report = json(&root, &[]);
    assert_eq!(report["scan"]["advisory_db"]["source"], "cargo-deny");
    assert_eq!(report["scan"]["advisory_db"]["advisories"], 7);
    assert_eq!(findings(&report, UNMAINTAINED).len(), 1);
    fs::remove_dir_all(root).unwrap();
}

/// Writes `.git/FETCH_HEAD` in a clone, dated `days_ago` days before `now`.
fn fetched(clone: &Path, now: std::time::SystemTime, days_ago: u64) {
    let path = clone.join(".git/FETCH_HEAD");
    fs::write(&path, "").unwrap();
    let when = now - std::time::Duration::from_secs(days_ago * 86_400);
    fs::File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
}

/// Self-scan remediation EP-002 US-011: discovery reads the RustSec clone
/// fetched last, cargo audit's on a tie or when neither records a fetch, never
/// a clone of another remote, and never over `--advisory-db`.
#[test]
fn discovery_reads_the_most_recently_fetched_rustsec_clone() {
    let root = scratch("freshest");
    let audit = root.join("cargo-home/advisory-db");
    let deny = root.join("cargo-home/advisory-dbs/advisory-db-1111111111111111");
    clone_at(&audit, RUSTSEC);
    clone_at(&deny, RUSTSEC);
    // The cargo deny clone knows one advisory more, published after the rest,
    // so the date the report names says which clone was read.
    fs::write(
        deny.join("crates/unrelated-crate/RUSTSEC-2026-9009.md"),
        "```toml\n[advisory]\nid = \"RUSTSEC-2026-9009\"\npackage = \"unrelated-crate\"\ndate = \"2026-10-28\"\n\n[versions]\npatched = []\n```\n\n# Newer\n",
    )
    .unwrap();
    let read = |arguments: &[&str]| {
        let report = json(&root, arguments);
        let database = &report["scan"]["advisory_db"];
        (database["source"].clone(), database["date"].clone())
    };
    let audit_read = (Value::from("cargo-audit"), Value::from("2020-01-20"));
    let deny_read = (Value::from("cargo-deny"), Value::from("2026-10-28"));

    // Neither records a fetch: cargo audit's is read, as before.
    assert_eq!(read(&[]), audit_read);
    // Fetched 88 days ago against today: the fresher cargo deny clone wins.
    let now = std::time::SystemTime::now();
    fetched(&audit, now, 88);
    fetched(&deny, now, 0);
    assert_eq!(read(&[]), deny_read);
    // The same fetch time is a tie, and cargo audit's wins it.
    fetched(&audit, now, 3);
    fetched(&deny, now, 3);
    assert_eq!(read(&[]), audit_read);
    // The explicit path wins over discovery whatever was fetched.
    fetched(&deny, now, 0);
    let named = database();
    let report = json(&root, &["--advisory-db", named.to_str().unwrap()]);
    assert_eq!(report["scan"]["advisory_db"]["source"], "explicit");
    // A fresher clone of another remote is ignored.
    fs::write(
        deny.join(".git/config"),
        "[remote \"origin\"]\n\turl = https://example.invalid/advisory-db\n",
    )
    .unwrap();
    assert_eq!(read(&[]), audit_read);
    fs::remove_dir_all(root).unwrap();
}

/// EP-003 US-013: a database cargo deny cloned from a remote other than
/// RustSec's is never discovered, even fetched after the RustSec clone, and
/// on its own it leaves both rules without a database. `--advisory-db` still
/// reads it, since the user named it.
#[test]
fn discovery_reads_only_a_clone_of_the_rustsec_remote() {
    let root = scratch("impostor");
    let databases = root.join("cargo-home/advisory-dbs");
    clone_at(&databases.join("advisory-db-1111111111111111"), &format!("{RUSTSEC}.git/"));
    std::thread::sleep(std::time::Duration::from_millis(20));
    let impostor = databases.join("advisory-db-2222222222222222");
    fs::create_dir_all(impostor.join(".git")).unwrap();
    fs::write(
        impostor.join(".git/config"),
        "[remote \"origin\"]\n\turl = https://example.invalid/advisory-db\n",
    )
    .unwrap();
    fs::write(impostor.join(".git/FETCH_HEAD"), "").unwrap();
    fs::create_dir_all(impostor.join("crates/unrelated-crate")).unwrap();
    fs::copy(
        database().join("crates/unrelated-crate/RUSTSEC-2020-9008.md"),
        impostor.join("crates/unrelated-crate/RUSTSEC-2020-9008.md"),
    )
    .unwrap();

    let report = json(&root, &[]);
    assert_eq!(report["scan"]["advisory_db"]["source"], "cargo-deny");
    assert_eq!(report["scan"]["advisory_db"]["advisories"], 7);
    assert_eq!(findings(&report, VULNERABLE).len(), 2);

    fs::remove_dir_all(databases.join("advisory-db-1111111111111111")).unwrap();
    let report = json(&root, &[]);
    assert!(report["scan"].get("advisory_db").is_none(), "{}", report["scan"]);
    for id in [VULNERABLE, UNMAINTAINED] {
        assert_eq!(rule(&report, id)["not_evaluated"], "advisory-db-missing");
    }

    let report = json(&root, &["--advisory-db", impostor.to_str().unwrap()]);
    assert_eq!(report["scan"]["advisory_db"]["source"], "explicit");
    assert_eq!(report["scan"]["advisory_db"]["advisories"], 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn without_a_database_both_rules_are_not_evaluated_and_the_score_keeps_its_authority() {
    let root = scratch("missing");
    let report = json(&root, &[]);
    assert_eq!(report["status"], "complete", "{}", report["errors"]);
    for id in [VULNERABLE, UNMAINTAINED] {
        assert_eq!(rule(&report, id)["not_evaluated"], "advisory-db-missing");
    }
    assert!(report["scan"].get("advisory_db").is_none());
    assert_eq!(report["audit"]["score"]["authoritative"], true);
    assert_eq!(report["audit"]["score"]["reasons"], serde_json::json!([]));

    let terminal = String::from_utf8(scan(&root, &[]).stdout).unwrap();
    let said: Vec<&str> = terminal.lines().filter(|line| line.contains("RustSec")).collect();
    assert_eq!(said, ["No local RustSec database: run cargo audit once to create one."]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_stale_database_is_read_and_its_age_is_stated() {
    let root = scratch("stale");
    let named = database();
    let terminal = String::from_utf8(scan(&root, &["--advisory-db", named.to_str().unwrap()]).stdout).unwrap();
    let notice = terminal
        .lines()
        .find(|line| line.starts_with("The advisory database is "));
    assert!(notice.is_some(), "no age notice in:\n{terminal}");
    let notice = notice.unwrap();
    let days: i64 = notice
        .trim_start_matches("The advisory database is ")
        .trim_end_matches(" days old.")
        .parse()
        .unwrap();
    // The newest fixture advisory is dated 2020-01-20.
    assert!(days > 2_400, "{notice}");
    fs::remove_dir_all(root).unwrap();
}

/// EP-002 US-006: a database path that names nothing is refused before any
/// scan, with one line naming the flag and the path as typed, and nothing on
/// stdout even under `--json`.
#[test]
fn a_missing_database_path_is_refused_before_any_scan() {
    let root = scratch("mistyped");
    let before = fs::read_dir(root.join("project")).unwrap().count();
    let output = scan(&root, &["--json", "--advisory-db", "advisory-dbb"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "{}", String::from_utf8_lossy(&output.stdout));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "rust-doctor: --advisory-db: advisory-dbb does not exist.\n"
    );
    assert!(!root.join("target").exists(), "the scan started");
    assert_eq!(fs::read_dir(root.join("project")).unwrap().count(), before);
    fs::remove_dir_all(root).unwrap();
}

/// EP-002 US-006: a database the scan could not read is refused the same way.
#[cfg(unix)]
#[test]
fn an_unreadable_database_path_is_refused_before_any_scan() {
    use std::os::unix::fs::PermissionsExt;

    // SAFETY: `geteuid` reads the effective user id and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let root = scratch("unreadable");
    let locked = root.join("locked-db");
    copy_tree(&database(), &locked);
    let file = root.join("locked-file");
    fs::write(&file, "").unwrap();
    for path in [&locked, &file] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
        let output = scan(&root, &["--json", "--advisory-db", path.to_str().unwrap()]);
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        assert!(stderr.starts_with(&format!("rust-doctor: --advisory-db: {} cannot be read", path.display())), "{stderr}");
    }
    assert!(!root.join("target").exists(), "the scan started");
    fs::remove_dir_all(root).unwrap();
}

/// EP-002 US-006: `/dev/null` stays the documented way to switch the check
/// off, and the help says so.
#[test]
fn dev_null_turns_the_advisory_check_off_as_the_help_says() {
    let root = scratch("dev-null");
    let output = scan(&root, &["--json", "--advisory-db", "/dev/null"]);
    assert_eq!(output.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    for id in [VULNERABLE, UNMAINTAINED] {
        assert_eq!(rule(&report, id)["not_evaluated"], "advisory-db-missing");
    }
    let help = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("/dev/null turns the advisory check off"), "{help}");
    fs::remove_dir_all(root).unwrap();
}
