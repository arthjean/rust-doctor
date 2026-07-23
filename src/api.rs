//! Versioned programmatic scan API.
//!
//! The API never renders terminal output, exits the process, mutates global
//! configuration, or enables network access by default. Scans may run Cargo
//! subprocesses and update Rust Doctor's bounded project cache. Cancellation is
//! thread-safe and shared with supported child process groups. Report V1 is the
//! semver-governed wire contract; orchestration request types remain additive
//! while the crate is pre-1.0.

use crate::cli::{AdapterGroup, Cli};
use crate::config::{AdapterPolicy, FileConfig};
use crate::diagnostics::{CompletenessState, GateResult, ReportV1, ScanMode};
use clap::Parser;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const MAX_BATCH_PROJECTS: usize = 256;
const MAX_BATCH_PARALLELISM: usize = 64;

/// Caller-owned cancellation shared by one scan or a complete batch.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Request cancellation. Supported child process groups are terminated and
    /// the resulting report records incomplete work instead of claiming clean.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Reporting and materialization scope for a programmatic scan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ScanScope {
    #[default]
    Full,
    Files(Vec<PathBuf>),
    Changed {
        base: Option<String>,
        include_untracked: bool,
    },
    Lines {
        base: Option<String>,
        include_untracked: bool,
    },
    Staged,
    Baseline {
        base: Option<String>,
    },
}

/// Options shared by single-project and batch scans.
#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub scope: ScanScope,
    pub deadline: Option<Duration>,
    pub cancellation: CancellationToken,
    /// Typed overrides merged above discovered project configuration.
    pub config_overrides: FileConfig,
    pub adapters: AdapterPolicy,
    pub workspace_parallelism: Option<usize>,
    pub use_project_config: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            scope: ScanScope::Full,
            deadline: None,
            cancellation: CancellationToken::default(),
            config_overrides: FileConfig::default(),
            adapters: AdapterPolicy {
                network: false,
                ..AdapterPolicy::default()
            },
            workspace_parallelism: None,
            use_project_config: true,
        }
    }
}

/// One project scan request.
#[derive(Clone, Debug)]
pub struct ScanRequest {
    pub root: PathBuf,
    pub options: ScanOptions,
}

impl ScanRequest {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            options: ScanOptions::default(),
        }
    }
}

/// Per-project overrides applied above [`BatchScanRequest::options`].
#[derive(Clone, Debug)]
pub struct BatchProjectRequest {
    pub root: PathBuf,
    pub config_overrides: FileConfig,
    pub adapters: Option<AdapterPolicy>,
}

impl BatchProjectRequest {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            config_overrides: FileConfig::default(),
            adapters: None,
        }
    }
}

/// A bounded batch. Projects execute concurrently up to `max_parallelism`.
#[derive(Clone, Debug)]
pub struct BatchScanRequest {
    pub projects: Vec<BatchProjectRequest>,
    pub options: ScanOptions,
    pub max_parallelism: usize,
}

/// One ordered project result. A failure never discards sibling reports.
#[derive(Debug)]
pub struct BatchProjectResult {
    pub requested_root: PathBuf,
    pub result: Result<ReportV1, ScanApiError>,
}

/// Deterministic batch summary without inventing an aggregate score formula.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchAggregate {
    pub projects: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub diagnostic_count: usize,
    pub all_successes_complete: bool,
}

#[derive(Debug)]
pub struct BatchScanResult {
    pub projects: Vec<BatchProjectResult>,
    pub aggregate: BatchAggregate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheInvalidation {
    NotPresent,
    Removed,
}

/// Typed setup, policy, scan, and cache failures.
#[derive(Debug, thiserror::Error)]
pub enum ScanApiError {
    #[error("invalid scan request: {0}")]
    InvalidRequest(String),
    #[error("project bootstrap failed: {0}")]
    Bootstrap(#[from] crate::error::BootstrapError),
    #[error("configuration failed: {0}")]
    Config(#[from] crate::error::ConfigError),
    #[error("scan failed: {0}")]
    Scan(#[from] crate::error::ScanError),
    #[error("batch worker panicked before returning a typed result")]
    WorkerPanicked,
    #[error("scan was cancelled before this project was scheduled")]
    Cancelled,
    #[error("failed to invalidate cache '{}': {source}", path.display())]
    CacheIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Scan one project and return Report V1 without rendering or process exit.
pub fn scan(request: ScanRequest) -> Result<ReportV1, ScanApiError> {
    validate_options(&request.options)?;
    let requested_root = request.root.clone();
    let cli = cli_for_request(&requested_root, &request.options)?;
    let (_, project, discovered_config) =
        crate::discovery::bootstrap_project(&requested_root, !request.options.adapters.network)?;
    let mut effective = if request.options.use_project_config {
        discovered_config.unwrap_or_default()
    } else {
        FileConfig::default()
    };
    merge_file_config(&mut effective, request.options.config_overrides);
    crate::config::validate_file_config(&effective, &project.root_dir.join("rust-doctor.toml"))?;
    let mut resolved = crate::config::resolve_config(&cli, Some(&effective));
    resolved.adapter_policy = request.options.adapters;
    resolved.max_parallelism = request.options.workspace_parallelism;
    resolved.diff = None;

    let result = crate::run::run_scan_cancellable(
        &cli,
        &project,
        &resolved,
        &request.options.cancellation.cancelled,
    )?;
    Ok(ReportV1::from_scan_with_context(
        &result,
        &project,
        &resolved,
        report_mode(&result.execution.reporting_scope),
        &requested_root,
        GateResult::NotEvaluated,
    ))
}

/// Scan a bounded batch, preserving input order and successful sibling reports.
pub fn scan_batch(request: BatchScanRequest) -> Result<BatchScanResult, ScanApiError> {
    if request.projects.is_empty() {
        return Err(ScanApiError::InvalidRequest(
            "batch must contain at least one project".to_string(),
        ));
    }
    if request.projects.len() > MAX_BATCH_PROJECTS {
        return Err(ScanApiError::InvalidRequest(format!(
            "batch contains {} projects; the maximum is {MAX_BATCH_PROJECTS}",
            request.projects.len()
        )));
    }
    if request.max_parallelism == 0 || request.max_parallelism > MAX_BATCH_PARALLELISM {
        return Err(ScanApiError::InvalidRequest(format!(
            "batch parallelism must be between 1 and {MAX_BATCH_PARALLELISM}"
        )));
    }
    validate_options(&request.options)?;
    let BatchScanRequest {
        projects: requests,
        options,
        max_parallelism,
    } = request;

    let mut projects = Vec::with_capacity(requests.len());
    let mut next_project = 0;
    while next_project < requests.len() {
        if options.cancellation.is_cancelled() {
            projects.extend(
                requests[next_project..]
                    .iter()
                    .map(|project| BatchProjectResult {
                        requested_root: project.root.clone(),
                        result: Err(ScanApiError::Cancelled),
                    }),
            );
            break;
        }
        let chunk_end = (next_project + max_parallelism).min(requests.len());
        let chunk = &requests[next_project..chunk_end];
        let mut chunk_results = std::thread::scope(|scope| {
            // All workers must be spawned before the first join to preserve concurrency.
            #[allow(clippy::needless_collect)]
            let handles: Vec<_> = chunk
                .iter()
                .cloned()
                .map(|project| {
                    let mut project_options = options.clone();
                    merge_file_config(
                        &mut project_options.config_overrides,
                        project.config_overrides,
                    );
                    if let Some(adapters) = project.adapters {
                        project_options.adapters = adapters;
                    }
                    let requested_root = project.root.clone();
                    (
                        requested_root,
                        scope.spawn(move || {
                            scan(ScanRequest {
                                root: project.root,
                                options: project_options,
                            })
                        }),
                    )
                })
                .collect();
            handles
                .into_iter()
                .map(|(requested_root, handle)| BatchProjectResult {
                    requested_root,
                    result: handle.join().unwrap_or(Err(ScanApiError::WorkerPanicked)),
                })
                .collect::<Vec<_>>()
        });
        projects.append(&mut chunk_results);
        next_project = chunk_end;
    }

    let succeeded = projects
        .iter()
        .filter(|project| project.result.is_ok())
        .count();
    let diagnostic_count = projects
        .iter()
        .filter_map(|project| project.result.as_ref().ok())
        .map(|report| report.diagnostics.len())
        .sum();
    let all_successes_complete = succeeded > 0
        && projects
            .iter()
            .filter_map(|project| project.result.as_ref().ok())
            .all(|report| report.completeness.state == CompletenessState::Complete);
    Ok(BatchScanResult {
        aggregate: BatchAggregate {
            projects: projects.len(),
            succeeded,
            failed: projects.len() - succeeded,
            diagnostic_count,
            all_successes_complete,
        },
        projects,
    })
}

/// Remove only Rust Doctor's cache file for the nearest Cargo project.
pub fn invalidate_cache(root: &Path) -> Result<CacheInvalidation, ScanApiError> {
    let requested = root
        .canonicalize()
        .map_err(|source| ScanApiError::CacheIo {
            path: root.to_path_buf(),
            source,
        })?;
    let project_root = crate::discovery::find_manifest_root(&requested).ok_or_else(|| {
        ScanApiError::InvalidRequest(format!(
            "no Cargo.toml found at or above '{}'",
            requested.display()
        ))
    })?;
    let path = project_root.join(".rust-doctor-cache.json");
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(CacheInvalidation::Removed),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(CacheInvalidation::NotPresent)
        }
        Err(source) => Err(ScanApiError::CacheIo { path, source }),
    }
}

fn validate_options(options: &ScanOptions) -> Result<(), ScanApiError> {
    if options.deadline.is_some_and(|deadline| deadline.is_zero()) {
        return Err(ScanApiError::InvalidRequest(
            "deadline must be greater than zero".to_string(),
        ));
    }
    if options
        .workspace_parallelism
        .is_some_and(|parallelism| parallelism == 0)
    {
        return Err(ScanApiError::InvalidRequest(
            "workspace parallelism must be greater than zero".to_string(),
        ));
    }
    if matches!(&options.scope, ScanScope::Files(files) if files.is_empty()) {
        return Err(ScanApiError::InvalidRequest(
            "files scope requires at least one path".to_string(),
        ));
    }
    Ok(())
}

fn cli_for_request(root: &Path, options: &ScanOptions) -> Result<Cli, ScanApiError> {
    let mut arguments = vec![OsString::from("rust-doctor"), root.as_os_str().to_owned()];
    if !options.adapters.network {
        arguments.push(OsString::from("--offline"));
    }
    for (group, enabled) in [
        (AdapterGroup::CompilerLint, options.adapters.compiler_lint),
        (AdapterGroup::CustomAst, options.adapters.custom_ast),
        (AdapterGroup::SupplyChain, options.adapters.supply_chain),
        (AdapterGroup::Quality, options.adapters.quality),
        (AdapterGroup::NetworkDependent, options.adapters.network),
    ] {
        arguments.push(OsString::from(if enabled {
            "--enable-adapter"
        } else {
            "--disable-adapter"
        }));
        arguments.push(OsString::from(group.to_string()));
    }
    if let Some(parallelism) = options.workspace_parallelism {
        arguments.push(OsString::from("--jobs"));
        arguments.push(OsString::from(parallelism.to_string()));
    }
    if let Some(deadline) = options.deadline {
        arguments.push(OsString::from("--max-duration"));
        let seconds = deadline
            .as_secs()
            .saturating_add(u64::from(deadline.subsec_nanos() > 0))
            .max(1);
        arguments.push(OsString::from(seconds.to_string()));
    }
    append_scope_arguments(&mut arguments, &options.scope);
    let cli = Cli::try_parse_from(arguments)
        .map_err(|error| ScanApiError::InvalidRequest(error.to_string()))?;
    cli.validate_contract()
        .map_err(|error| ScanApiError::InvalidRequest(error.to_string()))?;
    Ok(cli)
}

fn append_scope_arguments(arguments: &mut Vec<OsString>, scope: &ScanScope) {
    match scope {
        ScanScope::Full => {}
        ScanScope::Files(files) => {
            arguments.extend([OsString::from("--scope"), OsString::from("files")]);
            for file in files {
                arguments.push(OsString::from("--files"));
                arguments.push(file.as_os_str().to_owned());
            }
        }
        ScanScope::Changed {
            base,
            include_untracked,
        }
        | ScanScope::Lines {
            base,
            include_untracked,
        } => {
            arguments.extend([
                OsString::from("--scope"),
                OsString::from(if matches!(scope, ScanScope::Lines { .. }) {
                    "lines"
                } else {
                    "changed"
                }),
            ]);
            if let Some(base) = base {
                arguments.extend([OsString::from("--base"), OsString::from(base)]);
            }
            if *include_untracked {
                arguments.push(OsString::from("--include-untracked"));
            }
        }
        ScanScope::Staged => arguments.push(OsString::from("--staged")),
        ScanScope::Baseline { base } => {
            arguments.push(OsString::from("--baseline"));
            if let Some(base) = base {
                arguments.extend([OsString::from("--base"), OsString::from(base)]);
            }
        }
    }
}

fn merge_file_config(base: &mut FileConfig, overrides: FileConfig) {
    if !overrides.ignore.rules.is_empty() {
        base.ignore.rules = overrides.ignore.rules;
    }
    if !overrides.ignore.files.is_empty() {
        base.ignore.files = overrides.ignore.files;
    }
    if !overrides.ignore.enable.is_empty() {
        base.ignore.enable = overrides.ignore.enable;
    }
    if overrides.lint.is_some() {
        base.lint = overrides.lint;
    }
    if overrides.dependencies.is_some() {
        base.dependencies = overrides.dependencies;
    }
    if overrides.verbose.is_some() {
        base.verbose = overrides.verbose;
    }
    if overrides.diff.is_some() {
        base.diff = overrides.diff;
    }
    if overrides.fail_on.is_some() {
        base.fail_on = overrides.fail_on;
    }
    base.rules.extend(overrides.rules);
    base.categories.extend(overrides.categories);
    base.tags.extend(overrides.tags);
    if !overrides.path_overrides.is_empty() {
        base.path_overrides = overrides.path_overrides;
    }
    base.rules_config.extend(overrides.rules_config);
    if overrides.score.fail_below.is_some() {
        base.score = overrides.score;
    }
}

fn report_mode(reporting_scope: &str) -> ScanMode {
    match reporting_scope {
        "files" | "changed" => ScanMode::Files,
        "lines" => ScanMode::Lines,
        "staged" => ScanMode::Staged,
        "baseline" => ScanMode::Baseline,
        _ => ScanMode::Full,
    }
}
