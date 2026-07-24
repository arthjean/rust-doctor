// Integration test crates are outside Clippy's allow-unwrap-in-tests handling.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn prepare(directory: &Path, runner_temp: &Path, scope: &str, base: &str) -> String {
    let output_file = runner_temp.join("github-output");
    let _ = fs::remove_file(&output_file);
    let status = Command::new("bash")
        .arg(format!(
            "{}/scripts/action/prepare.sh",
            env!("CARGO_MANIFEST_DIR")
        ))
        .env("INPUT_SCOPE", scope)
        .env("INPUT_DIRECTORY", directory)
        .env("INPUT_VERSION", env!("CARGO_PKG_VERSION"))
        .env("EVENT_NAME", "pull_request")
        .env("PR_BASE_SHA", base)
        .env("PR_NUMBER", "1")
        .env("REPOSITORY", "fork-owner/repository")
        .env("GH_TOKEN", "")
        .env("RUNNER_TEMP", runner_temp)
        .env("GITHUB_OUTPUT", &output_file)
        .status()
        .unwrap();
    assert!(status.success());
    fs::read_to_string(output_file).unwrap()
}

#[test]
fn action_declares_complete_contract_and_pins_third_party_actions() {
    let action = fs::read_to_string("action.yml").unwrap();
    for input in [
        "directory:",
        "project:",
        "scope:",
        "blocking:",
        "require-complete:",
        "comment:",
        "review-comments:",
        "commit-status:",
        "sarif:",
        "version:",
    ] {
        assert!(action.contains(input), "missing action input {input}");
    }
    for line in action
        .lines()
        .filter(|line| line.trim().starts_with("uses:"))
    {
        let reference = line.split('#').next().unwrap().trim();
        let revision = reference.rsplit_once('@').unwrap().1.trim();
        assert_eq!(revision.len(), 40, "action is not commit-pinned: {line}");
        assert!(
            revision
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        );
    }
    assert!(action.contains("scripts/action/prepare.sh"));
    assert!(action.contains("scripts/action/report.sh"));
    assert!(action.contains("scripts/action/sarif.sh"));
}

#[test]
fn changed_path_resolution_preserves_unusual_rust_filenames() {
    let repository = tempfile::tempdir().unwrap();
    let root = repository.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "tests@rust-doctor.local"]);
    git(root, &["config", "user.name", "Rust Doctor Tests"]);
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn baseline() {}\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "baseline"]);
    let base = git(root, &["rev-parse", "HEAD"]);

    let unusual = ["src/space name.rs", "src/tab\tname.rs", "src/new\nline.rs"];
    for path in unusual {
        fs::write(root.join(path), "pub fn changed() {}\n").unwrap();
    }
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "head"]);

    let output_file = root.join("github-output");
    let status = Command::new("bash")
        .arg(format!(
            "{}/scripts/action/prepare.sh",
            env!("CARGO_MANIFEST_DIR")
        ))
        .env("INPUT_SCOPE", "baseline")
        .env("INPUT_DIRECTORY", root)
        .env("INPUT_VERSION", env!("CARGO_PKG_VERSION"))
        .env("EVENT_NAME", "pull_request")
        .env("PR_BASE_SHA", base)
        .env("PR_NUMBER", "1")
        .env("REPOSITORY", "example/repository")
        .env("GH_TOKEN", "")
        .env("RUNNER_TEMP", root)
        .env("GITHUB_OUTPUT", &output_file)
        .status()
        .unwrap();
    assert!(status.success());

    let outputs = fs::read_to_string(output_file).unwrap();
    let changed_file = outputs
        .lines()
        .find_map(|line| line.strip_prefix("changed-paths-file="))
        .unwrap();
    let changed = fs::read(changed_file).unwrap();
    let paths: Vec<_> = changed
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8(path.to_vec()).unwrap())
        .collect();
    for path in unusual {
        assert!(paths.iter().any(|candidate| candidate == path));
    }
    assert!(outputs.contains("scope=baseline"));
    assert!(outputs.contains("skip=false"));
}

#[test]
fn explicit_and_degraded_full_scopes_never_skip_a_pull_request() {
    let repository = tempfile::tempdir().unwrap();
    let root = repository.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "tests@rust-doctor.local"]);
    git(root, &["config", "user.name", "Rust Doctor Tests"]);
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "baseline"]);

    let explicit = prepare(root, root, "full", "");
    assert!(explicit.contains("scope=full"));
    assert!(explicit.contains("skip=false"));

    let degraded = prepare(
        root,
        root,
        "baseline",
        "0000000000000000000000000000000000000000",
    );
    assert!(degraded.contains("scope=full"));
    assert!(degraded.contains("skip=false"));
    assert!(degraded.contains("base history unavailable; running a full scan"));
}

#[test]
fn absolute_subdirectory_in_nested_detached_checkout_resolves_inner_repository() {
    let outer = tempfile::tempdir().unwrap();
    git(outer.path(), &["init", "-q"]);
    let inner = outer.path().join("nested");
    fs::create_dir(&inner).unwrap();
    git(&inner, &["init", "-q"]);
    git(&inner, &["config", "user.email", "tests@rust-doctor.local"]);
    git(&inner, &["config", "user.name", "Rust Doctor Tests"]);
    fs::create_dir(inner.join("src")).unwrap();
    fs::write(
        inner.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    fs::write(inner.join("src/lib.rs"), "pub fn nested() {}\n").unwrap();
    git(&inner, &["add", "."]);
    git(&inner, &["commit", "-qm", "baseline"]);
    git(&inner, &["checkout", "--detach", "-q"]);

    let outputs = prepare(&inner.join("src"), &inner, "full", "");
    assert!(outputs.contains(&format!("scan-root={}", inner.join("src").display())));
    assert!(outputs.contains(&format!("git-root={}", inner.display())));
    assert!(outputs.contains("skip=false"));
}

#[cfg(unix)]
fn executable(path: &Path, source: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, source).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
#[test]
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "the fake shell binary intentionally uses Bash parameter expansion"
)]
fn api_fallback_passes_repository_paths_through_the_relative_files_contract() {
    let repository = tempfile::tempdir().unwrap();
    let root = repository.path();
    let scan_root = root.join("nested");
    fs::create_dir_all(scan_root.join("src")).unwrap();
    fs::write(scan_root.join("src/lib.rs"), "pub fn value() {}\n").unwrap();
    let changed_paths = root.join("changed.paths");
    fs::write(&changed_paths, b"nested/src/lib.rs\0").unwrap();
    let fake_bin = root.join("bin");
    fs::create_dir(&fake_bin).unwrap();
    executable(
        &fake_bin.join("rust-doctor"),
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == validate-report ]]; then
  test -s "$2"
  exit 0
fi
printf '%s\0' "$@" > "$CAPTURE_ARGS"
report=""
while [[ "$#" -gt 0 ]]; do
  if [[ "$1" == --json-out ]]; then report=$2; shift 2; continue; fi
  shift
done
cp "$REPORT_FIXTURE" "$report"
"#,
    );
    let capture = root.join("arguments");
    let outputs = root.join("outputs");
    let path = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let status = Command::new("bash")
        .arg(format!(
            "{}/scripts/action/scan.sh",
            env!("CARGO_MANIFEST_DIR")
        ))
        .env("PATH", path)
        .env("CAPTURE_ARGS", &capture)
        .env(
            "REPORT_FIXTURE",
            format!(
                "{}/tests/fixtures/report-v1/nothing-to-scan.json",
                env!("CARGO_MANIFEST_DIR")
            ),
        )
        .env("INPUT_BLOCKING", "none")
        .env("INPUT_REQUIRE_COMPLETE", "false")
        .env("INPUT_PROJECT", "")
        .env("INPUT_MAX_DURATION", "")
        .env("RESOLVED_SCOPE", "api-files")
        .env("REQUESTED_SCOPE", "baseline")
        .env("BASE_SHA", "0000000000000000000000000000000000000000")
        .env("CHANGED_PATHS_FILE", &changed_paths)
        .env("SCAN_ROOT", &scan_root)
        .env("GIT_ROOT", root)
        .env("SKIP_SCAN", "false")
        .env(
            "DEGRADED_REASON",
            "base history unavailable; changed paths resolved through the GitHub API",
        )
        .env("RUNNER_TEMP", root)
        .env("GITHUB_OUTPUT", &outputs)
        .status()
        .unwrap();
    assert!(status.success());
    let arguments: Vec<_> = fs::read(capture)
        .unwrap()
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| String::from_utf8(argument.to_vec()).unwrap())
        .collect();
    let files = arguments
        .iter()
        .position(|argument| argument == "--files")
        .unwrap();
    assert_eq!(arguments[files + 1], "src/lib.rs");
    assert!(
        !arguments
            .iter()
            .any(|argument| argument == &scan_root.join("src/lib.rs").display().to_string())
    );
}

#[cfg(unix)]
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the integration test keeps one end-to-end review payload transaction visible"
)]
fn review_payload_is_json_safe_rebased_and_accepts_right_context_lines() {
    let repository = tempfile::tempdir().unwrap();
    let root = repository.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "tests@rust-doctor.local"]);
    git(root, &["config", "user.name", "Rust Doctor Tests"]);
    let scan_root = root.join("nested");
    fs::create_dir_all(scan_root.join("src")).unwrap();
    let relative = "src/tab\tline\n.rs";
    let source = scan_root.join(relative);
    fs::write(
        &source,
        "pub fn changed() -> u8 { 1 }\npub fn context() {}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "baseline"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    fs::write(
        &source,
        "pub fn changed() -> u8 { 2 }\npub fn context() {}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "head"]);
    let head = git(root, &["rev-parse", "HEAD"]);

    let report = root.join("report.json");
    fs::write(
        &report,
        serde_json::to_vec(&serde_json::json!({
            "report_constructed": true,
            "outcome": "findings",
            "mode": "baseline",
            "reporting_scope": "baseline",
            "completeness": {"state": "complete"},
            "diagnostics": [{
                "site_id": "aabbccdd",
                "rule": "unsafe`rule",
                "message": "line one\nline two\t\u{1}<tag>",
                "severity": "warning",
                "visible_on": ["pr-comment"],
                "location": {
                    "kind": "source",
                    "path": relative,
                    "range": {"start": {"line": 2, "column": 1}}
                }
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let fake_bin = root.join("bin");
    fs::create_dir(&fake_bin).unwrap();
    let captured_review_payload = root.join("review-payload.json");
    executable(
        &fake_bin.join("gh"),
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == *"/comments?per_page=100"* ]]; then
  printf '[[]]\n'
  exit 0
fi
if [[ "$*" == *"/reviews"* ]]; then
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == --input ]]; then cp "$2" "$CAPTURE_REVIEW_PAYLOAD"; exit 0; fi
    shift
  done
fi
exit 0
"#,
    );
    let path = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let status = Command::new("bash")
        .arg(format!(
            "{}/scripts/action/report.sh",
            env!("CARGO_MANIFEST_DIR")
        ))
        .current_dir(root)
        .env("PATH", path)
        .env("CAPTURE_REVIEW_PAYLOAD", &captured_review_payload)
        .env("REPORT_FILE", &report)
        .env("COMMENT_ENABLED", "false")
        .env("REVIEW_COMMENTS_ENABLED", "true")
        .env("COMMIT_STATUS_ENABLED", "false")
        .env("EVENT_NAME", "pull_request")
        .env("PR_NUMBER", "7")
        .env("REPOSITORY", "example/repository")
        .env("COMMIT_SHA", &head)
        .env("BASE_SHA", &base)
        .env("SCAN_ROOT", &scan_root)
        .env("GIT_ROOT", root)
        .env("DEGRADED_REASON", "")
        .env("SERVER_URL", "https://github.com")
        .env("RUN_ID", "1")
        .env("RUN_ATTEMPT", "1")
        .env("EXIT_CODE", "0")
        .env("SKIP_SCAN", "false")
        .env("RUNNER_TEMP", root)
        .status()
        .unwrap();
    assert!(status.success());
    let payload: serde_json::Value =
        serde_json::from_slice(&fs::read(captured_review_payload).unwrap()).unwrap();
    let comment = &payload["comments"][0];
    assert_eq!(comment["path"], format!("nested/{relative}"));
    assert_eq!(comment["line"], 2);
    assert_eq!(comment["side"], "RIGHT");
    let body = comment["body"].as_str().unwrap();
    assert!(body.contains("&lt;tag&gt;"));
    assert!(!body.contains('\t'));
    assert!(!body.contains('\u{1}'));
    assert!(!body.contains("line one\nline two"));
}

#[cfg(unix)]
#[test]
fn sarif_payload_rebases_nested_paths_and_preserves_canonical_identity() {
    let repository = tempfile::tempdir().unwrap();
    let root = repository.path();
    let scan_root = root.join("nested");
    fs::create_dir_all(&scan_root).unwrap();
    let report = root.join("report.json");
    fs::write(
        &report,
        serde_json::to_vec(&serde_json::json!({
            "tool_version": "0.2.0",
            "diagnostics": [{
                "site_id": "aabbccdd",
                "rule": "rust-doctor/unsafe-block-audit",
                "title": "Unsafe block requires an audit",
                "message": "document the invariant",
                "help": "add a safety comment",
                "url": "https://example.invalid/rule",
                "severity": "warning",
                "visible_on": ["sarif"],
                "location": {
                    "kind": "source",
                    "path": "src/lib.rs",
                    "range": {
                        "start": {"line": 3, "column": 2},
                        "end": {"line": 3, "column": 8}
                    }
                }
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let outputs = root.join("outputs");
    let status = Command::new("bash")
        .arg(format!(
            "{}/scripts/action/sarif.sh",
            env!("CARGO_MANIFEST_DIR")
        ))
        .env("REPORT_FILE", &report)
        .env("RUNNER_TEMP", root)
        .env("GIT_ROOT", root)
        .env("SCAN_ROOT", &scan_root)
        .env("GITHUB_OUTPUT", &outputs)
        .status()
        .unwrap();
    assert!(status.success());
    let output = fs::read_to_string(outputs).unwrap();
    let sarif_path = output.trim().strip_prefix("sarif-file=").unwrap();
    let sarif: serde_json::Value = serde_json::from_slice(&fs::read(sarif_path).unwrap()).unwrap();
    let result = &sarif["runs"][0]["results"][0];
    assert_eq!(result["ruleId"], "rust-doctor/unsafe-block-audit");
    assert_eq!(
        result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "nested/src/lib.rs"
    );
    assert_eq!(
        result["partialFingerprints"]["rustDoctorSiteId/v1"],
        "aabbccdd"
    );
    assert_eq!(
        result["locations"][0]["physicalLocation"]["region"]["startLine"],
        3
    );
}

#[cfg(unix)]
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the integration test keeps one cross-channel reporting transaction visible"
)]
fn sticky_summary_updates_after_an_independent_status_permission_failure() {
    let repository = tempfile::tempdir().unwrap();
    let root = repository.path();
    let report = root.join("report.json");
    fs::write(
        &report,
        serde_json::to_vec(&serde_json::json!({
            "report_constructed": true,
            "outcome": "findings",
            "mode": "baseline",
            "baseline": {"new_count": 2, "fixed_count": 1},
            "completeness": {"state": "complete"},
            "summary": {"score": 91, "error_count": 1, "warning_count": 2},
            "projects": [{"cargo_package_id": "example-package"}],
            "diagnostics": [{
                "rule": "rust-doctor/example",
                "severity": "error",
                "visible_on": ["pr-comment"]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let fake_bin = root.join("bin");
    fs::create_dir(&fake_bin).unwrap();
    let calls = root.join("gh-calls");
    let captured_summary = root.join("summary.md");
    executable(
        &fake_bin.join("gh"),
        r#"#!/usr/bin/env bash
set -euo pipefail
printf '__CALL__\0' >> "$GH_CALLS"
printf '%s\0' "$@" >> "$GH_CALLS"
if [[ "$*" == *"/statuses/"* ]]; then exit 1; fi
if [[ "$*" == *"/issues/7/comments?per_page=100"* ]]; then printf '42\n'; exit 0; fi
if [[ "$*" == *"/issues/comments/42"* ]]; then
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == body=@* ]]; then cp "${1#body=@}" "$CAPTURE_SUMMARY"; exit 0; fi
    shift
  done
fi
exit 1
"#,
    );
    let path = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let status = Command::new("bash")
        .arg(format!(
            "{}/scripts/action/report.sh",
            env!("CARGO_MANIFEST_DIR")
        ))
        .current_dir(root)
        .env("PATH", path)
        .env("GH_CALLS", &calls)
        .env("CAPTURE_SUMMARY", &captured_summary)
        .env("REPORT_FILE", &report)
        .env("COMMENT_ENABLED", "true")
        .env("REVIEW_COMMENTS_ENABLED", "false")
        .env("COMMIT_STATUS_ENABLED", "true")
        .env("EVENT_NAME", "pull_request")
        .env("PR_NUMBER", "7")
        .env("REPOSITORY", "example/repository")
        .env("COMMIT_SHA", "1111111111111111111111111111111111111111")
        .env("BASE_SHA", "0000000000000000000000000000000000000000")
        .env("SCAN_ROOT", root)
        .env("GIT_ROOT", root)
        .env("DEGRADED_REASON", "")
        .env("SERVER_URL", "https://github.com")
        .env("RUN_ID", "1")
        .env("RUN_ATTEMPT", "1")
        .env("EXIT_CODE", "3")
        .env("SKIP_SCAN", "false")
        .env("RUNNER_TEMP", root)
        .status()
        .unwrap();
    assert!(status.success());
    let call_arguments: Vec<_> = fs::read(calls)
        .unwrap()
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| String::from_utf8(argument.to_vec()).unwrap())
        .collect();
    assert!(
        call_arguments
            .iter()
            .any(|argument| argument == "state=failure")
    );
    assert!(
        call_arguments
            .iter()
            .any(|argument| argument == "repos/example/repository/issues/comments/42")
    );
    let summary = fs::read_to_string(captured_summary).unwrap();
    assert!(summary.contains("<!-- rust-doctor-report:v1 -->"));
    assert!(summary.contains("| Completeness | complete |"));
    assert!(summary.contains("| Score | 91/100 |"));
    assert!(summary.contains("| Introduced | 2 |"));
    assert!(summary.contains("| Fixed | 1 |"));
    assert!(summary.contains("Affected packages: example-package"));
    assert!(summary.contains("rust-doctor/example (1)"));
}
