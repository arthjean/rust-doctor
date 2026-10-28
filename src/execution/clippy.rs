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
use std::time::Instant;

use cargo_metadata::Metadata;

use super::lockfile::LockfileGuard;
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

/// Closes the lint section, after every catalogued `-W`: an inherited
/// `-D warnings` from any rustflags source Cargo reads, `[target.*]` tables
/// and `CARGO_BUILD_RUSTFLAGS` included, would otherwise turn the first
/// catalogued warning into a build failure. Observed on 1.97.1: the later
/// flag wins and the warning stays a warning.
const WARNINGS_STAY_WARNINGS: [&str; 2] = ["-W", "warnings"];

/// Pins incremental compilation off whatever the caller's environment says.
///
/// Observed on 1.97.1: a site a macro expands six times is reported six times
/// with incremental compilation on and once with it off, so the same scan
/// counted 6 occurrences on a developer machine and 1 under
/// `Swatinem/rust-cache`, which exports `CARGO_INCREMENTAL=0`. Measured on the
/// corpus's `bytes` on 2026-10-21: 41 `indexing_slicing` occurrences against
/// 31, the same 29 sites. The variable
/// overrides the profile's `incremental` key, so the count no longer depends on
/// where the scan runs, and a scan writes no incremental cache it never reads.
const INCREMENTAL: (&str, &str) = ("CARGO_INCREMENTAL", "0");

/// Appended when the installed Clippy's lint list could not be read, so a
/// catalogued lint it does not know is allowed instead of failing the build.
const TOLERATE_UNKNOWN: [&str; 2] = ["-A", "unknown_lints"];

/// What Cargo prints on stderr while another process holds one of its locks,
/// and what the deadline's message then says. Observed on 1.97.1: the build
/// directory's is rust-analyzer's own check most of the time, the package
/// cache's any other Cargo resolving or fetching under the same home.
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

/// Runs the pass and answers everything the report needs from it.
///
/// Stdout is drained to its end before the wait, and stderr on a thread of its
/// own: Cargo blocked on a pipe it cannot flush never exits, and this process
/// would wait on it forever.
pub(super) fn run(
    context: &ExecutionContext<'_>,
    metadata: &Metadata,
    target_dir: Option<&Path>,
    known: Option<KnownLints>,
) -> Result<ScanExecution, InternalError> {
    let workspace_root = metadata.workspace_root.as_std_path();
    let selected = context.options.packages.as_ref();
    let members = Members::of(metadata, selected);
    let timings = context.options.timings.as_ref();
    // Asked beside the version probes. A run that holds no answer passes
    // every `-W` with unknown lints allowed, as one whose probe failed does.
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
    // The first member Cargo checked ends the dependencies: what came before
    // it compiled what the workspace depends on, what comes after lints it.
    let spawned = Instant::now();
    let mut first_member = None;
    // Dropped when this function returns, however the pass ended.
    let _lockfile = LockfileGuard::capture(workspace_root);
    let finished = process::run(command, context.options.deadline, |stdout| {
        messages::collect_observed(std::io::BufReader::new(stdout), &mut |message| {
            members.observe(message, &mut linted, context.options.progress.as_ref());
            if first_member.is_none() && !linted.is_empty() {
                first_member = Some(Instant::now());
            }
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
        let linting_from = first_member.unwrap_or_else(Instant::now);
        timings.record(
            Phase::ClippyDependencies,
            linting_from.saturating_duration_since(spawned),
        );
        timings.record(Phase::ClippyLinting, linting_from.elapsed());
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
    // A build Cargo did not report finished linted nothing it did not name:
    // one that failed, one the deadline or a signal cut short, and one Cargo
    // refused before building, over a lockfile it could not parse or a
    // `rust-version` above the toolchain. The last used to name no member and
    // publish a hundred over a workspace nothing had linted.
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

/// Why the pass ended before linting everything, when it did: the deadline
/// killed it, a signal did, or members did not compile. The deadline names the
/// members it left unlinted and the Cargo lock it was waiting on, if any.
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

/// What became of the pass: never started, switched off by the policy, run
/// to an outcome, or ended by a panic its containment caught.
///
/// `Disabled` is a complete answer and `NotRun` is not, which is the whole
/// reason they are one enum rather than an `Option` and a flag. `Panicked` is
/// an outcome, so the native findings are still published, and an incomplete
/// one, whose error the containment publishes at the `execution` stage.
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
