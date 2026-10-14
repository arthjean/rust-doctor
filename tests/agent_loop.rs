#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! The agent loop from the binary's entry point: a base resolved from the
//! repository when `--base` is omitted, a scope as narrow as the changed lines,
//! and a staged scan that judges what `git commit` would record.
//!
//! Every repository here is built on `master`, the branch name a skill that
//! hard-coded `--base main` used to fail on.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

static NEXT_REPOSITORY: AtomicUsize = AtomicUsize::new(0);

/// Line 40 of the fixture's `src/lib.rs` holds a `todo!()`; lines 10 to 12 are
/// the ones every change below edits.
fn library_with_todo_on_line_40() -> String {
    let mut lines = vec!["pub fn first() -> u8 { 1 }".to_owned()];
    lines.extend((2..=39).map(|line| format!("// line {line}")));
    lines.push("pub fn pending() { todo!() }".to_owned());
    lines.join("\n") + "\n"
}

fn git(root: &Path, arguments: &[&str]) -> Output {
    let output = Command::new("git")
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
    output
}

fn commit(root: &Path, message: &str) {
    git(
        root,
        &[
            "-c",
            "user.name=Rust Doctor",
            "-c",
            "user.email=rust-doctor@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--no-verify",
            "-m",
            message,
        ],
    );
}

/// A one-crate repository on `master`, committed, then moved to a branch.
fn repository(name: &str) -> PathBuf {
    let root = support::temporary_target(&format!("agent-loop-{name}"), &NEXT_REPOSITORY);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"agent-loop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "/target\n").unwrap();
    fs::write(root.join("src/lib.rs"), library_with_todo_on_line_40()).unwrap();
    git(&root, &["init", "--quiet", "--initial-branch=master"]);
    git(&root, &["add", "."]);
    commit(&root, "initial");
    git(&root, &["checkout", "--quiet", "-b", "feature"]);
    root
}

fn edit_lines_10_to_12(root: &Path) {
    let mut lines: Vec<String> = library_with_todo_on_line_40()
        .lines()
        .map(str::to_owned)
        .collect();
    for line in 10..=12 {
        lines[line - 1] = format!("pub fn changed_{line}() {{}}");
    }
    fs::write(root.join("src/lib.rs"), lines.join("\n") + "\n").unwrap();
}

fn rust_doctor(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(root)
        .args(arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("RUSTFLAGS")
        .output()
        .unwrap()
}

fn json(root: &Path, arguments: &[&str]) -> (Option<i32>, Value) {
    let mut all = vec!["--json"];
    all.extend_from_slice(arguments);
    let output = rust_doctor(root, &all);
    let report = serde_json::from_slice(&output.stdout);
    assert!(
        report.is_ok(),
        "{arguments:?} printed no report: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), report.unwrap())
}

fn todo_lines(report: &Value) -> Vec<u64> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "clippy::todo")
        .map(|diagnostic| diagnostic["span"]["line_start"].as_u64().unwrap())
        .collect()
}

fn error_codes(report: &Value) -> Vec<(String, String)> {
    report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| {
            (
                error["stage"].as_str().unwrap().to_owned(),
                error["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// The definition of done of the epic: in a `master` repository the three
/// forms an agent runs succeed with no `--base`, and each names the base it
/// found. A finding on line 40 is out of a change to lines 10 to 12 under
/// `lines`, and in it under `files`.
#[test]
fn every_changed_scope_resolves_its_base_on_a_master_repository() {
    let root = repository("master");
    edit_lines_10_to_12(&root);

    let (code, lines) = json(&root, &["--scope", "lines"]);
    assert_eq!(code, Some(0), "{lines}");
    assert_eq!(lines["scope"]["mode"], "lines");
    assert_eq!(lines["scope"]["base_ref"], "master");
    assert_eq!(lines["scope"]["files"], serde_json::json!(["src/lib.rs"]));
    assert!(todo_lines(&lines).is_empty(), "{lines}");

    let (code, files) = json(&root, &["--scope", "files"]);
    assert_eq!(code, Some(0), "{files}");
    assert_eq!(todo_lines(&files), [40]);

    let (code, baseline) = json(&root, &["--scope", "baseline"]);
    assert_eq!(code, Some(0), "{baseline}");
    assert_eq!(baseline["scope"]["base_ref"], "master");
    assert_eq!(baseline["delta"]["summary"]["introduced"], 0, "{baseline}");

    git(&root, &["add", "src/lib.rs"]);
    let (code, staged) = json(&root, &["--staged", "--scope", "lines"]);
    assert_eq!(code, Some(0), "{staged}");
    assert_eq!(staged["scope"]["staged"], true);
    assert_eq!(staged["scope"]["base_ref"], "HEAD");

    // The terminal names the ref beside the commit it resolved to.
    let terminal = rust_doctor(&root, &["--yes", "--scope", "lines"]);
    let stdout = String::from_utf8_lossy(&terminal.stdout);
    assert!(
        stdout.contains("Scope: changed lines (1 files, base master at "),
        "{stdout}"
    );
    fs::remove_dir_all(root).unwrap();
}

/// On the default branch itself, the work to judge is what is not committed.
#[test]
fn on_the_default_branch_the_base_is_head() {
    let root = repository("on-master");
    git(&root, &["checkout", "--quiet", "master"]);
    edit_lines_10_to_12(&root);
    let (code, report) = json(&root, &["--scope", "files"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["scope"]["base_ref"], "HEAD");
    let head = git(&root, &["rev-parse", "HEAD"]);
    assert_eq!(
        report["scope"]["comparison_base"],
        String::from_utf8_lossy(&head.stdout).trim()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn no_base_to_find_fails_with_base_undetected() {
    let root = repository("detached");
    git(&root, &["checkout", "--quiet", "--detach"]);
    git(&root, &["branch", "--quiet", "-D", "master", "feature"]);
    let (code, report) = json(&root, &["--scope", "lines"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(
        error_codes(&report),
        [("scope".to_owned(), "base-undetected".to_owned())]
    );
    assert!(
        report["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("--base"),
        "{report}"
    );
    fs::remove_dir_all(root).unwrap();
}

/// A depth-1 clone has the branch tip and nothing below it, so the merge base
/// with a base that forked earlier is missing, and the error says why.
#[test]
fn a_shallow_clone_names_the_missing_fetch() {
    let origin = repository("shallow-origin");
    fs::write(origin.join("notes.md"), "one\n").unwrap();
    git(&origin, &["add", "notes.md"]);
    commit(&origin, "feature work");
    let clone = support::temporary_target("agent-loop-shallow-clone", &NEXT_REPOSITORY);
    fs::create_dir_all(clone.parent().unwrap()).unwrap();
    let url = format!("file://{}", origin.display());
    git(
        clone.parent().unwrap(),
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            "--no-single-branch",
            &url,
            clone.file_name().unwrap().to_str().unwrap(),
        ],
    );
    git(&clone, &["checkout", "--quiet", "feature"]);
    let (code, report) = json(&clone, &["--scope", "baseline", "--base", "origin/master"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(
        error_codes(&report),
        [("scope".to_owned(), "shallow-clone".to_owned())]
    );
    assert!(
        report["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("fetch-depth: 0")
    );
    fs::remove_dir_all(origin).unwrap();
    fs::remove_dir_all(clone).unwrap();
}

/// Untracked Rust files are counted where they could have been judged, and
/// judged whole once `--include-untracked` asks for them.
#[test]
fn untracked_files_are_counted_then_included_on_request() {
    let root = repository("untracked");
    edit_lines_10_to_12(&root);
    fs::write(root.join("src/extra.rs"), "pub fn extra() { todo!() }\n").unwrap();
    fs::write(
        root.join("src/lib.rs"),
        fs::read_to_string(root.join("src/lib.rs")).unwrap() + "pub mod extra;\n",
    )
    .unwrap();

    let terminal = rust_doctor(&root, &["--yes", "--scope", "lines"]);
    let stdout = String::from_utf8_lossy(&terminal.stdout);
    // The line wraps at the terminal width, so its two halves are matched.
    assert!(
        stdout.contains("1 untracked Rust file was compiled but not reported")
            && stdout.contains("pass --include-untracked"),
        "{stdout}"
    );
    let (_, excluded) = json(&root, &["--scope", "lines"]);
    assert_eq!(excluded["scope"]["untracked_unreported"], 1);
    assert!(
        excluded["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["path"] != "src/extra.rs")
    );

    let (code, included) = json(&root, &["--scope", "lines", "--include-untracked"]);
    assert_eq!(code, Some(0), "{included}");
    assert!(included["scope"]["untracked_unreported"].is_null());
    assert!(
        included["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["path"] == "src/extra.rs"
                && diagnostic["code"] == "clippy::todo"),
        "{included}"
    );
    fs::remove_dir_all(root).unwrap();
}

/// An added line whose text starts with `++ ` reads `+++ ` in the patch, and is
/// content of its hunk, not the header of another file.
#[test]
fn an_added_line_that_looks_like_a_file_header_is_content() {
    let root = repository("header-like-line");
    fs::write(root.join("notes.md"), "one\n").unwrap();
    git(&root, &["add", "notes.md"]);
    commit(&root, "notes");
    fs::write(root.join("notes.md"), "one\n++ plain\n").unwrap();
    let (code, report) = json(&root, &["--scope", "lines", "--base", "HEAD"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["scope"]["files"], serde_json::json!(["notes.md"]));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_diff_past_its_bound_fails_with_diff_too_large() {
    let root = repository("large-diff");
    fs::write(root.join("notes.md"), "small\n").unwrap();
    git(&root, &["add", "notes.md"]);
    commit(&root, "notes");
    fs::write(root.join("notes.md"), "line of notes\n".repeat(90_000)).unwrap();
    let (code, report) = json(&root, &["--scope", "lines", "--base", "HEAD"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(
        error_codes(&report),
        [("scope".to_owned(), "diff-too-large".to_owned())]
    );
    fs::remove_dir_all(root).unwrap();
}

/// The staged side is what the commit records: a staged defect is reported
/// whatever the working tree has done to it since, and an unstaged defect is
/// not, whichever scope judges the index.
#[test]
fn a_staged_scan_judges_the_index_and_never_the_working_tree() {
    let root = repository("staged");
    let defect = "pub fn staged() { todo!() }\n";
    let clean = "pub fn staged() {}\n";
    let library = library_with_todo_on_line_40() + "pub mod staged;\n";
    fs::write(root.join("src/lib.rs"), &library).unwrap();

    // Staged defect, unstaged fix.
    fs::write(root.join("src/staged.rs"), defect).unwrap();
    git(&root, &["add", "src/lib.rs", "src/staged.rs"]);
    fs::write(root.join("src/staged.rs"), clean).unwrap();
    for scope in ["files", "lines", "baseline"] {
        let (code, report) = json(&root, &["--staged", "--scope", scope]);
        assert_eq!(code, Some(0), "{scope}: {report}");
        let reported = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["path"] == "src/staged.rs" && diagnostic["code"] == "clippy::todo"
            });
        assert!(
            reported,
            "{scope}: the staged defect was not reported: {report}"
        );
    }

    // Staged clean file, unstaged defect.
    fs::write(root.join("src/staged.rs"), clean).unwrap();
    git(&root, &["add", "src/staged.rs"]);
    fs::write(root.join("src/staged.rs"), defect).unwrap();
    for scope in ["files", "lines", "baseline"] {
        let (code, report) = json(&root, &["--staged", "--scope", scope]);
        assert_eq!(code, Some(0), "{scope}: {report}");
        assert!(
            report["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .all(|diagnostic| diagnostic["path"] != "src/staged.rs"),
            "{scope}: the unstaged defect was reported: {report}"
        );
    }
    // No temporary path leaks into what the report publishes.
    let (_, report) = json(&root, &["--staged", "--scope", "files"]);
    let wire = report.to_string();
    assert!(
        !wire.contains(std::env::temp_dir().to_str().unwrap()),
        "{wire}"
    );
    assert_eq!(report["project"]["manifest_path"], "Cargo.toml");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nothing_staged_passes_and_says_so() {
    let root = repository("nothing-staged");
    edit_lines_10_to_12(&root);
    let (code, report) = json(&root, &["--staged", "--scope", "lines"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["scope"]["files"], serde_json::json!([]));
    assert!(
        report["diagnostics"].as_array().unwrap().is_empty(),
        "{report}"
    );
    let terminal = rust_doctor(&root, &["--yes", "--staged", "--scope", "lines"]);
    assert_eq!(terminal.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&terminal.stdout).contains("No staged Rust change to judge."),
        "{}",
        String::from_utf8_lossy(&terminal.stdout)
    );
    fs::remove_dir_all(root).unwrap();
}

/// A conflicted or locked index fails the scan with exit 2, never an empty
/// pass.
#[test]
fn an_unusable_index_fails_the_staged_scan() {
    let root = repository("conflicted");
    fs::write(root.join("notes.md"), "feature\n").unwrap();
    git(&root, &["add", "notes.md"]);
    commit(&root, "feature notes");
    git(&root, &["checkout", "--quiet", "master"]);
    fs::write(root.join("notes.md"), "master\n").unwrap();
    git(&root, &["add", "notes.md"]);
    commit(&root, "master notes");
    // The identity is passed here as `commit` passes it: a runner with no
    // global one refuses the merge before it writes the conflict.
    let merge = Command::new("git")
        .args([
            "-c",
            "user.name=Rust Doctor",
            "-c",
            "user.email=rust-doctor@example.invalid",
            "merge",
            "--quiet",
            "feature",
        ])
        .current_dir(&root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(!merge.status.success(), "the merge was meant to conflict");
    assert!(
        !git(&root, &["ls-files", "--unmerged"]).stdout.is_empty(),
        "the merge failed without leaving a conflict: {}",
        String::from_utf8_lossy(&merge.stderr)
    );
    let (code, report) = json(&root, &["--staged", "--scope", "files"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(
        error_codes(&report),
        [("scope".to_owned(), "index-conflicted".to_owned())]
    );
    fs::remove_dir_all(&root).unwrap();

    let root = repository("locked");
    fs::write(root.join(".git/index.lock"), b"").unwrap();
    let (code, report) = json(&root, &["--staged", "--scope", "files"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(
        error_codes(&report),
        [("scope".to_owned(), "index-unavailable".to_owned())]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn staged_is_refused_with_the_full_scope() {
    let root = repository("staged-full");
    let output = rust_doctor(&root, &["--json", "--staged", "--scope", "full"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--staged") && stderr.contains("full"),
        "{stderr}"
    );
    fs::remove_dir_all(root).unwrap();
}

/// The vendored-registry fixture, committed on `master` with a bounded
/// requirement, so a change can make it unbounded without touching the lock.
fn registry_repository(name: &str) -> PathBuf {
    fn copy(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == "target" {
                continue;
            }
            let to = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &to);
            } else {
                fs::copy(entry.path(), to).unwrap();
            }
        }
    }
    let root = support::temporary_target(&format!("agent-loop-{name}"), &NEXT_REPOSITORY);
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cargo-health/offline-registry"),
        &root,
    );
    set_requirement(&root, "1");
    fs::write(root.join(".gitignore"), "/target\n/cargo-home\n").unwrap();
    git(&root, &["init", "--quiet", "--initial-branch=master"]);
    git(&root, &["add", "."]);
    commit(&root, "initial");
    root
}

/// Rewrites line 7 of the manifest, the one dependency's declaration.
fn set_requirement(root: &Path, requirement: &str) {
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let mut lines: Vec<&str> = manifest.lines().collect();
    let declaration = format!(
        "fixture_registry_alias = {{ package = \"fixture-registry-dep\", version = \"{requirement}\" }}"
    );
    assert!(lines[6].starts_with("fixture_registry_alias = "), "{manifest}");
    lines[6] = &declaration;
    fs::write(root.join("Cargo.toml"), lines.join("\n") + "\n").unwrap();
}

fn offline(root: &Path, command: &mut Command) -> Output {
    command
        .current_dir(root)
        .env("CARGO_HOME", root.join("cargo-home"))
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CLAUDE_PROJECT_DIR", root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("RUSTFLAGS")
        .output()
        .unwrap()
}

fn registry_findings(report: &Value) -> Vec<(String, u64)> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| {
            diagnostic["code"] == "rust_doctor::cargo::unbounded_registry_dependency"
        })
        .map(|diagnostic| {
            (
                diagnostic["path"].as_str().unwrap().to_owned(),
                diagnostic["span"]["line_start"].as_u64().unwrap(),
            )
        })
        .collect()
}

/// US-002: a dependency requirement made unbounded in the change is judged by
/// the scope that judges the change, at its key, by the pre-commit command and
/// by the Stop hook alike; a change that leaves the manifest alone does not
/// bring the declaration back.
#[test]
fn an_unbounded_requirement_the_change_adds_is_reported_under_a_lines_scope() {
    let root = registry_repository("unbounded");
    let staged_lines = |root: &Path| {
        let output = offline(
            root,
            Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
                .arg(root)
                .args(["--json", "--yes", "--staged", "--scope", "lines"]),
        );
        let report = serde_json::from_slice::<Value>(&output.stdout);
        assert!(
            report.is_ok(),
            "no report: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        report.unwrap()
    };

    set_requirement(&root, "*");
    git(&root, &["add", "Cargo.toml"]);
    let report = staged_lines(&root);
    assert_eq!(
        registry_findings(&report),
        [("Cargo.toml".to_owned(), 7)],
        "{report}"
    );

    // The Stop hook judges the working tree against HEAD: the same addition,
    // unstaged, blocks the turn and is named at its key.
    git(&root, &["reset", "--quiet", "Cargo.toml"]);
    let mut hook = Command::new(env!("CARGO_BIN_EXE_rust-doctor"));
    hook.args(["hook", "run", "claude"]).stdin(std::process::Stdio::null());
    let blocked = offline(&root, &mut hook);
    let message = String::from_utf8_lossy(&blocked.stderr);
    assert_eq!(blocked.status.code(), Some(2), "{message}");
    assert!(
        message.contains("- rust_doctor::cargo::unbounded_registry_dependency Cargo.toml:7"),
        "{message}"
    );

    // Committed as unbounded, then a change elsewhere: the declaration is
    // legacy, and a lines scope that does not touch the manifest drops it.
    git(&root, &["add", "Cargo.toml"]);
    commit(&root, "unbounded");
    fs::write(
        root.join("src/lib.rs"),
        "pub fn registry_fixture() -> u8 {\n    dbg!(fixture_registry_alias::value())\n}\npub fn other() {}\n",
    )
    .unwrap();
    git(&root, &["add", "src/lib.rs"]);
    let report = staged_lines(&root);
    assert!(registry_findings(&report).is_empty(), "{report}");

    // The span is a location, not an identity: lines inserted above the key
    // move it, and a baseline run still pairs it with the base's finding.
    commit(&root, "other");
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        manifest.replacen("[dependencies]\n", "[dependencies]\n# moved down\n\n", 1),
    )
    .unwrap();
    let output = offline(
        &root,
        Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
            .arg(&root)
            .args(["--json", "--yes", "--scope", "baseline", "--base", "HEAD"]),
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["delta"]["summary"]["introduced"], 0, "{report}");
    assert_eq!(report["delta"]["summary"]["fixed"], 0, "{report}");
    fs::remove_dir_all(root).unwrap();
}

/// US-002: a lockfile finding has no line, so it is in a lines scope exactly
/// when the lockfile changed, a change that only deletes entries included.
#[test]
fn a_lockfile_deletion_alone_keeps_its_span_less_finding_in_scope() {
    let root = support::temporary_target("agent-loop-lockfile", &NEXT_REPOSITORY);
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cargo-health/resolution/duplicate");
    fs::create_dir_all(root.join("src")).unwrap();
    for file in ["Cargo.toml", "Cargo.lock", "src/lib.rs"] {
        fs::copy(fixture.join(file), root.join(file)).unwrap();
    }
    fs::write(root.join(".gitignore"), "/target\n").unwrap();
    git(&root, &["init", "--quiet", "--initial-branch=master"]);
    git(&root, &["add", "."]);
    commit(&root, "initial");

    let duplicates = |root: &Path| -> Vec<Value> {
        let output = offline(
            root,
            Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
                .arg(root)
                .args(["--json", "--yes", "--staged", "--scope", "lines"]),
        );
        let report = serde_json::from_slice::<Value>(&output.stdout);
        assert!(
            report.is_ok(),
            "no report: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        report.unwrap()["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|diagnostic| {
                diagnostic["code"] == "rust_doctor::cargo::duplicate_major_versions"
            })
            .cloned()
            .collect()
    };

    let lock = fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let pruned = lock
        .replacen("[[package]]\nname = \"aligned\"\nversion = \"0.3.1\"\n\n", "", 1)
        .replacen(" \"aligned 0.3.1\",\n", "", 1);
    assert_ne!(pruned, lock);
    fs::write(root.join("Cargo.lock"), &pruned).unwrap();
    git(&root, &["add", "Cargo.lock"]);
    let found = duplicates(&root);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["path"], "Cargo.lock");
    assert!(found[0]["span"].is_null(), "{found:?}");

    git(&root, &["reset", "--quiet", "--hard"]);
    fs::write(root.join("src/lib.rs"), "pub fn changed() {}\n").unwrap();
    git(&root, &["add", "src/lib.rs"]);
    assert!(duplicates(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

/// A function large enough for the clone rules, named once per copy.
fn clone_body(name: &str) -> String {
    format!(
        "pub fn {name}(values: &[u32]) -> u32 {{\n    let mut sum = 0;\n    for value in values {{\n        \
         if *value > 10 {{\n            sum += value * 2;\n        }} else {{\n            sum += value + 1;\n        }}\n    \
         }}\n    let doubled = sum * 3;\n    doubled + u32::from(values.is_empty())\n}}\n"
    )
}

fn duplicates(report: &Value) -> Vec<Value> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| {
            diagnostic["code"] == "rust_doctor::structure::duplicate_function_body"
        })
        .cloned()
        .collect()
}

/// US-003: a copy the change wrote next to committed code is reported at the
/// copy, under the same identity a full scan gives the family; a change that
/// reaches no member leaves the family out.
#[test]
fn a_staged_copy_of_committed_code_is_reported_at_the_copy() {
    let root = repository("clone");
    fs::write(root.join("src/a.rs"), clone_body("first_total")).unwrap();
    let library = library_with_todo_on_line_40() + "pub mod a;\n";
    fs::write(root.join("src/lib.rs"), &library).unwrap();
    git(&root, &["add", "."]);
    commit(&root, "original");

    fs::write(root.join("src/b.rs"), clone_body("second_total")).unwrap();
    fs::write(root.join("src/lib.rs"), library.clone() + "pub mod b;\n").unwrap();
    git(&root, &["add", "."]);
    let (code, scoped) = json(&root, &["--staged", "--scope", "lines"]);
    assert_eq!(code, Some(0), "{scoped}");
    let family = duplicates(&scoped);
    assert_eq!(family.len(), 1, "{scoped}");
    assert_eq!(family[0]["path"], "src/b.rs", "{scoped}");
    assert_eq!(family[0]["related"][0]["path"], "src/a.rs", "{scoped}");

    let (_, full) = json(&root, &["--scope", "full"]);
    let unscoped = duplicates(&full);
    assert_eq!(unscoped.len(), 1, "{full}");
    assert_eq!(unscoped[0]["path"], "src/a.rs", "{full}");
    assert_eq!(unscoped[0]["id"], family[0]["id"]);

    commit(&root, "copy");
    edit_lines_10_to_12(&root);
    let mut lines: Vec<String> = fs::read_to_string(root.join("src/lib.rs"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    lines.extend(["pub mod a;".to_owned(), "pub mod b;".to_owned()]);
    fs::write(root.join("src/lib.rs"), lines.join("\n") + "\n").unwrap();
    git(&root, &["add", "."]);
    let (_, elsewhere) = json(&root, &["--staged", "--scope", "lines"]);
    assert!(duplicates(&elsewhere).is_empty(), "{elsewhere}");
    fs::remove_dir_all(root).unwrap();
}
