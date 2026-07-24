//! Managed CI scaffolding with marker-scoped, atomic mutations.

use crate::cli::{CiConfigArgs, CiInstallArgs, CiProvider, CiScope, CiUpgradeArgs, FailOn};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const START_MARKER: &str = "# rust-doctor:managed:start";
const END_MARKER: &str = "# rust-doctor:managed:end";
const SETTINGS_PREFIX: &str = "# rust-doctor:settings ";
const CHECKOUT_SHA: &str = "11d5960a326750d5838078e36cf38b85af677262";

#[derive(thiserror::Error, Debug)]
pub enum CiError {
    #[error("CI setup requires a Git repository: {0}")]
    Git(String),
    #[error("CI workflow conflict at '{}': {message}", path.display())]
    Conflict { path: PathBuf, message: String },
    #[error("failed to mutate CI workflow '{}': {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid Rust Doctor-owned workflow metadata: {0}")]
    Metadata(String),
    #[error("CI pull-request operation failed before completion: {0}")]
    Remote(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct CiSettings {
    scope: CiScope,
    blocking: String,
    comment: bool,
    review_comments: bool,
    commit_status: bool,
    sarif: bool,
    action_major: String,
}

impl CiSettings {
    fn from_install(arguments: &CiInstallArgs) -> Self {
        Self {
            scope: arguments.scope,
            blocking: blocking(arguments.blocking).to_string(),
            comment: arguments.comment,
            review_comments: arguments.review_comments,
            commit_status: arguments.commit_status,
            sarif: arguments.sarif,
            action_major: arguments.version.clone(),
        }
    }

    fn apply_config(&mut self, arguments: &CiConfigArgs) {
        if let Some(scope) = arguments.scope {
            self.scope = scope;
        }
        if let Some(level) = arguments.blocking {
            self.blocking = blocking(level).to_string();
        }
        if let Some(value) = arguments.comment {
            self.comment = value;
        }
        if let Some(value) = arguments.review_comments {
            self.review_comments = value;
        }
        if let Some(value) = arguments.commit_status {
            self.commit_status = value;
        }
        if let Some(value) = arguments.sarif {
            self.sarif = value;
        }
        if let Some(version) = &arguments.version {
            self.action_major.clone_from(version);
        }
    }

    fn validate(&self) -> Result<(), CiError> {
        if !matches!(
            self.blocking.as_str(),
            "error" | "warning" | "info" | "none"
        ) {
            return Err(CiError::Metadata(
                "blocking must be error, warning, info, or none".to_string(),
            ));
        }
        let Some(major) = self.action_major.strip_prefix('v') else {
            return Err(CiError::Metadata(
                "action version must be a major reference such as v1".to_string(),
            ));
        };
        if major.is_empty()
            || major.starts_with('0')
            || !major.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(CiError::Metadata(
                "action version must be a non-zero major reference such as v1".to_string(),
            ));
        }
        Ok(())
    }
}

pub fn install(arguments: &CiInstallArgs) -> Result<String, CiError> {
    let root = git_root(&arguments.directory)?;
    let path = workflow_path(&root, arguments.provider);
    let before = read_optional(&path)?;
    let settings = CiSettings::from_install(arguments);
    let managed = render_managed(arguments.provider, &settings)?;
    let after = merge_install(arguments.provider, &path, before.as_deref(), &managed)?;

    if arguments.dry_run {
        return Ok(render_diff(&path, before.as_deref(), &after));
    }
    if arguments.pr {
        return install_through_pull_request(
            &root,
            &path,
            before.as_deref(),
            &after,
            &settings.action_major,
            arguments.issue,
        );
    }
    atomic_write(&root, &path, before.as_deref(), &after)?;
    Ok(format!("Installed Rust Doctor CI in {}", path.display()))
}

pub fn configure(arguments: &CiConfigArgs) -> Result<String, CiError> {
    let root = git_root(&arguments.directory)?;
    let path = workflow_path(&root, arguments.provider);
    let before = read_owned(&path)?;
    let mut settings = parse_settings(&before)?;
    settings.apply_config(arguments);
    let managed = render_managed(arguments.provider, &settings)?;
    let after = replace_managed(&path, &before, &managed)?;
    if arguments.dry_run {
        return Ok(render_diff(&path, Some(&before), &after));
    }
    atomic_write(&root, &path, Some(&before), &after)?;
    Ok(format!("Configured Rust Doctor CI in {}", path.display()))
}

pub fn upgrade(arguments: &CiUpgradeArgs) -> Result<String, CiError> {
    let root = git_root(&arguments.directory)?;
    let path = workflow_path(&root, arguments.provider);
    let before = read_owned(&path)?;
    let mut settings = parse_settings(&before)?;
    settings.action_major.clone_from(&arguments.version);
    let managed = render_managed(arguments.provider, &settings)?;
    let after = replace_managed(&path, &before, &managed)?;
    if arguments.dry_run {
        return Ok(render_diff(&path, Some(&before), &after));
    }
    atomic_write(&root, &path, Some(&before), &after)?;
    Ok(format!(
        "Upgraded Rust Doctor CI to {} in {}",
        arguments.version,
        path.display()
    ))
}

fn git_root(directory: &Path) -> Result<PathBuf, CiError> {
    let directory = directory.canonicalize().map_err(|error| {
        CiError::Git(format!(
            "failed to resolve '{}': {error}",
            directory.display()
        ))
    })?;
    let output = Command::new("git")
        .args(["-C"])
        .arg(&directory)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| CiError::Git(error.to_string()))?;
    if !output.status.success() {
        return Err(CiError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    root.canonicalize()
        .map_err(|error| CiError::Git(format!("failed to resolve Git root: {error}")))
}

fn workflow_path(root: &Path, provider: CiProvider) -> PathBuf {
    match provider {
        CiProvider::Github => root.join(".github/workflows/rust-doctor.yml"),
        CiProvider::Gitlab => root.join(".gitlab-ci.yml"),
    }
}

fn read_optional(path: &Path) -> Result<Option<String>, CiError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(CiError::Conflict {
            path: path.to_path_buf(),
            message: "refusing to mutate a symbolic link".to_string(),
        }),
        Ok(metadata) if !metadata.is_file() => Err(CiError::Conflict {
            path: path.to_path_buf(),
            message: "destination is not a regular file".to_string(),
        }),
        Ok(_) => fs::read_to_string(path)
            .map(Some)
            .map_err(|source| CiError::Io {
                path: path.to_path_buf(),
                source,
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(CiError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn read_owned(path: &Path) -> Result<String, CiError> {
    let content = read_optional(path)?.ok_or_else(|| CiError::Conflict {
        path: path.to_path_buf(),
        message: "managed workflow is not installed".to_string(),
    })?;
    marker_range(&content).ok_or_else(|| CiError::Conflict {
        path: path.to_path_buf(),
        message: "file has no Rust Doctor ownership markers".to_string(),
    })?;
    Ok(content)
}

fn merge_install(
    provider: CiProvider,
    path: &Path,
    before: Option<&str>,
    managed: &str,
) -> Result<String, CiError> {
    let Some(before) = before else {
        return Ok(managed.to_string());
    };
    if marker_range(before).is_some() {
        return replace_managed(path, before, managed);
    }
    if before.trim().is_empty() {
        return Ok(managed.to_string());
    }
    match provider {
        CiProvider::Github => Err(CiError::Conflict {
            path: path.to_path_buf(),
            message: "existing workflow is not Rust Doctor-owned; choose another file or add ownership markers"
                .to_string(),
        }),
        CiProvider::Gitlab => Ok(format!("{}\n{}", before.trim_end(), managed)),
    }
}

fn replace_managed(path: &Path, before: &str, managed: &str) -> Result<String, CiError> {
    let (start, end) = marker_range(before).ok_or_else(|| CiError::Conflict {
        path: path.to_path_buf(),
        message: "Rust Doctor ownership markers are missing or malformed".to_string(),
    })?;
    Ok(format!("{}{}{}", &before[..start], managed, &before[end..]))
}

fn marker_range(content: &str) -> Option<(usize, usize)> {
    let start = content.find(START_MARKER)?;
    let relative_end = content[start..].find(END_MARKER)?;
    let marker_end = start + relative_end + END_MARKER.len();
    let end = content[marker_end..]
        .strip_prefix('\n')
        .map_or(marker_end, |_| marker_end + 1);
    if content[marker_end..].contains(END_MARKER) || content[..start].contains(START_MARKER) {
        return None;
    }
    Some((start, end))
}

fn parse_settings(content: &str) -> Result<CiSettings, CiError> {
    let line = content
        .lines()
        .find_map(|line| line.strip_prefix(SETTINGS_PREFIX))
        .ok_or_else(|| CiError::Metadata("settings marker is missing".to_string()))?;
    let settings: CiSettings =
        serde_json::from_str(line).map_err(|error| CiError::Metadata(error.to_string()))?;
    settings.validate()?;
    Ok(settings)
}

fn render_managed(provider: CiProvider, settings: &CiSettings) -> Result<String, CiError> {
    settings.validate()?;
    let metadata =
        serde_json::to_string(settings).map_err(|error| CiError::Metadata(error.to_string()))?;
    let body = match provider {
        CiProvider::Github => render_github(settings),
        CiProvider::Gitlab => render_gitlab(settings),
    };
    Ok(format!(
        "{START_MARKER}\n{SETTINGS_PREFIX}{metadata}\n{body}{END_MARKER}\n"
    ))
}

fn render_github(settings: &CiSettings) -> String {
    let mut permissions = String::from("      contents: read\n");
    if settings.comment || settings.review_comments {
        permissions.push_str("      pull-requests: write\n");
    }
    if settings.commit_status {
        permissions.push_str("      statuses: write\n");
    }
    if settings.sarif {
        permissions.push_str("      security-events: write\n");
    }
    format!(
        "name: Rust Doctor\n\non:\n  pull_request:\n  push:\n    branches: [main]\n\npermissions:\n  contents: read\n\njobs:\n  rust-doctor:\n    runs-on: ubuntu-latest\n    permissions:\n{permissions}    steps:\n      - uses: actions/checkout@{CHECKOUT_SHA}\n        with:\n          fetch-depth: 0\n      - uses: arthjean/rust-doctor@{action_major}\n        with:\n          directory: .\n          scope: {scope}\n          blocking: {blocking}\n          require-complete: true\n          comment: {comment}\n          review-comments: {review_comments}\n          commit-status: {commit_status}\n          sarif: {sarif}\n          token: ${{{{ secrets.GITHUB_TOKEN }}}}\n",
        action_major = settings.action_major,
        scope = settings.scope,
        blocking = settings.blocking,
        comment = settings.comment,
        review_comments = settings.review_comments,
        commit_status = settings.commit_status,
        sarif = settings.sarif,
    )
}

fn render_gitlab(settings: &CiSettings) -> String {
    let (merge_request_arguments, fallback_arguments) = match settings.scope {
        CiScope::Full => ("--scope full", "--scope full"),
        CiScope::Changed => (
            "--scope changed --base \"$CI_MERGE_REQUEST_DIFF_BASE_SHA\"",
            "--scope full",
        ),
        CiScope::Baseline => (
            "--baseline --base \"$CI_MERGE_REQUEST_DIFF_BASE_SHA\"",
            "--scope full",
        ),
        CiScope::Staged => ("--staged", "--staged"),
    };
    format!(
        "rust-doctor:\n  image: rust:1.97\n  stage: test\n  variables:\n    CARGO_HOME: \"$CI_PROJECT_DIR/.cargo\"\n  cache:\n    key: \"rust-doctor-{action_major}\"\n    paths: [.cargo/bin, .rust-doctor-cache.json]\n  before_script:\n    - cargo install rust-doctor --locked\n  script:\n    - |\n      if [[ \"$CI_PIPELINE_SOURCE\" == \"merge_request_event\" && -n \"${{CI_MERGE_REQUEST_DIFF_BASE_SHA:-}}\" ]]; then\n        rust-doctor . {merge_request_arguments} --blocking {blocking} --require-complete\n      else\n        rust-doctor . {fallback_arguments} --blocking {blocking} --require-complete\n      fi\n  rules:\n    - if: '$CI_PIPELINE_SOURCE == \"merge_request_event\"'\n    - if: '$CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH'\n",
        action_major = settings.action_major,
        blocking = settings.blocking,
    )
}

fn atomic_write(
    root: &Path,
    path: &Path,
    before: Option<&str>,
    content: &str,
) -> Result<(), CiError> {
    ensure_contained(root, path)?;
    let current = read_optional(path)?;
    if current.as_deref() != before {
        return Err(CiError::Conflict {
            path: path.to_path_buf(),
            message: "file changed after planning; refusing to overwrite it".to_string(),
        });
    }
    let parent = path.parent().ok_or_else(|| CiError::Conflict {
        path: path.to_path_buf(),
        message: "workflow path has no parent".to_string(),
    })?;
    fs::create_dir_all(parent).map_err(|source| CiError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    ensure_contained(root, path)?;
    let temp = parent.join(format!(".rust-doctor-ci.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|source| CiError::Io {
            path: temp.clone(),
            source,
        })?;
    if let Err(source) = file
        .write_all(content.as_bytes())
        .and_then(|()| file.sync_all())
    {
        let _ = fs::remove_file(&temp);
        return Err(CiError::Io { path: temp, source });
    }
    drop(file);
    fs::rename(&temp, path).map_err(|source| {
        let _ = fs::remove_file(&temp);
        CiError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn ensure_contained(root: &Path, path: &Path) -> Result<(), CiError> {
    if !path.starts_with(root) {
        return Err(CiError::Conflict {
            path: path.to_path_buf(),
            message: "workflow path escapes the Git root".to_string(),
        });
    }
    let mut current = path.parent();
    while let Some(candidate) = current {
        if candidate == root {
            break;
        }
        match fs::symlink_metadata(candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CiError::Conflict {
                    path: candidate.to_path_buf(),
                    message: "workflow parent is a symbolic link".to_string(),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(CiError::Io {
                    path: candidate.to_path_buf(),
                    source,
                });
            }
        }
        current = candidate.parent();
    }
    Ok(())
}

fn render_diff(path: &Path, before: Option<&str>, after: &str) -> String {
    let before = before.unwrap_or("");
    if before == after {
        return format!("No change to {}\n", path.display());
    }
    let mut output = format!("--- {}\n+++ {}\n", path.display(), path.display());
    for line in before.lines() {
        output.push('-');
        output.push_str(line);
        output.push('\n');
    }
    for line in after.lines() {
        output.push('+');
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn install_through_pull_request(
    root: &Path,
    path: &Path,
    before: Option<&str>,
    after: &str,
    action_major: &str,
    issue: Option<u64>,
) -> Result<String, CiError> {
    install_through_pull_request_with(
        root,
        path,
        before,
        after,
        action_major,
        issue,
        &ProcessRunner,
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "the pull request install keeps its transactional steps and rollback boundary visible in one function"
)]
fn install_through_pull_request_with(
    root: &Path,
    path: &Path,
    before: Option<&str>,
    after: &str,
    action_major: &str,
    issue: Option<u64>,
    runner: &dyn CiCommandRunner,
) -> Result<String, CiError> {
    require_clean_worktree(root)?;
    runner.run(root, "gh", &[OsStr::new("auth"), OsStr::new("status")])?;
    let push_target = remote_push_target(root)?;
    let original = checkout_state(root)?;
    let branch = format!("rust-doctor/ci-{action_major}");
    let remote_ref = format!("refs/heads/{branch}");
    if runner.success(
        root,
        "git",
        &[
            OsStr::new("show-ref"),
            OsStr::new("--verify"),
            OsStr::new(&format!("refs/heads/{branch}")),
        ],
    ) {
        return Err(CiError::Remote(format!(
            "local branch '{branch}' already exists"
        )));
    }
    if remote_ref_target(root, &push_target, &remote_ref)?.is_some() {
        return Err(CiError::Remote(format!(
            "remote branch '{branch}' already exists"
        )));
    }
    runner.run(
        root,
        "git",
        &[OsStr::new("switch"), OsStr::new("-c"), OsStr::new(&branch)],
    )?;
    let relative = path
        .strip_prefix(root)
        .map_err(|error| CiError::Remote(error.to_string()))?;
    let mut transaction_commit = None;
    let operation = (|| {
        atomic_write(root, path, before, after)?;
        runner.run(root, "git", &[OsStr::new("add"), relative.as_os_str()])?;
        runner.run(
            root,
            "git",
            &[
                OsStr::new("commit"),
                OsStr::new("-m"),
                OsStr::new("ci: install rust-doctor workflow"),
            ],
        )?;
        transaction_commit = Some(command_stdout(
            root,
            "git",
            &["rev-parse", "--verify", "HEAD"],
        )?);
        let lease = format!("--force-with-lease={remote_ref}:");
        runner.run(
            root,
            "git",
            &[
                OsStr::new("push"),
                OsStr::new(&lease),
                OsStr::new(&push_target),
                OsStr::new(&branch),
            ],
        )?;
        let mut body = String::from(
            "Install the managed Rust Doctor workflow with baseline scans, explicit completeness, and least-privilege reporting permissions.",
        );
        if let Some(number) = issue {
            body.push_str("\n\nRefs #");
            body.push_str(&number.to_string());
        }
        runner.run(
            root,
            "gh",
            &[
                OsStr::new("pr"),
                OsStr::new("create"),
                OsStr::new("--title"),
                OsStr::new("ci: install Rust Doctor workflow"),
                OsStr::new("--body"),
                OsStr::new(&body),
                OsStr::new("--head"),
                OsStr::new(&branch),
            ],
        )
    })();
    match operation {
        Ok(()) => Ok(format!("Created CI pull request from branch {branch}")),
        Err(error) => {
            let rollback = rollback_pull_request_install(
                root,
                path,
                before,
                after,
                relative,
                &original,
                &branch,
                &push_target,
                transaction_commit.as_deref(),
                runner,
            );
            let detail = if rollback.is_empty() {
                "local and remote changes were rolled back".to_string()
            } else {
                rollback.join("; ")
            };
            Err(CiError::Remote(format!("{error}; {detail}")))
        }
    }
}

#[derive(Debug)]
struct CheckoutState {
    branch: Option<String>,
    head: String,
}

fn checkout_state(root: &Path) -> Result<CheckoutState, CiError> {
    let head = command_stdout(root, "git", &["rev-parse", "--verify", "HEAD"])?;
    let branch = Command::new("git")
        .current_dir(root)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .map_err(|error| CiError::Remote(format!("failed to inspect current branch: {error}")))
        .and_then(|output| {
            if output.status.success() {
                Ok(Some(
                    String::from_utf8_lossy(&output.stdout).trim().to_string(),
                ))
            } else if output.status.code() == Some(1) {
                Ok(None)
            } else {
                Err(CiError::Remote(
                    String::from_utf8_lossy(&output.stderr).trim().to_string(),
                ))
            }
        })?;
    Ok(CheckoutState { branch, head })
}

#[expect(
    clippy::too_many_arguments,
    reason = "rollback receives the exact transaction state needed to restore local and remote Git state"
)]
fn rollback_pull_request_install(
    root: &Path,
    path: &Path,
    before: Option<&str>,
    after: &str,
    relative: &Path,
    original: &CheckoutState,
    branch: &str,
    push_target: &str,
    transaction_commit: Option<&str>,
    runner: &dyn CiCommandRunner,
) -> Vec<String> {
    let mut failures = Vec::new();
    if let Err(error) = runner.run(
        root,
        "git",
        &[
            OsStr::new("reset"),
            OsStr::new(&original.head),
            OsStr::new("--"),
            relative.as_os_str(),
        ],
    ) {
        failures.push(format!("failed to reset the workflow index: {error}"));
    }
    if let Err(error) = restore_workflow(root, path, before, after) {
        failures.push(format!("failed to restore the workflow file: {error}"));
    }
    let switch_arguments: Vec<&OsStr> = original.branch.as_ref().map_or_else(
        || {
            vec![
                OsStr::new("switch"),
                OsStr::new("--detach"),
                OsStr::new(&original.head),
            ]
        },
        |branch| vec![OsStr::new("switch"), OsStr::new(branch)],
    );
    if let Err(error) = runner.run(root, "git", &switch_arguments) {
        failures.push(format!("failed to restore the original checkout: {error}"));
    } else if let Err(error) = runner.run(
        root,
        "git",
        &[OsStr::new("branch"), OsStr::new("-D"), OsStr::new(branch)],
    ) {
        failures.push(format!("failed to remove local branch '{branch}': {error}"));
    }
    if let Some(transaction_commit) = transaction_commit {
        let remote_ref = format!("refs/heads/{branch}");
        match remote_ref_target(root, push_target, &remote_ref) {
            Ok(Some(remote_commit)) if remote_commit == transaction_commit => {
                let lease = format!("--force-with-lease={remote_ref}:{transaction_commit}");
                if let Err(error) = runner.run(
                    root,
                    "git",
                    &[
                        OsStr::new("push"),
                        OsStr::new(&lease),
                        OsStr::new(push_target),
                        OsStr::new("--delete"),
                        OsStr::new(branch),
                    ],
                ) {
                    failures.push(format!(
                        "remote branch '{branch}' may remain and must be removed manually: {error}"
                    ));
                }
            }
            Ok(Some(remote_commit)) => failures.push(format!(
                "remote branch '{branch}' now points to unrelated commit {remote_commit}; it was preserved"
            )),
            Ok(None) => {}
            Err(error) => failures.push(format!(
                "remote branch '{branch}' could not be verified after rollback and may require manual removal: {error}"
            )),
        }
    }
    failures
}

fn remote_push_target(root: &Path) -> Result<String, CiError> {
    let targets = command_stdout(
        root,
        "git",
        &["remote", "get-url", "--push", "--all", "origin"],
    )?;
    let mut targets = targets.lines().filter(|target| !target.is_empty());
    let target = targets
        .next()
        .ok_or_else(|| CiError::Remote("origin has no push target".to_string()))?;
    if targets.next().is_some() {
        return Err(CiError::Remote(
            "origin defines multiple push targets; refusing transactional install".to_string(),
        ));
    }
    Ok(target.to_string())
}

fn remote_ref_target(
    root: &Path,
    push_target: &str,
    remote_ref: &str,
) -> Result<Option<String>, CiError> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["ls-remote", "--heads", push_target, remote_ref])
        .output()
        .map_err(|error| CiError::Remote(format!("failed to inspect remote branch: {error}")))?;
    if !output.status.success() {
        return Err(CiError::Remote(format!(
            "failed to inspect remote branch: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut targets = stdout
        .lines()
        .filter_map(|line| line.split_whitespace().next());
    let target = targets.next().map(str::to_string);
    if targets.next().is_some() {
        return Err(CiError::Remote(format!(
            "remote branch query for '{remote_ref}' returned multiple refs"
        )));
    }
    Ok(target)
}

fn restore_workflow(
    root: &Path,
    path: &Path,
    before: Option<&str>,
    after: &str,
) -> Result<(), CiError> {
    let current = read_optional(path)?;
    if current.as_deref() == before {
        return Ok(());
    }
    if current.as_deref() != Some(after) {
        return Err(CiError::Conflict {
            path: path.to_path_buf(),
            message: "workflow changed during CI transaction; preserving concurrent local content"
                .to_string(),
        });
    }
    if let Some(content) = before {
        atomic_write(root, path, current.as_deref(), content)
    } else {
        ensure_contained(root, path)?;
        fs::remove_file(path).map_err(|source| CiError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

fn require_clean_worktree(root: &Path) -> Result<(), CiError> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|error| CiError::Remote(error.to_string()))?;
    if !output.status.success() {
        return Err(CiError::Remote(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    if !output.stdout.is_empty() {
        return Err(CiError::Remote(
            "--pr requires a clean worktree so existing local work cannot be committed or overwritten"
                .to_string(),
        ));
    }
    Ok(())
}

fn command_stdout(root: &Path, program: &str, arguments: &[&str]) -> Result<String, CiError> {
    let output = Command::new(program)
        .current_dir(root)
        .args(arguments)
        .output()
        .map_err(|error| CiError::Remote(format!("failed to launch {program}: {error}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(CiError::Remote(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

trait CiCommandRunner {
    fn run(&self, root: &Path, program: &str, arguments: &[&OsStr]) -> Result<(), CiError>;
    fn success(&self, root: &Path, program: &str, arguments: &[&OsStr]) -> bool;
}

struct ProcessRunner;

impl CiCommandRunner for ProcessRunner {
    fn run(&self, root: &Path, program: &str, arguments: &[&OsStr]) -> Result<(), CiError> {
        let output = Command::new(program)
            .current_dir(root)
            .args(arguments)
            .output()
            .map_err(|error| CiError::Remote(format!("failed to launch {program}: {error}")))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(CiError::Remote(format!(
                "{program} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    fn success(&self, root: &Path, program: &str, arguments: &[&OsStr]) -> bool {
        Command::new(program)
            .current_dir(root)
            .args(arguments)
            .status()
            .is_ok_and(|status| status.success())
    }
}

const fn blocking(value: FailOn) -> &'static str {
    match value {
        FailOn::Error => "error",
        FailOn::Warning => "warning",
        FailOn::Info => "info",
        FailOn::None => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn settings() -> CiSettings {
        CiSettings {
            scope: CiScope::Baseline,
            blocking: "warning".to_string(),
            comment: true,
            review_comments: false,
            commit_status: true,
            sarif: true,
            action_major: "v1".to_string(),
        }
    }

    #[test]
    fn github_permissions_are_channel_scoped_and_pinned() {
        let rendered = render_managed(CiProvider::Github, &settings()).unwrap();
        assert!(rendered.contains(&format!("actions/checkout@{CHECKOUT_SHA}")));
        assert!(rendered.contains("pull-requests: write"));
        assert!(rendered.contains("statuses: write"));
        assert!(rendered.contains("security-events: write"));
        assert!(rendered.contains("scope: baseline"));
        assert_eq!(parse_settings(&rendered).unwrap(), settings());
    }

    #[test]
    fn replacement_preserves_everything_outside_markers() {
        let path = Path::new("workflow.yml");
        let old = render_managed(CiProvider::Gitlab, &settings()).unwrap();
        let before = format!("# user prefix\n{old}# user suffix\n");
        let mut changed = settings();
        changed.blocking = "error".to_string();
        let new = render_managed(CiProvider::Gitlab, &changed).unwrap();
        let after = replace_managed(path, &before, &new).unwrap();
        assert!(after.starts_with("# user prefix\n"));
        assert!(after.ends_with("# user suffix\n"));
        assert!(after.contains("--blocking error"));
    }

    #[test]
    fn github_install_rejects_unowned_destination() {
        let error = merge_install(
            CiProvider::Github,
            Path::new("workflow.yml"),
            Some("name: User workflow\n"),
            "managed",
        )
        .unwrap_err();
        assert!(error.to_string().contains("not Rust Doctor-owned"));
    }

    #[test]
    fn gitlab_install_appends_without_rewriting_user_content() {
        let merged = merge_install(
            CiProvider::Gitlab,
            Path::new(".gitlab-ci.yml"),
            Some("include: local.yml\n"),
            "managed\n",
        )
        .unwrap();
        assert_eq!(merged, "include: local.yml\nmanaged\n");
    }

    #[test]
    fn managed_metadata_rejects_yaml_injection() {
        let mut malicious = settings();
        malicious.action_major = "v1\n      pull-requests: write".to_string();
        let error = render_managed(CiProvider::Github, &malicious).unwrap_err();
        assert!(error.to_string().contains("major reference"));
    }

    #[test]
    #[expect(
        clippy::literal_string_with_formatting_args,
        reason = "the asserted GitLab shell condition intentionally contains parameter-expansion braces"
    )]
    fn gitlab_baseline_requires_merge_request_base_and_falls_back_to_full() {
        let rendered = render_managed(CiProvider::Gitlab, &settings()).unwrap();
        assert!(rendered.contains(
            "[[ \"$CI_PIPELINE_SOURCE\" == \"merge_request_event\" && -n \"${CI_MERGE_REQUEST_DIFF_BASE_SHA:-}\" ]]"
        ));
        assert!(
            rendered
                .contains("rust-doctor . --baseline --base \"$CI_MERGE_REQUEST_DIFF_BASE_SHA\"")
        );
        assert!(rendered.contains("else\n        rust-doctor . --scope full"));
    }

    #[derive(Clone, Copy)]
    enum InjectedFailure {
        Push,
        PullRequest,
    }

    struct InjectedRunner {
        failure: InjectedFailure,
        failed: Cell<bool>,
    }

    impl InjectedRunner {
        const fn new(failure: InjectedFailure) -> Self {
            Self {
                failure,
                failed: Cell::new(false),
            }
        }

        fn arguments(arguments: &[&OsStr]) -> Vec<String> {
            arguments
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect()
        }
    }

    impl CiCommandRunner for InjectedRunner {
        fn run(&self, root: &Path, program: &str, arguments: &[&OsStr]) -> Result<(), CiError> {
            let arguments_text = Self::arguments(arguments);
            if program == "gh" && arguments_text == ["auth", "status"] {
                return Ok(());
            }
            let fail_push = program == "git"
                && matches!(self.failure, InjectedFailure::Push)
                && arguments_text
                    .first()
                    .is_some_and(|argument| argument == "push")
                && !arguments_text.iter().any(|argument| argument == "--delete");
            let fail_pull_request = program == "gh"
                && matches!(self.failure, InjectedFailure::PullRequest)
                && arguments_text.starts_with(&["pr".to_string(), "create".to_string()]);
            if !self.failed.get() && (fail_push || fail_pull_request) {
                self.failed.set(true);
                return Err(CiError::Remote("injected command failure".to_string()));
            }
            ProcessRunner.run(root, program, arguments)
        }

        fn success(&self, root: &Path, program: &str, arguments: &[&OsStr]) -> bool {
            ProcessRunner.success(root, program, arguments)
        }
    }

    fn git(root: &Path, arguments: &[&str]) -> String {
        command_stdout(root, "git", arguments).unwrap()
    }

    fn transaction_fixture() -> (tempfile::TempDir, tempfile::TempDir, String) {
        let repository = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        git(repository.path(), &["init", "-q"]);
        git(
            repository.path(),
            &["config", "user.email", "tests@rust-doctor.local"],
        );
        git(
            repository.path(),
            &["config", "user.name", "Rust Doctor Tests"],
        );
        fs::write(repository.path().join("README.md"), "fixture\n").unwrap();
        git(repository.path(), &["add", "README.md"]);
        git(repository.path(), &["commit", "-qm", "baseline"]);
        git(remote.path(), &["init", "--bare", "-q"]);
        git(
            repository.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        let branch = git(repository.path(), &["branch", "--show-current"]);
        git(repository.path(), &["push", "-qu", "origin", &branch]);
        (repository, remote, branch)
    }

    fn assert_transaction_rolled_back(failure: InjectedFailure) {
        let (repository, remote, original_branch) = transaction_fixture();
        let root = repository.path();
        let path = root.join(".github/workflows/rust-doctor.yml");
        let rendered = render_managed(CiProvider::Github, &settings()).unwrap();
        let runner = InjectedRunner::new(failure);
        let error =
            install_through_pull_request_with(root, &path, None, &rendered, "v1", None, &runner)
                .unwrap_err();
        assert!(error.to_string().contains("rolled back"));
        assert_eq!(git(root, &["branch", "--show-current"]), original_branch);
        assert!(!path.exists());
        assert!(
            !command_success(
                root,
                "git",
                &["show-ref", "--verify", "refs/heads/rust-doctor/ci-v1"]
            ),
            "temporary local branch remains"
        );
        assert!(
            git(
                remote.path(),
                &["for-each-ref", "--format=%(refname)", "refs/heads"]
            )
            .lines()
            .all(|reference| reference != "refs/heads/rust-doctor/ci-v1"),
            "temporary remote branch remains"
        );
        assert!(git(root, &["status", "--porcelain"]).is_empty());
    }

    fn command_success(root: &Path, program: &str, arguments: &[&str]) -> bool {
        Command::new(program)
            .current_dir(root)
            .args(arguments)
            .status()
            .is_ok_and(|status| status.success())
    }

    #[test]
    fn push_failure_restores_original_branch_and_files() {
        assert_transaction_rolled_back(InjectedFailure::Push);
    }

    #[test]
    fn pull_request_failure_removes_remote_branch_and_restores_local_state() {
        assert_transaction_rolled_back(InjectedFailure::PullRequest);
    }

    #[test]
    fn preexisting_remote_branch_is_never_deleted() {
        let (repository, remote, original_branch) = transaction_fixture();
        let root = repository.path();
        git(root, &["branch", "rust-doctor/ci-v1"]);
        git(root, &["push", "-q", "origin", "rust-doctor/ci-v1"]);
        git(root, &["branch", "-D", "rust-doctor/ci-v1"]);
        let path = root.join(".github/workflows/rust-doctor.yml");
        let rendered = render_managed(CiProvider::Github, &settings()).unwrap();
        let runner = InjectedRunner::new(InjectedFailure::PullRequest);
        let error =
            install_through_pull_request_with(root, &path, None, &rendered, "v1", None, &runner)
                .unwrap_err();
        assert!(error.to_string().contains("remote branch"));
        assert_eq!(git(root, &["branch", "--show-current"]), original_branch);
        assert!(!path.exists());
        assert!(
            git(
                remote.path(),
                &["for-each-ref", "--format=%(refname)", "refs/heads"]
            )
            .lines()
            .any(|reference| reference == "refs/heads/rust-doctor/ci-v1")
        );
    }

    #[test]
    fn multiple_push_targets_are_rejected_before_local_or_remote_mutation() {
        let (repository, remote, original_branch) = transaction_fixture();
        let other_remote = tempfile::tempdir().unwrap();
        git(other_remote.path(), &["init", "--bare", "-q"]);
        let root = repository.path();
        git(
            root,
            &[
                "remote",
                "set-url",
                "--add",
                "--push",
                "origin",
                remote.path().to_str().unwrap(),
            ],
        );
        git(
            root,
            &[
                "remote",
                "set-url",
                "--add",
                "--push",
                "origin",
                other_remote.path().to_str().unwrap(),
            ],
        );
        let path = root.join(".github/workflows/rust-doctor.yml");
        let rendered = render_managed(CiProvider::Github, &settings()).unwrap();
        let runner = InjectedRunner::new(InjectedFailure::PullRequest);
        let error =
            install_through_pull_request_with(root, &path, None, &rendered, "v1", None, &runner)
                .unwrap_err();
        assert!(error.to_string().contains("multiple push targets"));
        assert_eq!(git(root, &["branch", "--show-current"]), original_branch);
        assert!(!path.exists());
        assert!(
            git(
                remote.path(),
                &["for-each-ref", "--format=%(refname)", "refs/heads"]
            )
            .lines()
            .all(|reference| reference != "refs/heads/rust-doctor/ci-v1")
        );
    }
}
