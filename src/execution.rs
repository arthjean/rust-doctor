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
#[cfg(test)]
pub(crate) use messages::{CapturedDiagnosticCode, CapturedTarget};

#[derive(Debug)]
pub(crate) struct ExecutionResult {
    pub(crate) manifest_path: Option<PathBuf>,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) toolchain: Option<Toolchain>,
    pub(crate) scan: ClippyExecution,
    pub(crate) source: Option<SourceScan>,
    pub(crate) source_measurement: Option<SourceMeasurement>,
    pub(crate) structure: Option<StructureScan>,
    pub(crate) cargo_health: Option<CargoHealthScan>,
    pub(crate) repo: Option<RepoScan>,
    pub(crate) deadline_skipped: Vec<(&'static str, String)>,
    pub(crate) panicked: Vec<containment::Pass>,
    pub(crate) exclusions: ScanExclusions,
    pub(crate) error: Option<InternalError>,
}

#[derive(Debug)]
pub(crate) struct ProducerError<'a> {
    pub(crate) stage: &'static str,
    pub(crate) code: &'static str,
    pub(crate) message: &'a str,
}

impl ExecutionResult {
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

    pub(crate) fn producer_errors(&self) -> impl Iterator<Item = ProducerError<'_>> {
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

    pub(crate) fn discovered(&self) -> Option<&str> {
        self.target.discovered.as_deref()
    }

    pub(crate) fn check_packages(&self, selected: &BTreeSet<String>) -> Result<(), InternalError> {
        exclusions::check_packages(&self.target.metadata, selected)
    }

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

#[derive(Debug, Clone, Default)]
pub(crate) struct RunOptions {
    pub(crate) deadline: Option<RunDeadline>,
    pub(crate) progress: Option<ProgressSink>,
    pub(crate) packages: Option<BTreeSet<String>>,
    pub(crate) advisory_db: Option<cargo_health::AdvisoryDatabase>,
    pub(crate) timings: Option<TimingRecorder>,
}

#[derive(Debug, Clone, Copy)]
struct ExecutionContext<'a> {
    programs: &'a Programs,
    plan: &'a PolicyPlan,
    settings: &'a structure::StructureSettings,
    environment: &'a CommandEnvironment,
    options: &'a RunOptions,
    declared: &'a BTreeSet<String>,
}

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
        let panicked = containment::Panicked::default();
        let turn = lockfile::Turn::take(metadata.workspace_root.as_std_path(), self.options.deadline);
        let ((mut cargo_health, repo), walked) = thread::scope(|scope| {
            let manifests = scope.spawn(|| self.manifest_passes(&metadata, view, &panicked));
            let walked = panicked.contain(containment::WALK, || self.walk(&metadata));
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

        let (clippy, natives) = thread::scope(|scope| {
            let clippy = self.active(Producer::Clippy).then(|| {
                let (metadata, panicked, turn) = (&metadata, &panicked, &turn);
                scope.spawn(move || {
                    panicked.contain(containment::CLIPPY, || {
                        clippy::run(self, metadata, target_dir, lints, turn)
                    })
                })
            });
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
        drop(turn);
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

    fn manifest_passes(
        &self,
        metadata: &Metadata,
        view: Option<&GitView>,
        panicked: &containment::Panicked,
    ) -> (Option<CargoHealthScan>, Option<RepoScan>) {
        let timings = self.options.timings.as_ref();
        let cargo_health = (self.active(Producer::CargoHealth) || self.active(Producer::Clippy)).then(|| {
            panicked.contain(containment::CARGO_HEALTH, || {
                timed(timings, Phase::CargoHealth, || {
                    cargo_health::inspect(metadata, self.plan, self.options.advisory_db.as_ref())
                })
            })
        })
        .flatten();
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

    fn walk(&self, metadata: &Metadata) -> (source_kernel::Enumeration, exclusions::ScanExclusions) {
        if let Some(progress) = &self.options.progress {
            progress.report(Progress::NativePasses);
        }
        let packages = self.options.packages.as_ref();
        timed(self.options.timings.as_ref(), Phase::Walk, || {
            let mut enumeration = source_kernel::enumerate(metadata);
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

    fn native_passes(
        &self,
        metadata: &Metadata,
        enumeration: &source_kernel::Enumeration,
        cargo_health: Option<&mut CargoHealthScan>,
        panicked: &containment::Panicked,
    ) -> (Option<SourceScan>, Option<StructureScan>, Vec<(&'static str, String)>) {
        let timings = self.options.timings.as_ref();
        let mut deadline_skipped = Vec::new();
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

pub(crate) type KnownLints = Result<BTreeSet<String>, InternalError>;

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
