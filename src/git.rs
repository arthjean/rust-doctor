//! The bounded git process layer every producer that shells out to git runs
//! through.
//!
//! Three stages reach it: `scope` resolves the comparison base and the changed
//! files, `baseline` materializes a snapshot of a commit, and `repo` enumerates
//! what git tracks. None of them compiles anything; all of them read output
//! that a hostile repository can make arbitrarily large, which is why every
//! stream is read under a bound and both streams are drained on their own
//! thread rather than after the child exits.
//!
//! One rule holds the module together: **a call names the stage every one of
//! its outcomes is reported at**. `GitCall::stage` is that name, and the four
//! exits (git could not start, stdout overflowed, stderr overflowed, the call
//! failed) all build their `InternalError` from it. The failure constants used
//! to carry a stage of their own, and the two exits no caller could name were
//! hard-coded to `scope`: a baseline snapshot whose git flooded stderr reported
//! `stage: "scope"` in the published JSON, at a bound `baseline` publishes as
//! one of its own limits.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use crate::bounded_read::collect_bounded;
use crate::internal_error::InternalError;

#[cfg(test)]
mod tests;

pub(crate) const STDERR_OUTPUT_LIMIT: usize = 65_536;

/// The variables that would redirect a call away from the workspace it names.
///
/// They are removed rather than overridden, so an inherited value cannot
/// survive as an empty string that git still reads.
const GIT_ENVIRONMENT_OVERRIDES: [&str; 10] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_EXTERNAL_DIFF",
    "GIT_PAGER",
];

/// One invocation of git, with the stage and the two failures its outcomes are
/// reported under.
#[derive(Debug)]
pub(crate) struct GitCall {
    pub(crate) arguments: Vec<OsString>,
    pub(crate) stdout_limit: usize,
    /// The stage every outcome of this call is reported at, including the two
    /// the caller does not name: a git that could not be started, and a stream
    /// that overflowed.
    pub(crate) stage: &'static str,
    pub(crate) failure: GitFailure,
    /// Reported when either stream exceeds its bound. One overflow outcome
    /// covers both, since a caller that cannot use the answer does not care
    /// which stream made it unusable.
    pub(crate) overflow: GitFailure,
}

/// A code and a message, stamped with the stage of the call that raised it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GitFailure {
    code: &'static str,
    message: &'static str,
}

impl GitFailure {
    pub(crate) const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }

    pub(crate) fn error(self, stage: &'static str) -> InternalError {
        InternalError::new(stage, self.code, self.message)
    }
}

pub(crate) const OUTPUT_TOO_LARGE: GitFailure = GitFailure::new(
    "git-output-too-large",
    "Git output exceeds the supported limit.",
);

const UNAVAILABLE: GitFailure = GitFailure::new("git-unavailable", "Git could not be started.");

/// Prefixes an operation with the configuration every call of this crate runs
/// under.
///
/// The operation is written in whatever string type the call site already
/// holds, so a fixed spelling stays a literal and only a computed argument
/// pays for an `OsString`.
pub(crate) fn git_arguments<T: Into<OsString>, const N: usize>(
    workspace_root: &Path,
    operation: [T; N],
) -> Vec<OsString> {
    [
        OsString::from("-c"),
        OsString::from("color.ui=false"),
        OsString::from("-c"),
        OsString::from("core.fsmonitor=false"),
        OsString::from("--no-pager"),
        OsString::from("-C"),
        workspace_root.as_os_str().to_owned(),
    ]
    .into_iter()
    .chain(operation.into_iter().map(Into::into))
    .collect()
}

/// Runs a call and answers its bounded stdout.
///
/// Stderr is read and discarded rather than returned: it is where git writes
/// URLs and credential prompts, and no diagnostic of this crate carries it.
pub(crate) fn run_git(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
) -> Result<Vec<u8>, InternalError> {
    run_git_configured(git, workspace_root, call, None)
}

pub(crate) fn run_git_with_index(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
    index: &Path,
) -> Result<Vec<u8>, InternalError> {
    run_git_configured(git, workspace_root, call, Some(index))
}

/// Runs a call whose answer is its exit code rather than its output.
///
/// Both streams are closed at the pipe, so a caller that reads an exit code
/// cannot buy an unbounded read the way `Command::output` would give it. This
/// is why `git_command` is private: every way out of this module is bounded.
pub(crate) fn run_git_status(
    git: &Path,
    workspace_root: &Path,
    arguments: &[OsString],
) -> Option<i32> {
    git_command(git, workspace_root, arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .and_then(|status| status.code())
}

fn run_git_configured(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
    index: Option<&Path>,
) -> Result<Vec<u8>, InternalError> {
    let mut command = git_command(git, workspace_root, &call.arguments);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let unavailable = || UNAVAILABLE.error(call.stage);
    let failure = || call.failure.error(call.stage);
    let mut child = command.spawn().map_err(|_| unavailable())?;
    let stdout = child.stdout.take().ok_or_else(unavailable)?;
    let stderr = child.stderr.take().ok_or_else(unavailable)?;
    let stdout_limit = call.stdout_limit;
    // Both streams are drained while the child runs. Waiting first would
    // deadlock the moment git fills a pipe buffer it cannot flush.
    let stdout_reader = thread::spawn(move || collect_bounded(stdout, stdout_limit));
    let stderr_reader = thread::spawn(move || collect_bounded(stderr, STDERR_OUTPUT_LIMIT));
    let status = child.wait().map_err(|_| failure())?;
    let stdout = stdout_reader
        .join()
        .map_err(|_| failure())?
        .map_err(|_| failure())?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| failure())?
        .map_err(|_| failure())?;

    if stdout.exceeded || stderr.exceeded {
        return Err(call.overflow.error(call.stage));
    }
    if !status.success() {
        return Err(failure());
    }

    Ok(stdout.bytes)
}

fn git_command(git: &Path, workspace_root: &Path, arguments: &[OsString]) -> Command {
    let mut command = Command::new(git);
    command
        .args(arguments)
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    for variable in GIT_ENVIRONMENT_OVERRIDES {
        command.env_remove(variable);
    }
    command
}
