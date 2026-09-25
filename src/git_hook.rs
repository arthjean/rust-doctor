//! Where a git pre-commit hook for a workspace belongs, read from the
//! repository the way git itself would run it.
//!
//! The binary writes the hook; this answers where, because every git call of
//! the crate goes through [`crate::git`], with the variables that could point
//! it at another repository removed.

use std::path::{Path, PathBuf};

use crate::git::{GitCall, GitFailure, OUTPUT_TOO_LARGE, git_arguments, run_git};

const STAGE: &str = "hook";
const ANSWER_LIMIT: usize = 4_096;
const REFUSED: GitFailure = GitFailure::new("git-refused", "Git refused the query.");

/// How the repository runs its hooks, in the order `hook install git` honors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookSetup {
    /// `core.hooksPath` is set: the hook goes into that directory.
    HooksPath(PathBuf),
    /// `.cargo-husky/hooks/` exists: cargo-husky copies it into `.git/hooks`.
    CargoHusky(PathBuf),
    /// A hook manager whose configuration file this tool never edits.
    Manager(&'static str),
    /// Git's own hook directory.
    Default(PathBuf),
}

/// The repository a workspace sits in, and how it runs its hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookLocation {
    /// The top of the working tree, where git starts a hook.
    pub toplevel: PathBuf,
    pub setup: HookSetup,
}

/// Answers where a pre-commit hook for `workspace` belongs, or `None` when the
/// workspace is not inside a git working tree.
pub fn locate(workspace: &Path) -> Option<HookLocation> {
    let toplevel = PathBuf::from(query(workspace, &["rev-parse", "--show-toplevel"])?);
    let setup = if query(workspace, &["config", "--get", "core.hooksPath"]).is_some() {
        HookSetup::HooksPath(hooks_directory(workspace)?)
    } else if toplevel.join(".cargo-husky/hooks").is_dir() {
        HookSetup::CargoHusky(toplevel.join(".cargo-husky/hooks"))
    } else if let Some(manager) = [".pre-commit-config.yaml", "lefthook.yml"]
        .into_iter()
        .find(|config| toplevel.join(config).is_file())
    {
        HookSetup::Manager(manager)
    } else {
        HookSetup::Default(hooks_directory(workspace)?)
    };
    Some(HookLocation { toplevel, setup })
}

/// The directory git reads hooks from: `core.hooksPath` resolved the way git
/// resolves it, or the hooks directory of the common git directory, which is
/// where a linked worktree's hooks live too.
fn hooks_directory(workspace: &Path) -> Option<PathBuf> {
    query(
        workspace,
        &["rev-parse", "--path-format=absolute", "--git-path", "hooks"],
    )
    .map(PathBuf::from)
}

/// One line of git's answer, or `None` when git refused or answered nothing.
fn query(workspace: &Path, operation: &[&str]) -> Option<String> {
    let mut arguments = git_arguments(workspace, [] as [&str; 0]);
    arguments.extend(operation.iter().map(Into::into));
    let answer = run_git(
        Path::new("git"),
        workspace,
        &GitCall {
            arguments,
            stdout_limit: ANSWER_LIMIT,
            stage: STAGE,
            failure: REFUSED,
            overflow: OUTPUT_TOO_LARGE,
        },
    )
    .ok()?;
    let answer = String::from_utf8(answer).ok()?;
    let answer = answer.trim_end_matches('\n');
    (!answer.is_empty() && !answer.contains('\n')).then(|| answer.to_owned())
}
