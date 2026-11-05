use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::git::{GitCall, GitFailure, HOOKS_OFF, OUTPUT_TOO_LARGE, git_arguments, run_git};

const STAGE: &str = "hook";
const ANSWER_LIMIT: usize = 4_096;
const REFUSED: GitFailure = GitFailure::new("git-refused", "Git refused the query.");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookSetup {
    HooksPath(PathBuf),
    CargoHusky(PathBuf),
    Manager(&'static str),
    Default(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookLocation {
    pub toplevel: PathBuf,
    pub setup: HookSetup,
}

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

pub fn toplevel(workspace: &Path) -> Option<PathBuf> {
    query(workspace, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confined {
    pub toplevel: PathBuf,
    pub workspace: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unconfined {
    Outside { workspace: PathBuf, toplevel: PathBuf },
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

fn hooks_directory(workspace: &Path) -> Option<PathBuf> {
    query(
        workspace,
        &["rev-parse", "--path-format=absolute", "--git-path", "hooks"],
    )
    .map(PathBuf::from)
}

fn query(workspace: &Path, operation: &[&str]) -> Option<String> {
    let mut arguments = git_arguments(workspace, [] as [&str; 0]);
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
