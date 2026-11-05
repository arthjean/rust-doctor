use std::path::Path;

use crate::git::GitView;
use crate::internal_error::InternalError;
use crate::policy::PolicyPlan;
use crate::scan_target;

use super::side_cache::{self, CacheOutcome, CachedSide, KeyInputs, Lookup, SideCache};
use super::toolchain::{self, NameOnlyPin};
use super::{
    CommandEnvironment, ExecutionContext, ExecutionResult, KnownLints, PreparedInspection,
    Programs, RunOptions, Toolchain,
};

#[derive(Debug)]
pub(crate) struct BaselineExecution {
    baseline: BaseSide,
    current: ExecutionResult,
    cache: Option<SideCache>,
    outcome: CacheOutcome,
}

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

#[derive(Debug, Clone, Copy)]
pub(crate) struct Side<'a> {
    pub(crate) workspace: &'a Path,
    pub(crate) target_dir: &'a Path,
    pub(crate) view: &'a GitView,
}

#[derive(Debug)]
pub(crate) struct BaselineSetup {
    environment: CommandEnvironment,
    toolchain: Option<Toolchain>,
    lints: Option<KnownLints>,
    declared: std::collections::BTreeSet<String>,
    cache: Lookup,
}

impl BaselineSetup {
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
    let declared = crate::generated::declared(workspace_root, options.deadline);
    let cache = side_cache::lookup(
        &KeyInputs {
            workspace_root,
            target_directory: prepared.target_directory(),
            comparison_base,
            toolchain: toolchain.as_ref(),
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
            let baseline = context.run(
                target,
                toolchain.clone(),
                lints.clone(),
                Some(base.target_dir),
                Some(base.view),
            );
            if !baseline.is_complete() {
                let mut current = prepared.fail(crate::baseline::scan_incomplete());
                current.toolchain = toolchain;
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
        None => context.run(prepared.target, toolchain, lints, None, None),
        Some(staged) => match scan_target::resolve_isolated(
            staged.workspace,
            &programs.cargo,
            staged.target_dir,
            environment.rustup_toolchain.as_deref(),
            options.deadline,
        ) {
            Ok(target) => {
                context.run(target, toolchain, lints, Some(staged.target_dir), Some(staged.view))
            }
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

fn command_environment(workspace_root: &Path) -> Result<CommandEnvironment, InternalError> {
    match toolchain::name_only_pin(workspace_root) {
        NameOnlyPin::Named(rustup_toolchain) => Ok(CommandEnvironment {
            rustup_toolchain: Some(rustup_toolchain),
        }),
        NameOnlyPin::NoRustup | NameOnlyPin::Refused(_) => Err(crate::baseline::scan_incomplete()),
    }
}
