#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::{Path, PathBuf};

mod audit;
mod baseline;
mod bounded_read;
mod cargo_health;
mod cargo_stderr;
mod configuration;
mod delta;
mod directive;
mod execution;
pub mod explain;
mod generated;
mod git;
pub mod git_hook;
mod git_scope;
mod internal_error;
mod path_glob;
#[cfg(test)]
mod permutations;
mod policy;
pub mod presentation;
pub mod progress;
pub mod render;
mod repo_hygiene;
mod report;
mod scan_target;
pub mod score_block;
mod source_kernel;
mod source_text;
mod structure;
#[cfg(test)]
mod test_clock;
#[cfg(test)]
#[path = "test_scratch.rs"]
mod test_scratch;
pub mod terminal_text;
mod timings;
mod workspace_path;

pub use audit::{
    Audit, AuditCategory, AuditCategoryName, AuditScore, SCORE_MODEL, ScoreDimensions, ScoreLabel,
    ScoreReason,
    SeverityCounts, ShareError,
};
pub use delta::{DeltaMatch, DeltaReport, DeltaSummary};
pub use git_scope::{ExecutionScope, ScopeMode, ScopeReport, default_branch};
pub use policy::{
    BlockingLevel, BlockingLevelSource, CatalogEntry, CategoryOverride, CrateGate, DiffScope,
    PathOverrideReport, RuleLevel, RuleLevelSource, RuleOverride, RuleTier, catalog,
};
/// The file length `rust_doctor::structure::oversized_unit` reports at.
///
/// Published because the binary crate holds the same bound over its own files
/// and must not restate the number: `src/tui/` and `src/handoff.rs` are
/// compiled into the binary, not the library, and a second `1000` written
/// there is a second place to keep in step with the rule.
pub use cargo_health::{AdvisoryDatabase, AdvisoryDbReport, AdvisorySource};
pub use structure::FILE_LINES as OVERSIZED_UNIT_FILE_LINES;
pub use timings::Timings;
pub use execution::CacheOutcome;

/// The native source detectors run in process on one unit, for the regression
/// seeds `tests/regression_seeds.rs` replays. Not a stable interface.
#[doc(hidden)]
pub mod seed_harness {
    pub use crate::source_kernel::harness::{CaseUnit, Detected, run_detectors};
}
/// The baseline comparison run on two reports already written, for the
/// pairing measurement `tests/delta_pairing.rs` replays. Not a stable
/// interface.
#[doc(hidden)]
pub mod delta_harness {
    use std::path::Path;

    use crate::{DeltaReport, Diagnostic};

    /// Pairs `current`, whose sources live under `current_root`, against
    /// `baseline`, whose sources live under `baseline_root`.
    ///
    /// # Errors
    ///
    /// The comparison's own error message, when a side exceeds its limit.
    pub fn pair(
        baseline: &[Diagnostic],
        baseline_root: &Path,
        current: &[Diagnostic],
        current_root: &Path,
    ) -> Result<DeltaReport, String> {
        let evidence = crate::delta::evidence(baseline, baseline_root);
        let map = crate::delta::DiffMap::default();
        crate::delta::compute_with(baseline, &evidence, current, current_root, &map)
            .map_err(|error| error.message)
    }
}
pub use report::{
    Applicability, Diagnostic, DiagnosticContext, DiagnosticSource, DiagnosticSpan, GateReport,
    GateStatus, InspectReport,
    InspectRequest, PackageReport, PolicyBlockingReport, PolicyReport, PolicyRuleReport,
    NotEvaluated, ProjectReport, RelatedLocation, ReportError, SCHEMA_VERSION, ScanReport,
    Severity, Status,
    Suggestion, Summary, SuppressionReport, SuppressionStatus, ToolchainReport, UnscoredReason,
};

pub fn inspect(request: InspectRequest) -> InspectReport {
    match InspectionSession::prepare(request) {
        Ok(session) => session.inspect(),
        Err(report) => *report,
    }
}

/// The root of the Cargo workspace a scan of `path` would judge, found the
/// way a scan finds it, or `None` when none resolves. `ci install` places the
/// workflow's scan from it.
pub fn cargo_workspace_root(path: &Path) -> Option<PathBuf> {
    scan_target::resolve(path, Path::new("cargo"), None)
        .ok()
        .map(|target| target.workspace_root().to_path_buf())
}

/// Installs SIGINT and SIGTERM handlers for a binary that scans. Every child
/// a scan starts then leads a process group the handlers kill, the scan
/// publishes a failed report with code `interrupted`, and [`interrupted`]
/// answers true so the binary can exit 130. A second signal exits at once,
/// and a signal outside a scan exits 130 straight away. Unix only: elsewhere
/// it does nothing.
pub fn handle_interrupts() {
    execution::interrupt::install();
}

/// Whether a signal the handlers of [`handle_interrupts`] caught stopped the
/// run.
pub fn interrupted() -> bool {
    execution::interrupt::requested()
}

/// The exit status of a run a handled signal stopped.
pub const INTERRUPTED_EXIT_STATUS: u8 = execution::interrupt::EXIT_STATUS;

#[derive(Debug)]
pub struct InspectionSession {
    prepared: execution::PreparedInspection,
    plan: policy::PolicyPlan,
    /// What the plan was compiled from, kept to compile it again from the
    /// configuration a `--staged` scan reads out of the index.
    policy: policy::PolicyInput,
    scope: git_scope::ValidatedScope,
    options: execution::RunOptions,
    _scan: execution::interrupt::ScanGuard,
}

/// Where a report's code frames read source: the workspace, or, after a
/// `--staged` scan, the tree the index was written into, which stays on disk
/// until [`FrameSource::close`] or the drop of this value removes it.
#[derive(Debug)]
pub struct FrameSource {
    root: PathBuf,
    staged: Option<baseline::Snapshot>,
}

impl FrameSource {
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Removes the staged tree, answering why it could not be.
    pub fn close(self) -> Result<(), String> {
        self.close_internal().map_err(|error| error.message)
    }

    fn close_internal(self) -> Result<(), internal_error::InternalError> {
        self.staged.map_or(Ok(()), baseline::Snapshot::cleanup)
    }
}

/// The report a handled signal leaves: one failed report naming the
/// interruption, whatever the run had reached.
fn settle(report: InspectReport) -> InspectReport {
    if interrupted() {
        report::interrupted_failure(report)
    } else {
        report
    }
}

impl InspectionSession {
    pub fn prepare(request: InspectRequest) -> Result<Self, Box<InspectReport>> {
        let policy = request.policy().clone();
        Self::prepare_with(request, &policy).map_err(|report| Box::new(settle(*report)))
    }

    fn prepare_with(
        request: InspectRequest,
        policy: &policy::PolicyInput,
    ) -> Result<Self, Box<InspectReport>> {
        let scan = execution::interrupt::ScanGuard::enter();
        // The deadline starts here, before Cargo is first asked anything, so
        // the time metadata and the baseline snapshot take counts against it,
        // and every child the run starts, `cargo metadata`, the toolchain
        // probes and git included, is killed at it.
        let options = request.run_options();
        // The overrides are read once, before any path is discovered and any
        // process is started. What comes back is what the plan compiles from,
        // so the compilation below cannot fail a second time over the same
        // answer.
        let validated = match policy.validate() {
            Ok(validated) => validated,
            Err(error) => {
                return Err(Box::new(report::policy_failure(
                    error,
                    policy.failure_blocking(),
                )));
            }
        };
        // The base selector is checked once, here, and resolution takes what
        // came back rather than the request, so it cannot fail a second time
        // over the same answer.
        let scope = match request.scope().validate() {
            Ok(scope) => scope,
            Err(error) => {
                return Err(Box::new(report::scope_failure(
                    error,
                    policy.failure_blocking(),
                )));
            }
        };
        // A staged scan reads its configuration from the index, once it is
        // written out, and never from the working tree.
        let prepared = match execution::prepare(&request.path, &options, !scope.staged()) {
            Ok(prepared) => prepared,
            Err(result) => {
                return Err(Box::new(report::preparation_failure(
                    *result,
                    policy.failure_blocking(),
                )));
            }
        };
        if let (Some(directory), Some(progress)) = (prepared.discovered(), &options.progress) {
            progress.report(progress::Progress::Discovered { directory });
        }
        // A member name is checked here, once the workspace answered which
        // members it has, and before any process is started for it.
        if let Some(Err(error)) = request
            .packages()
            .map(|selected| prepared.check_packages(selected))
        {
            return Err(Box::new(report::preparation_failure(
                prepared.fail(error),
                policy.failure_blocking(),
            )));
        }
        let plan =
            policy::PolicyPlan::compile_with_configuration(&validated, &prepared.configuration);
        Ok(Self {
            prepared,
            plan,
            policy: policy.clone(),
            scope,
            options,
            _scan: scan,
        })
    }

    pub fn workspace_root(&self) -> &Path {
        self.prepared.workspace_root()
    }

    pub fn inspect(self) -> InspectReport {
        let (report, frames) = self.inspect_with_frames();
        match frames.close_internal() {
            Ok(()) => report,
            Err(error) => settle(report::baseline_report_failure(report, error)),
        }
    }

    /// The report, and where its code frames read source. A `--staged` scan
    /// judged the index, so its frames are drawn from the tree the index was
    /// written into rather than from a working tree that may say otherwise.
    pub fn inspect_with_frames(self) -> (InspectReport, FrameSource) {
        let timings = self.options.timings.clone();
        let workspace_root = self.workspace_root().to_path_buf();
        let (report, staged) = inspect_prepared(self);
        let mut report = settle(report);
        report.timings = timings.map(|timings| timings.finish());
        let root = staged
            .as_ref()
            .map_or(workspace_root, |snapshot| snapshot.workspace().to_path_buf());
        (report, FrameSource { root, staged })
    }
}

#[cfg(test)]
fn inspect_with(request: InspectRequest, policy: &policy::PolicyInput) -> InspectReport {
    match InspectionSession::prepare_with(request, policy) {
        Ok(session) => session.inspect(),
        Err(report) => *report,
    }
}

/// The report, and the staged tree it judged, left on disk for its frames.
fn inspect_prepared(session: InspectionSession) -> (InspectReport, Option<baseline::Snapshot>) {
    let InspectionSession {
        mut prepared,
        mut plan,
        policy,
        scope: requested,
        options,
        _scan,
    } = session;
    // The index is located and read before the scope, so the staged diff and
    // the staged tree are the same index, and a conflicted or unreadable one
    // fails the scan rather than passing it empty.
    let index = if requested.staged() {
        match baseline::locate_index(prepared.workspace_root(), options.deadline) {
            Ok(index) => Some(index),
            Err(error) => {
                return (report::preparation_failure(prepared.fail(error), plan.blocking()), None);
            }
        }
    } else {
        None
    };
    let scope = match git_scope::resolve(
        &requested,
        prepared.workspace_root(),
        index.as_ref().map(baseline::StagedIndex::path),
        options.deadline,
    ) {
        Ok(scope) => scope,
        Err(error) => {
            return (report::preparation_failure(prepared.fail(error), plan.blocking()), None);
        }
    };
    let staged = match index.as_ref().map(|index| {
        baseline::materialize_index(prepared.workspace_root(), index, options.deadline)
    }) {
        None => None,
        Some(Ok(snapshot)) => {
            let target = baseline::persistent_target(prepared.target_directory(), "staged")
                .unwrap_or_else(|| snapshot.target().to_path_buf());
            Some((snapshot, target))
        }
        Some(Err(error)) => {
            return (report::from_execution_scoped(prepared.fail(error), &plan, scope), None);
        }
    };
    // The commit records the configuration the index holds, so that is the
    // one a staged scan obeys: an unstaged edit or an untracked file decides
    // nothing about it.
    if let Some(path) = staged
        .as_ref()
        .map(|(snapshot, _)| snapshot.workspace().join(configuration::CONFIG_FILE_NAME))
    {
        let validated = match policy.validate() {
            Ok(validated) => validated,
            Err(error) => {
                let report = report::policy_failure(error, policy.failure_blocking());
                return (report, staged.map(|(snapshot, _)| snapshot));
            }
        };
        match configuration::load_path(&path) {
            Ok(configuration) => {
                plan = policy::PolicyPlan::compile_with_configuration(&validated, &configuration);
                prepared.configuration = configuration;
            }
            Err(error) => {
                let report = report::preparation_failure(prepared.fail(error), plan.blocking());
                return (report, staged.map(|(snapshot, _)| snapshot));
            }
        }
    }
    let staged_side = staged.as_ref().map(|(snapshot, target)| execution::Side {
        workspace: snapshot.workspace(),
        target_dir: target,
        view: snapshot.view(),
    });
    let report = if let git_scope::ResolvedScope::Baseline { comparison_base } = scope.kind() {
        let comparison_base = comparison_base.clone();
        let index = index.as_ref().map(baseline::StagedIndex::path);
        inspect_baseline(prepared, &comparison_base, (staged_side, index), &plan, &options, scope)
    } else {
        let execution = match staged_side {
            Some(side) => execution::execute_staged(prepared, side, &plan, &options),
            None => execution::execute(prepared, &plan, &options),
        };
        report::from_execution_scoped(execution, &plan, scope)
    };
    (report, staged.map(|(snapshot, _)| snapshot))
}

/// A `--scope baseline` comparison: the base side read from the side cache
/// when it holds it, scanned from a snapshot otherwise, then the current side.
fn inspect_baseline(
    prepared: execution::PreparedInspection,
    comparison_base: &str,
    (staged, index): (Option<execution::Side<'_>>, Option<&Path>),
    plan: &policy::PolicyPlan,
    options: &execution::RunOptions,
    scope: git_scope::ScopeReport,
) -> InspectReport {
    // What the diff did is read before either side runs: the renames and the
    // line map the pairing places the base side through.
    let map = delta::DiffMap::read(prepared.workspace_root(), comparison_base, index, options.deadline);
    let setup = match execution::setup_baseline(&prepared, comparison_base, plan, options) {
        Ok(setup) => setup,
        Err(error) => {
            let failed = Err(Box::new(prepared.fail(error)));
            return report::from_baseline_execution(failed, plan, scope, &map);
        }
    };
    // A hit needs no snapshot: the cache holds the base side's findings and
    // the evidence the delta pairs them on.
    let snapshot = if setup.needs_snapshot() {
        match baseline::materialize(prepared.workspace_root(), comparison_base, options.deadline) {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                return report::from_execution_scoped(prepared.fail(error), plan, scope);
            }
        }
    } else {
        None
    };
    let target = snapshot.as_ref().map(|snapshot| {
        baseline::persistent_target(prepared.target_directory(), "baseline")
            .unwrap_or_else(|| snapshot.target().to_path_buf())
    });
    let base = snapshot.as_ref().zip(target.as_deref()).map(|(snapshot, target_dir)| {
        execution::Side {
            workspace: snapshot.workspace(),
            target_dir,
            view: snapshot.view(),
        }
    });
    let execution = execution::execute_baseline(prepared, setup, base, staged, plan, options);
    let report = report::from_baseline_execution(execution, plan, scope, &map);
    match snapshot.map(baseline::Snapshot::cleanup) {
        None | Some(Ok(())) => report,
        Some(Err(error)) => report::baseline_report_failure(report, error),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::policy::{PolicyInput, RuleLevel};

    #[test]
    fn invalid_policy_returns_before_execution_or_path_discovery() {
        let request = InspectRequest::new("/path/that/must/not/be/inspected");
        let policy = PolicyInput::default().with_rule("unknown::rule", RuleLevel::Warn);

        let report = inspect_with(request, &policy);

        assert_eq!(report.status, Status::Failed);
        assert!(!report.complete);
        assert!(report.project.is_none());
        assert!(report.scan.command.is_none());
        assert!(report.diagnostics.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].stage, "policy");
        assert_eq!(report.errors[0].code, "unknown-rule");
    }

    #[test]
    fn clippy_off_prunes_execution_and_error_preserves_warning_identity() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kernel-contract/todo");
        let baseline = inspect_with(InspectRequest::new(&fixture), &PolicyInput::default());
        assert_eq!(baseline.status, Status::Complete);
        let baseline_ids: Vec<_> = baseline
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_deref() == Some("clippy::todo"))
            .map(|diagnostic| diagnostic.id.clone())
            .collect();
        assert!(!baseline_ids.is_empty());

        let off_policy = PolicyInput::default().with_rule("clippy::todo", RuleLevel::Off);
        let off = inspect_with(InspectRequest::new(&fixture), &off_policy);
        assert_eq!(off.status, Status::Complete);
        assert!(
            off.diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code.as_deref() != Some("clippy::todo"))
        );
        let off_command = off
            .scan
            .command
            .expect("complete scan should expose its command");
        assert!(
            !off_command
                .iter()
                .any(|argument| argument == "clippy::todo")
        );

        let error_policy = PolicyInput::default().with_rule("clippy::todo", RuleLevel::Error);
        let error = inspect_with(InspectRequest::new(&fixture), &error_policy);
        assert_eq!(error.status, Status::Complete);
        let error_ids: Vec<_> = error
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_deref() == Some("clippy::todo"))
            .map(|diagnostic| {
                assert_eq!(diagnostic.base_severity, Severity::Warning);
                assert_eq!(diagnostic.severity, Severity::Error);
                diagnostic.id.clone()
            })
            .collect();
        assert_eq!(error_ids, baseline_ids);
        assert_eq!(error.gate.status, GateStatus::Failed);
        assert_eq!(error.gate.blocking_diagnostics, Some(error_ids.len()));
        let error_command = error
            .scan
            .command
            .expect("complete scan should expose its command");
        assert!(
            error_command
                .windows(2)
                .any(|pair| pair == ["-W", "clippy::todo"])
        );
        assert!(!error_command.iter().any(|argument| argument == "-D"));
    }
}
