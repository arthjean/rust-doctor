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
//! Nor does any call run a program the repository configures. Every filter
//! driver the effective configuration names, not only LFS, has its `clean`,
//! `smudge` and `process` blanked and `required` set to false, through
//! `GIT_CONFIG_COUNT` pairs rather than `-c`: git splits a `-c` argument at its
//! first `=`, so `-c filter.p=q.process=` leaves the driver `p=q` running. The
//! `filters` module reads the names and fails a call closed when it cannot.
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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;

use crate::bounded_read::collect_bounded;
use crate::execution::{RunDeadline, spawn_bounded};
use crate::internal_error::InternalError;

mod filters;
#[cfg(test)]
mod tests;

pub(crate) use filters::forget as forget_filter_drivers;

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

    pub(crate) const fn code(self) -> &'static str {
        self.code
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

/// The configuration that keeps every hook from running. Only the hook
/// installer, which asks git where hooks live, takes it back out.
pub(crate) const HOOKS_OFF: &str = "core.hooksPath=/dev/null";

/// Prefixes an operation with the configuration every call of this crate runs
/// under.
///
/// `safe.bareRepository=explicit` keeps git from binding to a git directory
/// the tree fabricates from tracked files (`HEAD`, `objects/`, `refs/`): only
/// a `--git-dir` this crate names reaches a repository that is not a `.git`.
/// Git honors the key only in protected configuration, which `-c` is.
///
/// [`HOOKS_OFF`] keeps every hook from running: a snapshot writes an index,
/// which starts `post-index-change`, and the repository's configuration names
/// where hooks live.
///
/// Filter drivers are switched off by `git_command` rather than here, through
/// environment pairs: see the `filters` module for why not `-c`.
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
        OsString::from("-c"),
        OsString::from("safe.bareRepository=explicit"),
        OsString::from("-c"),
        OsString::from(HOOKS_OFF),
        OsString::from("--no-pager"),
        OsString::from("-C"),
        // Absolute, since `git_command` also runs git in `workspace_root`: a
        // relative `-C` was resolved a second time from there, and
        // `crates/x` named `crates/x/crates/x`.
        std::path::absolute(workspace_root)
            .unwrap_or_else(|_| workspace_root.to_path_buf())
            .into_os_string(),
    ]
    .into_iter()
    .chain(operation.into_iter().map(Into::into))
    .collect()
}

/// `checkout-index` of every index entry under `prefix`, a snapshot's tree.
pub(crate) fn checkout_index_arguments(workspace_root: &Path, prefix: &Path) -> Vec<OsString> {
    let mut destination = OsString::from("--prefix=");
    destination.push(prefix.as_os_str());
    destination.push(std::path::MAIN_SEPARATOR_STR);
    git_arguments(
        workspace_root,
        [
            OsString::from("checkout-index"),
            OsString::from("--all"),
            OsString::from("--force"),
            destination,
        ],
    )
}

/// A snapshot seen through git: the repository's own git directory, the index
/// the snapshot was checked out from, and the snapshot as work tree.
///
/// A snapshot has no `.git` of its own, so a pass that asks git what it tracks
/// found nothing there: the repository pass judged the working tree and never
/// the commit or the index a baseline or a staged scan names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitView {
    /// The repository's `.git`, a directory or the file a linked work tree
    /// or a submodule holds: git resolves the second as it resolves the first.
    pub(crate) git_dir: PathBuf,
    pub(crate) index: PathBuf,
    pub(crate) work_tree: PathBuf,
}

impl GitView {
    /// Where `path`, inside the snapshot, sits in the repository's working
    /// tree: what a fact about the machine rather than the commit is read
    /// from, such as whether a build has written an ignored lockfile.
    pub(crate) fn working_path(&self, path: &Path) -> Option<PathBuf> {
        let relative = path.strip_prefix(&self.work_tree).ok()?;
        Some(self.git_dir.parent()?.join(relative))
    }
}

/// [`git_arguments`], run through `view` when there is one.
pub(crate) fn view_arguments<T: Into<OsString>, const N: usize>(
    workspace_root: &Path,
    view: Option<&GitView>,
    operation: [T; N],
) -> Vec<OsString> {
    let Some(view) = view else {
        return git_arguments(workspace_root, operation);
    };
    let mut git_dir = OsString::from("--git-dir=");
    git_dir.push(&view.git_dir);
    let mut work_tree = OsString::from("--work-tree=");
    work_tree.push(&view.work_tree);
    let mut arguments = git_arguments(workspace_root, [git_dir, work_tree]);
    arguments.extend(operation.into_iter().map(Into::into));
    arguments
}

/// Runs a call and answers its bounded stdout.
///
/// Stderr is read and discarded rather than returned: it is where git writes
/// URLs and credential prompts, and no diagnostic of this crate carries it.
/// A call made inside a scan carries the run's deadline, and a git still
/// running when it passes is killed and answers `deadline-exceeded` at the
/// call's stage.
pub(crate) fn run_git(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
    deadline: Option<RunDeadline>,
) -> Result<Vec<u8>, InternalError> {
    run_git_configured(git, workspace_root, call, None, None, deadline)
}

pub(crate) fn run_git_with_index(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
    index: &Path,
    deadline: Option<RunDeadline>,
) -> Result<Vec<u8>, InternalError> {
    run_git_configured(git, workspace_root, call, Some(index), None, deadline)
}

/// Runs a call that reads `input` on its stdin, as `check-attr --stdin` does.
/// The input is written on a thread of its own, so a git that answers before
/// it has read everything cannot block this process on a full pipe.
pub(crate) fn run_git_with_input(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
    input: Vec<u8>,
    deadline: Option<RunDeadline>,
) -> Result<Vec<u8>, InternalError> {
    run_git_configured(git, workspace_root, call, None, Some(input), deadline)
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
    index: Option<&Path>,
    deadline: Option<RunDeadline>,
) -> Option<i32> {
    // The enumeration's outcome is only ever "no answer" here, so its call
    // names no stage a caller would read.
    let enumeration = GitCall {
        arguments: Vec::new(),
        stdout_limit: filters::ENUMERATION_LIMIT,
        stage: "git",
        failure: UNAVAILABLE,
        overflow: OUTPUT_TOO_LARGE,
    };
    let drivers = drivers(git, workspace_root, arguments, &enumeration, deadline).ok()?;
    let pairs = filters::neutralizing_pairs(&drivers);
    let mut command = git_command(git, workspace_root, arguments, &pairs);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let (status, expired) = spawn_bounded(command, deadline).ok()?.wait();
    status.ok().filter(|_| !expired).and_then(|status| status.code())
}

fn run_git_configured(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
    index: Option<&Path>,
    input: Option<Vec<u8>>,
    deadline: Option<RunDeadline>,
) -> Result<Vec<u8>, InternalError> {
    let drivers = drivers(git, workspace_root, &call.arguments, call, deadline)?;
    let pairs = filters::neutralizing_pairs(&drivers);
    let command = git_command(git, workspace_root, &call.arguments, &pairs);
    run_command(command, call, index, input, deadline)
}

/// The filter drivers the repository `arguments` select configures, read
/// under the same global options on the first call of the scan that names
/// them. A failed enumeration is the call's failure, so a call whose drivers
/// are unknown never runs.
fn drivers(
    git: &Path,
    workspace_root: &Path,
    arguments: &[OsString],
    call: &GitCall,
    deadline: Option<RunDeadline>,
) -> Result<Arc<[String]>, InternalError> {
    let location = filters::location(arguments);
    let key = [git.as_os_str(), workspace_root.as_os_str()]
        .into_iter()
        .map(ToOwned::to_owned)
        .chain(location.iter().cloned())
        .collect();
    filters::drivers(key, call, || {
        let arguments = filters::enumeration_arguments(location);
        let command = git_command(git, workspace_root, &arguments, &filters::canary_pairs());
        let enumeration = GitCall {
            arguments: Vec::new(),
            stdout_limit: filters::ENUMERATION_LIMIT,
            stage: call.stage,
            failure: call.failure,
            overflow: call.overflow,
        };
        run_command(command, &enumeration, None, None, deadline)
    })
}

fn run_command(
    mut command: Command,
    call: &GitCall,
    index: Option<&Path>,
    input: Option<Vec<u8>>,
    deadline: Option<RunDeadline>,
) -> Result<Vec<u8>, InternalError> {
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let unavailable = || UNAVAILABLE.error(call.stage);
    let failure = || call.failure.error(call.stage);
    let mut bounded = spawn_bounded(command, deadline).map_err(|_| unavailable())?;
    let child = &mut bounded.child;
    // Dropping the handle once written is what tells git the input ended.
    let writer = match (input, child.stdin.take()) {
        (Some(input), Some(mut stdin)) => {
            Some(thread::spawn(move || stdin.write_all(&input)))
        }
        _ => None,
    };
    let stdout = child.stdout.take().ok_or_else(unavailable)?;
    let stderr = child.stderr.take().ok_or_else(unavailable)?;
    let stdout_limit = call.stdout_limit;
    // Both streams are drained while the child runs. Waiting first would
    // deadlock the moment git fills a pipe buffer it cannot flush.
    let stdout_reader = thread::spawn(move || collect_bounded(stdout, stdout_limit));
    let stderr_reader = thread::spawn(move || collect_bounded(stderr, STDERR_OUTPUT_LIMIT));
    let (status, expired) = bounded.wait();
    if let Some(deadline) = deadline.filter(|_| expired) {
        return Err(InternalError::new(
            call.stage,
            crate::report::DEADLINE_EXCEEDED,
            deadline.message(),
        ));
    }
    let status = status.map_err(|_| failure())?;
    writer.map(|writer| joined(writer, failure)).transpose()?;
    let stdout = joined(stdout_reader, failure)?;
    let stderr = joined(stderr_reader, failure)?;

    if stdout.exceeded || stderr.exceeded {
        return Err(call.overflow.error(call.stage));
    }
    if !status.success() {
        return Err(failure());
    }

    Ok(stdout.bytes)
}

/// What a writer or reader thread answered, its panic or its I/O error being
/// the call's failure.
fn joined<T, E>(
    handle: thread::JoinHandle<Result<T, E>>,
    failure: impl Fn() -> InternalError,
) -> Result<T, InternalError> {
    handle.join().map_err(|_| failure())?.map_err(|_| failure())
}

/// The command every call runs as: in the workspace, with closed stdin, the
/// variables that would redirect it removed, and `pairs` set as command-scope
/// configuration once the inherited `GIT_CONFIG_COUNT` is gone.
fn git_command(
    git: &Path,
    workspace_root: &Path,
    arguments: &[OsString],
    pairs: &[(String, &str)],
) -> Command {
    let mut command = Command::new(git);
    command
        .args(arguments)
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    for variable in GIT_ENVIRONMENT_OVERRIDES {
        command.env_remove(variable);
    }
    command.env("GIT_CONFIG_COUNT", pairs.len().to_string());
    for (index, (key, value)) in pairs.iter().enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{index}"), key)
            .env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    command
}
