//! `rust-doctor ci install`: the GitHub Actions workflows that run the scan on
//! every pull request, written without the interactive report. The report's
//! menu entry calls the same writer with the same defaults.
//!
//! The scan workflow judges a pull request in baseline scope, so the backlog
//! the default branch already carries is nobody's pull request's problem, and
//! scans a push to that branch with `--blocking none`, so it reports without
//! ever turning the branch red. The comment workflow is a second file, run by
//! `workflow_run` in the base repository's context: it holds the one token
//! that can write to a pull request, and it never checks out or builds the
//! pull request's code. The binary reaches no network in either: the comment
//! is posted by `gh` with the workflow's own token.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use rust_doctor::BlockingLevel;

use crate::workspace_write::{create_new, symlinked_component};

pub const WORKFLOW_PATH: &str = ".github/workflows/rust-doctor.yml";
pub const COMMENT_WORKFLOW_PATH: &str = ".github/workflows/rust-doctor-comment.yml";

/// The toolchain this release was validated on. The workflow pins it rather
/// than tracking `stable`: Clippy's diagnostics are the product, and a gate
/// whose lints move every six weeks fails builds for reasons nobody chose.
pub const VALIDATED_TOOLCHAIN: &str = "1.97.1";

/// The line every workflow this command writes opens with, and the one
/// `--update` requires before it rewrites a file.
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
        /// The repository root the workflow is written under and scans.
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
        /// Also write .github/workflows/rust-doctor-comment.yml, which posts
        /// the report as one pull request comment, forks included.
        #[arg(long)]
        comment: bool,
        /// Rewrite workflows this command wrote before.
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
    pub blocking: Option<BlockingLevel>,
    pub branch: String,
    pub toolchain: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiError {
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
            Self::AlreadyPresent(_) | Self::NotOurs(_) => 1,
            _ => FAILED,
        }
    }
}

impl fmt::Display for CiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
    let planned = options(path, *blocking, branch.clone(), toolchain.clone())
        .and_then(|options| plan(path, &options, *comment, *update));
    let planned = match planned {
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
    match write(path, &planned) {
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

/// Whether the interactive report offers to add the workflow: a repository
/// that does not carry it yet.
pub fn can_install(root: &Path) -> bool {
    root.join(".git").exists() && !root.join(WORKFLOW_PATH).exists()
}

/// The interactive report's entry: the scan workflow with every default
/// `ci install` has, answering with the path it wrote.
pub fn install_default(root: &Path) -> Result<PathBuf, CiError> {
    let options = options(root, None, None, VALIDATED_TOOLCHAIN.to_owned())?;
    let planned = plan(root, &options, false, false)?;
    write(root, &planned)?;
    Ok(PathBuf::from(WORKFLOW_PATH))
}

fn options(
    root: &Path,
    blocking: Option<BlockingLevel>,
    branch: Option<String>,
    toolchain: String,
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
        blocking,
        branch,
        toolchain,
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

/// Every file the install writes, checked before any is written: a refusal
/// of one leaves the other unwritten too.
fn plan(
    root: &Path,
    options: &WorkflowOptions,
    comment: bool,
    update: bool,
) -> Result<Vec<Planned>, CiError> {
    let mut files = vec![(WORKFLOW_PATH, scan_workflow(options))];
    if comment {
        files.push((COMMENT_WORKFLOW_PATH, comment_workflow()));
    }
    files
        .into_iter()
        .map(|(path, content)| {
            if let Some(link) = symlinked_component(root, Path::new(path)) {
                return Err(CiError::Symlink(link));
            }
            let target = root.join(path);
            if target.symlink_metadata().is_err() {
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
    SCAN_WORKFLOW
        .replace("{MARKER}", MARKER)
        .replace("{BRANCH}", &options.branch)
        .replace("{TOOLCHAIN}", &options.toolchain)
        .replace("{VERSION}", env!("CARGO_PKG_VERSION"))
        .replace("{BLOCKING}", &blocking)
}

/// The comment workflow, which reads what the scan uploaded and nothing else.
pub fn comment_workflow() -> String {
    COMMENT_WORKFLOW
        .replace("{MARKER}", MARKER)
        .replace("{VERSION}", env!("CARGO_PKG_VERSION"))
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
  contents: read

jobs:
  inspect:
    name: Inspect
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      # Baseline scope checks the base out into a temporary worktree, so the
      # shallow default clone is not enough. No token is left behind for the
      # code under review to find.
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
          persist-credentials: false

      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: '{TOOLCHAIN}'
          components: clippy

      - uses: Swatinem/rust-cache@v2

      # The version is pinned to the one that wrote this file: a gate whose
      # rule set moves on its own fails builds for reasons nobody chose.
      - name: Install rust-doctor
        run: npm install -g rust-doctor@{VERSION}

      - name: Inspect the workspace
        env:
          BASE_REF: ${{ github.base_ref }}
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
          rust-doctor . --yes --json "${scan_arguments[@]}" > "$report"
          scan_status=$?
          set -e

          rust-doctor report markdown "$report" >> "$GITHUB_STEP_SUMMARY" || true
          exit "$scan_status"

      - name: Upload the report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: rust-doctor-report
          path: ${{ runner.temp }}/rust-doctor/report.json
          if-no-files-found: ignore
          retention-days: 7
"#;

const COMMENT_WORKFLOW: &str = r#"{MARKER} --comment; `rust-doctor ci install --comment --update` rewrites it.
#
# Runs after the scan workflow, in this repository's context, so a pull
# request from a fork gets its comment too. It never checks out the pull
# request and never runs cargo: it reads the report the scan uploaded and
# posts it with this workflow's token. rust-doctor itself reaches no network.
# Never move this job to `pull_request_target`.
name: Rust Doctor comment

on:
  workflow_run:
    workflows: [Rust Doctor]
    types: [completed]

permissions: {}

jobs:
  comment:
    name: Comment
    if: github.event.workflow_run.event == 'pull_request'
    runs-on: ubuntu-latest
    timeout-minutes: 10
    permissions:
      actions: read
      pull-requests: write
    steps:
      - name: Download the report
        id: download
        continue-on-error: true
        uses: actions/download-artifact@v5
        with:
          name: rust-doctor-report
          path: ${{ runner.temp }}/rust-doctor
          run-id: ${{ github.event.workflow_run.id }}
          github-token: ${{ github.token }}

      - name: Install rust-doctor
        if: steps.download.outcome == 'success'
        run: npm install -g rust-doctor@{VERSION}

      # The pull request is looked up from GitHub by the commit the scan ran
      # on, never read from the report, which the code under review could
      # have written.
      - name: Post or update the comment
        env:
          GH_TOKEN: ${{ github.token }}
          REPOSITORY: ${{ github.repository }}
          HEAD_OWNER: ${{ github.event.workflow_run.head_repository.owner.login }}
          HEAD_BRANCH: ${{ github.event.workflow_run.head_branch }}
          HEAD_SHA: ${{ github.event.workflow_run.head_sha }}
        run: |
          set -euo pipefail
          report="$RUNNER_TEMP/rust-doctor/report.json"
          if [ ! -f "$report" ]; then
            echo "::notice::The scan uploaded no rust-doctor report, so no comment was posted."
            exit 0
          fi
          pull_request=$(gh api -X GET "repos/$REPOSITORY/pulls" -f state=open -f head="$HEAD_OWNER:$HEAD_BRANCH" \
            | jq -r --arg sha "$HEAD_SHA" '[.[] | select(.head.sha == $sha) | .number] | first // empty')
          if [ -z "$pull_request" ]; then
            echo "::notice::No open pull request is at $HEAD_SHA, so no comment was posted."
            exit 0
          fi
          body="$(printf '<!-- rust-doctor -->\n\n'; rust-doctor report markdown "$report")"
          comment=$(gh api --paginate "repos/$REPOSITORY/issues/$pull_request/comments" \
            | jq -s -r '[.[][] | select(.user.login == "github-actions[bot]" and (.body | startswith("<!-- rust-doctor -->"))) | .id] | first // empty')
          if [ -n "$comment" ]; then
            gh api -X PATCH "repos/$REPOSITORY/issues/comments/$comment" -f body="$body" > /dev/null
          else
            gh api -X POST "repos/$REPOSITORY/issues/$pull_request/comments" -f body="$body" > /dev/null
          fi
"#;

#[cfg(test)]
mod tests;
