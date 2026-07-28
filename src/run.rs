//! Helper functions extracted from `main.rs` for the scan pipeline orchestration.
//!
//! These functions handle MCP dispatch, project bootstrapping, scanning,
//! output rendering, and quality gate checks.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::io::{IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::output::PromptTheme;
use dialoguer::{MultiSelect, Select};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};

use crate::cli::{
    CiInstallArgs, CiProvider, CiScope, Cli, ColorMode, FailOn, ScanCategory, Scope,
    WarningVisibility,
};

/// Unified React Doctor-compatible failure code.
pub const EXIT_SCAN_ERROR: u8 = 1;
/// Unified React Doctor-compatible quality-gate code.
pub const EXIT_GATE_FAILURE: u8 = 1;
/// Unified React Doctor-compatible incomplete-analysis code.
pub const EXIT_INCOMPLETE_ANALYSIS: u8 = 1;
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
        ColorMode::Always => {
            owo_colors::set_override(true);
            dialoguer::console::set_colors_enabled(true);
            dialoguer::console::set_colors_enabled_stderr(true);
        }
        ColorMode::Never => {
            owo_colors::set_override(false);
            dialoguer::console::set_colors_enabled(false);
            dialoguer::console::set_colors_enabled_stderr(false);
        }
    }
}

/// Render the normal scan header without contaminating machine-readable modes.
pub fn render_scan_welcome(cli: &Cli) -> std::io::Result<()> {
    if cli.score || cli.wants_json() || cli.sarif {
        return Ok(());
    }
    let animate = can_animate_report(cli) && !cli.verbose;
    let returning_user = !is_onboarding_forced() && crate::onboarding::has_completed();
    output::render_welcome(animate, returning_user)
}

/// Strict terminal policy for prompts and spinners.
pub fn is_interactive_terminal(cli: &Cli) -> bool {
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
        && !cli.score
        && !cli.wants_json()
        && !cli.sarif
        && std::env::var_os("TERM").is_none_or(|value| value != "dumb")
        && !is_non_interactive_environment()
}

/// Width of the live stdout terminal.
pub(crate) fn stdout_columns() -> Option<usize> {
    terminal_columns(&dialoguer::console::Term::stdout())
}

fn stderr_columns() -> Option<usize> {
    terminal_columns(&dialoguer::console::Term::stderr())
}

fn terminal_columns(terminal: &dialoguer::console::Term) -> Option<usize> {
    terminal
        .size_checked()
        .and_then(|(_, columns)| (columns > 0).then_some(usize::from(columns)))
}

fn environment_flag(name: &str) -> bool {
    std::env::var(name)
        .is_ok_and(|value| !matches!(value.to_ascii_lowercase().as_str(), "" | "0" | "false"))
}

fn is_onboarding_forced() -> bool {
    environment_flag("RUST_DOCTOR_FORCE_ONBOARDING")
}

fn is_ci_environment() -> bool {
    ["GITHUB_ACTIONS", "GITLAB_CI", "CIRCLECI"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
        || environment_flag("CI")
}

/// React-compatible animation gate. Coding-agent shells are allowed when a
/// human is watching a real stdout TTY; prompts remain disabled there.
pub(crate) fn can_animate_report(cli: &Cli) -> bool {
    let real_tty = std::io::stdout().is_terminal()
        && stdout_columns().is_some()
        && std::env::var_os("TERM").is_none_or(|value| value != "dumb")
        && !cli.score
        && !cli.wants_json()
        && !cli.sarif;
    real_tty
        && (is_onboarding_forced()
            || (!is_ci_environment()
                && std::env::var_os("GIT_DIR").is_none_or(|value| value.is_empty())))
}

fn interactive_prompts_allowed(cli: &Cli) -> bool {
    !cli.yes
        && !cli.score
        && !cli.wants_json()
        && !cli.sarif
        && std::io::stdin().is_terminal()
        && !is_non_interactive_environment()
}

fn post_scan_prompts_allowed(cli: &Cli) -> bool {
    interactive_prompts_allowed(cli) && std::io::stdout().is_terminal()
}

fn is_non_interactive_environment() -> bool {
    const MARKERS: [&str; 27] = [
        "CI",
        "GITHUB_ACTIONS",
        "GITLAB_CI",
        "BUILDKITE",
        "JENKINS_URL",
        "TF_BUILD",
        "CODEBUILD_BUILD_ID",
        "TEAMCITY_VERSION",
        "BITBUCKET_BUILD_NUMBER",
        "CIRCLECI",
        "TRAVIS",
        "DRONE",
        "GIT_DIR",
        "CLAUDECODE",
        "CLAUDE_CODE",
        "CURSOR_AGENT",
        "CODEX_CI",
        "CODEX_SANDBOX",
        "CODEX_SANDBOX_NETWORK_DISABLED",
        "OPENCODE",
        "GOOSE_TERMINAL",
        "AMP_THREAD_ID",
        "CLINE_ACTIVE",
        "AUGMENT_AGENT",
        "TRAE_AI_SHELL_ID",
        "AGENT_SESSION_ID",
        "AGENT_THREAD_ID",
    ];
    MARKERS
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
        || std::env::var("AGENT")
            .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "amp" | "goose"))
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
    crate::telemetry::install_panic_hook(
        cli.no_telemetry,
        cli_network_disabled(cli),
        surface,
        &cli.directory,
    );
}

/// Let terminal animations observe the process-wide cancellation flag.
pub fn register_animation_cancellation(cancellation: &Arc<AtomicBool>) {
    output::register_animation_cancellation(cancellation);
}

/// Record one aggregate server session when explicit consent is active.
pub fn emit_server_telemetry(cli: &Cli) {
    let surface = if cli.mcp {
        crate::telemetry::mcp_surface()
    } else {
        crate::telemetry::lsp_surface()
    };
    crate::telemetry::record_session(cli.no_telemetry, cli_network_disabled(cli), surface);
}

/// Record aggregate scan metadata without affecting output or exit status.
pub fn emit_scan_telemetry(cli: &Cli, report: &ReportV1) {
    crate::telemetry::record_scan(cli.no_telemetry, cli_network_disabled(cli), report);
}

/// Print the explicit stateless share URL after normal terminal output.
pub fn emit_share_if_requested(cli: &Cli, report: &ReportV1) -> Result<(), String> {
    if !cli.share {
        return Ok(());
    }
    let report = share_report_fixture(report)?;
    let url = crate::share::build_url(report.as_ref()).map_err(|error| error.to_string())?;
    println!("\nShare: {url}");
    Ok(())
}

fn share_report_fixture(report: &ReportV1) -> Result<Cow<'_, ReportV1>, String> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("RUST_DOCTOR_INTERNAL_SHARE_REPORT_FIXTURE") {
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("could not read internal share fixture: {error}"))?;
        let fixture = serde_json::from_slice(&bytes)
            .map_err(|error| format!("could not parse internal share fixture: {error}"))?;
        return Ok(Cow::Owned(fixture));
    }
    Ok(Cow::Borrowed(report))
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
    discovery::bootstrap_project_for_scan(
        &cli.directory,
        cli_network_disabled(cli),
        cli.evaluation_profile,
    )
}

/// Resolve the interactive project and scan-scope choices before scanning.
///
/// Returns `false` when the user cancels either prompt.
pub fn prepare_interactive_scan(
    cli: &mut Cli,
    project_info: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
    scope_was_explicit: bool,
) -> Result<bool, dialoguer::Error> {
    let prompts_allowed = interactive_prompts_allowed(cli);
    if cli.project.is_empty() && project_info.workspace_members.len() == 1 {
        print_project_selection(cli, project_info);
    }
    if cli.project.is_empty() && project_info.workspace_members.len() > 1 && !prompts_allowed {
        cli.project.push("*".to_string());
        print_project_selection(cli, project_info);
    }
    if !prompts_allowed {
        return Ok(true);
    }
    if cli.project.is_empty() && project_info.workspace_members.len() > 1 {
        let terminal = dialoguer::console::Term::stdout();
        let ordered_members = workspace_members_in_display_order(
            &project_info.root_dir,
            &project_info.workspace_members,
        );
        let labels: Vec<_> = ordered_members
            .iter()
            .map(|member| {
                let relative = member
                    .root_dir
                    .strip_prefix(&project_info.root_dir)
                    .unwrap_or(&member.root_dir);
                format!("{}\n  {}", member.name, relative.display())
            })
            .collect();
        loop {
            let Some(selected) = MultiSelect::with_theme(&PromptTheme)
                .with_prompt("Select projects")
                .items(&labels)
                .interact_on_opt(&terminal)?
            else {
                return Ok(false);
            };
            if selected.is_empty() {
                eprintln!("Select at least one project.");
                continue;
            }
            cli.project = selected
                .into_iter()
                .filter_map(|index| ordered_members.get(index).map(|member| member.name.clone()))
                .collect();
            break;
        }
    }

    if scope_was_explicit || resolved.diff.is_some() {
        return Ok(true);
    }
    let request = diff::ScopeRequest {
        reporting_scope: diff::ReportingScope::Changed,
        base: None,
        files: Vec::new(),
        include_untracked: false,
    };
    let Ok(candidate) =
        diff::resolve_scope(&project_info.root_dir, &request, &resolved.ignore_files)
    else {
        return Ok(true);
    };
    let context = diff::scope_prompt_context(&project_info.root_dir, &candidate);
    if context.changed_rust_source_count == 0 {
        return Ok(true);
    }
    let changed_title = if context.is_current_changes {
        format!(
            "Uncommitted changes ({})",
            context.changed_rust_source_count
        )
    } else {
        format!(
            "Changed files on {} ({})",
            context.current_branch.as_deref().unwrap_or("this branch"),
            context.changed_rust_source_count
        )
    };
    let changed_description = if context.is_current_changes {
        "Compare working tree changes against HEAD".to_string()
    } else {
        format!(
            "Compare against {} from the branch merge-base",
            context.base_branch.as_deref().unwrap_or("the base branch")
        )
    };
    let choices = [
        "Full codebase\n  Scan every Rust source file".to_string(),
        format!("{changed_title}\n  {changed_description}"),
    ];
    let Some(selection) = Select::with_theme(&PromptTheme)
        .with_prompt("Choose what to scan")
        .items(&choices)
        .default(usize::from(!context.is_current_changes))
        .interact_on_opt(&dialoguer::console::Term::stdout())?
    else {
        return Ok(false);
    };
    if selection == 1 {
        cli.scope = Scope::Changed;
    }
    Ok(true)
}

fn print_project_selection(cli: &Cli, project_info: &discovery::ProjectInfo) {
    if cli.score || cli.wants_json() || cli.sarif {
        return;
    }
    let names =
        workspace_members_in_display_order(&project_info.root_dir, &project_info.workspace_members)
            .into_iter()
            .map(|member| member.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
    if !names.is_empty() {
        println!("✔ Select projects › {names}");
    }
}

fn workspace_members_in_display_order<'a>(
    workspace_root: &Path,
    members: &'a [discovery::WorkspaceMember],
) -> Vec<&'a discovery::WorkspaceMember> {
    let mut ordered: Vec<_> = members.iter().collect();
    ordered.sort_by(|left, right| {
        left.root_dir
            .strip_prefix(workspace_root)
            .unwrap_or(&left.root_dir)
            .cmp(
                right
                    .root_dir
                    .strip_prefix(workspace_root)
                    .unwrap_or(&right.root_dir),
            )
    });
    ordered
}

/// Burn the first-run marker only after an interactive guided render completed.
pub fn record_onboarding_completion(
    cli: &Cli,
    resolved: &config::ResolvedConfig,
    report: &ReportV1,
) {
    if can_animate_report(cli)
        && !resolved.verbose
        && !is_onboarding_forced()
        && !is_non_interactive_environment()
        && report.source_file_count > 0
        && report.error.is_none()
    {
        crate::onboarding::mark_completed();
    }
}

/// Run the scan passes and return the result.
pub fn run_scan(
    cli: &Cli,
    project_info: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
) -> Result<ScanResult, crate::error::ScanError> {
    run_scan_with_cancellation(
        cli,
        project_info,
        resolved,
        &Arc::new(AtomicBool::new(false)),
    )
}

/// Run the canonical scoped scan with a caller-owned cancellation flag.
pub fn run_scan_with_cancellation(
    cli: &Cli,
    project_info: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
    cancel: &Arc<AtomicBool>,
) -> Result<ScanResult, crate::error::ScanError> {
    run_scan_cancellable(cli, project_info, resolved, cancel)
}

/// Internal implementation shared by CLI and cancellation tests.
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
    let suppress_spinner =
        cli.score || cli.wants_json() || cli.sarif || !is_interactive_terminal(cli);
    let project_count = selected_project_count(cli, project_info);
    let multi_project = project_count > 1;
    let suppress_pass_spinner = suppress_spinner || multi_project;
    let mut batch_spinner = BatchSpinner::new(
        project_count,
        suppress_spinner,
        !cli.no_color && cli.color != ColorMode::Never,
    );
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
                suppress_pass_spinner,
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
            )?;
            result
        }
        Ok(scope) => match scope.reporting_scope {
            diff::ReportingScope::Staged => run_staged(
                cli,
                project_info,
                resolved,
                suppress_pass_spinner,
                &scope,
                &control,
                &selected_categories,
            )?,
            diff::ReportingScope::Baseline => run_baseline(
                cli,
                project_info,
                resolved,
                suppress_pass_spinner,
                &scope,
                &control,
                &selected_categories,
            )?,
            _ => scan::scan_project_scoped_for_categories(
                project_info,
                resolved,
                effective_offline(cli, resolved),
                &cli.project,
                suppress_pass_spinner,
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
                suppress_pass_spinner,
                &control,
                request.base.clone(),
                error.to_string(),
                &selected_categories,
            )?
        }
        Err(error)
            if matches!(
                request.reporting_scope,
                diff::ReportingScope::Changed | diff::ReportingScope::Lines
            ) =>
        {
            eprintln!(
                "Warning: changed-file scope is unavailable ({error}); scanning the full codebase."
            );
            scan::scan_project_scoped_for_categories(
                project_info,
                resolved,
                effective_offline(cli, resolved),
                &cli.project,
                suppress_pass_spinner,
                &diff::ScopePlan::full(),
                &control,
                &selected_categories,
            )?
        }
        Err(error) => return Err(error.into()),
    };
    batch_spinner.complete();
    Ok(result)
}

fn selected_project_count(cli: &Cli, project_info: &discovery::ProjectInfo) -> usize {
    if project_info.workspace_members.len() <= 1 {
        return 1;
    }
    if cli
        .project
        .iter()
        .any(|selector| matches!(selector.as_str(), "*" | "all"))
    {
        project_info.workspace_members.len()
    } else {
        cli.project.len().max(1)
    }
}

struct BatchSpinner {
    progress: Option<ProgressBar>,
    project_count: usize,
}

impl BatchSpinner {
    fn new(project_count: usize, suppress: bool, color: bool) -> Self {
        let progress = (!suppress && project_count > 1).then(|| {
            let progress = ProgressBar::new_spinner();
            let template = if color {
                "{spinner:.cyan} {msg}"
            } else {
                "{spinner} {msg}"
            };
            progress.set_style(
                ProgressStyle::default_spinner()
                    .template(template)
                    .unwrap_or_else(|_| ProgressStyle::default_spinner()),
            );
            progress.set_message(format!("Scanning {project_count} projects…"));
            progress.enable_steady_tick(Duration::from_millis(100));
            progress
        });
        Self {
            progress,
            project_count,
        }
    }

    fn complete(&mut self) {
        if let Some(progress) = self.progress.take() {
            progress.set_message(format!(
                "Scanning {} projects… ({}/{})",
                self.project_count, self.project_count, self.project_count
            ));
            progress.finish_and_clear();
        }
    }
}

impl Drop for BatchSpinner {
    fn drop(&mut self) {
        if let Some(progress) = self.progress.take() {
            progress.finish_and_clear();
        }
    }
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

    if cli.staged && matches!(cli.scope, Scope::Full | Scope::Changed) {
        return Err(crate::error::DiffError::InvalidScope(
            "--staged supports only --scope files or --scope lines".to_string(),
        )
        .into());
    }
    if cli.baseline && cli.scope != Scope::Full {
        return Err(crate::error::DiffError::InvalidScope(
            "--baseline cannot be combined with --scope".to_string(),
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
    if cli.staged && cli.include_untracked {
        return Err(crate::error::DiffError::InvalidScope(
            "--include-untracked cannot be combined with --staged".to_string(),
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
    if cli.staged && !cli.files.is_empty() {
        return Err(crate::error::DiffError::InvalidScope(
            "--files cannot be combined with --staged".to_string(),
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
            effective_offline(cli, resolved),
            &cli.project,
            suppress_spinner,
            scope,
            control,
            selected_categories,
        );
    }
    let mut staged_scope = scope.clone();
    if cli.scope == Scope::Lines {
        match diff::resolve_staged_line_ranges(&original_project.root_dir, scope) {
            Ok(line_ranges) => {
                staged_scope.reporting_scope = diff::ReportingScope::Lines;
                staged_scope.line_ranges = line_ranges;
            }
            Err(error) => {
                staged_scope.degradation_reason = Some(format!(
                    "staged line ranges were unavailable; degraded to files scope: {error}"
                ));
            }
        }
    }
    let snapshot = diff::materialize_staged(&original_project.root_dir)?;
    diff::validate_staged_policy_snapshot(&original_project.root_dir, snapshot.root())?;
    let policy_fingerprint = diff::policy_fingerprint(snapshot.root()).map_err(|error| {
        crate::error::DiffError::StagedSnapshot(format!(
            "failed to fingerprint staged configuration: {error}"
        ))
    })?;
    let snapshot_project = discovery::discover_project_for_scan(
        &snapshot.root().join("Cargo.toml"),
        effective_offline(cli, resolved),
        cli.evaluation_profile,
    )
    .map_err(|error| {
        crate::error::DiffError::StagedSnapshot(format!(
            "Cargo metadata failed for the index snapshot: {error}"
        ))
    })?;
    let mut result = scan::scan_project_scoped_for_categories(
        &snapshot_project,
        resolved,
        effective_offline(cli, resolved),
        &cli.project,
        suppress_spinner,
        &staged_scope,
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
        .analyzer_receipts
        .push(crate::completeness::AnalyzerReceipt::global(
            &provenance_check,
        ));
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
    let base_project = match discovery::discover_project_for_scan(
        &snapshot.root().join("Cargo.toml"),
        effective_offline(cli, resolved),
        cli.evaluation_profile,
    ) {
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
        effective_offline(cli, resolved),
        &cli.project,
        suppress_spinner,
        &full_scope,
        control,
        selected_categories,
    )?;
    if !required_analysis_complete(&head) {
        return degrade_scanned_baseline(
            head,
            scope,
            resolved,
            &project.root_dir,
            false,
            "head scan did not complete successfully".to_string(),
            selected_categories,
        );
    }
    let base = scan::scan_project_scoped_for_categories(
        &base_project,
        &base_resolved,
        effective_offline(cli, &base_resolved),
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
        return degrade_scanned_baseline(
            head,
            scope,
            resolved,
            &project.root_dir,
            true,
            "base scan did not complete successfully".to_string(),
            selected_categories,
        );
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
    head.execution.checks.push(baseline_check.clone());
    head.execution
        .analyzer_receipts
        .push(crate::completeness::AnalyzerReceipt::global(
            &baseline_check,
        ));
    recalculate_result(&mut head, resolved, &project.root_dir, selected_categories)?;
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
    head.execution.analyzer_receipts.extend(
        crate::completeness::effective_receipts(base)
            .into_iter()
            .map(crate::completeness::AnalyzerReceipt::for_baseline),
    );
}

fn degrade_scanned_baseline(
    mut result: ScanResult,
    scope: &diff::ScopePlan,
    resolved: &config::ResolvedConfig,
    workspace_root: &Path,
    base_attempted: bool,
    reason: String,
    selected_categories: &[Category],
) -> Result<ScanResult, crate::error::ScanError> {
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
    result.execution.reporting_scope = "changed".to_string();
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
    result.execution.checks.push(baseline_check.clone());
    result
        .execution
        .analyzer_receipts
        .push(crate::completeness::AnalyzerReceipt::global(
            &baseline_check,
        ));
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
    recalculate_result(&mut result, resolved, workspace_root, selected_categories)?;
    Ok(result)
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
        effective_offline(cli, resolved),
        &cli.project,
        suppress_spinner,
        &files_scope,
        control,
        selected_categories,
    )?;
    result.execution.reporting_scope = "changed".to_string();
    let baseline_check = CheckState {
        name: "baseline".to_string(),
        required: true,
        status: CheckStatus::Failed,
        reason: Some(reason.clone()),
    };
    for package in &mut result.execution.packages {
        package.checks.push(baseline_check.clone());
    }
    result.execution.checks.push(baseline_check.clone());
    result
        .execution
        .analyzer_receipts
        .push(crate::completeness::AnalyzerReceipt::global(
            &baseline_check,
        ));
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
    crate::completeness::compute(result).score_authoritative
}

fn cli_network_disabled(cli: &Cli) -> bool {
    cli.offline
        || cli
            .disable_adapter
            .contains(&crate::cli::AdapterGroup::NetworkDependent)
}

fn effective_offline(cli: &Cli, resolved: &config::ResolvedConfig) -> bool {
    cli_network_disabled(cli) || !resolved.adapter_policy.network
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
) -> Result<(), crate::error::ScanError> {
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
    let (score, label, dimensions) = scan::recalculate_package_scores_and_select_headline(
        &mut result.execution.packages,
        &score_diagnostics,
        workspace_root,
        selected_categories,
    )?;
    result.score = score;
    result.score_label = label;
    result.dimension_scores = dimensions;
    Ok(())
}

/// Render the appropriate output format (score, JSON, SARIF, or terminal).
pub fn emit_output(
    cli: &Cli,
    scan_result: &ScanResult,
    resolved: &config::ResolvedConfig,
    project_info: &discovery::ProjectInfo,
) -> Result<ReportV1, crate::error::OutputError> {
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
        let interactive = is_interactive_terminal(cli);
        let animate_report = can_animate_report(cli);
        let animate_stderr_details = animate_report
            && std::io::stderr().is_terminal()
            && stderr_columns().is_some()
            && !resolved.verbose;
        output::render_terminal_for_categories(
            &report,
            &scan_result.pass_timings,
            resolved.verbose,
            terminal_warnings_are_visible(cli, resolved),
            &cli.category,
            !cli.share && !is_ci_environment(),
            !interactive || scan_result.execution.packages.len() > 1,
            is_non_interactive_environment(),
            animate_report,
            animate_stderr_details,
        );
    }
    Ok(report)
}

/// Write the bounded diagnostic dump and optional agent handoff.
///
/// Failure is returned as a secondary warning so callers can preserve the
/// already-computed report and quality-gate result.
pub fn emit_handoff(
    cli: &Cli,
    report: &ReportV1,
    resolved: &config::ResolvedConfig,
) -> Result<(), String> {
    let interactive = post_scan_prompts_allowed(cli);
    if cli.output_dir.is_none()
        && cli.handoff.is_none()
        && !cli.reset_handoff_target
        && !interactive
    {
        return Ok(());
    }
    let visible_report = if cli.output_dir.is_some() || cli.handoff.is_some() {
        Cow::Borrowed(report)
    } else {
        let mut filtered = report.clone();
        filtered
            .diagnostics
            .retain(|diagnostic| terminal_diagnostic_is_visible(cli, resolved, diagnostic));
        if filtered.diagnostics.is_empty() {
            return Ok(());
        }
        Cow::Owned(filtered)
    };
    let outcome = crate::handoff::execute(
        visible_report.as_ref(),
        &crate::handoff::HandoffRequest {
            output_dir: cli.output_dir.clone(),
            target: cli.handoff,
            remember_target: cli.remember_handoff_target,
            reset_target: cli.reset_handoff_target,
            interactive,
        },
    )
    .map_err(|error| error.to_string())?;
    if let Some(outcome) = outcome {
        eprintln!("Diagnostic dump: {}", outcome.directory.display());
        if let Some(target) = outcome.target {
            eprintln!("Handoff target: {target}");
        }
    }
    Ok(())
}

/// Emit the once-per-project setup hint in a supported coding-agent run.
pub fn emit_agent_install_hint(
    cli: &Cli,
    report: &ReportV1,
    project_info: &discovery::ProjectInfo,
) {
    crate::agent_hint::emit_if_eligible(cli, report, project_info);
}

/// Offer the same one-time post-scan GitHub Actions setup used by React Doctor.
///
/// The decision store contains only a SHA-256 project key and the answer. It
/// never persists the repository name or path.
pub fn offer_ci_setup(
    cli: &Cli,
    report: &ReportV1,
    project_info: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
) -> Result<bool, crate::error::CiSetupPromptError> {
    if cli.score
        || !post_scan_prompts_allowed(cli)
        || !report
            .diagnostics
            .iter()
            .any(|diagnostic| terminal_diagnostic_is_visible(cli, resolved, diagnostic))
    {
        return Ok(true);
    }
    let Some(root) = nearest_git_root(&project_info.root_dir) else {
        return Ok(true);
    };
    if root.join(".github/workflows/rust-doctor.yml").is_file() || ci_prompt_was_handled(&root)? {
        return Ok(true);
    }

    println!();
    let terminal = dialoguer::console::Term::stdout();
    loop {
        let choices = ["Yes - Adds the workflow file", "Learn more", "No"];
        let selection = Select::with_theme(&PromptTheme)
            .with_prompt(
                "Add Rust Doctor to GitHub Actions?\n  Scan every pull request to prevent new Rust issues while you fix the backlog.\n  Uses the managed, least-privilege Rust Doctor workflow.",
            )
            .items(&choices)
            .default(0)
            .interact_on_opt(&terminal)
            ?;
        match selection {
            None => return Ok(false),
            Some(0) => {
                record_ci_prompt_decision(&root, "accepted")?;
                let message = crate::workflows::ci::install(&CiInstallArgs {
                    directory: root,
                    provider: CiProvider::Github,
                    scope: CiScope::Baseline,
                    blocking: FailOn::None,
                    comment: true,
                    review_comments: false,
                    commit_status: true,
                    sarif: true,
                    version: "v1".to_string(),
                    dry_run: false,
                    pr: false,
                    issue: None,
                })
                .map_err(|error| crate::error::CiSetupPromptError::Install(error.to_string()))?;
                eprintln!("{message}");
                return Ok(true);
            }
            Some(1) => {
                eprintln!("Visit https://rust-doctor.vercel.app to learn more.\n");
            }
            Some(_) => {
                record_ci_prompt_decision(&root, "declined")?;
                return Ok(true);
            }
        }
    }
}

fn nearest_git_root(path: &Path) -> Option<PathBuf> {
    let canonical = path.canonicalize().ok()?;
    canonical
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

fn ci_prompt_was_handled(root: &Path) -> Result<bool, crate::error::CiSetupPromptError> {
    let path = ci_prompt_decision_path(root)?;
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(crate::error::CiSetupPromptError::Io {
            path,
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "path is not a regular file",
            ),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(crate::error::CiSetupPromptError::Io { path, source }),
    }
}

fn record_ci_prompt_decision(
    root: &Path,
    decision: &str,
) -> Result<(), crate::error::CiSetupPromptError> {
    let path = ci_prompt_decision_path(root)?;
    let parent = path
        .parent()
        .ok_or_else(|| crate::error::CiSetupPromptError::Io {
            path: path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "CI prompt state path has no parent",
            ),
        })?;
    std::fs::create_dir_all(parent).map_err(|source| crate::error::CiSetupPromptError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|source| {
        crate::error::CiSetupPromptError::Io {
            path: path.clone(),
            source,
        }
    })?;
    writeln!(temporary, "{decision}")
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|source| crate::error::CiSetupPromptError::Io {
            path: path.clone(),
            source,
        })?;
    temporary
        .persist(&path)
        .map_err(|error| crate::error::CiSetupPromptError::Io {
            path,
            source: error.error,
        })?;
    Ok(())
}

fn ci_prompt_decision_path(root: &Path) -> Result<PathBuf, crate::error::CiSetupPromptError> {
    let config_root = crate::telemetry::config_root()
        .map_err(|_| crate::error::CiSetupPromptError::StateDirectoryUnavailable)?;
    Ok(config_root.join("ci-prompts").join(ci_prompt_key(root)))
}

fn ci_prompt_key(root: &Path) -> String {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    let mut key = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        let _ = write!(key, "{byte:02x}");
    }
    key
}

/// Emit a schema-valid failed Report V1 for expected bootstrap or scan errors.
pub fn emit_failure_report(
    cli: &Cli,
    kind: &str,
    error: &(dyn std::error::Error + 'static),
) -> Result<(), crate::error::OutputError> {
    emit_failure_report_for_mode(cli, scan_mode_for_request(cli, false), kind, error)
}

/// Return the report mode selected by CLI flags and resolved project config.
#[must_use]
pub fn requested_scan_mode(cli: &Cli, resolved: &config::ResolvedConfig) -> ScanMode {
    scan_mode_for_request(cli, resolved.diff.is_some())
}

/// Return the effective mode after dynamic scope promotion or degradation.
#[must_use]
pub fn completed_scan_mode(scan_result: &ScanResult) -> ScanMode {
    mode_from_reporting_scope(&scan_result.execution.reporting_scope)
}

fn scan_mode_for_request(cli: &Cli, config_diff: bool) -> ScanMode {
    if cli.baseline {
        ScanMode::Baseline
    } else if cli.staged {
        ScanMode::Staged
    } else if cli.diff.is_some() || config_diff || cli.scope == Scope::Changed {
        ScanMode::Files
    } else if cli.scope == Scope::Lines {
        ScanMode::Lines
    } else if cli.scope == Scope::Files {
        ScanMode::Files
    } else {
        ScanMode::Full
    }
}

/// Emit a schema-valid failed report with an already-resolved scan mode.
pub fn emit_failure_report_for_mode(
    cli: &Cli,
    mode: ScanMode,
    kind: &str,
    error: &(dyn std::error::Error + 'static),
) -> Result<(), crate::error::OutputError> {
    if !cli.wants_json() && !cli.sarif {
        return Ok(());
    }
    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    let message = error.to_string();
    let report = ReportV1::failure_with_causes(&cli.directory, mode, kind, &message, &causes);
    if cli.sarif {
        let rendered =
            sarif::render_report_sarif(&report).map_err(crate::error::OutputError::Serialize)?;
        println!("{rendered}");
        Ok(())
    } else {
        output::render_json(&report, cli.json_compact, cli.json_out.as_deref())
    }
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

fn terminal_warnings_are_visible(cli: &Cli, resolved: &config::ResolvedConfig) -> bool {
    cli.warnings != WarningVisibility::Hide
        || matches!(resolved.fail_on, FailOn::Warning | FailOn::Info)
}

fn terminal_diagnostic_is_visible(
    cli: &Cli,
    resolved: &config::ResolvedConfig,
    diagnostic: &crate::diagnostics::CanonicalDiagnostic,
) -> bool {
    diagnostic
        .visible_on
        .iter()
        .any(|surface| surface == "terminal")
        && (terminal_warnings_are_visible(cli, resolved)
            || diagnostic.severity != crate::diagnostics::Severity::Warning)
        && {
            let selected = diagnostic_categories(&cli.category);
            selected.is_empty() || selected.contains(&diagnostic.category)
        }
}

/// Fail when the primary lint engine produced no trustworthy result.
///
/// This applies in score mode too, matching React Doctor's hard lint-failure
/// gate. `--blocking none` remains the explicit fail-open override.
pub fn check_hard_analysis_failure(scan_result: &ScanResult, blocking: FailOn) -> Option<ExitCode> {
    has_hard_analysis_failure(scan_result, blocking).then(|| ExitCode::from(EXIT_SCAN_ERROR))
}

fn has_hard_analysis_failure(scan_result: &ScanResult, blocking: FailOn) -> bool {
    blocking != FailOn::None
        && crate::completeness::effective_receipts(scan_result)
            .iter()
            .any(|receipt| {
                receipt.required
                    && receipt.status == CheckStatus::Failed
                    && receipt.scope != crate::completeness::AnalyzerScope::Baseline
                    && receipt.analyzer == crate::completeness::AnalyzerIdentity::Clippy
            })
}

fn evaluate_gate_result(
    cli: &Cli,
    scan_result: &ScanResult,
    resolved: &config::ResolvedConfig,
) -> GateResult {
    let hard_failed = has_hard_analysis_failure(scan_result, resolved.fail_on);
    let configured = cli.require_complete
        || resolved.score_fail_below.is_some()
        || (!cli.score && resolved.fail_on != FailOn::None)
        || hard_failed;
    if !configured {
        return GateResult::NotEvaluated;
    }
    let incomplete = cli.require_complete
        && crate::completeness::compute(scan_result).state != CompletenessState::Complete;
    let score_decision = crate::completeness::score_decision(scan_result);
    let authority_failed = score_decision.published_score().is_none();
    let score_failed = resolved.score_fail_below.is_some_and(|threshold| {
        score_decision
            .published_score()
            .is_none_or(|score| score < threshold)
    });
    let findings_failed = !cli.score
        && check_fail_on_gate_for_config(scan_result, resolved, resolved.fail_on).is_some();
    if authority_failed || hard_failed || incomplete || score_failed || findings_failed {
        GateResult::Failed
    } else {
        GateResult::Passed
    }
}

/// Fail a bare score request when the canonical score is unavailable.
pub fn check_score_authority(scan_result: &ScanResult, required: bool) -> Option<ExitCode> {
    if !required {
        return None;
    }
    let decision = crate::completeness::score_decision(scan_result);
    if decision.published_score().is_some() {
        return None;
    }
    eprintln!(
        "Authoritative score unavailable: {}",
        decision.primary_reason()
    );
    Some(ExitCode::from(EXIT_SCAN_ERROR))
}

/// Fail a configured threshold when the canonical score is unavailable or low.
pub fn check_score_gate(scan_result: &ScanResult, threshold: Option<u32>) -> Option<ExitCode> {
    let threshold = threshold?;
    let decision = crate::completeness::score_decision(scan_result);
    let Some(score) = decision.published_score() else {
        eprintln!(
            "Score gate failed: authoritative score unavailable ({})",
            decision.primary_reason()
        );
        return Some(ExitCode::from(EXIT_GATE_FAILURE));
    };
    if score < threshold {
        eprintln!("Score {score} is below the configured threshold of {threshold}");
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
///
/// Only edits Report V1 marked machine-applicable are applied, grouped by root
/// cause and validated group by group. Everything else is reported as guidance
/// with its reason, never silently attempted (US-016).
pub fn apply_fixes_if_requested(
    cli: &Cli,
    scan_result: &ScanResult,
    resolved: &config::ResolvedConfig,
    project_info: &discovery::ProjectInfo,
) {
    if !cli.fix {
        return;
    }
    let mode = mode_from_reporting_scope(&scan_result.execution.reporting_scope);
    let report = ReportV1::from_scan(scan_result, project_info, resolved, mode);
    let plan = fixer::plan_fixes(&report);
    if plan.groups.is_empty() {
        eprintln!("No machine-applicable fixes available.");
    } else {
        let outcome = fixer::apply_plan(&plan, &cli.directory);
        let applied = outcome.applied();
        if applied > 0 {
            eprintln!(
                "Applied {applied} fix(es) across {} validated root-cause group(s).",
                outcome
                    .groups
                    .iter()
                    .filter(|(_, state)| matches!(state, fixer::GroupOutcome::Validated { .. }))
                    .count()
            );
        }
        if let Some((group, reason)) = outcome.failure() {
            let skipped = outcome
                .groups
                .iter()
                .filter(|(_, state)| matches!(state, fixer::GroupOutcome::NotAttempted))
                .count();
            eprintln!("Fix group '{group}' failed validation: {reason}");
            if skipped > 0 {
                eprintln!("{skipped} later group(s) were not attempted and are not validated.");
            }
        }
    }
    let guidance = plan.guidance_only.len();
    if guidance > 0 {
        eprintln!(
            "{guidance} finding(s) have guidance-only remediation; review them before editing."
        );
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

/// Render the shipping remediation plan from canonical Report V1 ordering.
///
/// The legacy `ScanResult` entry point remains available for library
/// compatibility, while the CLI consumes the same policy and comparator as
/// every other report surface.
pub fn emit_report_plan_if_requested(cli: &Cli, report: &ReportV1, scan_result: &ScanResult) {
    if cli.plan {
        let items = plan::generate_report_plan(report);
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
    if fail_on != FailOn::None {
        let decision = crate::completeness::score_decision(scan_result);
        if decision.published_score().is_none() {
            eprintln!(
                "Finding gate failed: authoritative score unavailable ({})",
                decision.primary_reason()
            );
            return Some(ExitCode::from(EXIT_GATE_FAILURE));
        }
    }
    if scan_result
        .execution
        .baseline
        .as_ref()
        .is_some_and(|baseline| baseline.baseline_degraded)
    {
        return None;
    }
    let Ok(catalog) = crate::catalog::built_in_catalog() else {
        return check_fail_on_gate(scan_result, fail_on);
    };
    let mut visible = [0_usize; 3];
    for diagnostic in &scan_result.diagnostics {
        let descriptor =
            catalog.resolve(&diagnostic.rule, &diagnostic.category, diagnostic.severity);
        if resolved.demotes_to_audit_only(&descriptor.as_descriptor().canonical_id) {
            continue;
        }
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
    use clap::Parser;
    use std::time::Duration;

    fn make_scan_result(score: u32, errors: usize, warnings: usize, infos: usize) -> ScanResult {
        let source = std::path::PathBuf::from("src/lib.rs");
        let package_id = "fixture 0.1.0 (path+file:///fixture)".to_string();
        let checks = vec![
            CheckState {
                name: "clippy".to_string(),
                required: true,
                status: CheckStatus::Completed,
                reason: None,
            },
            CheckState {
                name: "custom rules".to_string(),
                required: true,
                status: CheckStatus::Completed,
                reason: None,
            },
            CheckState {
                name: "msrv".to_string(),
                required: true,
                status: CheckStatus::Completed,
                reason: None,
            },
        ];
        let execution = crate::diagnostics::ScanExecution {
            checks: checks.clone(),
            analyzer_receipts: checks
                .iter()
                .map(crate::completeness::AnalyzerReceipt::root)
                .map(|receipt| receipt.for_package(package_id.clone(), false))
                .collect(),
            packages: vec![crate::diagnostics::PackageExecution {
                cargo_package_id: package_id,
                package_root: std::path::PathBuf::from("."),
                planned_files: vec![source.clone()],
                analyzed_files: vec![source.clone()],
                checks,
                elapsed: Duration::ZERO,
                score: Some(score),
            }],
            ..crate::diagnostics::ScanExecution::default()
        };
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
            planned_files: vec![source.clone()],
            analyzed_files: vec![source],
            compiler_evidence: vec![],
            execution,
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

    #[test]
    fn incomplete_hidden_score_fails_threshold_gate() {
        let mut result = make_scan_result(10, 0, 0, 0);
        result.analyzed_files.clear();
        assert!(check_score_gate(&result, Some(80)).is_some());
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
    fn degraded_baseline_fails_a_configured_finding_gate() {
        let mut result = make_scan_result(50, 1, 0, 0);
        result.execution.baseline = Some(BaselineReport {
            requested_base: "main".to_string(),
            resolved_base: None,
            base_commit: String::new(),
            head_config_fingerprint: String::new(),
            base_config_fingerprint: None,
            new_count: 1,
            fixed_count: 0,
            base_total: 0,
            cross_file_match_count: 0,
            baseline_degraded: true,
            degraded_reason: Some("base unavailable".to_string()),
        });
        let resolved = config::resolve_config_defaults(None);
        assert!(check_fail_on_gate_for_config(&result, &resolved, FailOn::Error).is_some());
    }

    #[test]
    fn required_non_clippy_failure_fails_a_configured_finding_gate() {
        let mut result = make_scan_result(100, 0, 0, 0);
        let receipt = result
            .execution
            .analyzer_receipts
            .iter_mut()
            .find(|receipt| receipt.analyzer == crate::completeness::AnalyzerIdentity::CustomRules)
            .expect("custom-rules receipt");
        receipt.status = CheckStatus::Failed;
        receipt.reason = Some("custom-rules pass panicked".to_string());
        let resolved = config::resolve_config_defaults(None);
        assert!(check_fail_on_gate_for_config(&result, &resolved, FailOn::Error).is_some());
    }

    #[test]
    fn test_require_complete_fails_for_a_timed_out_required_check() {
        let mut result = make_scan_result(100, 0, 0, 0);
        let receipt = result
            .execution
            .analyzer_receipts
            .iter_mut()
            .find(|receipt| receipt.analyzer == crate::completeness::AnalyzerIdentity::Clippy)
            .expect("clippy receipt");
        receipt.status = CheckStatus::TimedOut;
        receipt.reason = Some("deadline".to_string());
        assert!(check_completeness_gate(&result, true).is_some());
        assert!(check_completeness_gate(&result, false).is_none());
    }

    #[test]
    fn hard_clippy_failure_blocks_score_mode_unless_explicitly_disabled() {
        let mut result = make_scan_result(100, 0, 0, 0);
        let receipt = result
            .execution
            .analyzer_receipts
            .iter_mut()
            .find(|receipt| receipt.analyzer == crate::completeness::AnalyzerIdentity::Clippy)
            .expect("clippy receipt");
        receipt.status = CheckStatus::Failed;
        receipt.reason = Some("compiler invocation failed".to_string());

        assert!(check_hard_analysis_failure(&result, FailOn::Error).is_some());
        assert!(check_hard_analysis_failure(&result, FailOn::None).is_none());
    }

    #[test]
    fn degraded_baseline_does_not_promote_base_clippy_failure() {
        let mut result = make_scan_result(100, 0, 0, 0);
        let check = CheckState {
            name: "base:package:path#crate@1.0.0:clippy".to_string(),
            required: true,
            status: CheckStatus::Failed,
            reason: Some("base compiler invocation failed".to_string()),
        };
        result.execution.checks.push(check.clone());
        result
            .execution
            .analyzer_receipts
            .push(crate::completeness::AnalyzerReceipt::root(&check).for_baseline());
        result.execution.baseline = Some(BaselineReport {
            requested_base: "main".to_string(),
            resolved_base: Some("abc123".to_string()),
            base_commit: "abc123".to_string(),
            head_config_fingerprint: "head".to_string(),
            base_config_fingerprint: None,
            new_count: 0,
            fixed_count: 0,
            base_total: 0,
            cross_file_match_count: 0,
            baseline_degraded: true,
            degraded_reason: Some("base scan failed".to_string()),
        });

        assert!(check_hard_analysis_failure(&result, FailOn::Error).is_none());
    }

    #[test]
    fn score_mode_never_allows_interactive_prompts() {
        let cli = Cli::parse_from(["rust-doctor", "--score", "."]);
        assert!(!interactive_prompts_allowed(&cli));
        assert!(!post_scan_prompts_allowed(&cli));
    }

    #[test]
    fn workspace_display_order_is_root_first_then_relative_path() {
        let member = |name: &str, root: &str| discovery::WorkspaceMember {
            name: name.to_string(),
            root_dir: PathBuf::from(root),
            package_id: name.to_string(),
            targets: Vec::new(),
            cargo_targets: Vec::new(),
            frameworks: Vec::new(),
            framework_capabilities: Vec::new(),
            rust_version: None,
            edition: "2024".to_string(),
            enabled_features: Vec::new(),
        };
        let members = vec![
            member("shared", "/workspace/shared"),
            member("root", "/workspace"),
            member("api", "/workspace/crates/api"),
        ];

        let names = workspace_members_in_display_order(Path::new("/workspace"), &members)
            .into_iter()
            .map(|member| member.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, ["root", "api", "shared"]);
    }

    #[test]
    fn failure_report_mode_includes_configured_diff_scope() {
        let cli = Cli::parse_from(["rust-doctor", "."]);
        let mut resolved = config::resolve_config_defaults(None);
        resolved.diff = Some("develop".to_string());

        assert_eq!(requested_scan_mode(&cli, &resolved), ScanMode::Files);
    }

    #[test]
    fn ci_prompt_key_is_stable_and_path_free() {
        let root = Path::new("/private/project");
        let key = ci_prompt_key(root);
        assert_eq!(key.len(), 32);
        assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!key.contains("private"));
        assert_eq!(key, ci_prompt_key(root));
    }
}
