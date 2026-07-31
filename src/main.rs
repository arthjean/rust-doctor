#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use clap::builder::TypedValueParser;
use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use rust_doctor::render::{render_json, render_terminal};
use rust_doctor::{BlockingLevel, CategoryOverride, InspectRequest, RuleOverride, inspect};

const TRUST_WARNING: &str = "Cargo may execute build.rs files and procedural macros. Inspect trusted local repositories only.";

#[derive(Debug)]
struct RedactedOverrideParser<T>(PhantomData<T>);

impl<T> RedactedOverrideParser<T> {
    const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Clone for RedactedOverrideParser<T> {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl<T> TypedValueParser for RedactedOverrideParser<T>
where
    T: Clone + FromStr + Send + Sync + 'static,
    T::Err: fmt::Display,
{
    type Value = T;

    fn parse_ref(
        &self,
        command: &clap::Command,
        _argument: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let parsed = value.to_str().ok_or_else(|| {
            clap::Error::raw(
                ErrorKind::ValueValidation,
                "policy override must be valid UTF-8; expected KEY=LEVEL with LEVEL one of: off, warn, error",
            )
            .with_cmd(command)
        })?;
        parsed.parse().map_err(|error: T::Err| {
            clap::Error::raw(ErrorKind::ValueValidation, error).with_cmd(command)
        })
    }
}

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
        #[arg(
            long,
            value_name = "RULE_ID=LEVEL",
            value_parser = RedactedOverrideParser::<RuleOverride>::new()
        )]
        rule: Vec<RuleOverride>,
        #[arg(
            long,
            value_name = "CATEGORY=LEVEL",
            value_parser = RedactedOverrideParser::<CategoryOverride>::new()
        )]
        category: Vec<CategoryOverride>,
        #[arg(long, value_enum, default_value_t)]
        blocking: BlockingLevel,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect {
            path,
            json,
            rule,
            category,
            blocking,
        } => run_inspect(path, json, rule, category, blocking),
    }
}

fn run_inspect(
    path: PathBuf,
    json: bool,
    rule_overrides: Vec<RuleOverride>,
    category_overrides: Vec<CategoryOverride>,
    blocking: BlockingLevel,
) -> ExitCode {
    eprintln!("Inspecting Cargo workspace");
    let mut request = InspectRequest::new(path).with_blocking(blocking);
    for rule_override in rule_overrides {
        request = request.with_rule_override(rule_override);
    }
    for category_override in category_overrides {
        request = request.with_category_override(category_override);
    }
    let report = inspect(request);
    let stdout = io::stdout();
    let result = if json {
        render_json(&report, stdout.lock())
    } else {
        render_terminal(&report, stdout.lock())
    };

    match result {
        Ok(()) => ExitCode::from(report.exit_code()),
        Err(error) => {
            eprintln!("Failed to write report: {error}");
            ExitCode::from(2)
        }
    }
}
