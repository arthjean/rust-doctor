#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! `rust-doctor ci install` and `rust-doctor report markdown` from the binary's
//! entry point: the workflow written into a real repository and refused over
//! one it did not write, and a saved report rendered without a scan, pinned
//! against a golden file and refused whole when it cannot be trusted.

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use rust_doctor::SCHEMA_VERSION;
use rust_doctor::render::markdown::{read_saved_report, render_markdown};
use serde_json::{Value, json};

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn binary() -> Command {
    support::rust_doctor()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/report-markdown")
        .join(name)
}

fn workspace(name: &str) -> PathBuf {
    support::fresh_workspace(&format!("ci-report-{name}"), &NEXT_WORKSPACE)
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {arguments:?}");
}

/// A repository whose default branch is `trunk`, with one commit on it.
fn repository(name: &str) -> PathBuf {
    let root = workspace(name);
    git(&root, &["init", "--quiet", "--initial-branch=trunk"]);
    git(
        &root,
        &[
            "-c",
            "user.name=Rust Doctor",
            "-c",
            "user.email=rust-doctor@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    // A remote whose HEAD names trunk: the first candidate the base resolution
    // of a changed-work scope trusts.
    git(&root, &["remote", "add", "origin", "."]);
    git(&root, &["fetch", "--quiet", "origin"]);
    git(&root, &["remote", "set-head", "origin", "trunk"]);
    root
}

fn install(root: &Path, arguments: &[&str]) -> Output {
    binary()
        .args(["ci", "install"])
        .arg(root)
        .args(arguments)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn ci_install_writes_the_workflow_on_the_branch_the_repository_answers_for() {
    let root = repository("defaults");
    let written = install(&root, &[]);
    assert!(written.status.success(), "{}", text(&written.stderr));
    assert_eq!(text(&written.stdout), "Wrote .github/workflows/rust-doctor.yml\n");
    let workflow = fs::read_to_string(root.join(".github/workflows/rust-doctor.yml")).unwrap();
    assert!(workflow.contains("branches: ['trunk']"), "{workflow}");
    assert!(workflow.contains("toolchain: '1.99.1'"));
    assert!(workflow.contains("persist-credentials: false"));
    assert!(workflow.contains("permissions:\n  contents: read\n"));
    assert!(workflow.contains("--scope full --blocking none"));
    // The base is checked out into a snapshot under the temporary directory,
    // never a git worktree.
    assert!(workflow.contains("checks the base out into a temporary snapshot"));
    assert!(!workflow.contains("worktree"), "{workflow}");
    assert!(!root.join(".github/workflows/rust-doctor-comment.yml").exists());

    // An existing workflow is left alone without --update.
    let again = install(&root, &["--blocking", "warning"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(text(&again.stderr).contains("--update"), "{}", text(&again.stderr));
    assert_eq!(
        fs::read_to_string(root.join(".github/workflows/rust-doctor.yml")).unwrap(),
        workflow
    );

    // With it, the file rust-doctor wrote is rewritten, and --comment adds
    // the sticky comment as a second job, the only one that writes.
    let updated = install(
        &root,
        &["--update", "--blocking", "warning", "--branch", "main", "--toolchain", "1.98.0", "--comment"],
    );
    assert!(updated.status.success(), "{}", text(&updated.stderr));
    assert_eq!(text(&updated.stdout), "Rewrote .github/workflows/rust-doctor.yml\n");
    let rewritten = fs::read_to_string(root.join(".github/workflows/rust-doctor.yml")).unwrap();
    assert!(rewritten.contains("branches: ['main']"));
    assert!(rewritten.contains("toolchain: '1.98.0'"));
    assert!(rewritten.contains("--base \"origin/$BASE_REF\" --blocking warning)"));
    assert!(rewritten.contains("permissions:\n  contents: read\n\njobs:"));
    assert!(rewritten.contains("\n  comment:\n    name: Comment\n    needs: inspect\n"));
    assert!(rewritten.contains("    permissions:\n      pull-requests: write\n"));
    assert!(rewritten.contains("<!-- rust-doctor -->"));
    assert!(!root.join(".github/workflows/rust-doctor-comment.yml").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ci_install_never_writes_over_a_workflow_it_did_not_write() {
    let root = repository("foreign");
    let path = root.join(".github/workflows/rust-doctor.yml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "name: mine\n").unwrap();
    for arguments in [&[][..], &["--update"]] {
        let refused = install(&root, arguments);
        assert_eq!(refused.status.code(), Some(1), "{arguments:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "name: mine\n");
    }
    assert!(text(&install(&root, &["--update"]).stderr).contains("not written by rust-doctor"));

    // Nothing is written through a symlink either.
    fs::remove_dir_all(root.join(".github")).unwrap();
    let elsewhere = workspace("foreign-target");
    std::os::unix::fs::symlink(&elsewhere, root.join(".github")).unwrap();
    let linked = install(&root, &[]);
    assert_eq!(linked.status.code(), Some(2));
    assert!(fs::read_dir(&elsewhere).unwrap().next().is_none());
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(elsewhere).unwrap();
}

#[test]
fn ci_install_without_a_branch_names_the_flag_and_writes_nothing() {
    let root = workspace("no-branch");
    git(&root, &["init", "--quiet", "--initial-branch=trunk"]);
    let refused = install(&root, &[]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(text(&refused.stderr).contains("--branch"), "{}", text(&refused.stderr));
    assert!(!root.join(".github").exists());

    let dry = install(&root, &["--branch", "trunk", "--dry-run", "--comment"]);
    assert!(dry.status.success());
    assert!(text(&dry.stdout).starts_with("Would write .github/workflows/rust-doctor.yml:\n"));
    assert!(!root.join(".github").exists(), "a dry run writes nothing");
    fs::remove_dir_all(root).unwrap();
}

/// A Tauri-shaped repository: the Cargo workspace sits in `src-tauri/`, and
/// GitHub reads workflows only at the repository's top. Pointed at either
/// directory, the install writes at the top and scans the workspace.
#[test]
fn ci_install_writes_at_the_git_root_and_scans_the_nested_workspace() {
    let root = repository("nested");
    fs::create_dir_all(root.join("src-tauri/src")).unwrap();
    fs::write(
        root.join("src-tauri/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src-tauri/src/lib.rs"), "").unwrap();
    for pointed in [root.clone(), root.join("src-tauri")] {
        let written = binary()
            .args(["ci", "install"])
            .arg(&pointed)
            .env("CARGO_TARGET_DIR", support::scan_target(&pointed))
            .output()
            .unwrap();
        assert!(written.status.success(), "{}", text(&written.stderr));
        assert_eq!(text(&written.stdout), "Wrote .github/workflows/rust-doctor.yml\n");
        let workflow = fs::read_to_string(root.join(".github/workflows/rust-doctor.yml")).unwrap();
        assert!(workflow.contains("\"$launcher\" src-tauri --yes --json"), "{workflow}");
        assert!(workflow.contains("          workspaces: src-tauri\n"), "{workflow}");
        assert!(!root.join("src-tauri/.github").exists());
        fs::remove_dir_all(root.join(".github")).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

/// Outside a repository there is nowhere GitHub would read a workflow, and a
/// workflow that already runs rust-doctor under another name is the install.
#[test]
fn ci_install_refuses_outside_a_repository_and_beside_an_existing_install() {
    let outside = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "rust-doctor-ci-outside-{}",
        std::process::id()
    ));
    fs::create_dir_all(&outside).unwrap();
    let inside = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(&outside)
        .output()
        .unwrap();
    assert!(!inside.status.success(), "the temporary directory is inside a repository");
    let refused = install(&outside, &["--branch", "main"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(text(&refused.stderr).contains("not inside a git repository"), "{}", text(&refused.stderr));
    assert!(!outside.join(".github").exists());
    fs::remove_dir_all(outside).unwrap();

    let root = repository("existing");
    fs::create_dir_all(root.join(".github/workflows")).unwrap();
    let ci = "jobs:\n  scan:\n    steps:\n      - uses: arthjean/rust-doctor@v0\n";
    fs::write(root.join(".github/workflows/ci.yaml"), ci).unwrap();
    for arguments in [&[][..], &["--update"]] {
        let existing = install(&root, arguments);
        assert_eq!(existing.status.code(), Some(1), "{arguments:?}");
        assert!(
            text(&existing.stderr).contains(".github/workflows/ci.yaml already runs rust-doctor"),
            "{}",
            text(&existing.stderr)
        );
        assert!(!root.join(".github/workflows/rust-doctor.yml").exists());
    }
    fs::remove_dir_all(root).unwrap();
}

fn render(path: &Path, arguments: &[&str]) -> Output {
    binary()
        .args(["report", "markdown"])
        .arg(path)
        .args(arguments)
        .output()
        .unwrap()
}

/// The baseline report carries execution errors beside its findings; the
/// scope failure is a real baseline scan of a depth-1 clone, which failed
/// before it collected anything and must say so rather than "No finding."
#[test]
fn a_saved_report_renders_to_the_golden_markdown() {
    for name in ["baseline", "scope-failure"] {
        let rendered = render(&fixture(&format!("{name}.json")), &[]);
        assert!(rendered.status.success(), "{name}: {}", text(&rendered.stderr));
        assert_eq!(
            text(&rendered.stdout),
            fs::read_to_string(fixture(&format!("{name}.golden"))).unwrap(),
            "{name}"
        );
    }
    let failed = text(&render(&fixture("scope-failure.json"), &[]).stdout);
    assert!(failed.contains("- scope shallow-clone: "), "{failed}");
    assert!(failed.contains("`fetch-depth: 0`"), "{failed}");
    assert!(failed.ends_with("\nNo finding was collected.\n"), "{failed}");
    let limited = text(&render(&fixture("baseline.json"), &["--limit", "1"]).stdout);
    assert!(limited.contains("Showing 1 of 3 introduced findings."), "{limited}");
    assert_eq!(limited.matches("](https://rust-doctor.com/rules/").count(), 1);
}

#[test]
fn a_report_that_cannot_be_trusted_is_refused_with_nothing_on_stdout() {
    let root = workspace("refused");
    let mut other_schema: Value =
        serde_json::from_str(&fs::read_to_string(fixture("baseline.json")).unwrap()).unwrap();
    other_schema["schema_version"] = json!(17);
    let cases = [
        ("other-schema.json", serde_json::to_vec(&other_schema).unwrap()),
        ("invalid.json", b"{\"schema_version\": 20,".to_vec()),
    ];
    for (name, content) in cases {
        let path = root.join(name);
        fs::write(&path, content).unwrap();
        let refused = render(&path, &[]);
        assert_eq!(refused.status.code(), Some(2), "{name}");
        assert!(refused.stdout.is_empty(), "{name} printed to stdout");
        assert!(!refused.stderr.is_empty(), "{name} said nothing");
    }
    let stderr = text(&render(&root.join("other-schema.json"), &[]).stderr);
    assert!(
        stderr.contains("schema_version 17") && stderr.contains(&format!("reads {SCHEMA_VERSION}")),
        "{stderr}"
    );

    let oversized = root.join("oversized.json");
    fs::File::create(&oversized)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let refused = render(&oversized, &[]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(refused.stdout.is_empty());
    assert!(text(&refused.stderr).contains("64 MiB"), "{}", text(&refused.stderr));
    fs::remove_dir_all(root).unwrap();
}

/// Every tool a scan would start is on `PATH` as a script that leaves a mark,
/// and the render runs from a directory holding a Cargo workspace. Nothing is
/// marked.
#[test]
fn rendering_a_report_starts_no_process() {
    let root = workspace("no-process");
    let tools = root.join("bin");
    fs::create_dir_all(&tools).unwrap();
    for tool in ["cargo", "git", "rustc", "clippy-driver", "rustup", "npm", "gh"] {
        let script = tools.join(tool);
        fs::write(
            &script,
            format!("#!/bin/sh\ntouch \"{}/started-{tool}\"\n", root.display()),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let rendered = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .env_clear()
        .env("PATH", &tools)
        .current_dir(&root)
        .args(["report", "markdown"])
        .arg(fixture("baseline.json"))
        .output()
        .unwrap();
    assert!(rendered.status.success(), "{}", text(&rendered.stderr));
    let started: Vec<String> = fs::read_dir(&root)
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("started-"))
        .collect();
    assert!(started.is_empty(), "rendering started {started:?}");
    fs::remove_dir_all(root).unwrap();
}

/// EP-002 US-008 of the self-scan remediation: an authoritative score prints
/// its five dimensions under the score line, and a partial one does not.
#[test]
fn the_markdown_report_prints_the_dimensions_of_an_authoritative_score() {
    let root = workspace("dimensions");
    let mut report: Value =
        serde_json::from_str(&fs::read_to_string(fixture("baseline.json")).unwrap()).unwrap();
    let line = "Security 5 · Reliability 90 · Maintainability 72 · Performance 100 · Dependencies 51";
    let partial = text(&render(&fixture("baseline.json"), &[]).stdout);
    assert!(!partial.contains("Security "), "{partial}");

    report["audit"]["score"]["authoritative"] = json!(true);
    report["audit"]["score"]["reasons"] = json!([]);
    let path = root.join("authoritative.json");
    fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    let rendered = render(&path, &[]);
    assert!(rendered.status.success(), "{}", text(&rendered.stderr));
    let markdown = text(&rendered.stdout);
    assert!(
        markdown.contains(&format!("**Score: 40/100 (Critical)**, authoritative.\n\n{line}\n")),
        "{markdown}"
    );
    fs::remove_dir_all(root).unwrap();
}

/// EP-002 of the self-scan remediation: the Markdown report states the scope
/// the linear one does, names the members a `--package` scan selected and the
/// rules a crate version left unevaluated, and says when Clippy did not run.
#[test]
fn the_markdown_report_states_the_scope_and_the_rules_it_did_not_evaluate() {
    let root = workspace("disclosure");
    let mut report: Value =
        serde_json::from_str(&fs::read_to_string(fixture("baseline.json")).unwrap()).unwrap();
    report["project"]["packages"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "other-member" }));
    report["scan"]["command"] = Value::Null;
    report["policy"]["rules"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "rust_doctor::source::disabled_tls_verification", "not_evaluated": "crate-version-unsupported" }));
    let path = root.join("disclosure.json");
    fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    let rendered = render(&path, &[]);
    assert!(rendered.status.success(), "{}", text(&rendered.stderr));
    let markdown = text(&rendered.stdout);
    assert!(
        markdown.contains(
            "\nScored 1 production files (18 lines) of member policy-gate-product-loop; Clippy \
             did not run, native rules also read tests.\n"
        ),
        "{markdown}"
    );
    assert!(
        markdown.contains(
            "\nNot evaluated, the crate version is outside the range the rule was validated \
             against: `rust_doctor::source::disabled_tls_verification`.\n"
        ),
        "{markdown}"
    );
    fs::remove_dir_all(root).unwrap();
}

/// A 10,000-diagnostic report reads and renders inside the 500 ms the PRD
/// holds it to, times the machine allowance a slower runner declares.
#[test]
fn a_ten_thousand_diagnostic_report_renders_inside_its_budget() {
    let mut report: Value =
        serde_json::from_str(&fs::read_to_string(fixture("baseline.json")).unwrap()).unwrap();
    let template = report["diagnostics"][0].clone();
    let diagnostics: Vec<Value> = (0..10_000)
        .map(|index| {
            let mut diagnostic = template.clone();
            diagnostic["id"] = json!(format!("{index:064}"));
            diagnostic["path"] = json!(format!("src/module_{}.rs", index % 97));
            diagnostic["span"] = json!({ "line_start": index, "column_start": 1, "line_end": index, "column_end": 2 });
            diagnostic
        })
        .collect();
    report["delta"]["introduced"] = json!(
        diagnostics.iter().map(|diagnostic| diagnostic["id"].clone()).collect::<Vec<_>>()
    );
    report["diagnostics"] = json!(diagnostics);
    let root = workspace("budget");
    let path = root.join("large.json");
    fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();

    let allowance = std::env::var("RUST_DOCTOR_BENCHMARK_ALLOWANCE")
        .ok()
        .and_then(|factor| factor.parse::<u32>().ok())
        .filter(|factor| *factor >= 1)
        .unwrap_or(1);
    let started = Instant::now();
    let markdown = render_markdown(&read_saved_report(&path).unwrap(), 20).unwrap();
    let elapsed = started.elapsed();
    assert!(markdown.contains("Showing 20 of 10000 introduced findings."));
    assert!(
        elapsed < Duration::from_millis(500) * allowance,
        "rendering took {elapsed:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

/// EP-002 US-008: `why --report` refuses a document `report markdown`
/// refuses for its schema, with the same line and the same exit code, rather
/// than explaining a site from a report it cannot read.
#[test]
fn why_refuses_a_saved_report_of_another_schema_as_report_markdown_does() {
    let root = workspace("why-schema");
    let mut other_schema: Value =
        serde_json::from_str(&fs::read_to_string(fixture("baseline.json")).unwrap()).unwrap();
    other_schema["schema_version"] = json!(17);
    let mut unversioned = other_schema.clone();
    unversioned.as_object_mut().unwrap().remove("schema_version");
    for (name, document) in [("other-schema.json", other_schema), ("unversioned.json", unversioned)] {
        let path = root.join(name);
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let markdown = render(&path, &[]);
        let why = binary()
            .args(["why", "src/lib.rs:1", "--report"])
            .arg(&path)
            .env("PATH", "")
            .output()
            .unwrap();
        assert_eq!(why.status.code(), Some(2), "{name}");
        assert!(why.stdout.is_empty(), "{name}: {}", text(&why.stdout));
        assert_eq!(text(&why.stderr), text(&markdown.stderr), "{name}");
        assert!(text(&why.stderr).contains("schema_version"), "{name}");
    }
    fs::remove_dir_all(root).unwrap();
}

/// `why --report` writes a suppression directive only for a well-formed lint
/// path: a saved report whose `code` closes the attribute hands the reader no
/// Rust to paste, and its suppression fields reach the terminal as text.
#[test]
fn why_writes_a_directive_only_for_a_lint_path_from_a_saved_report() {
    let root = workspace("why-directive");
    let template: Value =
        serde_json::from_str(&fs::read_to_string(fixture("baseline.json")).unwrap()).unwrap();
    let why = |name: &str, code: &str, suppressions: Value, json: bool| {
        let mut report = template.clone();
        report["diagnostics"] = json!([{
            "code": code, "severity": "warning", "message": "m", "path": "src/lib.rs",
            "span": { "line_start": 1, "column_start": 1, "line_end": 1, "column_end": 2 },
            "related": [],
        }]);
        report["suppressions"] = suppressions;
        let path = root.join(name);
        fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
        let output = binary()
            .args(["why", "src/lib.rs:1", "--report"])
            .arg(&path)
            .args(json.then_some("--json"))
            .current_dir(&root)
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(output.status.success(), "{name}: {}", text(&output.stderr));
        format!("{}{}", text(&output.stdout), text(&output.stderr))
    };

    let hostile = "clippy::todo)]\nfn x(){}//";
    let printed = why("hostile.json", hostile, json!([]), false);
    assert!(!printed.lines().any(|line| line.contains("#[expect")), "{printed}");
    assert!(!printed.contains("To suppress this site"), "{printed}");
    let published: Value = serde_json::from_str(&why("hostile-json.json", hostile, json!([]), true)).unwrap();
    assert!(published["hits"][0].get("directive").is_none(), "{published}");

    let printed = why("clippy.json", "clippy::todo", json!([]), false);
    assert!(
        printed.contains("To suppress this site: #[expect(clippy::todo, reason = \"<why this site is not a defect>\")]"),
        "{printed}"
    );
    let native = "rust_doctor::source::blocking_call_in_async";
    let printed = why("native.json", native, json!([]), false);
    assert!(
        printed.contains(&format!("To suppress this site: // rust-doctor: allow({native}) -- ")),
        "{printed}"
    );

    let suppressions = json!([{
        "path": "src/lib.rs", "line": 1,
        "rules": ["rust_doctor::x\u{1b}]0;title\u{7}", "\u{9b}2Jy"], "status": "unused\u{9d}0;t\u{9c}",
    }]);
    let printed = why("escapes.json", "clippy::todo", suppressions, false);
    assert!(printed.contains("Suppression at src/lib.rs:1"), "{printed}");
    assert!(
        !printed.chars().any(|character| character.is_control() && character != '\n'),
        "{printed:?}"
    );
    fs::remove_dir_all(root).unwrap();
}
