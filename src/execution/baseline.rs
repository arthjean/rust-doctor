use std::env;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::policy::PolicyPlan;
use crate::scan_target;

use super::{
    CommandEnvironment, ExecutionResult, PreparedInspection, Programs, execute_target,
    resolve_toolchain,
};

#[derive(Debug)]
pub(crate) struct BaselineExecution {
    baseline: ExecutionResult,
    current: ExecutionResult,
}

impl BaselineExecution {
    pub(crate) fn into_sides(self) -> (ExecutionResult, ExecutionResult) {
        (self.baseline, self.current)
    }

    #[cfg(test)]
    pub(crate) fn from_complete_sides(baseline: ExecutionResult, current: ExecutionResult) -> Self {
        assert!(baseline.is_complete());
        Self { baseline, current }
    }
}

pub(crate) fn execute(
    prepared: PreparedInspection,
    baseline_workspace: &Path,
    baseline_target_dir: &Path,
    plan: &PolicyPlan,
) -> Result<BaselineExecution, Box<ExecutionResult>> {
    let programs = Programs::default();
    let environment = match command_environment(prepared.workspace_root()) {
        Ok(environment) => environment,
        Err(error) => return Err(Box::new(prepared.fail(error))),
    };
    let baseline_target = match scan_target::resolve_isolated(
        baseline_workspace,
        &programs.cargo,
        baseline_target_dir,
        environment.rustup_toolchain.as_deref(),
    ) {
        Ok(target) => target,
        Err(_) => {
            return Err(Box::new(prepared.fail(crate::baseline::scan_incomplete())));
        }
    };
    let toolchain = match resolve_toolchain(&programs, prepared.workspace_root(), &environment) {
        Ok(toolchain) => toolchain,
        Err(error) => return Err(Box::new(prepared.fail(error))),
    };
    // Both sides are measured under the current configuration: a threshold
    // moved between the two commits must not report the move as a finding.
    let settings = prepared.configuration.structure;
    let baseline = execute_target(
        baseline_target,
        &programs,
        plan,
        &settings,
        toolchain.clone(),
        Some(baseline_target_dir),
        &environment,
    );
    if !baseline.is_complete() {
        let mut current = prepared.fail(crate::baseline::scan_incomplete());
        current.toolchain = toolchain;
        return Err(Box::new(current));
    }
    let current = execute_target(
        prepared.target,
        &programs,
        plan,
        &settings,
        toolchain,
        None,
        &environment,
    );
    Ok(BaselineExecution { baseline, current })
}

fn command_environment(
    workspace_root: &Path,
) -> Result<CommandEnvironment, crate::internal_error::InternalError> {
    if let Some(toolchain) = env::var_os("RUSTUP_TOOLCHAIN") {
        return valid_toolchain(&toolchain)
            .then_some(CommandEnvironment {
                rustup_toolchain: Some(toolchain),
            })
            .ok_or_else(crate::baseline::scan_incomplete);
    }

    let output = Command::new("rustup")
        .args(["show", "active-toolchain"])
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    if let Ok(output) = output
        && output.status.success()
        && let Some(toolchain) = output
            .stdout
            .split(|byte| byte.is_ascii_whitespace())
            .find(|field| !field.is_empty())
            .map(os_str_from_bytes)
        && valid_toolchain(toolchain)
    {
        return Ok(CommandEnvironment {
            rustup_toolchain: Some(toolchain.to_owned()),
        });
    }
    Err(crate::baseline::scan_incomplete())
}

#[cfg(unix)]
fn os_str_from_bytes(bytes: &[u8]) -> &OsStr {
    use std::os::unix::ffi::OsStrExt;

    OsStr::from_bytes(bytes)
}

#[cfg(not(unix))]
fn os_str_from_bytes(bytes: &[u8]) -> &OsStr {
    std::str::from_utf8(bytes).map_or_else(|_| OsStr::new(""), OsStr::new)
}

fn valid_toolchain(toolchain: &OsStr) -> bool {
    toolchain.to_str().is_some_and(|toolchain| {
        !toolchain.is_empty()
            && toolchain.len() <= 255
            && toolchain
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    })
}
