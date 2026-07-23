//! Helper functions extracted from `main.rs` for the scan pipeline orchestration.
//!
//! These functions handle MCP dispatch, project bootstrapping, scanning,
//! output rendering, and quality gate checks.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::cli::{Cli, ColorMode, FailOn, ScanCategory, Scope, WarningVisibility};

/// Exit code for scan errors (project doesn't compile, discovery fails).
pub const EXIT_SCAN_ERROR: u8 = 2;
/// Exit code for quality gate failures (score below threshold, --fail-on).
pub const EXIT_GATE_FAILURE: u8 = 3;
/// Exit code for incomplete required analysis.
pub const EXIT_INCOMPLETE_ANALYSIS: u8 = 4;
use crate::diagnostics::{
    BaselineReport, Category, CheckState, CheckStatus, CompletenessState, GateResult, ReportV1,
    ScanMode, ScanResult,
};
use crate::{config, deps, diff, discovery, fixer, output, plan, sarif, scan};

/// Apply the root color policy before any command emits output.
pub fn configure_color(cli: &Cli) {
    match if cli.no_color {
        ColorMode::Never
    } else {
        cli.color
    } {
        ColorMode::Auto => owo_colors::unset_override(),
        ColorMode::Always => owo_colors::set_override(true),
        ColorMode::Never => owo_colors::set_override(false),
    }
}

/// Install an opt-in, privacy-scrubbed crash hook for the selected surface.
pub fn configure_telemetry(cli: &Cli) {
    let surface = if cli.mcp {
        crate::telemetry::mcp_surface()
    } else if cli.lsp {
        crate::telemetry::lsp_surface()
    } else {
        crate::telemetry::cli_surface()
    };
    crate::telemetry::install_panic_hook(cli.no_telemetry, cli.offline, surface, &cli.directory);
}

/// Record one aggregate server session when explicit consent is active.
pub fn emit_server_telemetry(cli: &Cli) {
    let surface = if cli.mcp {
        crate::telemetry::mcp_surface()
    } else {
        crate::telemetry::lsp_surface()
    };
    crate::telemetry::record_session(cli.no_telemetry, cli.offline, surface);
}

/// Record aggregate scan metadata without affecting output or exit status.
pub fn emit_scan_telemetry(cli: &Cli, result: &ScanResult) {
    crate::telemetry::record_scan(cli.no_telemetry, cli.offline, result);
}

/// Print the explicit stateless share URL after normal terminal output.
pub fn emit_share_if_requested(cli: &Cli, result: &ScanResult) -> Result<(), String> {
    if !cli.share {
        return Ok(());
    }
    let url = crate::share::build_url(result).map_err(|error| error.to_string())?;
    println!("\nShare: {url}");
    Ok(())
}

/// Dispatch a typed subcommand before project bootstrap and scan subprocesses.
pub fn handle_command(cli: &Cli) -> Option<ExitCode> {
    cli.command.as_ref().map(crate::workflows::dispatch::handle)
}

/// Run the interactive setup wizard. Returns exit code.
pub fn handle_setup() -> ExitCode {
    match crate::setup::run_setup() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Dispatch `--mcp` flag: start the MCP server or report a compile-time error.
/// Returns `Some(ExitCode)` if MCP was handled, `None` to continue normal flow.
pub fn handle_mcp_flag(cli: &Cli) -> Option<ExitCode> {
    #[cfg(feature = "mcp")]
    if cli.mcp {
        return Some(match crate::mcp::run_mcp_server() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Error: MCP server failed: {e}");
                ExitCode::FAILURE
            }
        });
    }

    #[cfg(not(feature = "mcp"))]
    if cli.mcp {
        eprintln!("Error: MCP support not compiled in. Rebuild with `--features mcp`.");
        return Some(ExitCode::FAILURE);
    }

    None
}

/// Dispatch `--lsp`: start the language server or report that the feature was
/// omitted from this binary. Returns `None` for normal scan execution.
pub fn handle_lsp_flag(cli: &Cli) -> Option<ExitCode> {
    #[cfg(feature = "lsp")]
    if cli.lsp {
        return Some(match crate::lsp::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Error: language server failed: {error}");
                ExitCode::FAILURE
            }
        });
    }

    #[cfg(not(feature = "lsp"))]
    if cli.lsp {
        eprintln!("Error: LSP support not compiled in. Rebuild with `--features lsp`.");
        return Some(ExitCode::FAILURE);
    }

    None
}

/// Resolve directory, discover the project, and load file-based configuration.
pub fn bootstrap_project(
    cli: &Cli,
) -> Result<
    (
        std::path::PathBuf,
        discovery::ProjectInfo,
        Option<config::FileConfig>,
    ),
    crate::error::BootstrapError,
> {
    discovery::bootstrap_project(&cli.directory, cli.offline)
}

/// Run the scan passes and return the result.
pub fn run_scan(
    cli: &Cli,
    project_info: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
) -> Result<ScanResult, crate::error::ScanError> {
    run_scan_cancellable(
        cli,
        project_info,
        resolved,
        &Arc::new(AtomicBool::new(false)),
    )
}

/// Run the canonical scoped scan with a caller-owned cancellation flag.
#[expect(
    clippy::too_many_lines,
    reason = "CLI scope dispatch keeps category selection aligned across full, changed, staged, and baseline scans"
)]
pub(crate) fn run_scan_cancellable(
    cli: &Cli,
    project_info: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
    cancel: &Arc<AtomicBool>,
) -> Result<ScanResult, crate::error::ScanError> {
    let suppress_spinner = cli.score || cli.wants_json() || cli.sarif;
    if cli.fail_on.is_some() {
        eprintln!("Warning: --fail-on is deprecated; use --blocking");
    }
    let request = scope_request(cli, resolved)?;
    let control = crate::process::ScanControl::new(
        Arc::clone(cancel),
        cli.max_duration.map(Duration::from_secs),
    );
    let selected_categories = diagnostic_categories(&cli.category);
    let result = match diff::resolve_scope(&project_info.root_dir, &request, &resolved.ignore_files)
    {
        Ok(scope)
            if scope.reporting_scope == diff::ReportingScope::Changed
                && scope.has_applicable_work()
                && !has_uncommitted_scope_work(
                    project_info,
                    resolved,
                    request.include_untracked,
                )? =>
        {
            let mut baseline_scope = scope.clone();
            baseline_scope.reporting_scope = diff::ReportingScope::Baseline;
            let mut result = run_baseline(
                cli,
                project_info,
                resolved,
                suppress_spinner,
                &baseline_scope,
                &control,
                &selected_categories,
            )?;
            result.diagnostics = diff::filter_diagnostics(
                std::mem::take(&mut result.diagnostics),
                &result.compiler_evidence,
                &project_info.root_dir,
                &scope,
            );
            let retained = result.diagnostics.clone();
            result.compiler_evidence.retain(|evidence| {
                retained
                    .iter()
                    .any(|diagnostic| evidence.matches(diagnostic))
            });
            result.execution.reporting_scope = "changed".to_string();
            if let Some(baseline) = &mut result.execution.baseline {
                baseline.new_count = result.diagnostics.len();
            }
            recalculate_result(
                &mut result,
                resolved,
                &project_info.root_dir,
                &selected_categories,
            );
            result
        }
        Ok(scope) => match scope.reporting_scope {
            diff::ReportingScope::Staged => run_staged(
                cli,
                project_info,
                resolved,
                suppress_spinner,
                &scope,
                &control,
                &selected_categories,
            )?,
            diff::ReportingScope::Baseline => run_baseline(
                cli,
                project_info,
                resolved,
                suppress_spinner,
                &scope,
                &control,
                &selected_categories,
            )?,
            _ => scan::scan_project_scoped_for_categories(
                project_info,
                resolved,
                cli.offline,
                &cli.project,
                suppress_spinner,
                &scope,
                &control,
                &selected_categories,
            )?,
        },
        Err(error)
            if request.reporting_scope == diff::ReportingScope::Baseline
                && baseline_error_may_degrade(&error) =>
        {
            degraded_baseline_without_base(
                cli,
                project_info,
                resolved,
                suppress_spinner,
                &control,
                request.base.clone(),
                error.to_string(),
                &selected_categories,
            )?
        }
        Err(error) => return Err(error.into()),
    };
    Ok(result)
}

const fn baseline_error_may_degrade(error: &crate::error::DiffError) -> bool {
    matches!(
        error,
        crate::error::DiffError::MergeBaseFailed(_)
            | crate::error::DiffError::IndexConflict(_)
            | crate::error::DiffError::BaselineUnavailable(_)
    )
}

fn has_uncommitted_scope_work(
    project_info: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
    include_untracked: bool,
) -> Result<bool, crate::error::ScanError> {
    let request = diff::ScopeRequest {
        reporting_scope: diff::ReportingScope::Changed,
        base: Some("HEAD".to_string()),
        files: Vec::new(),
        include_untracked,
    };
    Ok(
        diff::resolve_scope(&project_info.root_dir, &request, &resolved.ignore_files)?
            .has_applicable_work(),
    )
}

fn diagnostic_categories(selected: &[ScanCategory]) -> Vec<Category> {
    selected
        .iter()
        .map(|category| match category {
            ScanCategory::ErrorHandling => Category::ErrorHandling,
            ScanCategory::Performance => Category::Performance,
            ScanCategory::Security => Category::Security,
            ScanCategory::Correctness => Category::Correctness,
            ScanCategory::Architecture => Category::Architecture,
            ScanCategory::Dependencies => Category::Dependencies,
            ScanCategory::Async => Category::Async,
            ScanCategory::Framework => Category::Framework,
            ScanCategory::Cargo => Category::Cargo,
            ScanCategory::Style => Category::Style,
        })
        .collect()
}

fn scope_request(
    cli: &Cli,
    resolved: &config::ResolvedConfig,
) -> Result<diff::ScopeRequest, crate::error::ScanError> {
    let legacy_diff = cli.diff.as_ref().or(resolved.diff.as_ref());
    if let Some(base) = legacy_diff {
        if cli.scope != Scope::Full || cli.staged || cli.baseline || cli.base.is_some() {
            return Err(crate::error::DiffError::InvalidScope(
                "--diff cannot be combined with --scope, --base, --staged, or --baseline"
                    .to_string(),
            )
            .into());
        }
        eprintln!("Warning: --diff is deprecated; use --scope changed --base <ref>");
        return Ok(diff::ScopeRequest {
            reporting_scope: diff::ReportingScope::Changed,
            base: (base != "auto").then(|| base.clone()),
            files: Vec::new(),
            include_untracked: cli.include_untracked,
        });
    }

    if (cli.staged || cli.baseline) && cli.scope != Scope::Full {
        return Err(crate::error::DiffError::InvalidScope(
            "--staged and --baseline cannot be combined with --scope".to_string(),
        )
        .into());
    }

    let reporting_scope = if cli.staged {
        diff::ReportingScope::Staged
    } else if cli.baseline {
        diff::ReportingScope::Baseline
    } else {
        match cli.scope {
            Scope::Full => diff::ReportingScope::Full,
            Scope::Files => diff::ReportingScope::Files,
            Scope::Changed => diff::ReportingScope::Changed,
            Scope::Lines => diff::ReportingScope::Lines,
        }
    };
    if cli.base.is_some()
        && !matches!(
            reporting_scope,
            diff::ReportingScope::Changed
                | diff::ReportingScope::Lines
                | diff::ReportingScope::Baseline
        )
    {
        return Err(crate::error::DiffError::InvalidScope(
            "--base requires changed, lines, or baseline scope".to_string(),
        )
        .into());
    }
    if cli.include_untracked
        && !matches!(
            reporting_scope,
            diff::ReportingScope::Changed | diff::ReportingScope::Lines
        )
    {
        return Err(crate::error::DiffError::InvalidScope(
            "--include-untracked requires changed or lines scope".to_string(),
        )
        .into());
    }
    if !cli.files.is_empty() && reporting_scope != diff::ReportingScope::Files {
        return Err(crate::error::DiffError::InvalidScope(
            "--files requires --scope files".to_string(),
        )
        .into());
    }
    Ok(diff::ScopeRequest {
        reporting_scope,
        base: cli.base.clone(),
        files: cli.files.clone(),
        include_untracked: cli.include_untracked,
    })
}

fn run_staged(
    cli: &Cli,
    original_project: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
    suppress_spinner: bool,
    scope: &diff::ScopePlan,
    control: &crate::process::ScanControl,
    selected_categories: &[Category],
) -> Result<ScanResult, crate::error::ScanError> {
    if !scope.has_applicable_work() {
        return scan::scan_project_scoped_for_categories(
            original_project,
            resolved,
            cli.offline,
            &cli.project,
            suppress_spinner,
            scope,
            control,
            selected_categories,
        );
    }
    let snapshot = diff::materialize_staged(&original_project.root_dir)?;
    diff::validate_staged_policy_snapshot(&original_project.root_dir, snapshot.root())?;
    let policy_fingerprint = diff::policy_fingerprint(snapshot.root()).map_err(|error| {
        crate::error::DiffError::StagedSnapshot(format!(
            "failed to fingerprint staged configuration: {error}"
        ))
    })?;
    let snapshot_project =
        discovery::discover_project(&snapshot.root().join("Cargo.toml"), cli.offline).map_err(
            |error| {
                crate::error::DiffError::StagedSnapshot(format!(
                    "Cargo metadata failed for the index snapshot: {error}"
                ))
            },
        )?;
    let mut result = scan::scan_project_scoped_for_categories(
        &snapshot_project,
        resolved,
        cli.offline,
        &cli.project,
        suppress_spinner,
        scope,
        control,
        selected_categories,
    )?;
    rebase_result_paths(
        &mut result,
        &snapshot_project.root_dir,
        &original_project.root_dir,
    );
    result.execution.execution_scope = "isolated_snapshot".to_string();
    result.execution.reporting_scope = "staged".to_string();
    let provenance_check = CheckState {
        name: "staged snapshot".to_string(),
        required: true,
        status: CheckStatus::Completed,
        reason: Some(format!("policy_fingerprint={policy_fingerprint}")),
    };
    result.execution.checks.push(provenance_check.clone());
    result
        .execution
        .checks
        .sort_by(|left, right| left.name.cmp(&right.name));
    for package in &mut result.execution.packages {
        package.checks.push(provenance_check.clone());
        package
            .checks
            .sort_by(|left, right| left.name.cmp(&right.name));
    }
    Ok(result)
}

#[expect(
    clippy::too_many_lines,
    reason = "baseline orchestration keeps snapshot failure, paired scans, accounting, and comparison in one auditable flow"
)]
fn run_baseline(
    cli: &Cli,
    project: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
    suppress_spinner: bool,
    scope: &diff::ScopePlan,
    control: &crate::process::ScanControl,
    selected_categories: &[Category],
) -> Result<ScanResult, crate::error::ScanError> {
    let base_commit = scope.base_commit.as_deref().ok_or_else(|| {
        crate::error::DiffError::BaselineUnavailable("merge-base was not resolved".to_string())
    })?;
    let snapshot = match diff::materialize_commit(&project.root_dir, base_commit) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return degraded_baseline(
                cli,
                project,
                resolved,
                suppress_spinner,
                control,
                scope,
                error.to_string(),
                selected_categories,
            );
        }
    };
    let base_project =
        match discovery::discover_project(&snapshot.root().join("Cargo.toml"), cli.offline) {
            Ok(project) => project,
            Err(error) => {
                return degraded_baseline(
                    cli,
                    project,
                    resolved,
                    suppress_spinner,
                    control,
                    scope,
                    format!("base Cargo metadata failed: {error}"),
                    selected_categories,
                );
            }
        };
    let base_file_config = if cli.no_project_config {
        None
    } else {
        match config::load_file_config(&base_project.root_dir, Some(&base_project.package_metadata))
        {
            Ok(file_config) => file_config,
            Err(error) => {
                return degraded_baseline(
                    cli,
                    project,
                    resolved,
                    suppress_spinner,
                    control,
                    scope,
                    format!("base configuration failed: {error}"),
                    selected_categories,
                );
            }
        }
    };
    let base_resolved = config::resolve_config(cli, base_file_config.as_ref());
    let head_config_fingerprint = diff::policy_fingerprint(&project.root_dir).map_err(|error| {
        crate::error::DiffError::Other(format!("failed to fingerprint head configuration: {error}"))
    })?;
    let base_config_fingerprint = match diff::policy_fingerprint(&base_project.root_dir) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            return degraded_baseline(
                cli,
                project,
                resolved,
                suppress_spinner,
                control,
                scope,
                format!("base configuration fingerprint failed: {error}"),
                selected_categories,
            );
        }
    };

    let full_scope = diff::ScopePlan::full();
    let mut head = scan::scan_project_scoped_for_categories(
        project,
        resolved,
        cli.offline,
        &cli.project,
        suppress_spinner,
        &full_scope,
        control,
        selected_categories,
    )?;
    if !required_analysis_complete(&head) {
        return Ok(degrade_scanned_baseline(
            head,
            scope,
            resolved,
            &project.root_dir,
            false,
            "head scan did not complete successfully".to_string(),
            selected_categories,
        ));
    }
    let base = scan::scan_project_scoped_for_categories(
        &base_project,
        &base_resolved,
        cli.offline,
        &cli.project,
        true,
        &full_scope,
        control,
        selected_categories,
    )?;
    merge_base_accounting(&mut head, &base, &project.root_dir, &base_project.root_dir);
    if !required_analysis_complete(&base)
        || base
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic.rule.as_str(), "compiler-error" | "compiler-ice"))
    {
        return Ok(degrade_scanned_baseline(
            head,
            scope,
            resolved,
            &project.root_dir,
            true,
            "base scan did not complete successfully".to_string(),
            selected_categories,
        ));
    }

    let comparison = diff::compare_baseline(
        &head.diagnostics,
        &project.root_dir,
        &base.diagnostics,
        &base_project.root_dir,
    );
    head.diagnostics = comparison.introduced;
    let introduced = head.diagnostics.clone();
    head.compiler_evidence.retain(|evidence| {
        introduced
            .iter()
            .any(|diagnostic| evidence.matches(diagnostic))
    });
    head.execution.reporting_scope = "baseline".to_string();
    head.execution.execution_scope = "full_packages+isolated_snapshot".to_string();
    head.execution.baseline = Some(BaselineReport {
        requested_base: scope
            .requested_base
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
        resolved_base: Some(base_commit.to_string()),
        base_commit: base_commit.to_string(),
        head_config_fingerprint,
        base_config_fingerprint: Some(base_config_fingerprint),
        new_count: head.diagnostics.len(),
        fixed_count: comparison.fixed_count,
        base_total: comparison.base_total,
        cross_file_match_count: comparison.cross_file_match_count,
        baseline_degraded: false,
        degraded_reason: None,
    });
    let baseline_check = CheckState {
        name: "baseline".to_string(),
        required: true,
        status: CheckStatus::Completed,
        reason: None,
    };
    for package in &mut head.execution.packages {
        package.checks.push(baseline_check.clone());
    }
    head.execution.checks.push(baseline_check);
    recalculate_result(&mut head, resolved, &project.root_dir, selected_categories);
    Ok(head)
}

fn merge_base_accounting(
    head: &mut ScanResult,
    base: &ScanResult,
    head_root: &Path,
    base_root: &Path,
) {
    head.elapsed += base.elapsed;
    head.pass_timings.extend(
        base.pass_timings
            .iter()
            .map(|(name, elapsed)| (format!("base:{name}"), *elapsed)),
    );
    head.skipped_passes.extend(
        base.skipped_passes
            .iter()
            .map(|reason| format!("base:{reason}")),
    );
    for head_package in &mut head.execution.packages {
        let relative = head_package
            .package_root
            .strip_prefix(head_root)
            .unwrap_or(&head_package.package_root);
        if let Some(base_package) = base.execution.packages.iter().find(|package| {
            package
                .package_root
                .strip_prefix(base_root)
                .unwrap_or(&package.package_root)
                == relative
        }) {
            head_package.elapsed += base_package.elapsed;
            head_package
                .checks
                .extend(base_package.checks.iter().cloned().map(|mut check| {
                    check.name = format!("base:{}", check.name);
                    check
                }));
        }
    }
    head.execution
        .checks
        .extend(base.execution.checks.iter().cloned().map(|mut check| {
            check.name = format!("base:{}", check.name);
            check
        }));
}

fn degrade_scanned_baseline(
    mut result: ScanResult,
    scope: &diff::ScopePlan,
    resolved: &config::ResolvedConfig,
    workspace_root: &Path,
    base_attempted: bool,
    reason: String,
    selected_categories: &[Category],
) -> ScanResult {
    let mut files_scope = scope.clone();
    files_scope.reporting_scope = diff::ReportingScope::Changed;
    result.diagnostics = diff::filter_diagnostics(
        std::mem::take(&mut result.diagnostics),
        &result.compiler_evidence,
        workspace_root,
        &files_scope,
    );
    let retained = result.diagnostics.clone();
    result.compiler_evidence.retain(|evidence| {
        retained
            .iter()
            .any(|diagnostic| evidence.matches(diagnostic))
    });
    result.execution.reporting_scope = "baseline".to_string();
    if base_attempted {
        result.execution.execution_scope = "full_packages+isolated_snapshot".to_string();
    }
    let baseline_check = CheckState {
        name: "baseline".to_string(),
        required: true,
        status: CheckStatus::Failed,
        reason: Some(reason.clone()),
    };
    for package in &mut result.execution.packages {
        package.checks.push(baseline_check.clone());
    }
    result.execution.checks.push(baseline_check);
    result.execution.baseline = Some(BaselineReport {
        requested_base: scope
            .requested_base
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
        resolved_base: scope.base_commit.clone(),
        base_commit: scope
            .base_commit
            .clone()
            .unwrap_or_else(|| "unresolved".to_string()),
        head_config_fingerprint: diff::policy_fingerprint(workspace_root)
            .unwrap_or_else(|_| "unavailable".to_string()),
        base_config_fingerprint: None,
        new_count: result.diagnostics.len(),
        fixed_count: 0,
        base_total: 0,
        cross_file_match_count: 0,
        baseline_degraded: true,
        degraded_reason: Some(reason),
    });
    recalculate_result(&mut result, resolved, workspace_root, selected_categories);
    result
}

#[expect(
    clippy::too_many_arguments,
    reason = "baseline degradation preserves the complete scan and category context at the fallback boundary"
)]
fn degraded_baseline_without_base(
    cli: &Cli,
    project: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
    suppress_spinner: bool,
    control: &crate::process::ScanControl,
    requested_base: Option<String>,
    reason: String,
    selected_categories: &[Category],
) -> Result<ScanResult, crate::error::ScanError> {
    let fallback_request = diff::ScopeRequest {
        reporting_scope: diff::ReportingScope::Changed,
        base: Some("HEAD".to_string()),
        files: Vec::new(),
        include_untracked: cli.include_untracked,
    };
    let mut fallback =
        diff::resolve_scope(&project.root_dir, &fallback_request, &resolved.ignore_files)?;
    fallback.requested_base = Some(requested_base.unwrap_or_else(|| "auto".to_string()));
    fallback.base_commit = None;
    degraded_baseline(
        cli,
        project,
        resolved,
        suppress_spinner,
        control,
        &fallback,
        reason,
        selected_categories,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "baseline degradation preserves the complete scan and category context at the fallback boundary"
)]
fn degraded_baseline(
    cli: &Cli,
    project: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
    suppress_spinner: bool,
    control: &crate::process::ScanControl,
    scope: &diff::ScopePlan,
    reason: String,
    selected_categories: &[Category],
) -> Result<ScanResult, crate::error::ScanError> {
    let mut files_scope = scope.clone();
    files_scope.reporting_scope = diff::ReportingScope::Changed;
    let mut result = scan::scan_project_scoped_for_categories(
        project,
        resolved,
        cli.offline,
        &cli.project,
        suppress_spinner,
        &files_scope,
        control,
        selected_categories,
    )?;
    result.execution.reporting_scope = "baseline".to_string();
    let baseline_check = CheckState {
        name: "baseline".to_string(),
        required: true,
        status: CheckStatus::Failed,
        reason: Some(reason.clone()),
    };
    for package in &mut result.execution.packages {
        package.checks.push(baseline_check.clone());
    }
    result.execution.checks.push(baseline_check);
    result.execution.baseline = Some(BaselineReport {
        requested_base: scope
            .requested_base
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
        resolved_base: scope.base_commit.clone(),
        base_commit: scope
            .base_commit
            .clone()
            .unwrap_or_else(|| "unresolved".to_string()),
        head_config_fingerprint: diff::policy_fingerprint(&project.root_dir)
            .unwrap_or_else(|_| "unavailable".to_string()),
        base_config_fingerprint: None,
        new_count: result.diagnostics.len(),
        fixed_count: 0,
        base_total: 0,
        cross_file_match_count: 0,
        baseline_degraded: true,
        degraded_reason: Some(reason),
    });
    Ok(result)
}

fn required_analysis_complete(result: &ScanResult) -> bool {
    crate::completeness::score_is_authoritative(result)
}

fn rebase_result_paths(result: &mut ScanResult, from: &Path, to: &Path) {
    fn rebase(path: &mut PathBuf, from: &Path, to: &Path) {
        if let Ok(relative) = path.strip_prefix(from) {
            *path = to.join(relative);
        }
    }
    for path in &mut result.planned_files {
        rebase(path, from, to);
    }
    for path in &mut result.analyzed_files {
        rebase(path, from, to);
    }
    for package in &mut result.execution.packages {
        rebase(&mut package.package_root, from, to);
        for path in &mut package.planned_files {
            rebase(path, from, to);
        }
        for path in &mut package.analyzed_files {
            rebase(path, from, to);
        }
    }
}

fn recalculate_result(
    result: &mut ScanResult,
    resolved: &config::ResolvedConfig,
    workspace_root: &Path,
    selected_categories: &[Category],
) {
    result.skipped_passes.sort();
    result.skipped_passes.dedup();
    result
        .execution
        .checks
        .sort_by(|left, right| left.name.cmp(&right.name));
    for package in &mut result.execution.packages {
        package
            .checks
            .sort_by(|left, right| left.name.cmp(&right.name));
    }
    result.error_count = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == crate::diagnostics::Severity::Error)
        .count();
    result.warning_count = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == crate::diagnostics::Severity::Warning)
        .count();
    result.info_count = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == crate::diagnostics::Severity::Info)
        .count();
    let score_diagnostics: Vec<_> = crate::catalog::built_in_catalog().map_or_else(
        |_| result.diagnostics.clone(),
        |catalog| {
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    let descriptor = catalog.resolve(
                        &diagnostic.rule,
                        &diagnostic.category,
                        diagnostic.severity,
                    );
                    resolved
                        .rule_policy(descriptor.as_descriptor(), Some(&diagnostic.file_path))
                        .visible_on(config::VisibilitySurface::Score)
                })
                .cloned()
                .collect()
        },
    );
    let (score, label, dimensions) =
        output::calculate_score_for_categories(&score_diagnostics, selected_categories);
    result.score = score;
    result.score_label = label;
    result.dimension_scores = dimensions;
    scan::assign_package_scores(
        &mut result.execution.packages,
        &score_diagnostics,
        workspace_root,
        selected_categories,
    );
}

/// Render the appropriate output format (score, JSON, SARIF, or terminal).
pub fn emit_output(
    cli: &Cli,
    scan_result: &ScanResult,
    resolved: &config::ResolvedConfig,
    project_info: &discovery::ProjectInfo,
) -> Result<(), crate::error::OutputError> {
    let mode = mode_from_reporting_scope(&scan_result.execution.reporting_scope);
    let report = ReportV1::from_scan_with_context(
        scan_result,
        project_info,
        resolved,
        mode,
        &cli.directory,
        evaluate_gate_result(cli, scan_result, resolved),
    );
    if cli.score {
        output::render_score(scan_result);
    } else if cli.wants_json() {
        output::render_json(&report, cli.json_compact, cli.json_out.as_deref())?;
    } else if cli.sarif {
        let sarif_json =
            sarif::render_report_sarif(&report).map_err(crate::error::OutputError::Serialize)?;
        println!("{sarif_json}");
    } else {
        output::render_terminal_for_categories(
            &report,
            &scan_result.pass_timings,
            resolved.verbose,
            cli.warnings != WarningVisibility::Hide,
            &cli.category,
        );
    }
    Ok(())
}

/// Emit a schema-valid failed Report V1 for expected bootstrap or scan errors.
pub fn emit_failure_report(
    cli: &Cli,
    kind: &str,
    error: &(dyn std::error::Error + 'static),
) -> Result<(), crate::error::OutputError> {
    if !cli.wants_json() {
        return Ok(());
    }
    let mode = if cli.baseline {
        ScanMode::Baseline
    } else if cli.staged {
        ScanMode::Staged
    } else if cli.diff.is_some() || cli.scope == Scope::Changed {
        ScanMode::Files
    } else if cli.scope == Scope::Lines {
        ScanMode::Lines
    } else if cli.scope == Scope::Files {
        ScanMode::Files
    } else {
        ScanMode::Full
    };
    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    let message = error.to_string();
    let report = ReportV1::failure_with_causes(&cli.directory, mode, kind, &message, &causes);
    output::render_json(&report, cli.json_compact, cli.json_out.as_deref())
}

fn mode_from_reporting_scope(reporting_scope: &str) -> ScanMode {
    match reporting_scope {
        "files" | "changed" => ScanMode::Files,
        "lines" => ScanMode::Lines,
        "staged" => ScanMode::Staged,
        "baseline" => ScanMode::Baseline,
        _ => ScanMode::Full,
    }
}

/// Fail closed when CI requires complete required analysis.
pub fn check_completeness_gate(
    scan_result: &ScanResult,
    require_complete: bool,
) -> Option<ExitCode> {
    if !require_complete
        || crate::completeness::compute(scan_result).state == CompletenessState::Complete
    {
        return None;
    }
    let incomplete: Vec<_> = scan_result
        .execution
        .checks
        .iter()
        .filter(|check| check.required && check.status != CheckStatus::Completed)
        .map(|check| check.name.as_str())
        .collect();
    eprintln!(
        "Required analysis is incomplete: {}",
        if incomplete.is_empty() {
            "baseline".to_string()
        } else {
            incomplete.join(", ")
        }
    );
    Some(ExitCode::from(EXIT_INCOMPLETE_ANALYSIS))
}

fn evaluate_gate_result(
    cli: &Cli,
    scan_result: &ScanResult,
    resolved: &config::ResolvedConfig,
) -> GateResult {
    let configured = cli.require_complete
        || resolved.score_fail_below.is_some()
        || resolved.fail_on != FailOn::None;
    if !configured {
        return GateResult::NotEvaluated;
    }
    let incomplete = cli.require_complete
        && crate::completeness::compute(scan_result).state != CompletenessState::Complete;
    let score_failed = resolved
        .score_fail_below
        .is_some_and(|threshold| scan_result.score < threshold);
    let findings_failed =
        check_fail_on_gate_for_config(scan_result, resolved, resolved.fail_on).is_some();
    if incomplete || score_failed || findings_failed {
        GateResult::Failed
    } else {
        GateResult::Passed
    }
}

/// Returns `Some(ExitCode)` with `EXIT_GATE_FAILURE` if the score is below the configured threshold.
pub fn check_score_gate(scan_result: &ScanResult, threshold: Option<u32>) -> Option<ExitCode> {
    if let Some(threshold) = threshold
        && scan_result.score < threshold
    {
        eprintln!(
            "Score {} is below the configured threshold of {}",
            scan_result.score, threshold
        );
        return Some(ExitCode::from(EXIT_GATE_FAILURE));
    }
    None
}

/// Check and install missing external tools. Returns appropriate exit code.
pub fn handle_install_deps() -> ExitCode {
    deps::print_status();
    let all_ok = deps::install_missing_tools();
    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Apply auto-fixes if `--fix` was requested.
pub fn apply_fixes_if_requested(cli: &Cli, scan_result: &ScanResult) {
    if cli.fix {
        let applied = fixer::apply_fixes(&scan_result.diagnostics, &cli.directory);
        if applied > 0 {
            eprintln!("Applied {applied} fix(es).");
        } else {
            eprintln!("No machine-applicable fixes available.");
        }
    }
}

/// Show remediation plan if `--plan` was requested.
pub fn emit_plan_if_requested(cli: &Cli, scan_result: &ScanResult) {
    if cli.plan {
        let items = plan::generate_plan(scan_result);
        let plan_text = plan::format_plan_markdown(&items, scan_result);
        eprintln!("\n{plan_text}");
    }
}

/// Returns `Some(ExitCode)` with `EXIT_GATE_FAILURE` if any diagnostic exceeds the `--fail-on` severity level.
pub fn check_fail_on_gate(scan_result: &ScanResult, fail_on: FailOn) -> Option<ExitCode> {
    let should_fail = match fail_on {
        FailOn::Error => scan_result.error_count > 0,
        FailOn::Warning => scan_result.error_count > 0 || scan_result.warning_count > 0,
        FailOn::Info => {
            scan_result.error_count > 0
                || scan_result.warning_count > 0
                || scan_result.info_count > 0
        }
        FailOn::None => false,
    };
    if should_fail {
        Some(ExitCode::from(EXIT_GATE_FAILURE))
    } else {
        None
    }
}

/// Apply the CI-failure visibility surface before evaluating `--fail-on`.
pub fn check_fail_on_gate_for_config(
    scan_result: &ScanResult,
    resolved: &config::ResolvedConfig,
    fail_on: FailOn,
) -> Option<ExitCode> {
    let Ok(catalog) = crate::catalog::built_in_catalog() else {
        return check_fail_on_gate(scan_result, fail_on);
    };
    let mut visible = [0_usize; 3];
    for diagnostic in &scan_result.diagnostics {
        let descriptor =
            catalog.resolve(&diagnostic.rule, &diagnostic.category, diagnostic.severity);
        if !resolved
            .rule_policy(descriptor.as_descriptor(), Some(&diagnostic.file_path))
            .visible_on(config::VisibilitySurface::CiFailure)
        {
            continue;
        }
        match diagnostic.severity {
            crate::diagnostics::Severity::Error => visible[0] += 1,
            crate::diagnostics::Severity::Warning => visible[1] += 1,
            crate::diagnostics::Severity::Info => visible[2] += 1,
        }
    }
    let should_fail = match fail_on {
        FailOn::Error => visible[0] > 0,
        FailOn::Warning => visible[0] + visible[1] > 0,
        FailOn::Info => visible.into_iter().sum::<usize>() > 0,
        FailOn::None => false,
    };
    should_fail.then(|| ExitCode::from(EXIT_GATE_FAILURE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{DimensionScores, ScoreLabel};
    use std::time::Duration;

    fn make_scan_result(score: u32, errors: usize, warnings: usize, infos: usize) -> ScanResult {
        ScanResult {
            diagnostics: vec![],
            score,
            score_label: ScoreLabel::Great,
            dimension_scores: DimensionScores {
                security: 100,
                reliability: 100,
                maintainability: 100,
                performance: 100,
                dependencies: 100,
            },
            source_file_count: 10,
            elapsed: Duration::from_secs(1),
            skipped_passes: vec![],
            error_count: errors,
            warning_count: warnings,
            info_count: infos,
            pass_timings: vec![],
            suppressed_security: vec![],
            planned_files: vec![],
            analyzed_files: vec![],
            compiler_evidence: vec![],
            execution: crate::diagnostics::ScanExecution::default(),
        }
    }

    // --- check_score_gate ---

    #[test]
    fn test_score_gate_below_threshold_fails() {
        let result = make_scan_result(75, 0, 0, 0);
        assert!(check_score_gate(&result, Some(80)).is_some());
    }

    #[test]
    fn test_score_gate_above_threshold_passes() {
        let result = make_scan_result(85, 0, 0, 0);
        assert!(check_score_gate(&result, Some(80)).is_none());
    }

    #[test]
    fn test_score_gate_exact_threshold_passes() {
        let result = make_scan_result(80, 0, 0, 0);
        assert!(check_score_gate(&result, Some(80)).is_none());
    }

    #[test]
    fn test_score_gate_no_threshold_passes() {
        let result = make_scan_result(10, 0, 0, 0);
        assert!(check_score_gate(&result, None).is_none());
    }

    // --- check_fail_on_gate ---

    #[test]
    fn test_fail_on_error_with_errors() {
        let result = make_scan_result(50, 1, 0, 0);
        assert!(check_fail_on_gate(&result, FailOn::Error).is_some());
    }

    #[test]
    fn test_fail_on_error_without_errors() {
        let result = make_scan_result(50, 0, 5, 3);
        assert!(check_fail_on_gate(&result, FailOn::Error).is_none());
    }

    #[test]
    fn test_fail_on_warning_with_warnings() {
        let result = make_scan_result(50, 0, 1, 0);
        assert!(check_fail_on_gate(&result, FailOn::Warning).is_some());
    }

    #[test]
    fn test_fail_on_warning_with_errors_too() {
        let result = make_scan_result(50, 1, 0, 0);
        assert!(check_fail_on_gate(&result, FailOn::Warning).is_some());
    }

    #[test]
    fn test_fail_on_info_with_info() {
        let result = make_scan_result(50, 0, 0, 1);
        assert!(check_fail_on_gate(&result, FailOn::Info).is_some());
    }

    #[test]
    fn test_fail_on_none_never_fails() {
        let result = make_scan_result(50, 10, 20, 30);
        assert!(check_fail_on_gate(&result, FailOn::None).is_none());
    }

    #[test]
    fn test_require_complete_fails_for_a_timed_out_required_check() {
        let mut result = make_scan_result(100, 0, 0, 0);
        result.execution.checks.push(CheckState {
            name: "clippy".to_string(),
            required: true,
            status: CheckStatus::TimedOut,
            reason: Some("deadline".to_string()),
        });
        assert!(check_completeness_gate(&result, true).is_some());
        assert!(check_completeness_gate(&result, false).is_none());
    }
}
