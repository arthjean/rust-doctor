use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use rust_doctor::BlockingLevel;

use rust_doctor::workspace_write::{create_new, symlinked_component};

pub const WORKFLOW_PATH: &str = ".github/workflows/rust-doctor.yml";

pub const VALIDATED_TOOLCHAIN: &str = "1.99.1";

const MARKER: &str = "# Written by rust-doctor ci install";

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
        #[arg(
            default_value = ".",
            value_name = "PATH",
            help = "The workspace the workflow scans. It is written under the top of the git repository holding it"
        )]
        path: PathBuf,
        #[arg(
            long,
            value_enum,
            value_name = "LEVEL",
            help = "The level a pull request fails at. Without it, the workspace's configuration decides. A push to the branch never fails"
        )]
        blocking: Option<BlockingLevel>,
        #[arg(
            long,
            value_name = "NAME",
            help = "The branch pushes are scanned on. Without it, the default branch the repository answers for"
        )]
        branch: Option<String>,
        #[arg(
            long,
            value_name = "VERSION",
            default_value = VALIDATED_TOOLCHAIN,
            help = "The Rust toolchain the workflow installs"
        )]
        toolchain: String,
        #[arg(
            long,
            help = "Also post the summary as one sticky pull request comment, from a second job that holds `pull-requests: write` and runs no pull request code. A pull request from a fork gets a read-only token, and a warning instead of the comment"
        )]
        comment: bool,
        #[arg(long, help = "Rewrite the workflow this command wrote before")]
        update: bool,
        #[arg(long, help = "Print what would be written, and write nothing")]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowOptions {
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
    Unconfined(rust_doctor::git_hook::Unconfined),
    BranchUndetected,
    InvalidBranch,
    InvalidToolchain,
    Symlink(PathBuf),
    AlreadyPresent(&'static str),
    NotOurs(&'static str),
    Write(&'static str, io::ErrorKind),
}

impl CiError {
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
                "{} already runs rust-doctor, so nothing was written",
                rust_doctor::terminal_text::sanitize(path)
            ),
            Self::Unconfined(refusal) => write!(formatter, "{refusal}"),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub path: &'static str,
    pub content: String,
    pub replaces: bool,
}

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

pub fn can_install(root: &Path) -> bool {
    root.join(".git").exists() && !root.join(WORKFLOW_PATH).exists()
}

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

fn placement(path: &Path) -> Result<(PathBuf, String), CiError> {
    let toplevel = rust_doctor::git_hook::toplevel(path).ok_or(CiError::NotARepository)?;
    let toplevel = rust_doctor::git_hook::confine(path, &toplevel)
        .map_err(CiError::Unconfined)?
        .toplevel;
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

fn is_branch_name(branch: &str) -> bool {
    !branch.is_empty()
        && !branch.starts_with('-')
        && branch.chars().all(|character| {
            !character.is_whitespace()
                && !character.is_control()
                && !"~^:?*[\\'\"".contains(character)
        })
}

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
    let job = if options.comment {
        COMMENT_JOB.replace("{SCRIPT}", &indented(COMMENT_SCRIPT, 10))
    } else {
        String::new()
    };
    SCAN_WORKFLOW
        .replace("{COMMENT_JOB}", &job)
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
  contents: read

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
      # on success unless told otherwise. Only the default branch saves, so a
      # pull request's build scripts never write an entry a later run
      # restores, and `~/.cargo/bin` is never cached.
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          cache-on-failure: true
          cache-bin: false
          save-if: ${{ github.ref == format('refs/heads/{0}', github.event.repository.default_branch) }}{CACHE_WORKSPACE}

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
          # The launcher npm installed, by its absolute path: a binary
          # earlier on PATH, `~/.cargo/bin` first, never stands in for it.
          launcher="$(npm prefix -g)/bin/rust-doctor"
          report="$RUNNER_TEMP/rust-doctor/report.json"
          mkdir -p "$(dirname "$report")"
          if [ -n "${BASE_REF:-}" ]; then
            scan_arguments=(--scope baseline --base "origin/$BASE_REF"{BLOCKING})
          else
            scan_arguments=(--scope full --blocking none)
          fi

          set +e
          "$launcher" {WORKSPACE} --yes --json "${scan_arguments[@]}" > "$report"
          scan_status=$?
          set -e

          summary="$RUNNER_TEMP/rust-doctor/summary.md"
          "$launcher" report markdown "$report" > "$summary" || true
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
          path: |
            ${{ runner.temp }}/rust-doctor/report.json
            ${{ runner.temp }}/rust-doctor/summary.md
          if-no-files-found: ignore
          retention-days: 7
{COMMENT_JOB}"#;

const COMMENT_JOB: &str = r#"
  # One sticky comment per pull request, edited on each push, posted after the
  # scan whatever its verdict. A pull request from a fork gets a read-only
  # token, and a warning instead.
  comment:
    name: Comment
    needs: inspect
    if: always() && github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    timeout-minutes: 5
    permissions:
      pull-requests: write
    steps:
      # A scan that crashed before its upload leaves no artifact, and the
      # comment step then says so.
      - name: Download the report
        uses: actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093 # v4.3.0
        continue-on-error: true
        with:
          name: rust-doctor-report
          path: ${{ runner.temp }}/rust-doctor

      - name: Comment on the pull request
        env:
          GH_TOKEN: ${{ github.token }}
          REPOSITORY: ${{ github.repository }}
          PULL_REQUEST: ${{ github.event.pull_request.number }}
        run: |
{SCRIPT}
"#;

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
clean="$(cat "$RUNNER_TEMP/rust-doctor/clean" 2>/dev/null || true)"
if [ -z "$comment" ] && [ "$clean" = true ] && [ "${COMMENT_ON_CLEAN:-true}" = false ]; then
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
