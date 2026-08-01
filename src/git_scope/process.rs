use std::ffi::OsString;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use crate::execution::InternalError;

pub(crate) const STDERR_OUTPUT_LIMIT: usize = 65_536;

pub(super) const GIT_ENVIRONMENT_OVERRIDES: [&str; 10] = [
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
    pub(crate) failure: GitFailure,
    pub(crate) stdout_overflow: GitFailure,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GitFailure {
    stage: &'static str,
    code: &'static str,
    message: &'static str,
}

impl GitFailure {
    pub(crate) const fn new(
        stage: &'static str,
        code: &'static str,
        message: &'static str,
    ) -> Self {
        Self {
            stage,
            code,
            message,
        }
    }

    pub(crate) fn error(self) -> InternalError {
        InternalError::new(self.stage, self.code, self.message)
    }
}

pub(super) const GIT_OUTPUT_TOO_LARGE: GitFailure = GitFailure::new(
    "scope",
    "git-output-too-large",
    "Git output exceeds the supported limit.",
);

#[derive(Debug)]
pub(crate) struct GitOutput {
    pub(crate) stdout: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct BoundedOutput {
    pub(super) bytes: Vec<u8>,
    pub(super) exceeded: bool,
}

pub(crate) fn git_arguments<const N: usize>(
    workspace_root: &Path,
    operation: [OsString; N],
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
    .chain(operation)
    .collect()
}

pub(crate) fn run_git(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
) -> Result<GitOutput, InternalError> {
    run_git_configured(git, workspace_root, call, None)
}

pub(crate) fn run_git_with_index(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
    index: &Path,
) -> Result<GitOutput, InternalError> {
    run_git_configured(git, workspace_root, call, Some(index))
}

fn run_git_configured(
    git: &Path,
    workspace_root: &Path,
    call: &GitCall,
    index: Option<&Path>,
) -> Result<GitOutput, InternalError> {
    let mut command = git_command(git, workspace_root, &call.arguments);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let mut child = command.spawn().map_err(|_| git_unavailable())?;
    let stdout = child.stdout.take().ok_or_else(git_unavailable)?;
    let stderr = child.stderr.take().ok_or_else(git_unavailable)?;
    let stdout_limit = call.stdout_limit;
    let stdout_reader = thread::spawn(move || collect_bounded(stdout, stdout_limit));
    let stderr_reader = thread::spawn(move || collect_bounded(stderr, STDERR_OUTPUT_LIMIT));
    let status = child.wait().map_err(|_| call.failure.error())?;
    let stdout = stdout_reader
        .join()
        .map_err(|_| call.failure.error())?
        .map_err(|_| call.failure.error())?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| call.failure.error())?
        .map_err(|_| call.failure.error())?;

    if stdout.exceeded {
        return Err(call.stdout_overflow.error());
    }
    if stderr.exceeded {
        return Err(output_too_large());
    }
    if !status.success() {
        return Err(call.failure.error());
    }

    Ok(GitOutput {
        stdout: stdout.bytes,
    })
}

pub(super) fn git_command(git: &Path, workspace_root: &Path, arguments: &[OsString]) -> Command {
    let mut command = Command::new(git);
    command
        .args(arguments)
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    for variable in GIT_ENVIRONMENT_OVERRIDES {
        command.env_remove(variable);
    }
    command
}

pub(super) fn collect_bounded(mut reader: impl Read, limit: usize) -> io::Result<BoundedOutput> {
    let mut bytes = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0_u8; 8_192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if !exceeded {
            let remaining = limit.saturating_sub(bytes.len());
            let kept = remaining.min(read);
            bytes.extend_from_slice(&buffer[..kept]);
            exceeded = kept < read;
        }
    }
    Ok(BoundedOutput { bytes, exceeded })
}

pub(super) fn output_too_large() -> InternalError {
    GIT_OUTPUT_TOO_LARGE.error()
}

fn git_unavailable() -> InternalError {
    InternalError::new("scope", "git-unavailable", "Git could not be started.")
}
