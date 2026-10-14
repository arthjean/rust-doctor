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

use super::side_cache::{self, CacheOutcome, CachedSide, KeyInputs, Lookup, SideCache};
use super::{
    CommandEnvironment, ExecutionContext, ExecutionResult, KnownLints, PreparedInspection,
    Programs, RunOptions, Toolchain,
};

/// What `rustup show active-toolchain` may print before the answer is refused.
///
/// It prints one short line. A call that floods the pipe is a call whose answer
/// `accepted` would turn down anyway, and reading it under a bound is what
/// keeps a hostile `rust-toolchain.toml` from choosing this process's memory.
const RUSTUP_OUTPUT_LIMIT: usize = 4_096;

#[derive(Debug)]
pub(crate) struct BaselineExecution {
    baseline: BaseSide,
    current: ExecutionResult,
    /// Where a complete base side this run computed is stored.
    cache: Option<SideCache>,
    outcome: CacheOutcome,
}

/// The base side: scanned now, or read back from the side cache.
#[derive(Debug)]
pub(crate) enum BaseSide {
    Ran(Box<ExecutionResult>),
    Cached(CachedSide),
}

impl BaselineExecution {
    pub(crate) fn into_parts(self) -> (BaseSide, ExecutionResult, Option<SideCache>, CacheOutcome) {
        (self.baseline, self.current, self.cache, self.outcome)
    }

    #[cfg(test)]
    pub(crate) fn into_sides(self) -> (BaseSide, ExecutionResult) {
        (self.baseline, self.current)
    }

    #[cfg(test)]
    pub(crate) fn from_complete_sides(baseline: ExecutionResult, current: ExecutionResult) -> Self {
        assert!(baseline.is_complete());
        Self {
            baseline: BaseSide::Ran(Box::new(baseline)),
            current,
            cache: None,
            outcome: CacheOutcome::Disabled,
        }
    }
}

/// Where one side of the comparison reads its sources and builds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Side<'a> {
    pub(crate) workspace: &'a Path,
    pub(crate) target_dir: &'a Path,
}

/// What a comparison settles before either side runs: the toolchain both are
/// pinned to, and whether the base side is already in the cache, which is what
/// decides whether a snapshot is needed at all.
#[derive(Debug)]
pub(crate) struct BaselineSetup {
    environment: CommandEnvironment,
    toolchain: Toolchain,
    lints: Option<KnownLints>,
    declared: std::collections::BTreeSet<String>,
    cache: Lookup,
}

impl BaselineSetup {
    /// Whether the base side has to be scanned, and so materialized.
    pub(crate) const fn needs_snapshot(&self) -> bool {
        !matches!(self.cache, Lookup::Hit(_))
    }
}

pub(crate) fn setup(
    prepared: &PreparedInspection,
    comparison_base: &str,
    plan: &PolicyPlan,
    options: &RunOptions,
) -> Result<BaselineSetup, InternalError> {
    let workspace_root = prepared.workspace_root();
    let environment = command_environment(workspace_root)?;
    let lint_list = plan.active_rules(crate::policy::Producer::Clippy).next().is_some();
    let (toolchain, lints) = super::resolve_toolchain(
        &Programs::default(),
        workspace_root,
        &environment,
        options,
        lint_list,
    )?;
    // Both sides are measured under the current configuration: a threshold
    // moved between the two commits must not report the move as a finding.
    let declared = crate::generated::declared(workspace_root, options.deadline);
    let cache = side_cache::lookup(
        &KeyInputs {
            workspace_root,
            target_directory: prepared.target_directory(),
            comparison_base,
            toolchain: &toolchain,
            environment: &environment,
            lints: lints.as_ref(),
            plan,
            settings: &prepared.configuration.structure,
            declared: &declared,
            options,
        },
        &prepared.target.metadata,
    );
    Ok(BaselineSetup {
        environment,
        toolchain,
        lints,
        declared,
        cache,
    })
}

/// Runs the base side, then the current one: the working tree, or, when
/// `staged` names one, the tree the index was written into. A base side the
/// cache holds is not run, and `base` is then never read.
pub(crate) fn execute(
    prepared: PreparedInspection,
    setup: BaselineSetup,
    base: Option<Side<'_>>,
    staged: Option<Side<'_>>,
    plan: &PolicyPlan,
    options: &RunOptions,
) -> Result<BaselineExecution, Box<ExecutionResult>> {
    let programs = Programs::default();
    let BaselineSetup {
        environment,
        toolchain,
        lints,
        declared,
        cache,
    } = setup;
    let context = ExecutionContext {
        programs: &programs,
        plan,
        settings: &prepared.configuration.structure,
        environment: &environment,
        options,
        declared: &declared,
    };
    let outcome = cache.outcome();
    let (baseline, cache) = match cache {
        Lookup::Hit(cached) => (BaseSide::Cached(cached), None),
        Lookup::Disabled | Lookup::Miss(_) => {
            let Some(base) = base else {
                return Err(Box::new(prepared.fail(crate::baseline::scan_incomplete())));
            };
            let target = crate::timings::timed(
                options.timings.as_ref(),
                crate::timings::Phase::Metadata,
                || {
                    scan_target::resolve_isolated(
                        base.workspace,
                        &programs.cargo,
                        base.target_dir,
                        environment.rustup_toolchain.as_deref(),
                        options.deadline,
                    )
                },
            );
            let Ok(target) = target else {
                return Err(Box::new(prepared.fail(crate::baseline::scan_incomplete())));
            };
            let baseline = context.run(target, toolchain.clone(), lints.clone(), Some(base.target_dir));
            if !baseline.is_complete() {
                let mut current = prepared.fail(crate::baseline::scan_incomplete());
                current.toolchain = Some(toolchain);
                return Err(Box::new(current));
            }
            let store = match cache {
                Lookup::Miss(store) => Some(store),
                Lookup::Disabled | Lookup::Hit(_) => None,
            };
            (BaseSide::Ran(Box::new(baseline)), store)
        }
    };
    let current = match staged {
        None => context.run(prepared.target, toolchain, lints, None),
        Some(staged) => match scan_target::resolve_isolated(
            staged.workspace,
            &programs.cargo,
            staged.target_dir,
            environment.rustup_toolchain.as_deref(),
            options.deadline,
        ) {
            Ok(target) => context.run(target, toolchain, lints, Some(staged.target_dir)),
            Err(failure) => return Err(Box::new(prepared.fail(failure.error))),
        },
    };
    Ok(BaselineExecution {
        baseline,
        current,
        cache,
        outcome,
    })
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
