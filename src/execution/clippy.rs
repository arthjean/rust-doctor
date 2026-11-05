use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use cargo_metadata::Metadata;

use super::lockfile::{LockfileGuard, Turn};
use super::members::Members;
use super::containment;
use super::messages::{self, ScanExecution};
use super::rustflags::RustflagsOverride;
use super::{
    CommandEnvironment, ExecutionContext, KnownLints, RunDeadline, interrupt, lint_list, process,
};
use crate::cargo_stderr;
use crate::internal_error::InternalError;
#[cfg(test)]
use crate::policy::PolicyPlan;
use crate::policy::{Producer, RuleDefinition, RuleLevel};
use crate::progress::Progress;
use crate::timings::Phase;

const BASE_ARGS: [&str; 5] = [
    "clippy",
    "--workspace",
    "--no-deps",
    "--keep-going",
    "--message-format=json",
];

const SILENCE_UNCATALOGUED: [&str; 2] = ["-A", "clippy::all"];

const WARNINGS_STAY_WARNINGS: [&str; 2] = ["-W", "warnings"];

const INCREMENTAL: (&str, &str) = ("CARGO_INCREMENTAL", "0");

const TOLERATE_UNKNOWN: [&str; 2] = ["-A", "unknown_lints"];

const LOCK_WAITS: [(&str, &str); 2] = [
    (
        "Blocking waiting for file lock on build directory",
        " Cargo waited on the build directory lock held by another Cargo or rust-analyzer.",
    ),
    (
        "Blocking waiting for file lock on package cache",
        " Cargo waited on the package cache lock held by another Cargo.",
    ),
];

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
    arguments.extend(WARNINGS_STAY_WARNINGS);
    arguments
}

pub(super) fn run(
    context: &ExecutionContext<'_>,
    metadata: &Metadata,
    target_dir: Option<&Path>,
    known: Option<KnownLints>,
    turn: &Turn,
) -> Result<ScanExecution, InternalError> {
    let workspace_root = metadata.workspace_root.as_std_path();
    let selected = context.options.packages.as_ref();
    let members = Members::of(metadata, selected);
    let timings = context.options.timings.as_ref();
    let known = known.unwrap_or_else(|| Err(lint_list::unavailable()));
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
    let spawned = Instant::now();
    let mut last_dependency = None;
    let _lockfile = LockfileGuard::capture(workspace_root, turn);
    let finished = process::run(command, context.options.deadline, |stdout| {
        messages::collect_observed(std::io::BufReader::new(stdout), &mut |message| {
            members.observe(message, &mut linted, context.options.progress.as_ref());
            members.observe_dependency(message, &mut last_dependency);
            containment::observe_stream(message);
        })
    })
    .map_err(|error| {
        InternalError::new(
            "execution",
            "clippy-start-failed",
            format!("Clippy could not be started: {error}"),
        )
    })?;

    if let Some(timings) = timings {
        let (dependencies, linting) = split_phases(spawned, last_dependency, Instant::now());
        timings.record(Phase::ClippyDependencies, dependencies);
        timings.record(Phase::ClippyLinting, linting);
    }

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
    let unlinted = if finished.expired || stream.build_finished != Some(true) {
        members.unlinted(&stream.messages)
    } else {
        Vec::new()
    };
    let deadline = context.options.deadline.filter(|_| finished.expired);
    stream
        .errors
        .extend(cut_short(deadline, &unlinted, &finished.stderr));

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
        unlinted,
    })
}

fn split_phases(
    spawned: Instant,
    last_dependency: Option<Instant>,
    ended: Instant,
) -> (Duration, Duration) {
    let boundary = last_dependency.unwrap_or(spawned).clamp(spawned, ended);
    (
        boundary.duration_since(spawned),
        ended.duration_since(boundary),
    )
}

fn cut_short(
    deadline: Option<RunDeadline>,
    unlinted: &[String],
    stderr: &[u8],
) -> Option<InternalError> {
    if let Some(deadline) = deadline {
        let mut message = deadline.message();
        if !unlinted.is_empty() {
            message.push_str(&format!(" Not linted: {}.", unlinted.join(", ")));
        }
        let stderr = String::from_utf8_lossy(stderr);
        for (wait, sentence) in LOCK_WAITS {
            if stderr.contains(wait) {
                message.push_str(sentence);
            }
        }
        return Some(InternalError::new("clippy", "deadline-exceeded", message));
    }
    if interrupt::requested() {
        return Some(InternalError::new("clippy", interrupt::CODE, interrupt::MESSAGE));
    }
    (!unlinted.is_empty()).then(|| {
        InternalError::new(
            "clippy",
            "packages-unlinted",
            format!(
                "Not linted because it did not compile: {}",
                unlinted.join(", ")
            ),
        )
    })
}

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
    command
        .args(arguments)
        .current_dir(workspace_root)
        .env(INCREMENTAL.0, INCREMENTAL.1);
    if let Some(target_dir) = target_dir {
        command.env("CARGO_TARGET_DIR", target_dir);
    }
    environment.apply(&mut command);
    command
}

#[derive(Debug, Default)]
pub(crate) enum ClippyExecution {
    #[default]
    NotRun,
    Disabled,
    Finished(Box<ScanExecution>),
    Panicked,
}

impl ClippyExecution {
    pub(crate) fn from_scan(scan: ScanExecution) -> Self {
        Self::Finished(Box::new(scan))
    }

    pub(crate) fn finished(&self) -> Option<&ScanExecution> {
        match self {
            Self::Finished(scan) => Some(scan),
            Self::NotRun | Self::Disabled | Self::Panicked => None,
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
            Self::NotRun | Self::Panicked => false,
        }
    }

    #[cfg(test)]
    pub(super) fn into_finished(self) -> Option<ScanExecution> {
        match self {
            Self::Finished(scan) => Some(*scan),
            Self::NotRun | Self::Disabled | Self::Panicked => None,
        }
    }
}

#[cfg(test)]
mod tests;
