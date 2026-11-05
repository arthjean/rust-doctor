use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::internal_error::InternalError;
use crate::execution::RunDeadline;
use crate::git::{
    GitCall, GitFailure, GitView, run_git, run_git_status, run_git_with_index,
    view_arguments,
};
use crate::policy::{
    ActiveRules, Producer,
    PolicyPlan, REPO_HARDCODED_CREDENTIAL, REPO_IGNORED_LOCKFILE, REPO_SECRET_IN_BUILD_STEP,
    REPO_TRACKED_SECRET_FILE, REPO_UNIGNORED_BUILD_OUTPUT,
    RuleDefinition,
};
use crate::report::DiagnosticContext;
use crate::source_text::SourceSpan;
use crate::workspace_path;

mod credentials;
mod workflows;

pub(crate) use credentials::{
    REDACTED, REDACTED_MIN_CHARS, carries_credential, credential_ranges, key_continues,
    mask_credentials, masked, open_body_end,
};

const STAGE: &str = "repo";

const MAX_TRACKED_PATHS: usize = 50_000;

const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoFinding {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: String,
    pub(crate) path: String,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) context: Option<DiagnosticContext>,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IgnoreProbe {
    Ignored,
    NotIgnored,
    Unavailable,
}

const LOCKFILE: &str = "Cargo.lock";

pub(crate) fn inspect(
    workspace_root: &Path,
    view: Option<&GitView>,
    plan: &PolicyPlan,
    binary: bool,
    deadline: Option<RunDeadline>,
) -> RepoScan {
    let lockfile_present = match view {
        None => workspace_root.join(".git").exists() && workspace_root.join(LOCKFILE).is_file(),
        Some(view) => {
            workspace_root == view.work_tree
                && view
                    .working_path(workspace_root)
                    .is_some_and(|working| working.join(LOCKFILE).is_file())
        }
    };
    inspect_with(
        workspace_root,
        view,
        plan,
        (binary, lockfile_present),
        |call| match view {
            Some(view) if !view.index.is_file() => Err(call.failure.error(call.stage)),
            Some(view) => {
                run_git_with_index(Path::new("git"), workspace_root, call, &view.index, deadline)
            }
            None => run_git(Path::new("git"), workspace_root, call, deadline),
        },
        |path| match path.strip_suffix('/') {
            Some(directory) => probe_ignore(workspace_root, view, directory, deadline),
            None => ask_git_check_ignore(workspace_root, view, path, deadline),
        },
    )
}

fn inspect_with(
    workspace_root: &Path,
    view: Option<&GitView>,
    plan: &PolicyPlan,
    (binary, lockfile_present): (bool, bool),
    mut run: impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
    mut probe: impl FnMut(&str) -> IgnoreProbe,
) -> RepoScan {
    let active = ActiveRules::of(plan, Producer::Repo);
    let mut scan = RepoScan::default();
    if !active.any() {
        return scan;
    }

    let (paths, truncated, in_repository) = match enumerate_tracked(workspace_root, view, &mut run) {
        Ok(output) => {
            let (paths, truncated) = tracked_paths(&output);
            (paths, truncated, true)
        }
        Err(None) => {
            let (paths, truncated) = present_paths(workspace_root);
            (paths, truncated, false)
        }
        Err(Some(error)) => {
            scan.errors.push(error);
            return scan;
        }
    };
    if truncated {
        scan.errors.push(RepoError {
            code: "repo-enumeration-truncated",
            message: "Repository enumeration stopped at 50000 files; results are partial.",
        });
    }

    for path in &paths {
        inspect_tracked(workspace_root, path, in_repository, &active, &mut scan);
    }
    if !in_repository {
        return scan;
    }

    if active.on(&REPO_IGNORED_LOCKFILE) && binary {
        scan.findings.extend(ignored_lockfile(lockfile_present, &paths, &mut probe));
    }

    if active.on(&REPO_UNIGNORED_BUILD_OUTPUT)
        && let Some(directory) = build_directory(workspace_root)
        && probe(&format!("{directory}/")) == IgnoreProbe::NotIgnored
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

fn inspect_tracked(
    workspace_root: &Path,
    path: &str,
    in_repository: bool,
    active: &ActiveRules,
    scan: &mut RepoScan,
) {
    if active.on(&REPO_TRACKED_SECRET_FILE)
        && !is_test_material(path)
        && (is_secret_file_name(file_name(path))
            || (workflows::is_cargo_credentials(path)
                && read_bounded(&workspace_root.join(path), MAX_FILE_BYTES)
                    .is_some_and(|source| workflows::holds_registry_token(&source))))
        && let Some(published) = workspace_path::normalize_changed(path)
    {
        scan.findings.push(RepoFinding {
            definition: &REPO_TRACKED_SECRET_FILE,
            message: if in_repository {
                format!("the repository tracks {published}, whose name marks it as secret-bearing")
            } else {
                format!("the workspace holds {published}, whose name marks it as secret-bearing")
            },
            path: published,
            span: None,
            context: path_context(path),
        });
    }
    if active.on(&REPO_HARDCODED_CREDENTIAL) {
        scan_credentials(workspace_root, path, scan);
    }
    if active.on(&REPO_SECRET_IN_BUILD_STEP) && workflows::is_workflow(path) {
        workflows::scan_workflow(workspace_root, path, scan);
    }
}

fn enumerate_tracked(
    workspace_root: &Path,
    view: Option<&GitView>,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<Vec<u8>, Option<RepoError>> {
    run(&GitCall {
        arguments: view_arguments(workspace_root, view, ["ls-files", "-z"]),
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
    })
    .map_err(|error| match error.code {
        "repo-not-a-repository" if view.is_none() => None,
        "repo-not-a-repository" => Some(RepoError {
            code: "repo-snapshot-unreadable",
            message: "Repository hygiene could not read the snapshot through git.",
        }),
        "git-unavailable" => Some(RepoError {
            code: "git-unavailable",
            message: "Repository hygiene skipped: git was not available.",
        }),
        _ => Some(RepoError {
            code: "repo-enumeration-truncated",
            message:
                "Repository enumeration exceeded the supported size; repository hygiene was skipped.",
        }),
    })
}

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

fn present_paths(root: &Path) -> (Vec<String>, bool) {
    let build = build_directory(root).unwrap_or_else(|| "target".to_owned());
    let mut paths = Vec::new();
    let mut pending = vec![String::new()];
    while let Some(directory) = pending.pop() {
        let absolute = root.join(&directory);
        if absolute.join("CACHEDIR.TAG").is_file() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&absolute) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort_unstable();
        for name in names {
            let relative = if directory.is_empty() { name } else { format!("{directory}/{name}") };
            if relative == ".git" || relative == build {
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(root.join(&relative)) else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(relative);
            } else if metadata.is_file() && workspace_path::normalize_changed(&relative).is_some() {
                if paths.len() == MAX_TRACKED_PATHS {
                    return (paths, true);
                }
                paths.push(relative);
            }
        }
    }
    paths.sort_unstable();
    (paths, false)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

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

fn is_test_material(path: &str) -> bool {
    path.split('/')
        .any(|segment| matches!(segment, "tests" | "fixtures" | "testdata"))
}

fn path_context(path: &str) -> Option<DiagnosticContext> {
    path.split('/').find_map(|segment| match segment {
        "tests" | "fixtures" | "testdata" => Some(DiagnosticContext::Tests),
        "examples" => Some(DiagnosticContext::Example),
        "benches" => Some(DiagnosticContext::Benchmark),
        _ => None,
    })
}

fn scan_credentials(workspace_root: &Path, path: &str, scan: &mut RepoScan) {
    let Some(source) = read_bounded(&workspace_root.join(path), MAX_FILE_BYTES) else {
        return;
    };
    let Some(published) = workspace_path::normalize_changed(path) else {
        return;
    };
    let context = path_context(path);
    let lines: Vec<&str> = source.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let following = lines.get(index + 1..).unwrap_or_default();
        let Some((name, range)) = credentials::first_credential(line, following) else {
            continue;
        };
        let line_number = index + 1;
        let columns = |offset: usize| line.get(..offset).unwrap_or_default().chars().count() + 1;
        let (column_start, column_end) = (columns(range.0), columns(range.1));
        scan.findings.push(RepoFinding {
            definition: &REPO_HARDCODED_CREDENTIAL,
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

fn ignored_lockfile(
    lockfile_present: bool,
    tracked: &[String],
    probe: &mut impl FnMut(&str) -> IgnoreProbe,
) -> Option<RepoFinding> {
    (lockfile_present
        && !tracked.iter().any(|path| path == LOCKFILE)
        && probe(LOCKFILE) == IgnoreProbe::Ignored)
        .then(|| RepoFinding {
            definition: &REPO_IGNORED_LOCKFILE,
            message: format!("git ignores {LOCKFILE}, so this binary's resolved versions are never committed"),
            path: LOCKFILE.to_owned(),
            span: None,
            context: None,
        })
}

fn read_bounded(file: &Path, limit: u64) -> Option<String> {
    let metadata = fs::symlink_metadata(file).ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    String::from_utf8(fs::read(file).ok()?).ok()
}

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

fn probe_ignore(
    workspace_root: &Path,
    view: Option<&GitView>,
    directory: &str,
    deadline: Option<RunDeadline>,
) -> IgnoreProbe {
    match ask_git_check_ignore(workspace_root, view, &format!("{directory}/"), deadline) {
        IgnoreProbe::Unavailable => ask_git_check_ignore(workspace_root, view, directory, deadline),
        answered => answered,
    }
}

fn ask_git_check_ignore(
    workspace_root: &Path,
    view: Option<&GitView>,
    path: &str,
    deadline: Option<RunDeadline>,
) -> IgnoreProbe {
    let arguments = view_arguments(workspace_root, view, ["check-ignore", "--quiet", "--", path]);
    let index = view.map(|view| view.index.as_path());
    match run_git_status(Path::new("git"), workspace_root, &arguments, index, deadline) {
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

        fs::write(workspace.join(".gitignore"), "/target/\n").expect("gitignore should write");
        symlink(&elsewhere, workspace.join("target")).expect("the symlink should be created");
        assert_eq!(
            probe_ignore(&workspace, None, "target", None),
            IgnoreProbe::NotIgnored,
            "a directory-only pattern does not cover the link that stands in for it"
        );

        fs::write(workspace.join(".gitignore"), "/target\n").expect("gitignore should write");
        assert_eq!(probe_ignore(&workspace, None, "target", None), IgnoreProbe::Ignored);

        fs::remove_dir_all(&scratch).expect("the scratch tree should be removable");
    }

    #[test]
    fn a_workspace_outside_git_is_judged_on_the_files_it_holds() {
        let workspace = std::env::temp_dir().join(format!("rust-doctor-outside-git-{}", std::process::id()));
        let _ = fs::remove_dir_all(&workspace);
        for (path, contents) in [
            (".env", "API=1\n"),
            (".github/workflows/release.yml", "on: push\njobs:\n  publish:\n    runs-on: ubuntu-latest\n    steps:\n      - env:\n          CARGO_REGISTRY_TOKEN: ${{ secrets.TOKEN }}\n        run: cargo publish\n"),
            ("src/lib.rs", "pub fn shipped() {}\n"),
            ("target/CACHEDIR.TAG", "Signature: 8a477f597d28d172789f06886806bc55\n"),
            ("target/debug/leftover.pem", "not scanned\n"),
            ("nested/build/CACHEDIR.TAG", "Signature: 8a477f597d28d172789f06886806bc55\n"),
            ("nested/build/server.key", "not scanned\n"),
        ] {
            let file = workspace.join(path);
            fs::create_dir_all(file.parent().expect("a fixture path has a parent")).expect("the workspace should be writable");
            fs::write(file, contents).expect("the fixture should write");
        }
        let scan = inspect_with(
            &workspace,
            None,
            &plan(),
            (true, true),
            |call| Err(call.failure.error(call.stage)),
            |_| IgnoreProbe::NotIgnored,
        );
        let found: Vec<(&str, &str)> = scan
            .findings
            .iter()
            .map(|finding| (finding.definition.id, finding.path.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("rust_doctor::repo::tracked_secret_file", ".env"),
                ("rust_doctor::repo::secret_in_build_step", ".github/workflows/release.yml"),
            ],
            "{:?}",
            scan.findings
        );
        assert!(scan.findings[0].message.contains("the workspace holds .env"));
        assert!(scan.errors.is_empty());

        let git_missing = inspect_with(
            &workspace,
            None,
            &plan(),
            (false, false),
            |_| Err(InternalError::new("scope", "git-unavailable", "Git could not be started.")),
            |_| IgnoreProbe::Unavailable,
        );
        assert!(git_missing.findings.is_empty(), "an absent git is an error, not a bare tree");
        assert_eq!(git_missing.errors.len(), 1);
        fs::remove_dir_all(&workspace).expect("the scratch tree should be removable");
    }

    #[test]
    fn an_ignored_lockfile_abstains_without_an_answer_from_git() {
        let workspace = std::env::temp_dir().join(format!("rust-doctor-ignored-lockfile-{}", std::process::id()));
        let _ = fs::remove_dir_all(&workspace);
        fs::create_dir_all(workspace.join(".git")).expect("the workspace should be writable");
        fs::write(workspace.join(LOCKFILE), "version = 4\n").expect("the lockfile should write");
        let reported = |scan: RepoScan| {
            scan.findings
                .iter()
                .filter(|finding| finding.definition.id == REPO_IGNORED_LOCKFILE.id)
                .count()
        };
        let answered = |probe: IgnoreProbe| {
            inspect_with(&workspace, None, &plan(), (true, true), |_| Ok(ls_files_output(&["Cargo.toml"])), |_| probe)
        };
        assert_eq!(reported(answered(IgnoreProbe::Ignored)), 1);
        assert_eq!(reported(answered(IgnoreProbe::NotIgnored)), 0);
        assert_eq!(reported(answered(IgnoreProbe::Unavailable)), 0);
        let outside_git = inspect_with(
            &workspace,
            None,
            &plan(),
            (true, true),
            |call| Err(call.failure.error(call.stage)),
            |_| IgnoreProbe::Ignored,
        );
        assert_eq!(reported(outside_git), 0);
        fs::remove_dir_all(&workspace).expect("the scratch tree should be removable");
    }

    #[test]
    fn an_absent_git_degrades_to_a_recorded_error() {
        let scan = inspect_with(
            Path::new("."),
            None,
            &plan(),
            (false, false),
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
            None,
            &plan(),
            (false, false),
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
            None,
            &plan(),
            (false, false),
            |_| Ok(ls_files_output(&borrowed)),
            |_| IgnoreProbe::Unavailable,
        );
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(scan.errors[0].code, "repo-enumeration-truncated");
        assert!(scan.errors[0].message.contains("50000"));
        assert_eq!(scan.findings.len(), 1);
        assert_eq!(
            scan.findings[0].definition.id,
            "rust_doctor::repo::tracked_secret_file"
        );
    }

    #[test]
    fn a_secret_file_outside_shipped_code_carries_its_context() {
        let scan = inspect_with(
            Path::new("/nonexistent/rust-doctor-repo-hygiene"),
            None,
            &plan(),
            (false, false),
            |_| Ok(ls_files_output(&["examples/certificate.pem", "deploy/server.key"])),
            |_| IgnoreProbe::Unavailable,
        );
        let contexts: Vec<_> = scan
            .findings
            .iter()
            .map(|finding| (finding.path.as_str(), finding.context))
            .collect();
        assert_eq!(
            contexts,
            [
                ("examples/certificate.pem", Some(DiagnosticContext::Example)),
                ("deploy/server.key", None),
            ]
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
