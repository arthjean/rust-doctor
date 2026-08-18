//! Running the producers over one resolved workspace, and saying what came
//! back.
//!
//! The module is the orchestration and nothing else. `clippy` is the pass that
//! compiles, `messages` the stream it answers on, and `baseline` the dual run a
//! comparison needs; the four native producers live in their own modules and
//! are called from here.
//!
//! One rule holds it together: **the result is assembled from what came back,
//! never asked back for what it was given.** `ExecutionContext::run` owns the
//! metadata until the end, so the three `Option` dances that could not be
//! `None`, and the workspace-root clone that only existed to survive an early
//! move, are gone with them.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cargo_metadata::Metadata;

use crate::cargo_health::{self, CargoHealthScan};
use crate::configuration::{self, WorkspaceConfiguration};
use crate::internal_error::InternalError;
use crate::policy::{PolicyPlan, Producer};
use crate::repo_hygiene::{self, RepoScan};
use crate::scan_target::{self, ResolvedScanTarget};
use crate::source_kernel::{self, SourceScan};
use crate::structure::{self, StructureScan};

mod baseline;
mod clippy;
mod messages;
#[cfg(test)]
mod tests;

pub(crate) use baseline::{BaselineExecution, execute as execute_baseline};
pub(crate) use clippy::ClippyExecution;
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
    pub(crate) structure: Option<StructureScan>,
    pub(crate) cargo_health: Option<CargoHealthScan>,
    pub(crate) repo: Option<RepoScan>,
    pub(crate) error: Option<InternalError>,
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
            structure: None,
            cargo_health: None,
            repo: None,
            error: Some(error),
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.error.is_none()
            && self.scan.is_complete()
            && self
                .source
                .as_ref()
                .is_none_or(|source| source.errors.is_empty())
            && self
                .structure
                .as_ref()
                .is_none_or(|structure| structure.errors.is_empty())
            && self.repo.as_ref().is_none_or(|repo| repo.errors.is_empty())
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
    target: ResolvedScanTarget,
    pub(crate) configuration: WorkspaceConfiguration,
}

impl PreparedInspection {
    pub(crate) fn workspace_root(&self) -> &Path {
        self.target.workspace_root()
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
}

impl Default for Programs {
    fn default() -> Self {
        Self {
            cargo: PathBuf::from("cargo"),
            rustc: PathBuf::from("rustc"),
        }
    }
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

pub(crate) fn prepare(path: &Path) -> Result<PreparedInspection, Box<ExecutionResult>> {
    prepare_with(path, &Programs::default())
}

fn prepare_with(
    path: &Path,
    programs: &Programs,
) -> Result<PreparedInspection, Box<ExecutionResult>> {
    let target = match scan_target::resolve(path, &programs.cargo) {
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

pub(crate) fn execute(prepared: PreparedInspection, plan: &PolicyPlan) -> ExecutionResult {
    execute_with_plan(prepared, &Programs::default(), plan)
}

fn execute_with_plan(
    prepared: PreparedInspection,
    programs: &Programs,
    plan: &PolicyPlan,
) -> ExecutionResult {
    let environment = CommandEnvironment::default();
    let toolchain = match resolve_toolchain(programs, prepared.workspace_root(), &environment) {
        Ok(toolchain) => toolchain,
        Err(error) => return prepared.fail(error),
    };
    let context = ExecutionContext {
        programs,
        plan,
        settings: &prepared.configuration.structure,
        environment: &environment,
    };
    context.run(prepared.target, toolchain, None)
}

impl ExecutionContext<'_> {
    /// Runs every producer the plan leaves on against one resolved target.
    fn run(
        &self,
        target: ResolvedScanTarget,
        toolchain: Toolchain,
        target_dir: Option<&Path>,
    ) -> ExecutionResult {
        let ResolvedScanTarget {
            manifest_path,
            metadata,
        } = target;
        let source_rules = self.active(Producer::SourceKernel);
        let structure_rules = self.active(Producer::Structure);
        let dependency_truth = cargo_health::dependency_truth_required(self.plan);

        // The dependency pack reads the resolved graph from disk, so it
        // produces bounded errors like the source kernel: it runs here, not
        // during normalization, so that its errors join those of the scan.
        //
        // It runs before Clippy: `cargo clippy` creates or rewrites
        // `Cargo.lock`, so reading it afterwards would measure the graph the
        // tool just wrote instead of the one the repository commits.
        let mut cargo_health = self
            .active(Producer::CargoHealth)
            .then(|| cargo_health::inspect(&metadata, self.plan));
        // The repository pass reads git, not Cargo: it sits in the
        // manifest-level slot beside the dependency pack, before the
        // compilation that Clippy triggers, and reads nothing that pass will
        // rewrite.
        let repo = self
            .active(Producer::Repo)
            .then(|| repo_hygiene::inspect(metadata.workspace_root.as_std_path(), self.plan));

        let scan = if self.active(Producer::Clippy) {
            match clippy::run(
                self.programs,
                metadata.workspace_root.as_std_path(),
                self.plan,
                target_dir,
                self.environment,
            ) {
                Ok(scan) => ClippyExecution::Finished(scan),
                // Clippy could not be started, so this result has already
                // failed and nothing it publishes will be read. The passes
                // below are skipped here rather than behind an `error.is_none()`
                // clause further down, which read as a general guard and was
                // really "Clippy started".
                Err(error) => {
                    return ExecutionResult {
                        toolchain: Some(toolchain),
                        cargo_health,
                        repo,
                        ..ExecutionResult::failed(Some(manifest_path), Some(metadata), error)
                    };
                }
            }
        } else {
            ClippyExecution::Disabled
        };

        // One walk feeds every producer that reads source text. The source
        // kernel decides per call site, the structural pass decides per family,
        // the dependency-truth rules read crate references off the same units,
        // and enumerating the workspace twice for any of them would double the
        // only expensive part of each.
        let (source, structure) = if source_rules || structure_rules || dependency_truth {
            let enumeration = source_kernel::enumerate(&metadata);
            let scans = (
                source_rules.then(|| source_kernel::inspect(&enumeration, self.plan)),
                structure_rules
                    .then(|| structure::analyze(&metadata, &enumeration, self.plan, self.settings)),
            );
            // Both dependency-truth rules belong to the dependency pack, so a
            // plan that asks for either has already put a scan here to merge
            // into.
            if let Some(scan) = cargo_health.as_mut().filter(|_| dependency_truth) {
                let references = source_kernel::references::collect(&enumeration);
                cargo_health::inspect_dependency_truth(
                    &metadata,
                    &enumeration,
                    &references,
                    self.plan,
                    scan,
                );
            }
            scans
        } else {
            (None, None)
        };

        ExecutionResult {
            manifest_path: Some(manifest_path),
            metadata: Some(metadata),
            toolchain: Some(toolchain),
            scan,
            source,
            structure,
            cargo_health,
            repo,
            error: None,
        }
    }

    fn active(&self, producer: Producer) -> bool {
        self.plan.active_rules(producer).next().is_some()
    }
}

fn resolve_toolchain(
    programs: &Programs,
    workspace_root: &Path,
    environment: &CommandEnvironment,
) -> Result<Toolchain, InternalError> {
    let cargo = tool_version(
        &programs.cargo,
        &["--version"],
        workspace_root,
        "cargo-unavailable",
        "Cargo",
        environment,
    )?;
    let rustc = tool_version(
        &programs.rustc,
        &["--version"],
        workspace_root,
        "rustc-unavailable",
        "rustc",
        environment,
    )?;
    let clippy = tool_version(
        &programs.cargo,
        &["clippy", "--version"],
        workspace_root,
        "clippy-unavailable",
        "Clippy",
        environment,
    )?;
    Ok(Toolchain {
        cargo,
        rustc,
        clippy,
    })
}

fn tool_version(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    code: &'static str,
    label: &str,
    environment: &CommandEnvironment,
) -> Result<String, InternalError> {
    let output = version_command(program, arguments, working_directory, environment)
        .output()
        .map_err(|error| {
            InternalError::new(
                "execution",
                code,
                format!("{label} could not be started: {error}"),
            )
        })?;

    if !output.status.success() {
        return Err(InternalError::new(
            "execution",
            code,
            format!("{label} exited with status {}", output.status),
        ));
    }

    let version = String::from_utf8(output.stdout).map_err(|error| {
        InternalError::new(
            "execution",
            code,
            format!("{label} returned non-UTF-8 version output: {error}"),
        )
    })?;
    let version = version.trim();
    if version.is_empty() {
        return Err(InternalError::new(
            "execution",
            code,
            format!("{label} returned an empty version"),
        ));
    }

    Ok(version.to_owned())
}

fn version_command(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    environment: &CommandEnvironment,
) -> Command {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    environment.apply(&mut command);
    command
}
