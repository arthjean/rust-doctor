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
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use cargo_metadata::Metadata;

use crate::cargo_health::{self, CargoHealthScan};
use crate::configuration::{self, WorkspaceConfiguration};
use crate::generated;
use crate::git::GitView;
use crate::internal_error::InternalError;
use crate::policy::{PolicyPlan, Producer};
use crate::progress::{Progress, ProgressSink};
use crate::repo_hygiene::{self, RepoScan};
use crate::scan_target::{self, ResolvedScanTarget};
use crate::source_kernel::{self, SourceMeasurement, SourceScan};
use crate::structure::{self, StructureScan};
use crate::timings::{Phase, TimingRecorder, timed};

mod baseline;
mod clippy;
pub(crate) mod containment;
mod exclusions;
pub(crate) mod interrupt;
mod lint_list;
mod lockfile;
mod manifest_allows;
mod members;
mod messages;
mod probes;
mod process;
mod rustflags;
mod side_cache;
pub(crate) mod toolchain;
#[cfg(test)]
mod tests;

pub(crate) use baseline::{
    BaseSide, BaselineExecution, Side, execute as execute_baseline, setup as setup_baseline,
};
pub use side_cache::CacheOutcome;
pub(crate) use side_cache::{SideCache, members as side_cache_members};
pub(crate) use clippy::ClippyExecution;
pub(crate) use exclusions::ScanExclusions;
use probes::{CARGO_PROBE, CLIPPY_PROBE, Probe, RUSTC_PROBE, tool_version};
#[cfg(test)]
use probes::version_command;
pub(crate) use process::{RunDeadline, run as run_bounded, spawn as spawn_bounded};
#[cfg(unix)]
pub(crate) use process::{effective_user, is_running};
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
    /// The passes that panicked, each dropped and named at its stage.
    pub(crate) panicked: Vec<containment::Pass>,
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
            panicked: Vec::new(),
            exclusions: ScanExclusions::default(),
            error: Some(error),
        }
    }

    /// Every error the four native producers raised, and one per pass a
    /// contained panic dropped, Clippy's included, in the order the report
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
        let panicked = self
            .panicked
            .iter()
            .map(|pass| ProducerError::new(pass.stage, containment::PASS_PANICKED, pass.message));
        source
            .chain(structure)
            .chain(dependencies)
            .chain(repo)
            .chain(skipped)
            .chain(panicked)
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
/// and no code path could ever produce. `None` also means "not probed": a
/// plan with no Clippy rule asks the toolchain nothing.
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

/// `read_configuration` is false for a scan that reads `rust-doctor.toml`
/// from somewhere else than the working tree, and the default stands in until
/// it does. `name_only` pins Cargo to a toolchain name, for a scan that
/// compiles nothing.
pub(crate) fn prepare(
    path: &Path,
    options: &RunOptions,
    read_configuration: bool,
    name_only: bool,
) -> Result<PreparedInspection, Box<ExecutionResult>> {
    timed(options.timings.as_ref(), Phase::Metadata, || {
        prepare_with(path, &Programs::default(), options.deadline, read_configuration, name_only)
    })
}

fn prepare_with(
    path: &Path,
    programs: &Programs,
    deadline: Option<RunDeadline>,
    read_configuration: bool,
    name_only: bool,
) -> Result<PreparedInspection, Box<ExecutionResult>> {
    let target = match scan_target::resolve(path, &programs.cargo, name_only, deadline) {
        Ok(target) => target,
        Err(failure) => {
            return Err(Box::new(ExecutionResult::failed(
                failure.manifest_path,
                None,
                failure.error,
            )));
        }
    };
    let loaded = if read_configuration {
        configuration::load(&target)
    } else {
        Ok(WorkspaceConfiguration::default())
    };
    let configuration = match loaded {
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
    staged: Side<'_>,
    plan: &PolicyPlan,
    options: &RunOptions,
) -> ExecutionResult {
    let Side {
        workspace: staged_workspace,
        target_dir,
        view,
    } = staged;
    let programs = Programs::default();
    let environment = CommandEnvironment::default();
    let lint_list = plan.active_rules(Producer::Clippy).next().is_some();
    // The index can carry a toolchain file of its own, so a scan that
    // compiles nothing pins the snapshot's metadata by name too.
    let pin = if lint_list {
        Ok(None)
    } else {
        toolchain::metadata_pin(staged_workspace)
    };
    let target = pin.and_then(|pin| {
        timed(options.timings.as_ref(), Phase::Metadata, || {
            scan_target::resolve_isolated(
                staged_workspace,
                &programs.cargo,
                target_dir,
                pin.as_deref(),
                options.deadline,
            )
            .map_err(|failure| failure.error)
        })
    });
    let target = match target {
        Ok(target) => target,
        Err(error) => return prepared.fail(error),
    };
    // Probed where Clippy runs, so a toolchain that rustup picks by directory
    // is the one the report names.
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
    context.run(target, toolchain, lints, Some(target_dir), Some(view))
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
    context.run(prepared.target, toolchain, lints, target_dir, None)
}

impl ExecutionContext<'_> {
    /// Runs every producer the plan leaves on against one resolved target.
    /// `view` is how git sees the target when it is a snapshot rather than
    /// the working tree.
    fn run(
        &self,
        target: ResolvedScanTarget,
        toolchain: Option<Toolchain>,
        lints: Option<KnownLints>,
        target_dir: Option<&Path>,
        view: Option<&GitView>,
    ) -> ExecutionResult {
        let ResolvedScanTarget {
            manifest_path,
            metadata,
            ..
        } = target;
        // Every pass below is contained: one that panics is dropped and named
        // at its stage, and the others' findings stay.
        let panicked = containment::Panicked::default();
        // The manifest-level passes read files nothing else writes before
        // Clippy, so they run on a thread of their own beside the walk.
        let ((mut cargo_health, repo), walked) = thread::scope(|scope| {
            let manifests = scope.spawn(|| self.manifest_passes(&metadata, view, &panicked));
            let walked = panicked.contain(containment::WALK, || self.walk(&metadata));
            // Both passes on that thread are contained, so only the code
            // between them can end it, and neither result is whole then.
            let manifests = manifests.join().unwrap_or_else(|_| {
                for (pass, producer) in [
                    (containment::CARGO_HEALTH, Producer::CargoHealth),
                    (containment::REPO, Producer::Repo),
                ] {
                    if self.active(producer) {
                        panicked.record(pass);
                    }
                }
                (None, None)
            });
            (manifests, walked)
        });
        let (enumeration, exclusions) = walked.unzip();

        // Clippy compiles on a thread of its own while the native passes read
        // the walk on this one, so a scan costs the longer of the two rather
        // than their sum. Neither writes anything the other reads, and the
        // report sorts what both produced, so the output is the serial one.
        let (clippy, natives) = thread::scope(|scope| {
            let clippy = self.active(Producer::Clippy).then(|| {
                let (metadata, panicked) = (&metadata, &panicked);
                scope.spawn(move || {
                    panicked.contain(containment::CLIPPY, || clippy::run(self, metadata, target_dir, lints))
                })
            });
            // A walk that panicked left no units, and judging none would
            // report every dependency unused: the passes reading it stay off.
            let natives = enumeration.as_ref().map(|enumeration| {
                self.native_passes(&metadata, enumeration, cargo_health.as_mut(), &panicked)
            });
            let clippy = clippy.map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    panicked.record(containment::CLIPPY);
                    None
                })
            });
            (clippy, natives)
        });
        let (source, structure, deadline_skipped) = natives.unwrap_or_default();
        let scan = match clippy {
            Some(Some(Ok(mut scan))) => {
                if let Some(health) = &cargo_health {
                    manifest_allows::honor(&mut scan, &metadata, self.plan, &health.allows);
                }
                ClippyExecution::from_scan(scan)
            }
            Some(None) => ClippyExecution::Panicked,
            None => ClippyExecution::Disabled,
            // Clippy could not be started, so this result has already failed
            // and nothing the native passes found will be read: the early
            // return and its stage error are what a serial run published.
            Some(Some(Err(error))) => {
                return ExecutionResult {
                    toolchain,
                    cargo_health,
                    repo,
                    panicked: panicked.into_passes(),
                    ..ExecutionResult::failed(Some(manifest_path), Some(metadata), error)
                };
            }
        };
        // Taken last, once every producer above has read the units. A walk
        // that never finished measures as one that never happened.
        let source_measurement = Some(enumeration.unwrap_or_default().into_measurement());

        ExecutionResult {
            manifest_path: Some(manifest_path),
            metadata: Some(metadata),
            toolchain,
            scan,
            source,
            source_measurement,
            structure,
            cargo_health,
            repo,
            deadline_skipped,
            panicked: panicked.into_passes(),
            exclusions: exclusions.unwrap_or_default(),
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
        view: Option<&GitView>,
        panicked: &containment::Panicked,
    ) -> (Option<CargoHealthScan>, Option<RepoScan>) {
        let timings = self.options.timings.as_ref();
        // The dependency pack reads the resolved graph from disk, so it
        // produces bounded errors like the source kernel: it runs here, not
        // during normalization, so that its errors join those of the scan.
        //
        // It runs before Clippy: `cargo clippy` creates or rewrites
        // `Cargo.lock`, so reading it afterwards would measure the graph the
        // tool just wrote instead of the one the repository commits.
        // It also runs when only Clippy does, since it reads the `[lints]`
        // allows the Clippy findings are filtered by.
        let cargo_health = (self.active(Producer::CargoHealth) || self.active(Producer::Clippy)).then(|| {
            panicked.contain(containment::CARGO_HEALTH, || {
                timed(timings, Phase::CargoHealth, || {
                    cargo_health::inspect(metadata, self.plan, self.options.advisory_db.as_ref())
                })
            })
        })
        .flatten();
        // The repository pass reads git, not Cargo: it sits in the
        // manifest-level slot beside the dependency pack, before the
        // compilation that Clippy triggers, and reads nothing that pass will
        // rewrite.
        // A repository is not a member: under `--package` the pass would judge
        // files the selection never named, so it does not run, and the report
        // says its rules were not evaluated.
        let repo = (self.active(Producer::Repo) && self.options.packages.is_none()).then(|| {
            panicked.contain(containment::REPO, || {
                timed(timings, Phase::Repo, || {
                    repo_hygiene::inspect(
                        metadata.workspace_root.as_std_path(),
                        view,
                        self.plan,
                        cargo_health::workspace_produces_a_binary(metadata),
                        self.options.deadline,
                    )
                })
            })
        })
        .flatten();
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
        panicked: &containment::Panicked,
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
                    panicked.contain(containment::STRUCTURE, || {
                        timed(timings, Phase::Structure, || {
                            structure::analyze(metadata, enumeration, self.plan, self.settings, cap)
                        })
                    })
                })
            });
            let source = source_in_time.then(|| {
                panicked.contain(containment::SOURCE_KERNEL, || {
                    timed(timings, Phase::SourceKernel, || {
                        source_kernel::inspect(enumeration, self.plan)
                    })
                })
            });
            // Both dependency-truth rules belong to the dependency pack, so a
            // plan that asks for either has already put a scan here to merge
            // into.
            // What a pass that panicked midway added is not kept either.
            if let Some(scan) = cargo_health.filter(|_| dependency_truth) {
                let before = scan.candidates.len();
                let judged = panicked.contain(containment::DEPENDENCY_TRUTH, || {
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
                });
                if judged.is_none() {
                    scan.candidates.truncate(before);
                }
            }
            let structure = structure.and_then(|handle| {
                handle.join().unwrap_or_else(|_| {
                    panicked.record(containment::STRUCTURE);
                    None
                })
            });
            (source.flatten(), structure)
        });
        (source, structure, deadline_skipped)
    }
}

/// What the installed Clippy lists, or the notice saying it could not be
/// asked.
pub(crate) type KnownLints = Result<BTreeSet<String>, InternalError>;

/// The three versions and the lint list, probed concurrently when
/// `lint_list` says a Clippy rule is active: four short-lived processes cost
/// the slowest of them.
///
/// A plan with no Clippy rule probes nothing and publishes the versions as
/// null, which reads "not probed". `cargo clippy --version` runs whatever a
/// workspace `[alias] clippy` names, and every probe runs the toolchain a
/// `rust-toolchain.toml` selects, so probing would start a program the
/// scanned tree chose in a scan that promises to compile nothing.
fn resolve_toolchain(
    programs: &Programs,
    workspace_root: &Path,
    environment: &CommandEnvironment,
    options: &RunOptions,
    lint_list: bool,
) -> Result<(Option<Toolchain>, Option<KnownLints>), InternalError> {
    if !lint_list {
        return Ok((None, None));
    }
    let timings = options.timings.as_ref();
    let deadline = options.deadline;
    timed(timings, Phase::Probes, || {
        thread::scope(|scope| {
            let lints = scope.spawn(move || {
                timed(timings, Phase::LintList, || {
                    lint_list::probe(&programs.clippy_driver, workspace_root, environment, deadline)
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
            let lints = lints.join().unwrap_or_else(|_| Err(lint_list::unavailable()));
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
            Ok((Some(toolchain), Some(lints)))
        })
    })
}
