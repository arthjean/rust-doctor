//! `rust-doctor ci install`: the GitHub Actions workflow that runs the scan on
//! every pull request, written without the interactive report. The report's
//! menu entry calls the same writer with the same defaults.
//!
//! The workflow judges a pull request in baseline scope, so the backlog the
//! default branch already carries is nobody's pull request's problem, and
//! scans a push to that branch with `--blocking none`, so it reports without
//! ever turning the branch red. With `--comment`, the same job posts the
//! summary as one sticky pull request comment, the way React Doctor's Action
//! does: the job gains `pull-requests: write`, which GitHub reduces to read
//! on a pull request from a fork, so a fork gets a warning instead of a
//! comment. The binary reaches no network: the comment is posted by `gh` with
//! the workflow's own token.
//!
//! GitHub reads workflows only at the top of the repository, so the file goes
//! under the git toplevel, and a Cargo workspace below it (`src-tauri/`,
//! `backend/`) is the path the workflow scans and caches. Every action it
//! uses is pinned to a commit, with the ref it was read from beside it.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use rust_doctor::BlockingLevel;

use rust_doctor::workspace_write::{create_new, symlinked_component};

pub const WORKFLOW_PATH: &str = ".github/workflows/rust-doctor.yml";

/// The toolchain this release was validated on. The workflow pins it rather
/// than tracking `stable`: Clippy's diagnostics are the product, and a gate
/// whose lints move every six weeks fails builds for reasons nobody chose.
pub const VALIDATED_TOOLCHAIN: &str = "1.97.1";

/// The line the workflow this command writes opens with, and the one
/// `--update` requires before it rewrites the file.
const MARKER: &str = "# Written by rust-doctor ci install";

/// Exit code of an install that could not run: no branch, an invalid value, a
/// failed write.
const FAILED: u8 = 2;

#[derive(Debug, Clone, Args)]
pub struct CiArgs {
    #[command(subcommand)]
    command: CiCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum CiCommand {
    #[command(about = "Write .github/workflows/rust-doctor.yml, never over a file rust-doctor did not write")]
    Install {
        /// The workspace the workflow scans. It is written under the top of
        /// the git repository holding it.
        #[arg(default_value = ".", value_name = "PATH")]
        path: PathBuf,
        /// The level a pull request fails at. Without it, the workspace's
        /// configuration decides. A push to the branch never fails.
        #[arg(long, value_enum, value_name = "LEVEL")]
        blocking: Option<BlockingLevel>,
        /// The branch pushes are scanned on. Without it, the default branch
        /// the repository answers for.
        #[arg(long, value_name = "NAME")]
        branch: Option<String>,
        /// The Rust toolchain the workflow installs.
        #[arg(long, value_name = "VERSION", default_value = VALIDATED_TOOLCHAIN)]
        toolchain: String,
        /// Also post the summary as one sticky pull request comment, which
        /// grants the job `pull-requests: write`. A pull request from a fork
        /// gets a read-only token, and a warning instead of the comment.
        #[arg(long)]
        comment: bool,
        /// Rewrite the workflow this command wrote before.
        #[arg(long)]
        update: bool,
        /// Print what would be written, and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

/// What a workflow is rendered from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowOptions {
    /// The Cargo workspace, relative to the repository's top, `.` at it.
    pub workspace: String,
    pub blocking: Option<BlockingLevel>,
    pub branch: String,
    pub toolchain: String,
    pub comment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiError {
    NotARepository,
    InvalidWorkspace,
    Installed(String),
    BranchUndetected,
    InvalidBranch,
    InvalidToolchain,
    Symlink(PathBuf),
    AlreadyPresent(&'static str),
    NotOurs(&'static str),
    Write(&'static str, io::ErrorKind),
}

impl CiError {
    /// A file that is already there is nothing broken: the install did what it
    /// could, which was nothing.
    const fn exit_code(&self) -> u8 {
        match self {
            Self::AlreadyPresent(_) | Self::NotOurs(_) | Self::Installed(_) => 1,
            _ => FAILED,
        }
    }
}

impl fmt::Display for CiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotARepository => write!(
                formatter,
                "the path is not inside a git repository, and GitHub reads workflows from one"
            ),
            Self::InvalidWorkspace => write!(
                formatter,
                "the workspace's path from the repository's top is not one a workflow can carry"
            ),
            Self::Installed(path) => write!(
                formatter,
                "{path} already runs rust-doctor, so nothing was written"
            ),
            Self::BranchUndetected => write!(
                formatter,
                "no default branch found: pass --branch <NAME>"
            ),
            Self::InvalidBranch => write!(formatter, "--branch is not a branch name"),
            Self::InvalidToolchain => write!(formatter, "--toolchain is not a toolchain name"),
            Self::Symlink(path) => write!(
                formatter,
                "{} is a symlink, and nothing is written through one",
                path.display()
            ),
            Self::AlreadyPresent(path) => write!(
                formatter,
                "{path} already exists, so nothing was written: pass --update to rewrite it"
            ),
            Self::NotOurs(path) => write!(
                formatter,
                "{path} was not written by rust-doctor, so nothing was written"
            ),
            Self::Write(path, kind) => write!(formatter, "could not write {path} ({kind})"),
        }
    }
}

/// A file the install wrote or would write, relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub path: &'static str,
    pub content: String,
    pub replaces: bool,
}

/// `rust-doctor ci install`.
pub fn run(arguments: &CiArgs) -> ExitCode {
    let CiCommand::Install {
        path,
        blocking,
        branch,
        toolchain,
        comment,
        update,
        dry_run,
    } = &arguments.command;
    let path = match crate::path_argument::resolve("PATH", path) {
        Ok(path) => path,
        Err(code) => return ExitCode::from(code),
    };
    let planned = placement(&path).and_then(|(toplevel, workspace)| {
        let options = WorkflowOptions {
            workspace,
            ..options(&toplevel, *blocking, branch.clone(), toolchain.clone(), *comment)?
        };
        Ok((plan(&toplevel, &options, *update)?, toplevel))
    });
    let (planned, toplevel) = match planned {
        Ok(planned) => planned,
        Err(error) => {
            eprintln!("rust-doctor: the workflow was not installed, {error}.");
            return ExitCode::from(error.exit_code());
        }
    };
    if *dry_run {
        for file in &planned {
            print!("Would write {}:\n\n{}\n", file.path, file.content);
        }
        return ExitCode::SUCCESS;
    }
    match write(&toplevel, &planned) {
        Ok(()) => {
            for file in &planned {
                let verb = if file.replaces { "Rewrote" } else { "Wrote" };
                println!("{verb} {}", file.path);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rust-doctor: the workflow was not installed, {error}.");
            ExitCode::from(error.exit_code())
        }
    }
}

/// Whether the interactive report offers to add the workflow: a workspace at
/// the top of its repository that does not carry it yet. A workspace nested
/// in a repository is left to `ci install`, which the reader points at it.
pub fn can_install(root: &Path) -> bool {
    root.join(".git").exists() && !root.join(WORKFLOW_PATH).exists()
}

/// The interactive report's entry: the scan workflow with every default
/// `ci install` has, answering with the path it wrote.
pub fn install_default(root: &Path) -> Result<PathBuf, CiError> {
    let (toplevel, workspace) = placement(root)?;
    let options = WorkflowOptions {
        workspace,
        ..options(&toplevel, None, None, VALIDATED_TOOLCHAIN.to_owned(), false)?
    };
    let planned = plan(&toplevel, &options, false)?;
    write(&toplevel, &planned)?;
    Ok(PathBuf::from(WORKFLOW_PATH))
}

/// The top of the repository the workflow goes under, and the Cargo
/// workspace it scans, relative to that top: the one a scan of `path` would
/// judge, or `path` itself when none resolves.
fn placement(path: &Path) -> Result<(PathBuf, String), CiError> {
    let toplevel = rust_doctor::git_hook::toplevel(path).ok_or(CiError::NotARepository)?;
    let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let workspace = rust_doctor::manifest_workspace_root(path).unwrap_or_else(|| path.to_path_buf());
    let relative = canonical(&workspace)
        .strip_prefix(canonical(&toplevel))
        .map_err(|_| CiError::InvalidWorkspace)?
        .to_string_lossy()
        .into_owned();
    let relative = if relative.is_empty() { ".".to_owned() } else { relative };
    if !is_workspace_path(&relative) {
        return Err(CiError::InvalidWorkspace);
    }
    Ok((toplevel, relative))
}

/// A relative path a YAML scalar and a shell word both carry as written:
/// `[A-Za-z0-9._/-]`, no leading dash, no `..` segment.
fn is_workspace_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['-', '/'])
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._/-".contains(&byte))
        && !path.split('/').any(|segment| segment == "..")
}

fn options(
    root: &Path,
    blocking: Option<BlockingLevel>,
    branch: Option<String>,
    toolchain: String,
    comment: bool,
) -> Result<WorkflowOptions, CiError> {
    let branch = branch
        .or_else(|| rust_doctor::default_branch(root))
        .ok_or(CiError::BranchUndetected)?;
    if !is_branch_name(&branch) {
        return Err(CiError::InvalidBranch);
    }
    if toolchain.is_empty()
        || !toolchain
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
    {
        return Err(CiError::InvalidToolchain);
    }
    Ok(WorkflowOptions {
        workspace: ".".to_owned(),
        blocking,
        branch,
        toolchain,
        comment,
    })
}

/// A name git could hold as a branch, and nothing a YAML scalar or a branch
/// filter could read as more than one: no whitespace, no control character,
/// none of the characters `git check-ref-format` refuses.
fn is_branch_name(branch: &str) -> bool {
    !branch.is_empty()
        && !branch.starts_with('-')
        && branch.chars().all(|character| {
            !character.is_whitespace()
                && !character.is_control()
                && !"~^:?*[\\'\"".contains(character)
        })
}

/// Every file the install writes, checked before any is written.
fn plan(root: &Path, options: &WorkflowOptions, update: bool) -> Result<Vec<Planned>, CiError> {
    let files = vec![(WORKFLOW_PATH, scan_workflow(options))];
    files
        .into_iter()
        .map(|(path, content)| {
            if let Some(link) = symlinked_component(root, Path::new(path)) {
                return Err(CiError::Symlink(link));
            }
            let target = root.join(path);
            if target.symlink_metadata().is_err() {
                if let Some(existing) = existing_install(root) {
                    return Err(CiError::Installed(existing));
                }
                return Ok(Planned {
                    path,
                    content,
                    replaces: false,
                });
            }
            if !update {
                return Err(CiError::AlreadyPresent(path));
            }
            let ours = fs::read_to_string(&target)
                .is_ok_and(|existing| existing.starts_with(MARKER));
            if !ours {
                return Err(CiError::NotOurs(path));
            }
            Ok(Planned {
                path,
                content,
                replaces: true,
            })
        })
        .collect()
}

/// A workflow already under `.github/workflows/` that runs rust-doctor,
/// through the Action or a file this command wrote under another name.
fn existing_install(root: &Path) -> Option<String> {
    let mut names: Vec<String> = fs::read_dir(root.join(".github/workflows"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".yml") || name.ends_with(".yaml"))
        .collect();
    names.sort();
    names.into_iter().find_map(|name| {
        let path = format!(".github/workflows/{name}");
        fs::read_to_string(root.join(&path))
            .ok()
            .filter(|text| text.contains("arthjean/rust-doctor@") || text.contains(MARKER))
            .map(|_| path)
    })
}

/// A new file is created only if absent. A rewrite goes through a sibling
/// created the same way and renamed over the old file, and a rename replaces
/// the name rather than writing wherever it pointed.
fn write(root: &Path, planned: &[Planned]) -> Result<(), CiError> {
    for file in planned {
        let target = root.join(file.path);
        let failed = |error: io::Error| CiError::Write(file.path, error.kind());
        if file.replaces {
            let staging = target.with_extension("yml.rust-doctor");
            create_new(&staging, file.content.as_bytes()).map_err(failed)?;
            fs::rename(&staging, &target).map_err(|error| {
                let _ = fs::remove_file(&staging);
                failed(error)
            })?;
        } else {
            create_new(&target, file.content.as_bytes()).map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    CiError::AlreadyPresent(file.path)
                } else {
                    failed(error)
                }
            })?;
        }
    }
    Ok(())
}

/// The scan workflow. The base branch reaches the shell through the
/// environment, never through `${{ }}` inside `run`: on a fork pull request a
/// branch name is attacker-controlled text, and interpolating it into a run
/// block is a command injection.
pub fn scan_workflow(options: &WorkflowOptions) -> String {
    let blocking = options
        .blocking
        .map(|level| format!(" --blocking {}", level.as_str()))
        .unwrap_or_default();
    let cache_workspace = if options.workspace == "." {
        String::new()
    } else {
        format!("\n          workspaces: {}", options.workspace)
    };
    let (permission, step) = if options.comment {
        ("\n  pull-requests: write", COMMENT_STEP.replace("{SCRIPT}", &indented(COMMENT_SCRIPT, 10)))
    } else {
        ("", String::new())
    };
    SCAN_WORKFLOW
        .replace("{COMMENT_PERMISSION}", permission)
        .replace("{COMMENT_STEP}", &step)
        .replace("{MARKER}", MARKER)
        .replace("{BRANCH}", &options.branch)
        .replace("{TOOLCHAIN}", &options.toolchain)
        .replace("{VERSION}", env!("CARGO_PKG_VERSION"))
        .replace("{BLOCKING}", &blocking)
        .replace("{CACHE_WORKSPACE}", &cache_workspace)
        .replace("{WORKSPACE}", &options.workspace)
}

fn indented(script: &str, depth: usize) -> String {
    let margin = " ".repeat(depth);
    script
        .lines()
        .map(|line| if line.is_empty() { String::new() } else { format!("{margin}{line}") })
        .collect::<Vec<_>>()
        .join("\n")
}

const SCAN_WORKFLOW: &str = r#"{MARKER}; `rust-doctor ci install --update` rewrites it.
name: Rust Doctor

on:
  push:
    branches: ['{BRANCH}']
  pull_request:
    types: [opened, synchronize, reopened, ready_for_review]

concurrency:
  group: rust-doctor-${{ github.event.pull_request.number || github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}

permissions:
  contents: read{COMMENT_PERMISSION}

jobs:
  inspect:
    name: Inspect
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      # Baseline scope checks the base out into a temporary snapshot, so the
      # shallow default clone is not enough. No token is left behind for the
      # code under review to find.
      - uses: actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09 # v5.1.0
        with:
          fetch-depth: 0
          persist-credentials: false

      - uses: dtolnay/rust-toolchain@e2a55d2ffb04f378e9626c28d38b36d230d1e12f # master
        with:
          toolchain: '{TOOLCHAIN}'
          components: clippy

      # A failed gate keeps the build its retry needs: rust-cache saves only
      # on success unless told otherwise.
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          cache-on-failure: true{CACHE_WORKSPACE}

      # The version is pinned to the one that wrote this file: a gate whose
      # rule set moves on its own fails builds for reasons nobody chose.
      - name: Install rust-doctor
        run: npm install -g rust-doctor@{VERSION}

      - name: Inspect the workspace
        env:
          BASE_REF: ${{ github.base_ref }}
          EVENT_NAME: ${{ github.event_name }}
        run: |
          set -euo pipefail
          report="$RUNNER_TEMP/rust-doctor/report.json"
          mkdir -p "$(dirname "$report")"
          if [ -n "${BASE_REF:-}" ]; then
            scan_arguments=(--scope baseline --base "origin/$BASE_REF"{BLOCKING})
          else
            scan_arguments=(--scope full --blocking none)
          fi

          set +e
          rust-doctor {WORKSPACE} --yes --json "${scan_arguments[@]}" > "$report"
          scan_status=$?
          set -e

          summary="$RUNNER_TEMP/rust-doctor/summary.md"
          rust-doctor report markdown "$report" > "$summary" || true
          cat "$summary" >> "$GITHUB_STEP_SUMMARY"
          # Only a pull request fails on the gate. A push reports the
          # branch's health without turning it red, and a scan that could
          # not run (2) fails on every event.
          if [ "$scan_status" -eq 1 ] && [ "${EVENT_NAME:-}" != pull_request ]; then
            scan_status=0
          fi
          exit "$scan_status"

      - name: Upload the report
        if: always()
        uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4.6.2
        with:
          name: rust-doctor-report
          path: ${{ runner.temp }}/rust-doctor/report.json
          if-no-files-found: ignore
          retention-days: 7
{COMMENT_STEP}"#;

const COMMENT_STEP: &str = r#"
      # One sticky comment per pull request, edited on each push. A pull
      # request from a fork gets a read-only token, and a warning instead.
      - name: Comment on the pull request
        if: always() && github.event_name == 'pull_request'
        env:
          GH_TOKEN: ${{ github.token }}
          REPOSITORY: ${{ github.repository }}
          PULL_REQUEST: ${{ github.event.pull_request.number }}
        run: |
{SCRIPT}
"#;

/// Posts the summary the scan step rendered as the pull request's one
/// `<!-- rust-doctor -->` comment, editing the bot's earlier one. `action.yml`
/// runs the same script, and a test holds the two to it. Under the Action's
/// `comment-on-clean: false`, a clean pull request gets no new comment, while
/// one already posted is still edited so it never reports a stale result. Nothing here fails
/// the job: the scan's exit code is the verdict, and a fork's read-only token
/// is expected.
const COMMENT_SCRIPT: &str = r#"set -euo pipefail
summary="$RUNNER_TEMP/rust-doctor/summary.md"
if [ ! -s "$summary" ]; then
  echo "::notice::rust-doctor wrote no report, so no comment was posted."
  exit 0
fi
body="$(printf '<!-- rust-doctor -->\n\n'; cat "$summary")"
if ! comments=$(gh api --paginate "repos/$REPOSITORY/issues/$PULL_REQUEST/comments"); then
  echo "::warning::rust-doctor could not read the pull request's comments, so none was posted."
  exit 0
fi
comment=$(jq -s -r '[.[][] | select(.user.login == "github-actions[bot]" and (.body | startswith("<!-- rust-doctor -->"))) | .id] | first // empty' <<< "$comments")
if [ -z "$comment" ] && [ "${CLEAN:-false}" = true ] && [ "${COMMENT_ON_CLEAN:-true}" = false ]; then
  echo "::notice::The pull request is clean and comment-on-clean is false, so no comment was posted."
  exit 0
fi
if [ -n "$comment" ]; then
  gh api -X PATCH "repos/$REPOSITORY/issues/comments/$comment" -f body="$body" > /dev/null
else
  gh api -X POST "repos/$REPOSITORY/issues/$PULL_REQUEST/comments" -f body="$body" > /dev/null
fi || echo "::warning::rust-doctor could not post the pull request comment. A pull request from a fork gets a read-only token."
"#;

#[cfg(test)]
mod tests;
