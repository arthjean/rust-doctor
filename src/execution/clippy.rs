//! The Clippy pass, whole: the command it runs, the arguments the catalog
//! decides, the process it starts, and the three states its outcome can be in.
//!
//! It used to be split the wrong way. This file held the argument vector and
//! the outcome enum while the orchestrator held the process, the wire model and
//! the parser, so `mod clippy` named a fifth of the Clippy story and
//! `execution.rs` carried the rest at 977 lines of a 1000-line bound.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use cargo_metadata::Metadata;

use super::members::Members;
use super::messages::{self, ScanExecution};
use super::rustflags::RustflagsOverride;
use super::{CommandEnvironment, ExecutionContext, lint_list, process};
use crate::cargo_stderr;
use crate::internal_error::InternalError;
#[cfg(test)]
use crate::policy::PolicyPlan;
use crate::policy::{Producer, RuleDefinition, RuleLevel};
use crate::progress::Progress;

/// Cargo's default targets: libraries and binaries, that is, what the project
/// publishes.
///
/// `--all-targets` also compiled tests, benches, examples and build scripts.
/// Measured on the corpus on 2026-08-04, 69.9% of the pack findings came from
/// there, along with 1252 of the 1279 findings of the self-scan. An
/// `.unwrap()` under `#[cfg(test)]` is the expected failure mechanism of the
/// test, a `println!` in `build.rs` is the channel Cargo imposes, a `dbg!`
/// under `examples/` is the demonstration: none of them is a defect of the
/// shipped codebase, so none belongs in a score that judges it.
///
/// The filtering cannot happen afterwards: Cargo labels `test: true` on every
/// message under `--all-targets`, including those of a binary with no test at
/// all. The scope is therefore set here, at the source.
///
/// `--keep-going` lets every member that compiles be linted when another does
/// not. Without it the first broken crate stopped the build, and one red member
/// erased the report for all the others.
const BASE_ARGS: [&str; 5] = [
    "clippy",
    "--workspace",
    "--no-deps",
    "--keep-going",
    "--message-format=json",
];

/// Silences everything Clippy warns about by default, so the only lints left
/// are the ones the catalog names right after it.
///
/// A lint the catalog does not know would still reach the report otherwise,
/// with no category, no tier and no help. Measured on four corpus repositories
/// on 2026-08-06: 9 findings out of 164 came from there. What still arrives
/// uncatalogued, a rustc lint or a Clippy lint the workspace's own `[lints]`
/// enables, is published as an unscored compiler note and weighs nothing.
///
/// Order matters, `-W` after `-A` wins, so this stays the first argument of the
/// lint section.
const SILENCE_UNCATALOGUED: [&str; 2] = ["-A", "clippy::all"];

/// Appended when the installed Clippy's lint list could not be read, so a
/// catalogued lint it does not know is allowed instead of failing the build.
const TOLERATE_UNKNOWN: [&str; 2] = ["-A", "unknown_lints"];

#[cfg(test)]
pub(super) fn arguments_for_plan(plan: &PolicyPlan) -> Vec<&'static str> {
    arguments_for_rules(plan.active_rules(Producer::Clippy))
}

pub(crate) fn arguments_for_rules<'a>(
    rules: impl IntoIterator<Item = (&'a RuleDefinition, RuleLevel)>,
) -> Vec<&'static str> {
    let mut arguments = Vec::with_capacity(BASE_ARGS.len() + 1 + SILENCE_UNCATALOGUED.len() + 16);
    arguments.extend(BASE_ARGS);
    arguments.push("--");
    arguments.extend(SILENCE_UNCATALOGUED);
    for (definition, level) in rules {
        if let Some(flag) = level.clippy_flag() {
            arguments.extend([flag, definition.id]);
        }
    }
    arguments
}

/// Runs the pass and answers everything the report needs from it.
///
/// Stdout is drained to its end before the wait, and stderr on a thread of its
/// own: Cargo blocked on a pipe it cannot flush never exits, and this process
/// would wait on it forever.
pub(super) fn run(
    context: &ExecutionContext<'_>,
    metadata: &Metadata,
    target_dir: Option<&Path>,
) -> Result<ScanExecution, InternalError> {
    let workspace_root = metadata.workspace_root.as_std_path();
    let selected = context.options.packages.as_ref();
    let members = Members::of(metadata, selected);
    let known = lint_list::probe(context, workspace_root);
    let not_evaluated: Vec<&'static str> = known.as_ref().map_or_else(|_| Vec::new(), |known| {
        context
            .plan
            .active_rules(Producer::Clippy)
            .map(|(definition, _)| definition.id)
            .filter(|id| !known.contains(*id))
            .collect()
    });
    let mut arguments: Vec<String> = arguments_for_rules(
        context
            .plan
            .active_rules(Producer::Clippy)
            .filter(|(definition, _)| !not_evaluated.contains(&definition.id)),
    )
    .into_iter()
    .map(str::to_owned)
    .collect();
    if let Some(selected) = selected {
        select_packages(&mut arguments, selected);
    }
    let mut notices = Vec::new();
    if let Err(notice) = known {
        // Every `-W` is still passed, and a lint this Clippy does not know is
        // allowed rather than warned about: the scan runs narrower, not blind.
        arguments.extend(TOLERATE_UNKNOWN.map(str::to_owned));
        notices.push(notice);
    }
    let rustflags = RustflagsOverride::resolve(workspace_root);
    let mut command = command(
        &context.programs.cargo,
        workspace_root,
        &arguments,
        target_dir,
        context.environment,
    );
    rustflags.apply(&mut command);

    if let Some(progress) = &context.options.progress {
        progress.report(Progress::Dependencies);
    }
    let mut linted = BTreeSet::new();
    let finished = process::run(command, context.options.deadline, |stdout| {
        messages::collect_observed(std::io::BufReader::new(stdout), &mut |message| {
            members.observe(message, &mut linted, context.options.progress.as_ref());
        })
    })
    .map_err(|error| {
        InternalError::new(
            "execution",
            "clippy-start-failed",
            format!("Clippy could not be started: {error}"),
        )
    })?;

    let Some(mut stream) = finished.output else {
        return Err(InternalError::new(
            "execution",
            "clippy-stdout-unavailable",
            "Clippy started without a readable stdout pipe",
        ));
    };
    let (exit_code, exit_success) = match &finished.status {
        Ok(status) => (status.code(), Some(status.success())),
        Err(error) => {
            stream.errors.push(InternalError::new(
                "execution",
                "clippy-wait-failed",
                format!("could not collect Clippy exit status: {error}"),
            ));
            (None, None)
        }
    };
    let succeeded = exit_success == Some(true) && stream.build_finished == Some(true);
    if finished.expired
        && let Some(deadline) = context.options.deadline
    {
        stream
            .errors
            .push(InternalError::new("clippy", "deadline-exceeded", deadline.message()));
    } else if stream.build_finished == Some(false) {
        // Only a build Cargo ran and reported failed has members to name. One
        // that never started, over a lockfile it could not parse, linted
        // nothing, and its exit error already quotes why.
        let unlinted = members.unlinted(&stream.messages);
        if !unlinted.is_empty() {
            stream.errors.push(InternalError::new(
                "clippy",
                "packages-unlinted",
                format!(
                    "Not linted because it did not compile: {}",
                    unlinted.join(", ")
                ),
            ));
        }
    }

    Ok(ScanExecution {
        command: std::iter::once("cargo".to_owned())
            .chain(arguments)
            .collect(),
        exit_code,
        exit_success,
        build_finished: stream.build_finished,
        noise_lines: stream.noise_lines,
        malformed_messages: stream.malformed_messages,
        messages: stream.messages,
        errors: stream.errors,
        notices,
        cargo_cause: (!succeeded && !finished.expired)
            .then(|| {
                let target = target_dir.unwrap_or(metadata.target_directory.as_std_path());
                cargo_stderr::excerpt(&finished.stderr, workspace_root, Some(target))
            })
            .flatten(),
        deadline_exceeded: finished.expired,
        not_evaluated,
        removed_lint_flags: rustflags.removed,
    })
}

/// Lints the selected members only: one `-p <NAME>` each where
/// `--workspace` stood, so Cargo still compiles what they depend on and lints
/// nothing else.
fn select_packages(arguments: &mut Vec<String>, selected: &BTreeSet<String>) {
    let Some(position) = arguments.iter().position(|argument| argument == "--workspace") else {
        return;
    };
    let packages = selected
        .iter()
        .flat_map(|name| ["-p".to_owned(), name.clone()]);
    arguments.splice(position..=position, packages);
}

fn command(
    cargo: &Path,
    workspace_root: &Path,
    arguments: &[String],
    target_dir: Option<&Path>,
    environment: &CommandEnvironment,
) -> Command {
    let mut command = Command::new(cargo);
    command.args(arguments).current_dir(workspace_root);
    if let Some(target_dir) = target_dir {
        command.env("CARGO_TARGET_DIR", target_dir);
    }
    environment.apply(&mut command);
    command
}

/// What became of the pass: never started, switched off by the policy, or run
/// to an outcome.
///
/// `Disabled` is a complete answer and `NotRun` is not, which is the whole
/// reason the three are one enum rather than an `Option` and a flag.
#[derive(Debug, Default)]
pub(crate) enum ClippyExecution {
    #[default]
    NotRun,
    Disabled,
    Finished(ScanExecution),
}

impl ClippyExecution {
    pub(crate) const fn finished(&self) -> Option<&ScanExecution> {
        match self {
            Self::Finished(scan) => Some(scan),
            Self::NotRun | Self::Disabled => None,
        }
    }

    pub(crate) const fn has_outcome(&self) -> bool {
        !matches!(self, Self::NotRun)
    }

    pub(super) fn is_complete(&self) -> bool {
        match self {
            Self::Disabled => true,
            Self::Finished(scan) => {
                scan.exit_success == Some(true)
                    && scan.build_finished == Some(true)
                    && scan.malformed_messages == 0
                    && scan.errors.is_empty()
            }
            Self::NotRun => false,
        }
    }

    #[cfg(test)]
    pub(super) fn into_finished(self) -> Option<ScanExecution> {
        match self {
            Self::Finished(scan) => Some(scan),
            Self::NotRun | Self::Disabled => None,
        }
    }
}

#[cfg(test)]
mod tests;
