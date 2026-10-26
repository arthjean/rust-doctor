//! Where a git pre-commit hook for a workspace belongs, read from the
//! repository the way git itself would run it.
//!
//! The binary writes the hook; this answers where, because every git call of
//! the crate goes through [`crate::git`], with the variables that could point
//! it at another repository removed.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::git::{GitCall, GitFailure, HOOKS_OFF, OUTPUT_TOO_LARGE, git_arguments, run_git};

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
    let toplevel = toplevel(workspace)?;
    let setup = if query(workspace, &["config", "--get", "core.hooksPath"]).is_some() {
        HookSetup::HooksPath(hooks_directory(workspace)?)
    } else if toplevel.join(".cargo-husky/hooks").is_dir() {
        HookSetup::CargoHusky(toplevel.join(".cargo-husky/hooks"))
    } else if let Some(manager) = [
        ".pre-commit-config.yaml",
        "lefthook.yml",
        "lefthook.yaml",
        ".lefthook.yml",
        ".lefthook.yaml",
    ]
    .into_iter()
        .find(|config| toplevel.join(config).is_file())
    {
        HookSetup::Manager(manager)
    } else {
        HookSetup::Default(hooks_directory(workspace)?)
    };
    Some(HookLocation { toplevel, setup })
}

/// The top of the working tree `workspace` sits in, or `None` outside one.
/// `ci install` writes its workflow there, where GitHub reads it.
pub fn toplevel(workspace: &Path) -> Option<PathBuf> {
    query(workspace, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

/// Where an installer may write: git's toplevel, checked against the disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confined {
    /// The toplevel, canonical.
    pub toplevel: PathBuf,
    /// The workspace relative to it, empty at the toplevel itself.
    pub workspace: PathBuf,
}

/// Why git's answer is not where an installer may write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unconfined {
    /// The workspace is not inside the toplevel git reports.
    Outside { workspace: PathBuf, toplevel: PathBuf },
    /// The nearest `.git` entry above the workspace, if any, is not at the
    /// toplevel git reports.
    Moved { repository: Option<PathBuf>, toplevel: PathBuf },
}

impl fmt::Display for Unconfined {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shown = |path: &Path| crate::terminal_text::sanitize(&path.display().to_string());
        match self {
            Self::Outside {
                workspace,
                toplevel,
            } => write!(
                formatter,
                "the workspace {} is not inside the repository {}",
                shown(workspace),
                shown(toplevel)
            ),
            Self::Moved {
                repository: Some(repository),
                toplevel,
            } => write!(
                formatter,
                "git reports the repository at {}, but the nearest .git above the workspace is in {}",
                shown(toplevel),
                shown(repository)
            ),
            Self::Moved {
                repository: None,
                toplevel,
            } => write!(
                formatter,
                "git reports the repository at {}, but no .git sits above the workspace",
                shown(toplevel)
            ),
        }
    }
}

/// Checks git's `toplevel` for `workspace` against the filesystem before an
/// installer writes under it: the workspace must sit inside it, and the
/// nearest ancestor holding a `.git` directory or gitfile must be it.
///
/// Git derives the toplevel from configuration the tree can carry: a
/// `core.worktree` in a `.git/config` that came with an archive, or an
/// embedded repository made of tracked files. Either moves the toplevel onto
/// another tree, or onto `/` with surplus `..` segments, and every write an
/// installer confines to it with it. A linked worktree and a submodule hold a
/// gitfile where git reports their toplevel, and pass.
pub fn confine(workspace: &Path, toplevel: &Path) -> Result<Confined, Unconfined> {
    let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let workspace = canonical(workspace);
    let toplevel = canonical(toplevel);
    let Ok(relative) = workspace.strip_prefix(&toplevel) else {
        return Err(Unconfined::Outside {
            workspace,
            toplevel,
        });
    };
    let repository = workspace.ancestors().find(|ancestor| {
        ancestor
            .join(".git")
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_dir() || metadata.is_file())
    });
    if repository != Some(toplevel.as_path()) {
        return Err(Unconfined::Moved {
            repository: repository.map(Path::to_path_buf),
            toplevel,
        });
    }
    Ok(Confined {
        workspace: relative.to_path_buf(),
        toplevel,
    })
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
    // Every scan call switches hooks off, which would make git answer
    // `/dev/null` for the directory this installer writes into. A query runs
    // `rev-parse`, which starts no hook.
    if let Some(at) = arguments.iter().position(|argument| argument == HOOKS_OFF) {
        arguments.drain(at.saturating_sub(1)..=at);
    }
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
        None,
    )
    .ok()?;
    let answer = String::from_utf8(answer).ok()?;
    let answer = answer.trim_end_matches('\n');
    (!answer.is_empty() && !answer.contains('\n')).then(|| answer.to_owned())
}
