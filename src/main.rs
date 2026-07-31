#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rust_doctor::render::{render_json, render_terminal};
use rust_doctor::{InspectRequest, inspect};

const TRUST_WARNING: &str = "Cargo may execute build.rs files and procedural macros. Inspect trusted local repositories only.";

#[derive(Debug, Parser)]
#[command(name = "rust-doctor", version)]
#[command(about = "Inspect a trusted local Rust workspace with Cargo and Clippy")]
#[command(long_about = format!(
    "Inspect a trusted local Rust workspace with Cargo and Clippy.\n\n{TRUST_WARNING}"
))]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Inspect a trusted local Cargo workspace")]
    #[command(long_about = format!(
        "Inspect a trusted local Cargo workspace.\n\n{TRUST_WARNING}"
    ))]
    Inspect {
        #[arg(default_value = ".", value_name = "PATH")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect { path, json } => run_inspect(path, json),
    }
}

fn run_inspect(path: PathBuf, json: bool) -> ExitCode {
    eprintln!("Inspecting Cargo workspace");
    let report = inspect(InspectRequest::new(path));
    let stdout = io::stdout();
    let result = if json {
        render_json(&report, stdout.lock())
    } else {
        render_terminal(&report, stdout.lock())
    };

    match result {
        Ok(()) => ExitCode::from(report.status.exit_code()),
        Err(error) => {
            eprintln!("Failed to write report: {error}");
            ExitCode::from(2)
        }
    }
}
