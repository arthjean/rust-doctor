#![forbid(unsafe_code)]
#![allow(clippy::multiple_crate_versions)]

use clap::Parser;
use rust_doctor::cli::{Cli, Command, FailOn};
use rust_doctor::diagnostics::ScanResult;
use rust_doctor::{config, deps, discovery, fixer, output, plan, sarif, scan};
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Setup wizard
    if matches!(cli.command, Some(Command::Setup)) {
        return match rust_doctor::setup::run_setup() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    // Install external tools and exit
    if cli.install_deps {
        deps::print_status();
        let all_ok = deps::install_missing_tools();
        return if all_ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    // MCP mode: run as a stdio MCP server for AI tool integration
    if let Some(code) = handle_mcp_flag(&cli) {
        return code;
    }

    // Bootstrap: resolve directory, discover project, load file config
    let (_target_dir, project_info, file_config) = match bootstrap_project(&cli) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Merge CLI flags with file config (skip project config if --no-project-config)
    let effective_config = if cli.no_project_config {
        None
    } else {
        file_config.as_ref()
    };
    let resolved = config::resolve_config(&cli, effective_config);

    // Run scan
    let scan_result = match run_scan(&cli, &project_info, &resolved) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Apply fixes if requested
    if cli.fix {
        let applied = fixer::apply_fixes(&scan_result.diagnostics, &cli.directory);
        if applied > 0 {
            eprintln!("Applied {applied} fix(es).");
        } else {
            eprintln!("No machine-applicable fixes available.");
        }
    }

    // Output
    if let Err(e) = emit_output(&cli, &scan_result, &resolved) {
        eprintln!("Error: {e}");
        return ExitCode::FAILURE;
    }

    // Show remediation plan if requested
    if cli.plan {
        let items = plan::generate_plan(&scan_result);
        let plan_text = plan::format_plan_markdown(&items, &scan_result);
        eprintln!("\n{plan_text}");
    }

    // Quality gates
    if let Some(code) = check_score_gate(&scan_result, resolved.score_fail_below) {
        return code;
    }
    if check_fail_on_gate(&scan_result, resolved.fail_on) {
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Dispatch `--mcp` flag: start the MCP server or report a compile-time error.
/// Returns `Some(ExitCode)` if MCP was handled, `None` to continue normal flow.
fn handle_mcp_flag(cli: &Cli) -> Option<ExitCode> {
    #[cfg(feature = "mcp")]
    if cli.mcp {
        return Some(match rust_doctor::mcp::run_mcp_server() {
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

/// Resolve directory, discover the project, and load file-based configuration.
fn bootstrap_project(
    cli: &Cli,
) -> Result<
    (
        std::path::PathBuf,
        discovery::ProjectInfo,
        Option<config::FileConfig>,
    ),
    rust_doctor::error::BootstrapError,
> {
    discovery::bootstrap_project(&cli.directory, cli.offline)
}

/// Run the scan passes and return the result.
fn run_scan(
    cli: &Cli,
    project_info: &discovery::ProjectInfo,
    resolved: &config::ResolvedConfig,
) -> Result<ScanResult, rust_doctor::error::ScanError> {
    let suppress_spinner = cli.score || cli.json || cli.sarif;
    scan::scan_project(
        project_info,
        resolved,
        cli.offline,
        &cli.project,
        suppress_spinner,
    )
}

/// Render the appropriate output format (score, JSON, SARIF, or terminal).
fn emit_output(
    cli: &Cli,
    scan_result: &ScanResult,
    resolved: &config::ResolvedConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    if cli.score {
        output::render_score(scan_result);
    } else if cli.json {
        output::render_json(scan_result)?;
    } else if cli.sarif {
        let sarif_json = sarif::render_sarif(scan_result)?;
        println!("{sarif_json}");
    } else {
        // --category implies verbose for the selected category
        let effective_verbose = resolved.verbose || resolved.category_filter.is_some();
        output::render_terminal(scan_result, effective_verbose, resolved.category_filter);
    }
    Ok(())
}

/// Returns `Some(ExitCode::FAILURE)` if the score is below the configured threshold.
fn check_score_gate(scan_result: &ScanResult, threshold: Option<u32>) -> Option<ExitCode> {
    if let Some(threshold) = threshold {
        if scan_result.score < threshold {
            eprintln!(
                "Score {} is below the configured threshold of {}",
                scan_result.score, threshold
            );
            return Some(ExitCode::FAILURE);
        }
    }
    None
}

/// Returns `true` if any diagnostic exceeds the `--fail-on` severity level.
const fn check_fail_on_gate(scan_result: &ScanResult, fail_on: FailOn) -> bool {
    match fail_on {
        FailOn::Error => scan_result.error_count > 0,
        FailOn::Warning => scan_result.error_count > 0 || scan_result.warning_count > 0,
        FailOn::Info => {
            scan_result.error_count > 0
                || scan_result.warning_count > 0
                || scan_result.info_count > 0
        }
        FailOn::None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_doctor::diagnostics::{DimensionScores, ScoreLabel};
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
        assert!(check_fail_on_gate(&result, FailOn::Error));
    }

    #[test]
    fn test_fail_on_error_without_errors() {
        let result = make_scan_result(50, 0, 5, 3);
        assert!(!check_fail_on_gate(&result, FailOn::Error));
    }

    #[test]
    fn test_fail_on_warning_with_warnings() {
        let result = make_scan_result(50, 0, 1, 0);
        assert!(check_fail_on_gate(&result, FailOn::Warning));
    }

    #[test]
    fn test_fail_on_warning_with_errors_too() {
        let result = make_scan_result(50, 1, 0, 0);
        assert!(check_fail_on_gate(&result, FailOn::Warning));
    }

    #[test]
    fn test_fail_on_info_with_info() {
        let result = make_scan_result(50, 0, 0, 1);
        assert!(check_fail_on_gate(&result, FailOn::Info));
    }

    #[test]
    fn test_fail_on_none_never_fails() {
        let result = make_scan_result(50, 10, 20, 30);
        assert!(!check_fail_on_gate(&result, FailOn::None));
    }
}
