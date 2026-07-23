#![forbid(unsafe_code)]
#![allow(clippy::multiple_crate_versions)]

use clap::Parser;
use rust_doctor::cli::Cli;
use rust_doctor::{config, run};
use std::any::Any;
use std::process::ExitCode;

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

fn run() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

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

    // Bootstrap: resolve directory, discover project, load file config
    let (_target_dir, project_info, file_config) = match run::bootstrap_project(&cli) {
        Ok(result) => result,
        Err(e) => {
            if cli.wants_json() {
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

    // Run scan
    let scan_result = match run::run_scan(&cli, &project_info, &resolved) {
        Ok(result) => result,
        Err(e) => {
            if cli.wants_json() {
                if let Err(output_error) = run::emit_failure_report(&cli, "scan", &e) {
                    eprintln!("Error: {output_error}");
                }
            } else {
                eprintln!("Error: {e}");
            }
            return ExitCode::from(run::EXIT_SCAN_ERROR);
        }
    };

    // Apply fixes, emit output, show plan
    run::apply_fixes_if_requested(&cli, &scan_result);

    if let Err(e) = run::emit_output(&cli, &scan_result, &resolved, &project_info) {
        eprintln!("Error: {e}");
        return ExitCode::from(run::EXIT_SCAN_ERROR);
    }
    if let Err(error) = run::emit_handoff(&cli, &scan_result, &resolved, &project_info) {
        eprintln!("Warning: diagnostic handoff failed: {error}");
    }
    run::emit_scan_telemetry(&cli, &scan_result);
    if let Err(error) = run::emit_share_if_requested(&cli, &scan_result) {
        eprintln!("Error: share URL was not created: {error}");
        return ExitCode::from(run::EXIT_SCAN_ERROR);
    }

    run::emit_plan_if_requested(&cli, &scan_result);

    // Quality gates
    if let Some(code) = run::check_completeness_gate(&scan_result, cli.require_complete) {
        return code;
    }
    if let Some(code) = run::check_score_gate(&scan_result, resolved.score_fail_below) {
        return code;
    }
    if let Some(code) =
        run::check_fail_on_gate_for_config(&scan_result, &resolved, resolved.fail_on)
    {
        return code;
    }

    ExitCode::SUCCESS
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
