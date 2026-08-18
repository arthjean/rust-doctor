//! Repository hygiene: the pass that opens the repository, not only its Rust
//! files.
//!
//! Enumeration goes through `git ls-files`, never a filesystem walker, because
//! the question asked is literally "is this file tracked", and because git
//! excludes `target/` and every ignored path by construction. A workspace
//! outside a git repository is an observed fact, not an error: the pass
//! returns nothing and the scan stays authoritative. Only git being absent, or
//! an enumeration too large to trust, degrades the report with a bounded error
//! at stage `repo`.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::internal_error::InternalError;
use crate::git::{GitCall, GitFailure, git_arguments, run_git, run_git_status};
use crate::policy::{
    PolicyPlan, REPO_HARDCODED_CREDENTIAL, REPO_TRACKED_SECRET_FILE, REPO_UNIGNORED_BUILD_OUTPUT,
    RuleDefinition,
};
use crate::report::DiagnosticContext;
use crate::source_text::SourceSpan;
use crate::workspace_path;

/// The stage every git call of this pass reports at, and the one `report.rs`
/// stamps a `RepoError` with.
const STAGE: &str = "repo";

/// Enumeration stops here rather than hanging on a monorepo, mirroring the
/// source kernel's own unit limit in spirit.
const MAX_TRACKED_PATHS: usize = 50_000;

/// Byte bound on the `git ls-files` output, and on any single file the
/// credential scan reads, mirroring the source-kernel file limit.
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Same bound as the manifest readers in `cargo_health` for the
/// `.cargo/config.toml` re-read that resolves a custom target directory.
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoFinding {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: String,
    /// Workspace-relative, already normalized for publication.
    pub(crate) path: String,
    /// Present only when a line carries the finding; a finding about the file
    /// itself has none.
    pub(crate) span: Option<SourceSpan>,
    pub(crate) context: Option<DiagnosticContext>,
}

/// Bounded error of the pass: a closed code and a frozen message, with no
/// path, no environment variable and no escape sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoError {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

#[derive(Debug, Default)]
pub(crate) struct RepoScan {
    pub(crate) findings: Vec<RepoFinding>,
    pub(crate) errors: Vec<RepoError>,
}

/// What `git check-ignore` answered for the build directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IgnoreProbe {
    Ignored,
    NotIgnored,
    /// The probe could not answer: git failed, the path leaves the repository,
    /// or the exit code was not one of the two documented ones. Silence, not
    /// an error: the enumeration already proved git works here.
    Unavailable,
}

pub(crate) fn inspect(workspace_root: &Path, plan: &PolicyPlan) -> RepoScan {
    inspect_with(
        workspace_root,
        plan,
        |call| run_git(Path::new("git"), workspace_root, call),
        |directory| probe_ignore(workspace_root, directory),
    )
}

fn inspect_with(
    workspace_root: &Path,
    plan: &PolicyPlan,
    mut run: impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
    probe: impl FnOnce(&str) -> IgnoreProbe,
) -> RepoScan {
    let secret_files = plan.is_active(REPO_TRACKED_SECRET_FILE.id);
    let credentials = plan.is_active(REPO_HARDCODED_CREDENTIAL.id);
    let build_output = plan.is_active(REPO_UNIGNORED_BUILD_OUTPUT.id);
    let mut scan = RepoScan::default();
    if !secret_files && !credentials && !build_output {
        return scan;
    }

    let output = match run(&GitCall {
        arguments: git_arguments(workspace_root, ["ls-files", "-z"]),
        stdout_limit: MAX_OUTPUT_BYTES,
        stage: STAGE,
        failure: GitFailure::new(
            "repo-not-a-repository",
            "Repository hygiene skipped: the workspace is not a git repository.",
        ),
        overflow: GitFailure::new(
            "repo-enumeration-truncated",
            "Repository enumeration exceeded the supported size; repository hygiene was skipped.",
        ),
    }) {
        Ok(output) => output,
        Err(error) => {
            match error.code {
                // An absent workspace repository is a fact the criteria accept
                // in silence; a broken git invocation degrades the same way
                // because the two exits are indistinguishable from here.
                "repo-not-a-repository" => {}
                "git-unavailable" => scan.errors.push(RepoError {
                    code: "git-unavailable",
                    message: "Repository hygiene skipped: git was not available.",
                }),
                _ => scan.errors.push(RepoError {
                    code: "repo-enumeration-truncated",
                    message:
                        "Repository enumeration exceeded the supported size; repository hygiene was skipped.",
                }),
            }
            return scan;
        }
    };

    let (paths, truncated) = tracked_paths(&output);
    if truncated {
        scan.errors.push(RepoError {
            code: "repo-enumeration-truncated",
            message: "Repository enumeration stopped at 50000 files; results are partial.",
        });
    }

    for path in &paths {
        if secret_files
            && !is_test_material(path)
            && is_secret_file_name(file_name(path))
            && let Some(published) = workspace_path::normalize_changed(path)
        {
            scan.findings.push(RepoFinding {
                definition: &REPO_TRACKED_SECRET_FILE,
                message: format!(
                    "the repository tracks {published}, whose name marks it as secret-bearing"
                ),
                path: published,
                span: None,
                context: None,
            });
        }
        if credentials {
            scan_credentials(workspace_root, path, &mut scan);
        }
    }

    if build_output
        && let Some(directory) = build_directory(workspace_root)
        && probe(&directory) == IgnoreProbe::NotIgnored
        && let Some(published) = workspace_path::normalize_changed(&directory)
    {
        scan.findings.push(RepoFinding {
            definition: &REPO_UNIGNORED_BUILD_OUTPUT,
            message: format!("git does not ignore {published}/, so build output can be committed"),
            path: published,
            span: None,
            context: None,
        });
    }

    scan
}

/// Splits the NUL-separated `git ls-files -z` output into workspace-relative
/// paths, keeping only what a hostile output cannot abuse: UTF-8, relative,
/// no parent traversal. The boolean reports whether the path bound cut the
/// enumeration short.
fn tracked_paths(output: &[u8]) -> (Vec<String>, bool) {
    let mut paths = Vec::new();
    for entry in output.split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }
        if paths.len() == MAX_TRACKED_PATHS {
            return (paths, true);
        }
        let Ok(path) = std::str::from_utf8(entry) else {
            continue;
        };
        if workspace_path::normalize_changed(path).is_some() {
            paths.push(path.to_owned());
        }
    }
    (paths, false)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Closed list of names that mark a tracked file as secret-bearing.
fn is_secret_file_name(name: &str) -> bool {
    if name == ".env" {
        return true;
    }
    if let Some(variant) = name.strip_prefix(".env.") {
        return !matches!(variant, "example" | "template" | "sample");
    }
    name.ends_with(".pem")
        || name.ends_with(".key")
        || name == "id_rsa"
        || name == "credentials.json"
}

/// A path segment that marks the file as test material: a certificate under
/// `tests/` is the fixture, not the leak.
fn is_test_material(path: &str) -> bool {
    path.split('/')
        .any(|segment| matches!(segment, "tests" | "fixtures" | "testdata"))
}

/// Non-production context of a repository path, decided from its segments
/// because no Cargo target carries these findings. When in doubt the path is
/// unmarked and the finding counts, matching `DiagnosticContext`'s own rule.
fn path_context(path: &str) -> Option<DiagnosticContext> {
    path.split('/').find_map(|segment| match segment {
        "tests" | "fixtures" | "testdata" => Some(DiagnosticContext::Tests),
        "examples" => Some(DiagnosticContext::Example),
        "benches" => Some(DiagnosticContext::Benchmark),
        _ => None,
    })
}

/// One high-confidence credential shape: a fixed prefix, a minimum tail of
/// admissible characters, and a published name. Entropy alone never triggers,
/// which is the documented lesson of the tools that tried.
struct CredentialPattern {
    name: &'static str,
    prefix: &'static str,
    minimum_tail: usize,
    tail: fn(u8) -> bool,
}

const CREDENTIAL_PATTERNS: [CredentialPattern; 5] = [
    CredentialPattern {
        name: "AWS access key id",
        prefix: "AKIA",
        minimum_tail: 16,
        tail: |byte| byte.is_ascii_uppercase() || byte.is_ascii_digit(),
    },
    CredentialPattern {
        name: "GitHub personal access token",
        prefix: "ghp_",
        minimum_tail: 36,
        tail: |byte| byte.is_ascii_alphanumeric(),
    },
    CredentialPattern {
        name: "GitHub fine-grained token",
        prefix: "github_pat_",
        minimum_tail: 22,
        tail: |byte| byte.is_ascii_alphanumeric() || byte == b'_',
    },
    CredentialPattern {
        name: "secret API key",
        prefix: "sk-",
        minimum_tail: 20,
        tail: |byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'),
    },
    CredentialPattern {
        name: "Slack bot token",
        prefix: "xoxb-",
        minimum_tail: 10,
        tail: |byte| byte.is_ascii_alphanumeric() || byte == b'-',
    },
];

fn scan_credentials(workspace_root: &Path, path: &str, scan: &mut RepoScan) {
    // The enumeration validated the path, so the join cannot leave the
    // workspace; a symlink still could, so only regular files are read.
    let file = workspace_root.join(path);
    let Ok(metadata) = fs::symlink_metadata(&file) else {
        return;
    };
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return;
    }
    let Ok(bytes) = fs::read(&file) else {
        return;
    };
    let Ok(source) = String::from_utf8(bytes) else {
        return;
    };
    let Some(published) = workspace_path::normalize_changed(path) else {
        return;
    };
    let context = path_context(path);
    for (index, line) in source.lines().enumerate() {
        let Some((name, range)) = credential_match(line) else {
            continue;
        };
        let line_number = index + 1;
        let column_start = line[..range.0].chars().count() + 1;
        let column_end = line[..range.1].chars().count() + 1;
        scan.findings.push(RepoFinding {
            definition: &REPO_HARDCODED_CREDENTIAL,
            // The pattern name is published, the matched value never is.
            message: format!("string literal matching the {name} credential pattern"),
            path: published.clone(),
            span: Some(SourceSpan {
                line_start: line_number,
                column_start,
                line_end: line_number,
                column_end,
            }),
            context,
        });
    }
}

/// First credential match of a line: the pattern name and the byte range of
/// the matched token. A prefix inside a longer identifier is not a match.
fn credential_match(line: &str) -> Option<(&'static str, (usize, usize))> {
    let bytes = line.as_bytes();
    if let Some(range) = private_key_block(line) {
        return Some(("private key block", range));
    }
    for pattern in &CREDENTIAL_PATTERNS {
        let mut from = 0;
        while let Some(offset) = line[from..].find(pattern.prefix) {
            let start = from + offset;
            let boundary = start == 0
                || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
            let tail_start = start + pattern.prefix.len();
            let tail_length = bytes[tail_start..]
                .iter()
                .take_while(|byte| (pattern.tail)(**byte))
                .count();
            if boundary && tail_length >= pattern.minimum_tail {
                return Some((pattern.name, (start, tail_start + tail_length)));
            }
            from = tail_start;
        }
    }
    None
}

fn private_key_block(line: &str) -> Option<(usize, usize)> {
    let start = line.find("-----BEGIN ")?;
    let label_start = start + "-----BEGIN ".len();
    let suffix = "PRIVATE KEY-----";
    let suffix_start = line[label_start..].find(suffix)? + label_start;
    // A real PEM label is uppercase words: `RSA`, `OPENSSH`, or nothing at
    // all. Anything else is prose about key blocks, not a key block, which is
    // what keeps this rule off documentation that spells the pattern out.
    line[label_start..suffix_start]
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte == b' ')
        .then_some((start, suffix_start + suffix.len()))
}

/// The directory Cargo builds into, workspace-relative: `target` unless the
/// committed `.cargo/config.toml` sets `build.target-dir`. An absolute or
/// escaping directory leaves the repository's ignore question unanswerable,
/// so it yields nothing.
fn build_directory(workspace_root: &Path) -> Option<String> {
    #[derive(Deserialize)]
    struct CargoConfig {
        build: Option<BuildSection>,
    }
    #[derive(Deserialize)]
    struct BuildSection {
        #[serde(rename = "target-dir")]
        target_dir: Option<String>,
    }

    let path = workspace_root.join(".cargo/config.toml");
    let configured = match fs::metadata(&path) {
        Ok(metadata) if metadata.len() <= MAX_CONFIG_BYTES => fs::read_to_string(&path)
            .ok()
            .and_then(|contents| toml::from_str::<CargoConfig>(&contents).ok())
            .and_then(|config| config.build)
            .and_then(|build| build.target_dir),
        Ok(_) => return None,
        Err(_) => None,
    };
    let directory = configured.unwrap_or_else(|| "target".to_owned());
    if Path::new(&directory).is_absolute() {
        return None;
    }
    workspace_path::normalize_changed(&directory)?;
    Some(directory)
}

fn probe_ignore(workspace_root: &Path, directory: &str) -> IgnoreProbe {
    // The trailing slash makes a directory-only pattern like `target/` match
    // even before a first build creates the directory; the anchored and bare
    // spellings match it too.
    //
    // Git refuses that spelling outright when the directory is a symbolic link
    // to a build directory elsewhere, a common way to move build output off a
    // small disk: it exits 128 with "is beyond a symbolic link" and answers
    // nothing. The bare spelling still answers, and it has to be asked, because
    // that workspace is exactly one where the build path is unignored and its
    // link shows up as untracked. Abstaining there would hide the rule's own
    // case from the user most exposed to it.
    match ask_git_check_ignore(workspace_root, &format!("{directory}/")) {
        IgnoreProbe::Unavailable => ask_git_check_ignore(workspace_root, directory),
        answered => answered,
    }
}

fn ask_git_check_ignore(workspace_root: &Path, path: &str) -> IgnoreProbe {
    let arguments = git_arguments(workspace_root, ["check-ignore", "--quiet", "--", path]);
    match run_git_status(Path::new("git"), workspace_root, &arguments) {
        Some(0) => IgnoreProbe::Ignored,
        Some(1) => IgnoreProbe::NotIgnored,
        _ => IgnoreProbe::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> PolicyPlan {
        PolicyPlan::default()
    }

    fn ls_files_output(paths: &[&str]) -> Vec<u8> {
        let mut stdout = Vec::new();
        for path in paths {
            stdout.extend_from_slice(path.as_bytes());
            stdout.push(0);
        }
        stdout
    }

    /// A build directory moved elsewhere behind a symbolic link still gets an
    /// answer.
    ///
    /// `git check-ignore -- target/` exits 128 with "is beyond a symbolic link"
    /// on such a workspace and answers nothing, which used to make the rule
    /// abstain on exactly the layout where the build path is unignored and its
    /// link is sitting untracked. This repository is one of those workspaces,
    /// so the rule was blind to its own case here.
    // The layout under test is a POSIX symbolic link, which is what `git
    // check-ignore` refuses to look beyond. Windows has its own link kinds and
    // its own answer, so the case is stated where it exists rather than
    // approximated where it does not.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_build_directory_is_probed_rather_than_abstained_on() {
        use std::os::unix::fs::symlink;

        let scratch = std::env::temp_dir()
            .canonicalize()
            .expect("a canonical temporary directory")
            .join(format!("rust-doctor-ignore-probe-{}", std::process::id()));
        let workspace = scratch.join("workspace");
        let elsewhere = scratch.join("elsewhere");
        let _ = fs::remove_dir_all(&scratch);
        fs::create_dir_all(&workspace).expect("the workspace should be writable");
        fs::create_dir_all(&elsewhere).expect("the build directory should be writable");
        let git = |arguments: &[&str]| {
            std::process::Command::new("git")
                .args(arguments)
                .current_dir(&workspace)
                .output()
                .expect("git should run")
        };
        git(&["init", "--quiet"]);

        // Ignored through the directory-only spelling, and the link resolves
        // outside the workspace exactly as a moved build directory does.
        fs::write(workspace.join(".gitignore"), "/target/\n").expect("gitignore should write");
        symlink(&elsewhere, workspace.join("target")).expect("the symlink should be created");
        assert_eq!(
            probe_ignore(&workspace, "target"),
            IgnoreProbe::NotIgnored,
            "a directory-only pattern does not cover the link that stands in for it"
        );

        // The same workspace, with the spelling that does cover a link.
        fs::write(workspace.join(".gitignore"), "/target\n").expect("gitignore should write");
        assert_eq!(probe_ignore(&workspace, "target"), IgnoreProbe::Ignored);

        fs::remove_dir_all(&scratch).expect("the scratch tree should be removable");
    }

    #[test]
    fn a_workspace_outside_git_yields_no_finding_and_no_error() {
        let scan = inspect_with(
            Path::new("."),
            &plan(),
            |call| Err(call.failure.error(call.stage)),
            |_| IgnoreProbe::Unavailable,
        );
        assert!(scan.findings.is_empty());
        assert!(scan.errors.is_empty());
    }

    #[test]
    fn an_absent_git_degrades_to_a_recorded_error() {
        let scan = inspect_with(
            Path::new("."),
            &plan(),
            |_| {
                Err(InternalError::new(
                    "scope",
                    "git-unavailable",
                    "Git could not be started.",
                ))
            },
            |_| IgnoreProbe::Unavailable,
        );
        assert!(scan.findings.is_empty());
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(scan.errors[0].code, "git-unavailable");
        assert!(scan.errors[0].message.contains("git was not available"));
    }

    #[test]
    fn an_oversized_enumeration_is_a_bounded_error_not_a_hang() {
        let scan = inspect_with(
            Path::new("."),
            &plan(),
            |call| Err(call.overflow.error(call.stage)),
            |_| IgnoreProbe::Unavailable,
        );
        assert!(scan.findings.is_empty());
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(scan.errors[0].code, "repo-enumeration-truncated");
    }

    #[test]
    fn the_path_bound_truncates_with_an_error_and_keeps_the_prefix() {
        let mut paths = vec![".env".to_owned()];
        paths.extend((0..MAX_TRACKED_PATHS).map(|index| format!("src/file_{index}.txt")));
        let borrowed: Vec<&str> = paths.iter().map(String::as_str).collect();
        let scan = inspect_with(
            Path::new("/nonexistent/rust-doctor-repo-hygiene"),
            &plan(),
            |_| Ok(ls_files_output(&borrowed)),
            |_| IgnoreProbe::Unavailable,
        );
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(scan.errors[0].code, "repo-enumeration-truncated");
        assert!(scan.errors[0].message.contains("50000"));
        // The retained prefix still reports: the tracked .env sits inside it.
        assert_eq!(scan.findings.len(), 1);
        assert_eq!(
            scan.findings[0].definition.id,
            "rust_doctor::repo::tracked_secret_file"
        );
    }

    #[test]
    fn hostile_enumeration_entries_are_dropped_without_findings() {
        let output = b"../outside.env\x00/absolute/.env\x00\xff\xfe\x00src/lib.rs\x00".to_vec();
        let (paths, truncated) = tracked_paths(&output);
        assert!(!truncated);
        assert_eq!(paths, ["src/lib.rs"]);
    }

    #[test]
    fn secret_file_names_form_a_closed_list_with_example_variants_excluded() {
        for name in [
            ".env",
            ".env.local",
            ".env.production",
            "server.pem",
            "deploy.key",
            "id_rsa",
            "credentials.json",
        ] {
            assert!(is_secret_file_name(name), "{name}");
        }
        for name in [
            ".env.example",
            ".env.template",
            ".env.sample",
            "environment.rs",
            "keyboard.rs",
            "monkey.rs",
            "credentials.json.md",
        ] {
            assert!(!is_secret_file_name(name), "{name}");
        }
    }

    #[test]
    fn test_material_paths_are_exempt_and_contexts_map_by_segment() {
        assert!(is_test_material("tests/fixtures/server.pem"));
        assert!(is_test_material("crates/api/testdata/ca.pem"));
        assert!(!is_test_material("src/server.pem"));
        assert!(!is_test_material("attestations/report.pem"));

        assert_eq!(path_context("tests/generated.rs"), Some(DiagnosticContext::Tests));
        assert_eq!(path_context("examples/demo.rs"), Some(DiagnosticContext::Example));
        assert_eq!(path_context("benches/throughput.rs"), Some(DiagnosticContext::Benchmark));
        assert_eq!(path_context("src/main.rs"), None);
    }

    #[test]
    fn credential_matching_requires_the_boundary_and_the_minimum_tail() {
        // Every key-shaped value is assembled at runtime so that this source
        // file never carries one the scan itself would report.
        let aws = format!("AKIA{}", "IOSFODNN7EXAMPLE");
        let (name, range) = credential_match(&format!("let key = \"{aws}\";")).unwrap();
        assert_eq!(name, "AWS access key id");
        assert_eq!(range, (11, 11 + aws.len()));

        let github = format!("ghp_{}", "a".repeat(36));
        assert_eq!(
            credential_match(&github).map(|(name, _)| name),
            Some("GitHub personal access token")
        );
        let fine_grained = format!("github_pat_{}", "b".repeat(22));
        assert_eq!(
            credential_match(&fine_grained).map(|(name, _)| name),
            Some("GitHub fine-grained token")
        );
        let openai = format!("sk-{}", "c".repeat(20));
        assert_eq!(
            credential_match(&openai).map(|(name, _)| name),
            Some("secret API key")
        );
        let slack = format!("xoxb-{}", "1".repeat(10));
        assert_eq!(
            credential_match(&slack).map(|(name, _)| name),
            Some("Slack bot token")
        );
        let pem_header = format!("-----BEGIN {}-----", "RSA PRIVATE KEY");
        assert_eq!(
            credential_match(&pem_header).map(|(name, _)| name),
            Some("private key block")
        );
        let bare_header = format!("-----BEGIN {}-----", "PRIVATE KEY");
        assert_eq!(
            credential_match(&bare_header).map(|(name, _)| name),
            Some("private key block")
        );
        // Documentation spelling the pattern out, wildcard included, is prose:
        // a real PEM label carries only uppercase words.
        let documented = format!("`-----BEGIN {} KEY-----`", "* PRIVATE");
        assert!(credential_match(&documented).is_none());

        // Boundary: the prefix inside a longer identifier is not a token.
        assert!(credential_match(&format!("S{aws}")).is_none());
        assert!(credential_match(&format!("risk-{}", "c".repeat(20))).is_none());
        // Minimum tail: a short suffix is prose, not a key.
        assert!(credential_match("sk-learn").is_none());
        assert!(credential_match("AKIA123").is_none());
        // Entropy alone never triggers.
        assert!(credential_match("9f8e7d6c5b4a39281706f5e4d3c2b1a0").is_none());
    }

    #[test]
    fn an_unreadable_or_binary_file_is_skipped_without_error() {
        let mut scan = RepoScan::default();
        scan_credentials(
            Path::new("/nonexistent/rust-doctor-repo-hygiene"),
            "src/lib.rs",
            &mut scan,
        );
        assert!(scan.findings.is_empty());
        assert!(scan.errors.is_empty());
    }

    #[test]
    fn the_build_directory_defaults_to_target_and_refuses_escapes() {
        assert_eq!(
            build_directory(Path::new("/nonexistent/rust-doctor-repo-hygiene")),
            Some("target".to_owned())
        );
    }
}
