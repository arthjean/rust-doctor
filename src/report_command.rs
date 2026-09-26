//! `rust-doctor report`: what can be done with a report a scan already saved,
//! without scanning again.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::builder::TypedValueParser as _;
use clap::{Args, Subcommand};
use rust_doctor::render::markdown::{
    DEFAULT_FINDING_LIMIT, read_saved_report, render_markdown,
};

#[derive(Debug, Clone, Args)]
pub struct ReportArgs {
    #[command(subcommand)]
    command: ReportCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum ReportCommand {
    #[command(
        about = "Render a saved --json report as Markdown, for a job summary or a pull request comment"
    )]
    Markdown {
        /// The report `rust-doctor --json` wrote.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// How many findings the table lists.
        #[arg(
            long,
            value_name = "N",
            default_value_t = DEFAULT_FINDING_LIMIT,
            value_parser = clap::value_parser!(u16).range(0..=1000).map(usize::from)
        )]
        limit: usize,
    },
}

/// Opens the one file named and spawns nothing: rendering a report is not a
/// scan, and a CI job that renders one never needs Cargo.
pub fn run(arguments: &ReportArgs) -> ExitCode {
    let ReportCommand::Markdown { file, limit } = &arguments.command;
    let rendered =
        read_saved_report(file).and_then(|report| render_markdown(&report, *limit));
    let markdown = match rendered {
        Ok(markdown) => markdown,
        Err(error) => {
            eprintln!("rust-doctor: {}: {error}.", file.display());
            return ExitCode::from(2);
        }
    };
    match io::stdout().lock().write_all(markdown.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(2),
    }
}
