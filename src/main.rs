#![forbid(unsafe_code)]
#![allow(clippy::multiple_crate_versions)]

use clap::Parser;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use rust_doctor::cli::Cli;
use rust_doctor::diagnostics::{ScanMode, ScanResult};
use rust_doctor::{config, run};
use std::any::Any;
use std::ffi::OsString;
use std::fmt;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn main() -> ExitCode {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        if !is_closed_pipe_panic(information.payload()) {
            previous_hook(information);
        }
    }));
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(code) => code,
        Err(payload) if is_closed_pipe_panic(payload.as_ref()) => ExitCode::SUCCESS,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the CLI lifecycle remains visible in one dependency-ordered function"
)]
fn run() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let arguments: Vec<_> = std::env::args_os().collect();
    let scope_was_explicit = scan_scope_was_explicit(&arguments);
    let mut cli = match try_parse_react_compatible(arguments) {
        Ok(cli) => cli,
        Err(error) => {
            let code = if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(run::EXIT_SCAN_ERROR)
            };
            let _ = error.print();
            return code;
        }
    };

    if let Err(error) = cli.validate_contract() {
        let _ = error.print();
        return ExitCode::from(run::EXIT_SCAN_ERROR);
    }
    run::configure_color(&cli);
    run::configure_telemetry(&cli);
    if let Some(code) = run::handle_command(&cli) {
        return code;
    }
    if cli.install_deps {
        return run::handle_install_deps();
    }

    // Stdio server modes
    if cli.lsp || cli.mcp {
        run::emit_server_telemetry(&cli);
    }
    if let Some(code) = run::handle_lsp_flag(&cli) {
        return code;
    }
    if let Some(code) = run::handle_mcp_flag(&cli) {
        return code;
    }
    let cancellation = install_cancellation_handlers();
    run::register_animation_cancellation(&cancellation);

    // Bootstrap: resolve directory, discover project, load file config
    let (_target_dir, project_info, file_config) = match run::bootstrap_project(&cli) {
        Ok(result) => result,
        Err(e) => {
            if cancellation.load(Ordering::SeqCst) {
                return cancelled_exit(&cli, None);
            }
            if cli.wants_json() || cli.sarif {
                if let Err(output_error) = run::emit_failure_report(&cli, "bootstrap", &e) {
                    eprintln!("Error: {output_error}");
                }
            } else {
                eprintln!("Error: {e}");
            }
            return ExitCode::from(run::EXIT_SCAN_ERROR);
        }
    };

    // Merge CLI flags with file config
    let effective_config = if cli.no_project_config {
        None
    } else {
        file_config.as_ref()
    };
    let resolved = config::resolve_config(&cli, effective_config);
    let configured_mode = run::requested_scan_mode(&cli, &resolved);
    if cancellation.load(Ordering::SeqCst) {
        return cancelled_exit(&cli, Some(configured_mode));
    }
    if let Err(error) = run::render_scan_welcome(&cli) {
        if error.kind() == std::io::ErrorKind::BrokenPipe {
            return ExitCode::SUCCESS;
        }
        eprintln!("Error: failed to render terminal header: {error}");
        return ExitCode::from(run::EXIT_SCAN_ERROR);
    }
    if cancellation.load(Ordering::SeqCst) {
        return cancelled_exit(&cli, Some(configured_mode));
    }
    match run::prepare_interactive_scan(&mut cli, &project_info, &resolved, scope_was_explicit) {
        Ok(true) => {}
        Ok(false) => {
            return if cancellation.load(Ordering::SeqCst) {
                cancelled_exit(&cli, Some(configured_mode))
            } else {
                prompt_cancelled_exit(ExitCode::SUCCESS)
            };
        }
        Err(error) => {
            if cancellation.load(Ordering::SeqCst) {
                return cancelled_exit(&cli, Some(configured_mode));
            }
            eprintln!("Error: interactive scan selection failed: {error}");
            return ExitCode::from(run::EXIT_SCAN_ERROR);
        }
    }
    let requested_mode = run::requested_scan_mode(&cli, &resolved);

    // Run scan
    let scan_result =
        match run::run_scan_with_cancellation(&cli, &project_info, &resolved, &cancellation) {
            Ok(result) => result,
            Err(e) => {
                if cancellation.load(Ordering::SeqCst) {
                    return cancelled_exit(&cli, Some(requested_mode));
                }
                if cli.wants_json() || cli.sarif {
                    if let Err(output_error) =
                        run::emit_failure_report_for_mode(&cli, requested_mode, "scan", &e)
                    {
                        eprintln!("Error: {output_error}");
                    }
                } else {
                    eprintln!("Error: {e}");
                }
                return ExitCode::from(run::EXIT_SCAN_ERROR);
            }
        };
    let scan_mode = run::completed_scan_mode(&scan_result);
    if cancellation.load(Ordering::SeqCst) {
        return cancelled_exit(&cli, Some(scan_mode));
    }

    // Apply fixes, emit output, show plan
    run::apply_fixes_if_requested(&cli, &scan_result, &resolved, &project_info);

    let report = match run::emit_output(&cli, &scan_result, &resolved, &project_info) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("Error: {error}");
            return ExitCode::from(run::EXIT_SCAN_ERROR);
        }
    };
    if cancellation.load(Ordering::SeqCst) {
        return if cli.wants_json() || cli.sarif {
            ExitCode::from(130)
        } else {
            cancelled_exit(&cli, Some(scan_mode))
        };
    }
    let gate_exit = quality_gate_exit(&cli, &scan_result, &resolved);
    run::record_onboarding_completion(&cli, &resolved, &report);
    if let Err(error) = run::emit_share_if_requested(&cli, &report) {
        eprintln!("Error: share URL was not created: {error}");
        return ExitCode::from(run::EXIT_SCAN_ERROR);
    }
    match run::offer_ci_setup(&cli, &report, &project_info, &resolved) {
        Ok(true) => {}
        Ok(false) => {
            return if cancellation.load(Ordering::SeqCst) {
                cancelled_exit(&cli, Some(scan_mode))
            } else {
                prompt_cancelled_exit(gate_exit.unwrap_or(ExitCode::SUCCESS))
            };
        }
        Err(_) if cancellation.load(Ordering::SeqCst) => {
            return cancelled_exit(&cli, Some(scan_mode));
        }
        Err(error) => eprintln!("Warning: GitHub Actions setup was not completed: {error}"),
    }
    if let Err(error) = run::emit_handoff(&cli, &report, &resolved) {
        if cancellation.load(Ordering::SeqCst) {
            return cancelled_exit(&cli, Some(scan_mode));
        }
        eprintln!("Warning: diagnostic handoff failed: {error}");
    }
    if cancellation.load(Ordering::SeqCst) {
        return cancelled_exit(&cli, Some(scan_mode));
    }
    run::emit_agent_install_hint(&cli, &report, &project_info);
    run::emit_scan_telemetry(&cli, &report);

    run::emit_report_plan_if_requested(&cli, &report, &scan_result);

    gate_exit.unwrap_or(ExitCode::SUCCESS)
}

fn quality_gate_exit(
    cli: &Cli,
    scan_result: &ScanResult,
    resolved: &config::ResolvedConfig,
) -> Option<ExitCode> {
    run::check_hard_analysis_failure(scan_result, resolved.fail_on)
        .or_else(|| run::check_completeness_gate(scan_result, cli.require_complete))
        .or_else(|| run::check_score_authority(scan_result, cli.score))
        .or_else(|| run::check_score_gate(scan_result, resolved.score_fail_below))
        .or_else(|| {
            if cli.score {
                None
            } else {
                run::check_fail_on_gate_for_config(scan_result, resolved, resolved.fail_on)
            }
        })
}

fn install_cancellation_handlers() -> Arc<AtomicBool> {
    let cancellation = Arc::new(AtomicBool::new(false));
    let handler_cancellation = Arc::clone(&cancellation);
    if let Err(error) = ctrlc::set_handler(move || {
        handler_cancellation.store(true, Ordering::SeqCst);
    }) {
        eprintln!("Warning: failed to install cancellation handler: {error}");
    }
    cancellation
}

fn cancelled_exit(cli: &Cli, mode: Option<ScanMode>) -> ExitCode {
    if cli.wants_json() || cli.sarif {
        let result = mode.map_or_else(
            || run::emit_failure_report(cli, "cancelled", &CancelledScan),
            |mode| run::emit_failure_report_for_mode(cli, mode, "cancelled", &CancelledScan),
        );
        if let Err(error) = result {
            eprintln!("Error: failed to render cancellation report: {error}");
        }
    } else {
        println!("\nCancelled.\n");
    }
    ExitCode::from(130)
}

fn prompt_cancelled_exit(exit_code: ExitCode) -> ExitCode {
    println!("\nCancelled.\n");
    exit_code
}

#[derive(Debug)]
struct CancelledScan;

impl fmt::Display for CancelledScan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Scan cancelled by user (SIGINT/SIGTERM)")
    }
}

impl std::error::Error for CancelledScan {}

fn scan_scope_was_explicit(arguments: &[OsString]) -> bool {
    const FLAGS: [&str; 7] = [
        "--scope",
        "--diff",
        "--staged",
        "--baseline",
        "--base",
        "--files",
        "--include-untracked",
    ];
    arguments.iter().skip(1).any(|argument| {
        argument.to_str().is_some_and(|argument| {
            FLAGS
                .iter()
                .any(|flag| argument == *flag || argument.starts_with(&format!("{flag}=")))
        })
    })
}

fn try_parse_react_compatible(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Cli, clap::Error> {
    let mut arguments: Vec<_> = arguments.into_iter().collect();
    loop {
        match Cli::try_parse_from(&arguments) {
            Ok(cli) => return Ok(cli),
            Err(error) if error.kind() == ErrorKind::UnknownArgument => {
                let Some(ContextValue::String(argument)) = error.get(ContextKind::InvalidArg)
                else {
                    return Err(error);
                };
                if !argument.starts_with('-') {
                    return Err(error);
                }
                let Some(index) = arguments.iter().position(|candidate| {
                    candidate == argument.as_str()
                        || candidate.to_str().is_some_and(|candidate| {
                            candidate
                                .split_once('=')
                                .is_some_and(|(name, _)| name == argument)
                        })
                }) else {
                    return Err(error);
                };
                arguments.remove(index);
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_closed_pipe_panic(payload: &(dyn Any + Send)) -> bool {
    let message = payload.downcast_ref::<&str>().map_or_else(
        || payload.downcast_ref::<String>().map_or("", String::as_str),
        |message| *message,
    );
    message.contains("failed printing to stdout")
        || message.contains("failed printing to stderr")
        || message.contains("Broken pipe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_doctor::cli::FailOn;
    use std::path::PathBuf;

    fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
        try_parse_react_compatible(arguments.iter().map(|argument| OsString::from(*argument)))
    }

    #[test]
    fn react_compatible_parser_ignores_standalone_unknown_flags() {
        let cli = parse(&[
            "rust-doctor",
            "--not-a-real-option",
            ".",
            "--blocking",
            "none",
        ])
        .unwrap();

        assert_eq!(cli.directory, PathBuf::from("."));
        assert_eq!(cli.blocking, Some(FailOn::None));
    }

    #[test]
    fn react_compatible_parser_ignores_inline_unknown_flags() {
        let cli = parse(&[
            "rust-doctor",
            "--not-a-real-option=value",
            ".",
            "--blocking=none",
        ])
        .unwrap();

        assert_eq!(cli.directory, PathBuf::from("."));
        assert_eq!(cli.blocking, Some(FailOn::None));
    }

    #[test]
    fn react_compatible_parser_preserves_known_invalid_value_errors() {
        let error = parse(&["rust-doctor", "--blocking=definitely-invalid"]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn explicit_scan_scope_detects_value_and_boolean_forms() {
        assert!(scan_scope_was_explicit(&[
            OsString::from("rust-doctor"),
            OsString::from("--scope=full"),
        ]));
        assert!(scan_scope_was_explicit(&[
            OsString::from("rust-doctor"),
            OsString::from("--diff"),
            OsString::from("main"),
        ]));
        assert!(scan_scope_was_explicit(&[
            OsString::from("rust-doctor"),
            OsString::from("--staged"),
        ]));
    }

    #[test]
    fn unrelated_flags_do_not_make_the_default_scope_explicit() {
        assert!(!scan_scope_was_explicit(&[
            OsString::from("rust-doctor"),
            OsString::from("--project"),
            OsString::from("core"),
            OsString::from("--scope-adjacent=value"),
        ]));
    }
}
