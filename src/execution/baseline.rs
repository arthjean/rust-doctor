//! The dual run a `--scope baseline` comparison needs: the same plan, the same
//! configuration and the same toolchain applied to a snapshot of the comparison
//! base and to the working tree, in that order.
//!
//! The order is the whole point. A baseline side that did not complete cannot
//! be subtracted from anything, so the current side is never even scanned when
//! it fails: a delta computed against a partial baseline reports findings older
//! than the branch as introduced by it.

use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::bounded_read::collect_bounded;
use crate::internal_error::InternalError;
use crate::policy::PolicyPlan;
use crate::scan_target;

use super::{CommandEnvironment, ExecutionContext, ExecutionResult, PreparedInspection, Programs};

/// What `rustup show active-toolchain` may print before the answer is refused.
///
/// It prints one short line. A call that floods the pipe is a call whose answer
/// `accepted` would turn down anyway, and reading it under a bound is what
/// keeps a hostile `rust-toolchain.toml` from choosing this process's memory.
const RUSTUP_OUTPUT_LIMIT: usize = 4_096;

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
    let toolchain =
        match super::resolve_toolchain(&programs, prepared.workspace_root(), &environment) {
            Ok(toolchain) => toolchain,
            Err(error) => return Err(Box::new(prepared.fail(error))),
        };
    // Both sides are measured under the current configuration: a threshold
    // moved between the two commits must not report the move as a finding.
    let context = ExecutionContext {
        programs: &programs,
        plan,
        settings: &prepared.configuration.structure,
        environment: &environment,
    };
    let baseline = context.run(
        baseline_target,
        toolchain.clone(),
        Some(baseline_target_dir),
    );
    if !baseline.is_complete() {
        let mut current = prepared.fail(crate::baseline::scan_incomplete());
        current.toolchain = Some(toolchain);
        return Err(Box::new(current));
    }
    let current = context.run(prepared.target, toolchain, None);
    Ok(BaselineExecution { baseline, current })
}

/// The toolchain both sides are pinned to, or the refusal to compare at all.
///
/// An inherited `RUSTUP_TOOLCHAIN` is authoritative and never falls back to
/// rustup: it is what the caller already runs under, and silently scanning the
/// baseline under a different one is exactly the drift this pin exists to stop.
fn command_environment(workspace_root: &Path) -> Result<CommandEnvironment, InternalError> {
    let inherited = env::var_os("RUSTUP_TOOLCHAIN");
    let toolchain = match inherited.as_deref() {
        // An inherited pin is taken whole. A value carrying a space is one
        // rustup would not accept either, and trimming it to its first field
        // would run both sides under a toolchain nobody named.
        Some(inherited) => inherited.to_str().and_then(accepted),
        None => {
            let answer = active_toolchain(workspace_root);
            answer
                .as_deref()
                .and_then(|answer| answer.split_whitespace().next())
                .and_then(accepted)
        }
    };
    toolchain
        .map(|rustup_toolchain| CommandEnvironment {
            rustup_toolchain: Some(rustup_toolchain),
        })
        .ok_or_else(crate::baseline::scan_incomplete)
}

/// Asks rustup which toolchain this workspace resolves to.
///
/// The answer is `<name> (<why>)`, so only its first field is a toolchain.
fn active_toolchain(workspace_root: &Path) -> Option<String> {
    let mut child = Command::new("rustup")
        .args(["show", "active-toolchain"])
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    // Drained before the wait: a rustup that filled the pipe would never exit,
    // and this process would wait on it forever.
    let output = collect_bounded(stdout, RUSTUP_OUTPUT_LIMIT).ok()?;
    if output.exceeded || !child.wait().ok()?.success() {
        return None;
    }
    String::from_utf8(output.bytes).ok()
}

/// A toolchain name this crate will pass on to a child process.
///
/// Only ASCII alphanumerics and `-`, `_`, `.` are accepted, so every value that
/// gets through is valid UTF-8 by construction. That is why both sources are
/// read as text: preserving bytes no accepted value can contain used to buy a
/// `#[cfg(unix)]` split over `OsStr` and nothing else.
fn accepted(toolchain: &str) -> Option<OsString> {
    (!toolchain.is_empty()
        && toolchain.len() <= 255
        && toolchain
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    .then(|| OsString::from(toolchain))
}

#[cfg(test)]
mod tests {
    use super::accepted;

    #[test]
    fn only_a_bare_toolchain_name_is_passed_on_to_a_child() {
        assert_eq!(
            accepted("1.97.1-x86_64-unknown-linux-gnu"),
            Some("1.97.1-x86_64-unknown-linux-gnu".into())
        );
        assert_eq!(accepted("stable"), Some("stable".into()));
        assert_eq!(accepted(&"x".repeat(255)), Some("x".repeat(255).into()));

        // A whole value is judged whole: nothing that carries a separator, a
        // path, a shell character or a second field survives.
        for refused in [
            "",
            "stable extra",
            "sta ble; rm -rf /",
            "../../etc",
            "a/b",
            "$(id)",
            "nightly\n",
        ] {
            assert_eq!(accepted(refused), None, "{refused:?} was accepted");
        }
        assert_eq!(accepted(&"x".repeat(256)), None);
    }
}
