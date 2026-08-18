//! The Clippy pass, whole: the command it runs, the arguments the catalog
//! decides, the process it starts, and the three states its outcome can be in.
//!
//! It used to be split the wrong way. This file held the argument vector and
//! the outcome enum while the orchestrator held the process, the wire model and
//! the parser, so `mod clippy` named a fifth of the Clippy story and
//! `execution.rs` carried the rest at 977 lines of a 1000-line bound.

use std::path::Path;
use std::process::{Command, Stdio};

use super::messages::{self, ScanExecution};
use super::{CommandEnvironment, Programs};
use crate::internal_error::InternalError;
use crate::policy::{PolicyPlan, Producer, RuleDefinition, RuleLevel};

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
const BASE_ARGS: [&str; 4] = [
    "clippy",
    "--workspace",
    "--no-deps",
    "--message-format=json",
];

/// Silences everything Clippy warns about by default, so the only lints left
/// are the ones the catalog names right after it.
///
/// A lint the catalog does not know still reaches the report otherwise:
/// `report::diagnostics` only drops a diagnostic whose rule is catalogued and
/// inactive. It arrives with no category, no tier and no help, it cannot weigh
/// on the score, and its mere presence costs the score its authoritative flag.
/// Measured on four corpus repositories on 2026-08-06: 9 findings out of 164
/// came from there, and they were enough to disqualify three of the four
/// reports. Dropping them makes every finding one the tool can explain.
///
/// Order matters, `-W` after `-A` wins, so this stays the first argument of the
/// lint section.
const SILENCE_UNCATALOGUED: [&str; 2] = ["-A", "clippy::all"];

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
/// Stdout is drained to its end before the wait: Cargo blocked on a pipe it
/// cannot flush never exits, and this process would wait on it forever.
pub(super) fn run(
    programs: &Programs,
    workspace_root: &Path,
    plan: &PolicyPlan,
    target_dir: Option<&Path>,
    environment: &CommandEnvironment,
) -> Result<ScanExecution, InternalError> {
    let arguments = arguments_for_plan(plan);
    let mut child = command(
        &programs.cargo,
        workspace_root,
        &arguments,
        target_dir,
        environment,
    )
    .spawn()
    .map_err(|error| {
        InternalError::new(
            "execution",
            "clippy-start-failed",
            format!("Clippy could not be started: {error}"),
        )
    })?;

    let Some(stdout) = child.stdout.take() else {
        let _ = child.wait();
        return Err(InternalError::new(
            "execution",
            "clippy-stdout-unavailable",
            "Clippy started without a readable stdout pipe",
        ));
    };

    let mut stream = messages::collect(std::io::BufReader::new(stdout));
    let (exit_code, exit_success) = match child.wait() {
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

    Ok(ScanExecution {
        command: std::iter::once("cargo")
            .chain(arguments)
            .map(str::to_owned)
            .collect(),
        exit_code,
        exit_success,
        build_finished: stream.build_finished,
        noise_lines: stream.noise_lines,
        malformed_messages: stream.malformed_messages,
        messages: stream.messages,
        errors: stream.errors,
    })
}

fn command(
    cargo: &Path,
    workspace_root: &Path,
    arguments: &[&str],
    target_dir: Option<&Path>,
    environment: &CommandEnvironment,
) -> Command {
    let mut command = Command::new(cargo);
    command
        .args(arguments)
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
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
