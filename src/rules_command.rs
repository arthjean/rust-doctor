//! `rust-doctor rules`: the catalog, printed rather than described, and one
//! rule explained.
//!
//! `rules list` reads nothing from the filesystem: the catalog is what the
//! binary was compiled with, so it answers the same thing everywhere and needs
//! no workspace. It exists so that what publishes the rule list, the website
//! included, reads it from the tool instead of restating it. `rules explain`
//! reads the `rust-doctor.toml` a scan of its PATH reads, at the workspace
//! root `cargo metadata` names, and starts no other process.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use rust_doctor::explain::explain_rule;

#[derive(Debug, Clone, Args)]
pub struct RulesArgs {
    #[command(subcommand)]
    command: RulesCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum RulesCommand {
    #[command(about = "List every catalogued rule")]
    List {
        #[arg(long)]
        json: bool,
    },
    #[command(
        about = "Explain one rule, and with PATH how that workspace's rust-doctor.toml sets it"
    )]
    Explain {
        /// A catalogued id, or its last segment when only one rule ends with
        /// it: `todo` is `clippy::todo`.
        #[arg(value_name = "RULE")]
        rule: String,
        /// The workspace whose rust-doctor.toml sets the effective level.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

pub fn run(arguments: &RulesArgs) -> ExitCode {
    let written = match &arguments.command {
        RulesCommand::List { json } => list(*json),
        RulesCommand::Explain { rule, path, json } => {
            let path = match path.as_deref().map(|path| crate::path_argument::resolve("PATH", path)) {
                Some(Err(code)) => return ExitCode::from(code),
                resolved => resolved.and_then(Result::ok),
            };
            match explain_rule(rule, path.as_deref()) {
            Ok(explained) if *json => json_line(&explained),
            Ok(explained) => io::stdout().lock().write_all(explained.text().as_bytes()),
            Err(error) => {
                eprintln!("rust-doctor: {error}");
                return ExitCode::from(2);
            }
            }
        }
    };
    // A closed pipe is how `| head` ends, not a failure to report.
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn list(json: bool) -> io::Result<()> {
    let catalog = rust_doctor::catalog();
    if json {
        return json_line(&catalog);
    }
    let mut out = io::stdout().lock();
    catalog.iter().try_for_each(|rule| {
        writeln!(
            out,
            "{}\t{}\t{}\t{}",
            rule.id,
            rule.category,
            rule.tier.as_str(),
            rule.help
        )
    })
}

pub fn json_line(value: &impl serde::Serialize) -> io::Result<()> {
    let payload = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    writeln!(io::stdout().lock(), "{payload}")
}
