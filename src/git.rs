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

#[derive(Debug)]
pub(crate) struct GitCall {
    pub(crate) arguments: Vec<OsString>,
    pub(crate) stdout_limit: usize,
    pub(crate) stage: &'static str,
    pub(crate) failure: GitFailure,
    pub(crate) overflow: GitFailure,
}

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

pub(crate) const HOOKS_OFF: &str = "core.hooksPath=/dev/null";

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
        std::path::absolute(workspace_root)
            .unwrap_or_else(|_| workspace_root.to_path_buf())
            .into_os_string(),
    ]
    .into_iter()
    .chain(operation.into_iter().map(Into::into))
    .collect()
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitView {
    pub(crate) git_dir: PathBuf,
    pub(crate) index: PathBuf,
    pub(crate) work_tree: PathBuf,
}

impl GitView {
    pub(crate) fn working_path(&self, path: &Path) -> Option<PathBuf> {
        let relative = path.strip_prefix(&self.work_tree).ok()?;
        Some(self.git_dir.parent()?.join(relative))
    }
}

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

pub(crate) fn run_git_with_input(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
    input: Vec<u8>,
    deadline: Option<RunDeadline>,
) -> Result<Vec<u8>, InternalError> {
    run_git_configured(git, workspace_root, call, None, Some(input), deadline)
}

pub(crate) fn run_git_status(
    git: &Path,
    workspace_root: &Path,
    arguments: &[OsString],
    index: Option<&Path>,
    deadline: Option<RunDeadline>,
) -> Option<i32> {
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
    let writer = match (input, child.stdin.take()) {
        (Some(input), Some(mut stdin)) => {
            Some(thread::spawn(move || stdin.write_all(&input)))
        }
        _ => None,
    };
    let stdout = child.stdout.take().ok_or_else(unavailable)?;
    let stderr = child.stderr.take().ok_or_else(unavailable)?;
    let stdout_limit = call.stdout_limit;
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

fn joined<T, E>(
    handle: thread::JoinHandle<Result<T, E>>,
    failure: impl Fn() -> InternalError,
) -> Result<T, InternalError> {
    handle.join().map_err(|_| failure())?.map_err(|_| failure())
}

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
