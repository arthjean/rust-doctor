//! Running the producers over one resolved workspace, and saying what came
//! back.
//!
//! The module is the orchestration and nothing else. `clippy` is the pass that
//! compiles, `messages` the stream it answers on, and `baseline` the dual run a
//! comparison needs; the four native producers live in their own modules and
//! are called from here.
//!
//! Two rules hold it together, and each of them replaced something that had a
//! cost.
//!
//! **One list of the producers that degrade rather than abort.**
//! `ExecutionResult::producer_errors` is that list. `is_complete` reads it to
//! decide whether the score may call itself authoritative, and `report::errors`
//! reads it to publish them, so a producer cannot reach one and miss the other.
//! It used to be written twice, as a four-clause conjunction here and four
//! `if let` blocks with the stage spelled again in `report.rs`, and
//! `cargo_health` was in the second list only: a workspace whose
//! `.cargo/config.toml` could not be read published a `dependencies` error
//! under `"status": "complete"`.
//!
//! **The result is assembled from what came back, never asked back for what it
//! was given.** `ExecutionContext::run` owns the metadata until the end, so the
//! three `Option` dances that could not be `None`, and the workspace-root clone
//! that only existed to survive an early move, are gone with them.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::thread;

use cargo_metadata::Metadata;

use crate::bounded_read::collect_bounded;
use crate::cargo_health::{self, CargoHealthScan};
use crate::configuration::{self, WorkspaceConfiguration};
use crate::generated;
use crate::internal_error::InternalError;
use crate::policy::{PolicyPlan, Producer};
use crate::progress::{Progress, ProgressSink};
use crate::repo_hygiene::{self, RepoScan};
use crate::scan_target::{self, ResolvedScanTarget};
use crate::source_kernel::{self, SourceMeasurement, SourceScan};
use crate::structure::{self, StructureScan};
use crate::terminal_text::{sanitize, truncate};
use crate::timings::{Phase, TimingRecorder, timed};

mod baseline;
mod clippy;
mod exclusions;
pub(crate) mod interrupt;
mod lint_list;
mod members;
mod messages;
mod process;
mod rustflags;
mod side_cache;
#[cfg(test)]
mod tests;

pub(crate) use baseline::{
    BaseSide, BaselineExecution, Side, execute as execute_baseline, setup as setup_baseline,
};
pub use side_cache::CacheOutcome;
pub(crate) use side_cache::{SideCache, members as side_cache_members};
pub(crate) use clippy::ClippyExecution;
pub(crate) use exclusions::ScanExclusions;
pub(crate) use process::{RunDeadline, run as run_bounded, spawn as spawn_bounded};
#[cfg(test)]
pub(crate) use clippy::arguments_for_rules as clippy_arguments_for_rules;
pub(crate) use messages::{
    CapturedDiagnostic, CapturedMessage, CapturedSpan, CompilerMessageData, ScanExecution,
};
/// Named by the report's own tests, which build a captured message by hand.
#[cfg(test)]
pub(crate) use messages::{CapturedDiagnosticCode, CapturedTarget};

#[derive(Debug)]
pub(crate) struct ExecutionResult {
    pub(crate) manifest_path: Option<PathBuf>,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) toolchain: Option<Toolchain>,
    pub(crate) scan: ClippyExecution,
    pub(crate) source: Option<SourceScan>,
    /// What the walk saw, on every plan that reached a workspace, including one
    /// with no native rule active. The score divides by it, so it cannot be
    /// conditional on a rule set the reader is free to switch off.
    pub(crate) source_measurement: Option<SourceMeasurement>,
    pub(crate) structure: Option<StructureScan>,
    pub(crate) cargo_health: Option<CargoHealthScan>,
    pub(crate) repo: Option<RepoScan>,
    /// The stages a passed deadline kept from starting, each with the sentence
    /// its error carries.
    pub(crate) deadline_skipped: Vec<(&'static str, String)>,
    /// What the report leaves out, and the directives that ask it to.
    pub(crate) exclusions: ScanExclusions,
    pub(crate) error: Option<InternalError>,
}

/// One error of a producer that degraded instead of aborting, stamped with the
/// stage the report publishes it at.
#[derive(Debug)]
pub(crate) struct ProducerError<'a> {
    pub(crate) stage: &'static str,
    pub(crate) code: &'static str,
    pub(crate) message: &'a str,
}

impl ExecutionResult {
    /// The result of a run that ended before any producer could report.
    fn failed(
        manifest_path: Option<PathBuf>,
        metadata: Option<Metadata>,
        error: InternalError,
    ) -> Self {
        Self {
            manifest_path,
            metadata,
            toolchain: None,
            scan: ClippyExecution::NotRun,
            source: None,
            source_measurement: None,
            structure: None,
            cargo_health: None,
            repo: None,
            deadline_skipped: Vec::new(),
            exclusions: ScanExclusions::default(),
            error: Some(error),
        }
    }

    /// Every error the four native producers raised, in the order the report
    /// publishes their stages.
    ///
    /// This is the only enumeration of those producers in the crate. Adding a
    /// fifth is one arm here, and both the completeness verdict and the
    /// published error list follow from it.
    pub(crate) fn producer_errors(&self) -> impl Iterator<Item = ProducerError<'_>> {
        // Two sources at one stage: what the walk refused, which every plan
        // produces, and what soliciting the detectors produced, which only a
        // plan with a source rule active does.
        let walked = self.source_measurement.iter().flat_map(|measurement| {
            measurement
                .errors
                .iter()
                .map(|error| ProducerError::new("source", error.code, &error.message))
        });
        let source = walked.chain(self.source.iter().flat_map(|scan| {
            scan.errors
                .iter()
                .map(|error| ProducerError::new("source", error.code, &error.message))
        }));
        let structure = self.structure.iter().flat_map(|scan| {
            scan.errors
                .iter()
                .map(|error| ProducerError::new("structure", error.code, &error.message))
        });
        let dependencies = self.cargo_health.iter().flat_map(|scan| {
            scan.errors
                .iter()
                .map(|error| ProducerError::new("dependencies", error.code, error.message))
        });
        let repo = self.repo.iter().flat_map(|scan| {
            scan.errors
                .iter()
                .map(|error| ProducerError::new("repo", error.code, error.message))
        });
        let skipped = self
            .deadline_skipped
            .iter()
            .map(|(stage, message)| ProducerError::new(stage, "deadline-exceeded", message));
        source
            .chain(structure)
            .chain(dependencies)
            .chain(repo)
            .chain(skipped)
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.error.is_none() && self.scan.is_complete() && self.producer_errors().next().is_none()
    }
}

impl<'a> ProducerError<'a> {
    const fn new(stage: &'static str, code: &'static str, message: &'a str) -> Self {
        Self {
            stage,
            code,
            message,
        }
    }
}

/// The three versions a report attributes its findings to.
///
/// They are resolved together or the run fails before any producer starts, so
/// the result holds one `Option` over the three rather than three `Option`
/// fields. A report naming a cargo and no rustc was a shape the type allowed
/// and no code path could ever produce.
#[derive(Debug, Clone)]
pub(crate) struct Toolchain {
    pub(crate) cargo: String,
    pub(crate) rustc: String,
    pub(crate) clippy: String,
}

#[derive(Debug)]
pub(crate) struct PreparedInspection {
    pub(crate) target: ResolvedScanTarget,
    pub(crate) configuration: WorkspaceConfiguration,
}

impl PreparedInspection {
    pub(crate) fn workspace_root(&self) -> &Path {
        self.target.workspace_root()
    }

    /// Where discovery found the workspace below the requested path.
    pub(crate) fn discovered(&self) -> Option<&str> {
        self.target.discovered.as_deref()
    }

    /// Refuses a `--package` selection naming anything but a member.
    pub(crate) fn check_packages(&self, selected: &BTreeSet<String>) -> Result<(), InternalError> {
        exclusions::check_packages(&self.target.metadata, selected)
    }

    /// The directory Cargo builds this workspace into.
    pub(crate) fn target_directory(&self) -> &Path {
        self.target.metadata.target_directory.as_std_path()
    }

    pub(crate) fn fail(self, error: InternalError) -> ExecutionResult {
        ExecutionResult::failed(
            Some(self.target.manifest_path),
            Some(self.target.metadata),
            error,
        )
    }
}

#[derive(Debug)]
struct Programs {
    cargo: PathBuf,
    rustc: PathBuf,
    clippy_driver: PathBuf,
}

impl Default for Programs {
    fn default() -> Self {
        Self {
            cargo: PathBuf::from("cargo"),
            rustc: PathBuf::from("rustc"),
            clippy_driver: PathBuf::from("clippy-driver"),
        }
    }
}

/// What the caller set on the whole run rather than on one producer: the
/// deadline `--max-duration` fixes, where the phases are reported, and the
/// members `--package` selects.
#[derive(Debug, Clone, Default)]
pub(crate) struct RunOptions {
    pub(crate) deadline: Option<RunDeadline>,
    pub(crate) progress: Option<ProgressSink>,
    /// `None` scans every member. A selection is checked against the
    /// workspace before the run starts, so every name in it is a member.
    pub(crate) packages: Option<BTreeSet<String>>,
    /// The RustSec database the advisory rules read, `None` when there is
    /// none to read.
    pub(crate) advisory_db: Option<cargo_health::AdvisoryDatabase>,
    /// Where each phase adds its duration, `None` when nobody asked.
    pub(crate) timings: Option<TimingRecorder>,
}

/// Everything one run holds constant, gathered once.
///
/// The four of them are the same for the whole execution and identical on both
/// sides of a baseline comparison, so the only thing a call has left to say is
/// which target it runs on and where that target builds. They used to be four
/// of the seven positional parameters of one function, threaded through every
/// call site that had nothing to do with them.
#[derive(Debug, Clone, Copy)]
struct ExecutionContext<'a> {
    programs: &'a Programs,
    plan: &'a PolicyPlan,
    settings: &'a structure::StructureSettings,
    environment: &'a CommandEnvironment,
    options: &'a RunOptions,
    /// What the workspace's `.gitattributes` declares generated or vendored,
    /// read once from the repository and applied to every side: a snapshot
    /// sits outside git, and a side that could not read the declaration would
    /// report its files' findings as fixed.
    declared: &'a BTreeSet<String>,
}

/// The environment every child of one run is started under.
///
/// Empty for an ordinary scan, which inherits whatever the caller has, and
/// carrying a resolved pin for a baseline comparison, whose two sides have to
/// agree on a compiler before their findings can be subtracted.
#[derive(Debug, Clone, Default)]
struct CommandEnvironment {
    rustup_toolchain: Option<OsString>,
}

impl CommandEnvironment {
    fn apply(&self, command: &mut Command) {
        if let Some(toolchain) = self.rustup_toolchain.as_ref() {
            command.env("RUSTUP_TOOLCHAIN", toolchain);
        }
    }
}

pub(crate) fn prepare(
    path: &Path,
    options: &RunOptions,
) -> Result<PreparedInspection, Box<ExecutionResult>> {
    timed(options.timings.as_ref(), Phase::Metadata, || {
        prepare_with(path, &Programs::default(), options.deadline)
    })
}

fn prepare_with(
    path: &Path,
    programs: &Programs,
    deadline: Option<RunDeadline>,
) -> Result<PreparedInspection, Box<ExecutionResult>> {
    let target = match scan_target::resolve(path, &programs.cargo, deadline) {
        Ok(target) => target,
        Err(failure) => {
            return Err(Box::new(ExecutionResult::failed(
                failure.manifest_path,
                None,
                failure.error,
            )));
        }
    };
    let configuration = match configuration::load(&target) {
        Ok(configuration) => configuration,
        Err(error) => {
            return Err(Box::new(ExecutionResult::failed(
                Some(target.manifest_path),
                Some(target.metadata),
                error,
            )));
        }
    };

    Ok(PreparedInspection {
        target,
        configuration,
    })
}

pub(crate) fn execute(
    prepared: PreparedInspection,
    plan: &PolicyPlan,
    options: &RunOptions,
) -> ExecutionResult {
    execute_into(prepared, &Programs::default(), plan, None, options)
}

/// The same scan, run on the tree `--staged` wrote the index into rather than
/// on the working tree, building into `target_dir`.
///
/// Every producer reads that tree and nothing else: its metadata is resolved
/// there, so the source walk, the manifest pass and Clippy all start from it,
/// and the paths they report are relative to its root, which is the
/// workspace's own relative layout.
pub(crate) fn execute_staged(
    prepared: PreparedInspection,
    staged_workspace: &Path,
    target_dir: &Path,
    plan: &PolicyPlan,
    options: &RunOptions,
) -> ExecutionResult {
    let programs = Programs::default();
    let environment = CommandEnvironment::default();
    let target = timed(options.timings.as_ref(), Phase::Metadata, || {
        scan_target::resolve_isolated(
            staged_workspace,
            &programs.cargo,
            target_dir,
            None,
            options.deadline,
        )
    });
    let target = match target {
        Ok(target) => target,
        Err(failure) => return prepared.fail(failure.error),
    };
    // Probed where Clippy runs, so a toolchain that rustup picks by directory
    // is the one the report names.
    let lint_list = plan.active_rules(Producer::Clippy).next().is_some();
    let probed = resolve_toolchain(&programs, staged_workspace, &environment, options, lint_list);
    let (toolchain, lints) = match probed {
        Ok(probed) => probed,
        Err(error) => return prepared.fail(error),
    };
    let declared = generated::declared(prepared.workspace_root(), options.deadline);
    let context = ExecutionContext {
        programs: &programs,
        plan,
        settings: &prepared.configuration.structure,
        environment: &environment,
        options,
        declared: &declared,
    };
    context.run(target, toolchain, lints, Some(target_dir))
}

/// The same scan, told where Cargo may keep its artifacts.
///
/// A run of the tool passes `None` and lets the scanned workspace use its own
/// `target/`. The tests pass a directory of their own, because they share one
/// process: a fixture already compiled under the harness's inherited
/// `CARGO_TARGET_DIR` replays with no warning at all, and the assertion then
/// reads as a scan that found nothing rather than as a cache hit.
fn execute_into(
    prepared: PreparedInspection,
    programs: &Programs,
    plan: &PolicyPlan,
    target_dir: Option<&Path>,
    options: &RunOptions,
) -> ExecutionResult {
    let environment = CommandEnvironment::default();
    let lint_list = plan.active_rules(Producer::Clippy).next().is_some();
    let probed =
        resolve_toolchain(programs, prepared.workspace_root(), &environment, options, lint_list);
    let (toolchain, lints) = match probed {
        Ok(probed) => probed,
        Err(error) => return prepared.fail(error),
    };
    let declared = generated::declared(prepared.workspace_root(), options.deadline);
    let context = ExecutionContext {
        programs,
        plan,
        settings: &prepared.configuration.structure,
        environment: &environment,
        options,
        declared: &declared,
    };
    context.run(prepared.target, toolchain, lints, target_dir)
}

impl ExecutionContext<'_> {
    /// Runs every producer the plan leaves on against one resolved target.
    fn run(
        &self,
        target: ResolvedScanTarget,
        toolchain: Toolchain,
        lints: Option<KnownLints>,
        target_dir: Option<&Path>,
    ) -> ExecutionResult {
        let ResolvedScanTarget {
            manifest_path,
            metadata,
            ..
        } = target;
        // The manifest-level passes read files nothing else writes before
        // Clippy, so they run on a thread of their own beside the walk.
        let ((mut cargo_health, repo), (enumeration, exclusions)) = thread::scope(|scope| {
            let manifests = scope.spawn(|| self.manifest_passes(&metadata));
            let walked = self.walk(&metadata);
            let manifests = manifests
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
            (manifests, walked)
        });

        // Clippy compiles on a thread of its own while the native passes read
        // the walk on this one, so a scan costs the longer of the two rather
        // than their sum. Neither writes anything the other reads, and the
        // report sorts what both produced, so the output is the serial one.
        let (clippy, natives) = thread::scope(|scope| {
            let clippy = self.active(Producer::Clippy).then(|| {
                let metadata = &metadata;
                scope.spawn(move || clippy::run(self, metadata, target_dir, lints))
            });
            let natives = self.native_passes(&metadata, &enumeration, cargo_health.as_mut());
            let clippy = clippy.map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    Err(InternalError::new(
                        "execution",
                        "clippy-thread-failed",
                        "The Clippy pass ended without an answer.",
                    ))
                })
            });
            (clippy, natives)
        });
        let (source, structure, deadline_skipped) = natives;
        let scan = match clippy {
            Some(Ok(scan)) => ClippyExecution::from_scan(scan),
            None => ClippyExecution::Disabled,
            // Clippy could not be started, so this result has already failed
            // and nothing the native passes found will be read: the early
            // return and its stage error are what a serial run published.
            Some(Err(error)) => {
                return ExecutionResult {
                    toolchain: Some(toolchain),
                    cargo_health,
                    repo,
                    ..ExecutionResult::failed(Some(manifest_path), Some(metadata), error)
                };
            }
        };
        // Taken last, once every producer above has read the units.
        let source_measurement = Some(enumeration.into_measurement());

        ExecutionResult {
            manifest_path: Some(manifest_path),
            metadata: Some(metadata),
            toolchain: Some(toolchain),
            scan,
            source,
            source_measurement,
            structure,
            cargo_health,
            repo,
            deadline_skipped,
            exclusions,
            error: None,
        }
    }

    fn active(&self, producer: Producer) -> bool {
        self.plan.active_rules(producer).next().is_some()
    }

    /// The dependency pack and the repository pass.
    fn manifest_passes(
        &self,
        metadata: &Metadata,
    ) -> (Option<CargoHealthScan>, Option<RepoScan>) {
        let timings = self.options.timings.as_ref();
        // The dependency pack reads the resolved graph from disk, so it
        // produces bounded errors like the source kernel: it runs here, not
        // during normalization, so that its errors join those of the scan.
        //
        // It runs before Clippy: `cargo clippy` creates or rewrites
        // `Cargo.lock`, so reading it afterwards would measure the graph the
        // tool just wrote instead of the one the repository commits.
        let cargo_health = self.active(Producer::CargoHealth).then(|| {
            timed(timings, Phase::CargoHealth, || {
                cargo_health::inspect(metadata, self.plan, self.options.advisory_db.as_ref())
            })
        });
        // The repository pass reads git, not Cargo: it sits in the
        // manifest-level slot beside the dependency pack, before the
        // compilation that Clippy triggers, and reads nothing that pass will
        // rewrite.
        // A repository is not a member: under `--package` the pass would judge
        // files the selection never named, so it does not run, and the report
        // says its rules were not evaluated.
        let repo = (self.active(Producer::Repo) && self.options.packages.is_none()).then(|| {
            timed(timings, Phase::Repo, || {
                repo_hygiene::inspect(
                    metadata.workspace_root.as_std_path(),
                    self.plan,
                    cargo_health::workspace_produces_a_binary(metadata),
                    self.options.deadline,
                )
            })
        });
        (cargo_health, repo)
    }

    /// One walk feeds every producer that reads source text. The source
    /// kernel decides per call site, the structural pass decides per family,
    /// the dependency-truth rules read crate references off the same units,
    /// and enumerating the workspace twice for any of them would double the
    /// only expensive part of each.
    ///
    /// It runs whatever the plan asked for, because the walk is also the
    /// measurement: the score divides by the production lines it counts, and
    /// a plan with every native rule off still has a score. What the plan
    /// decides is which detectors are solicited, never whether the workspace
    /// is measured.
    ///
    /// It runs before Clippy starts, so the tree it measures is the committed
    /// one: a `build.rs` Clippy runs may write into the workspace.
    fn walk(&self, metadata: &Metadata) -> (source_kernel::Enumeration, exclusions::ScanExclusions) {
        if let Some(progress) = &self.options.progress {
            progress.report(Progress::NativePasses);
        }
        let packages = self.options.packages.as_ref();
        timed(self.options.timings.as_ref(), Phase::Walk, || {
            let mut enumeration = source_kernel::enumerate(metadata);
            // A generated file is left out of the measurement as an ignored
            // one is: lines nobody writes by hand would otherwise dilute the
            // density the score divides by.
            enumeration.narrow(
                |unit| {
                    let path = unit.relative_path();
                    self.plan.is_ignored(path)
                        || self.declared.contains(path)
                        || crate::generated::has_generator_header(unit.source())
                },
                packages,
            );
            let exclusions = exclusions::gather(metadata, &enumeration, packages, self.declared);
            (enumeration, exclusions)
        })
    }

    /// The source kernel, the structural pass and the dependency-truth rules,
    /// over one walk, with the passes the deadline kept from starting.
    fn native_passes(
        &self,
        metadata: &Metadata,
        enumeration: &source_kernel::Enumeration,
        cargo_health: Option<&mut CargoHealthScan>,
    ) -> (Option<SourceScan>, Option<StructureScan>, Vec<(&'static str, String)>) {
        let timings = self.options.timings.as_ref();
        // A pass the deadline has already passed is not started: it is named
        // at its own stage instead, so the report says what it did not read.
        let mut deadline_skipped = Vec::new();
        // A pass an interrupted run has not started is not started either:
        // the report the run publishes names the interruption, not the pass.
        let mut in_time = |active: bool, stage: &'static str| match self.options.deadline {
            _ if interrupt::requested() => false,
            Some(deadline) if active && deadline.passed() => {
                deadline_skipped.push((stage, deadline.message()));
                false
            }
            _ => active,
        };
        let source_in_time = in_time(self.active(Producer::SourceKernel), "source");
        let structure_in_time = in_time(self.active(Producer::Structure), "structure");
        let dependency_truth = cargo_health::dependency_truth_required(self.plan);
        let dependency_truth = in_time(dependency_truth && cargo_health.is_some(), "dependencies");
        // The structural pass, the longest of the three, reads the walk on a
        // thread of its own while the other two read it on this one.
        let (source, structure) = thread::scope(|scope| {
            let structure = structure_in_time.then(|| {
                scope.spawn(|| {
                    let cap = self.options.deadline.map(RunDeadline::remaining);
                    timed(timings, Phase::Structure, || {
                        structure::analyze(metadata, enumeration, self.plan, self.settings, cap)
                    })
                })
            });
            let source = source_in_time.then(|| {
                timed(timings, Phase::SourceKernel, || {
                    source_kernel::inspect(enumeration, self.plan)
                })
            });
            // Both dependency-truth rules belong to the dependency pack, so a
            // plan that asks for either has already put a scan here to merge
            // into.
            if let Some(scan) = cargo_health.filter(|_| dependency_truth) {
                timed(timings, Phase::CargoHealth, || {
                    let references = source_kernel::references::collect(enumeration);
                    cargo_health::inspect_dependency_truth(
                        metadata,
                        enumeration,
                        &references,
                        self.plan,
                        scan,
                    );
                });
            }
            let structure = structure.map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
            });
            (source, structure)
        });
        (source, structure, deadline_skipped)
    }
}

/// What the installed Clippy lists, or the notice saying it could not be
/// asked.
pub(crate) type KnownLints = Result<BTreeSet<String>, InternalError>;

/// The three versions, and the lint list when `lint_list` asks for it, probed
/// concurrently: four short-lived processes cost the slowest of them.
fn resolve_toolchain(
    programs: &Programs,
    workspace_root: &Path,
    environment: &CommandEnvironment,
    options: &RunOptions,
    lint_list: bool,
) -> Result<(Toolchain, Option<KnownLints>), InternalError> {
    let timings = options.timings.as_ref();
    let deadline = options.deadline;
    timed(timings, Phase::Probes, || {
        thread::scope(|scope| {
            let lints = lint_list.then(|| {
                scope.spawn(move || {
                    timed(timings, Phase::LintList, || {
                        lint_list::probe(
                            &programs.clippy_driver,
                            workspace_root,
                            environment,
                            deadline,
                        )
                    })
                })
            });
            let probe = move |program: &Path, arguments: &[&str], probe: Probe| {
                tool_version(program, arguments, workspace_root, probe, environment, deadline)
                    .map_err(Some)
            };
            let rustc = scope.spawn(move || probe(&programs.rustc, &["--version"], RUSTC_PROBE));
            let clippy = scope
                .spawn(move || probe(&programs.cargo, &["clippy", "--version"], CLIPPY_PROBE));
            let cargo = probe(&programs.cargo, &["--version"], CARGO_PROBE);
            let rustc = rustc.join().unwrap_or(Err(None));
            let clippy = clippy.join().unwrap_or(Err(None));
            let lints = lints.map(|lints| lints.join().unwrap_or_else(|_| Err(lint_list::unavailable())));
            // The first failure in the order a serial run met them.
            let failed = |probe: Probe| {
                move |error: Option<InternalError>| {
                    error.unwrap_or_else(|| probe.failure(format!("{} probe ended without an answer", probe.label)))
                }
            };
            let toolchain = Toolchain {
                cargo: cargo.map_err(failed(CARGO_PROBE))?,
                rustc: rustc.map_err(failed(RUSTC_PROBE))?,
                clippy: clippy.map_err(failed(CLIPPY_PROBE))?,
            };
            Ok((toolchain, lints))
        })
    })
}

/// One toolchain probe: what it is called, what it reports at, and what the
/// reader is supposed to do when it fails.
///
/// The remedy belongs to the probe rather than to the message, because it
/// differs per tool: a missing `clippy` is one rustup command away, and a
/// missing cargo is a toolchain that is not installed at all. Every failure of
/// a probe ends the scan before any producer runs, so every one of them carries
/// it.
#[derive(Debug, Clone, Copy)]
struct Probe {
    code: &'static str,
    label: &'static str,
    remedy: &'static str,
}

const CARGO_PROBE: Probe = Probe {
    code: "cargo-unavailable",
    label: "Cargo",
    remedy: "Install a Rust toolchain from https://rustup.rs and put `cargo` on PATH.",
};

const RUSTC_PROBE: Probe = Probe {
    code: "rustc-unavailable",
    label: "rustc",
    remedy: "Install a Rust toolchain from https://rustup.rs and put `rustc` on PATH.",
};

const CLIPPY_PROBE: Probe = Probe {
    code: "clippy-unavailable",
    label: "Clippy",
    remedy: "Clippy is required: install the component with `rustup component add clippy`, \
        or add `components: clippy` to the toolchain step in CI.",
};

impl Probe {
    fn failure(self, message: String) -> InternalError {
        InternalError::new("execution", self.code, format!("{message}. {}", self.remedy))
    }
}

/// What one probe may print before this process stops keeping it.
///
/// A version is one line. The rest of the budget is for what a failing probe
/// wrote to stderr, which is the only text that says which of "the component is
/// not installed" and "it crashed" happened, in the toolchain's own words.
const PROBE_OUTPUT_LIMIT: usize = 4_096;

/// How much of that text reaches the published message. The rest of the answer
/// is one command away, and the reader is told which one.
const PROBE_DETAIL_COLUMNS: usize = 200;

fn tool_version(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    probe: Probe,
    environment: &CommandEnvironment,
    deadline: Option<RunDeadline>,
) -> Result<String, InternalError> {
    // The nearer of the probe's own ten seconds and the run's deadline: a
    // toolchain that hangs never carries the scan past `--max-duration`.
    let (bound, run_bound) = RunDeadline::capped(lint_list::PROBE_TIMEOUT, deadline);
    let output = run_probe(program, arguments, working_directory, environment, bound)
        .map_err(|error| {
            probe.failure(format!("{} could not be started: {error}", probe.label))
        })?;
    if output.expired && run_bound {
        return Err(InternalError::new(
            "toolchain",
            crate::report::DEADLINE_EXCEEDED,
            bound.message(),
        ));
    }
    if output.expired {
        return Err(InternalError::new(
            "toolchain",
            "probe-timeout",
            format!(
                "{} did not report a version within {} s. {}",
                probe.label,
                lint_list::PROBE_TIMEOUT.as_secs(),
                probe.remedy
            ),
        ));
    }

    if !output.status.success() {
        return Err(probe.failure(format!(
            "{} could not report a version ({}){}",
            probe.label,
            output.status,
            probe_detail(&output.stderr)
        )));
    }

    let version = String::from_utf8(output.stdout).map_err(|error| {
        probe.failure(format!(
            "{} returned non-UTF-8 version output: {error}",
            probe.label
        ))
    })?;
    let version = version.trim();
    if version.is_empty() || output.truncated {
        return Err(probe.failure(format!("{} returned no usable version", probe.label)));
    }

    Ok(version.to_owned())
}

/// What a probe printed, bounded.
struct ProbeOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    /// The version itself did not fit the budget, so what was kept is a prefix
    /// of one rather than a version.
    truncated: bool,
    /// The probe did not answer within its bound and was killed.
    expired: bool,
}

/// Runs one probe to its end and keeps a bounded amount of what it printed.
///
/// Both pipes are drained, and stderr on a thread of its own: a probe blocked
/// on a stream this process has not read yet never exits, and draining one to
/// its end before touching the other is what would turn that into a deadlock
/// rather than a slow answer. Reading stderr at all is what lets the failure
/// say why, since the toolchain writes that there and the report used to
/// publish the exit status alone.
fn run_probe(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    environment: &CommandEnvironment,
    deadline: RunDeadline,
) -> io::Result<ProbeOutput> {
    let finished = process::run(
        version_command(program, arguments, working_directory, environment),
        Some(deadline),
        |stream| collect_bounded(stream, PROBE_OUTPUT_LIMIT),
    )?;
    let status = finished.status?;
    let (stdout, truncated) = finished
        .output
        .transpose()?
        .map_or_else(|| (Vec::new(), false), |output| (output.bytes, output.exceeded));

    Ok(ProbeOutput {
        status,
        stdout,
        stderr: finished.stderr,
        truncated,
        expired: finished.expired,
    })
}

/// The first thing a failing probe said, on one line and bounded.
///
/// Empty when it said nothing, so the message reads as a sentence either way.
fn probe_detail(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| format!(": {}", truncate(&sanitize(line), PROBE_DETAIL_COLUMNS)))
        .unwrap_or_default()
}

fn version_command(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    environment: &CommandEnvironment,
) -> Command {
    let mut command = Command::new(program);
    command.args(arguments).current_dir(working_directory);
    environment.apply(&mut command);
    command
}
