//! The GitHub Actions workflow the CI screens offer to add.
//!
//! The interactive report is the only place that writes into the inspected
//! workspace, and it writes exactly one file, only when the reader picks it out
//! of a menu, and never over an existing one.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const WORKFLOW_PATH: &str = ".github/workflows/rust-doctor.yml";

/// A pull request is scanned in baseline scope, which is the mode the tool
/// exists for in CI: the existing backlog is not the contributor's problem, so
/// only what the change introduces is judged. That needs the full history, and
/// the checkout asks for it.
const WORKFLOW: &str = r#"name: Rust Doctor

on:
  push:
    branches: [main]
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
      # Baseline scope checks the comparison ref out into a temporary worktree,
      # so the shallow default clone is not enough.
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      # rust-doctor shells out to `cargo clippy` inside the scanned workspace.
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy

      - uses: Swatinem/rust-cache@v2

      # The npm launcher resolves the prebuilt binary for the runner rather
      # than compiling rust-doctor from source on every run. The version is
      # pinned: a CI gate whose rule set moves on its own fails builds for
      # reasons nobody chose.
      - name: Install rust-doctor
        run: npm install -g rust-doctor@{VERSION}

      # BASE_REF reaches the shell through the environment rather than through
      # `${{ }}` interpolation: a branch name is attacker-controlled text on a
      # fork pull request, and interpolating it into a run block is a command
      # injection.
      - name: Inspect the workspace
        env:
          BASE_REF: ${{ github.base_ref }}
        run: |
          set -euo pipefail
          if [ -n "${BASE_REF:-}" ]; then
            git fetch --no-tags origin "$BASE_REF"
            scope_arguments=(--scope baseline --base "origin/$BASE_REF")
          else
            scope_arguments=(--scope full)
          fi

          set +e
          rust-doctor . --yes --verbose "${scope_arguments[@]}" > scan.txt 2>&1
          scan_status=$?
          set -e

          {
            echo '## Rust Doctor'
            echo
            echo '```'
            cat scan.txt
            echo '```'
          } >> "$GITHUB_STEP_SUMMARY"

          exit "$scan_status"
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    AlreadyPresent,
    Write(io::ErrorKind),
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyPresent => write!(formatter, "{WORKFLOW_PATH} already exists"),
            Self::Write(kind) => {
                write!(formatter, "could not write {WORKFLOW_PATH} ({})", reason(*kind))
            }
        }
    }
}

const fn reason(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::PermissionDenied => "permission denied",
        io::ErrorKind::NotFound => "directory not found",
        io::ErrorKind::ReadOnlyFilesystem => "read-only filesystem",
        _ => "write error",
    }
}

/// Whether the CI entry belongs in the menu at all: a workspace under git that
/// does not already carry the workflow.
pub fn can_install(workspace_root: &Path) -> bool {
    workspace_root.join(".git").exists() && !workspace_root.join(WORKFLOW_PATH).exists()
}

pub fn install(workspace_root: &Path) -> Result<PathBuf, WorkflowError> {
    let destination = workspace_root.join(WORKFLOW_PATH);
    if destination.exists() {
        return Err(WorkflowError::AlreadyPresent);
    }
    if let Some(directory) = destination.parent() {
        fs::create_dir_all(directory).map_err(|error| WorkflowError::Write(error.kind()))?;
    }
    fs::write(&destination, workflow()).map_err(|error| WorkflowError::Write(error.kind()))?;
    Ok(PathBuf::from(WORKFLOW_PATH))
}

/// The workflow, pinned to the version that wrote it.
///
/// Deriving the pin from the running binary rather than restating it keeps the
/// generated gate scanning with the rule set its author saw, and keeps a
/// release from having to remember to bump a string in a template.
fn workflow() -> String {
    WORKFLOW.replace("{VERSION}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static TEMPORARY: AtomicUsize = AtomicUsize::new(0);

    fn temporary_root(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/workflow-tests")
            .join(format!(
                "{}-{name}-{}",
                std::process::id(),
                TEMPORARY.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn the_entry_is_offered_only_under_git_and_only_once() {
        let root = temporary_root("availability");
        assert!(!can_install(&root));

        fs::create_dir_all(root.join(".git")).unwrap();
        assert!(can_install(&root));

        assert_eq!(install(&root).unwrap(), PathBuf::from(WORKFLOW_PATH));
        assert!(!can_install(&root));
        assert_eq!(install(&root), Err(WorkflowError::AlreadyPresent));

        let written = fs::read_to_string(root.join(WORKFLOW_PATH)).unwrap();
        assert!(written.starts_with("name: Rust Doctor\n"));
        assert!(written.contains("--scope baseline"));
        // The gate installs the published launcher, pinned to the version that
        // wrote the file, and no placeholder survives into the workspace.
        assert!(written.contains(&format!(
            "npm install -g rust-doctor@{}",
            env!("CARGO_PKG_VERSION")
        )));
        assert!(!written.contains("{VERSION}"));
        // The branch name never reaches the shell through interpolation.
        assert!(!written.contains("${{ github.base_ref }}\"") && written.contains("BASE_REF:"));
        fs::remove_dir_all(root).unwrap();
    }
}
