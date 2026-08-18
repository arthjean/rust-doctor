#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod handoff;
mod skill;
mod tui;

use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Instant;

use clap::builder::TypedValueParser;
use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use handoff::{
    HandoffError, RescanCommand, available_agents, build_prompt, copy_to_clipboard, launch_agent,
};
use rust_doctor::presentation::ReportPresentation;
use rust_doctor::render::{TerminalOptions, render_json, render_terminal_with_presentation};
use rust_doctor::{
    BlockingLevel, CategoryOverride, InspectReport, InspectRequest, InspectionSession, RuleOverride,
    ScopeMode, Status,
};

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
    "Inspect a trusted local Rust workspace with Cargo and Clippy.\n\n{TRUST_WARNING}\n\nRun `rust-doctor ./inspect` to inspect a directory literally named `inspect`."
))]
struct Cli {
    #[command(flatten)]
    inspect: InspectArgs,
    #[command(subcommand)]
    command: Option<CliCommand>,
}

impl Cli {
    fn into_inspect_args(self) -> InspectArgs {
        match self.command {
            Some(CliCommand::Inspect(arguments)) => arguments,
            Some(CliCommand::Rules(_) | CliCommand::Skill(_)) | None => self.inspect,
        }
    }
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    #[command(about = "Inspect a trusted local Cargo workspace")]
    #[command(long_about = format!(
        "Inspect a trusted local Cargo workspace.\n\n{TRUST_WARNING}\n\nUse `rust-doctor ./inspect` when the path itself is named `inspect`."
    ))]
    Inspect(InspectArgs),
    #[command(about = "Print the rule catalog")]
    Rules(RulesArgs),
    #[command(about = "Install the agent skill")]
    Skill(SkillArgs),
}

/// The skill an agent reads to drive the tool, written into the workspace.
///
/// It ships inside the binary, so this reaches no network and installs the
/// skill of the running version rather than whatever the latest branch holds.
#[derive(Debug, Clone, Args)]
struct SkillArgs {
    #[command(subcommand)]
    command: SkillCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum SkillCommand {
    #[command(about = "Write the skill into .claude/skills/, never over an existing one")]
    Install {
        #[arg(default_value = ".", value_name = "PATH")]
        path: PathBuf,
    },
}

/// The catalog, printed rather than described.
///
/// Nothing here reads the filesystem: the catalog is what the binary was
/// compiled with, so this command answers the same thing everywhere and needs
/// no workspace to answer it. It exists so that what publishes the rule list,
/// the website included, reads it from the tool instead of restating it.
#[derive(Debug, Clone, Args)]
struct RulesArgs {
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
}

#[derive(Debug, Clone, Args)]
struct InspectArgs {
    #[arg(default_value = ".", value_name = "PATH")]
    path: PathBuf,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    verbose: bool,
    #[arg(long)]
    yes: bool,
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
    #[arg(long, value_enum)]
    blocking: Option<BlockingLevel>,
    #[arg(long, value_enum)]
    scope: Option<ScopeArgument>,
    #[arg(long, value_name = "REF")]
    base: Option<String>,
}

impl InspectArgs {
    fn request(&self, scoped_base: Option<&(ScopeArgument, String)>) -> InspectRequest {
        let mut request = InspectRequest::new(&self.path);
        if let Some((mode, base)) = scoped_base {
            request = match mode {
                ScopeArgument::Files => request.with_files_scope(base),
                ScopeArgument::Baseline => request.with_baseline_scope(base),
                ScopeArgument::Full => request,
            };
        }
        if let Some(blocking) = self.blocking {
            request = request.with_blocking(blocking);
        }
        for rule_override in &self.rule {
            request = request.with_rule_override(rule_override.clone());
        }
        for category_override in &self.category {
            request = request.with_category_override(category_override.clone());
        }
        request
    }

    fn rescan_command(
        &self,
        scoped_base: Option<&(ScopeArgument, String)>,
    ) -> Result<RescanCommand, HandoffError> {
        let scope = scoped_base.map(|(scope, base)| {
            (
                match scope {
                    ScopeArgument::Full => ScopeMode::Full,
                    ScopeArgument::Files => ScopeMode::Files,
                    ScopeArgument::Baseline => ScopeMode::Baseline,
                },
                base.as_str(),
            )
        });
        RescanCommand::for_inspection(
            self.verbose,
            self.blocking,
            &self.rule,
            &self.category,
            scope,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ScopeArgument {
    Full,
    Files,
    Baseline,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(CliCommand::Rules(arguments)) = &cli.command {
        return run_rules(arguments);
    }
    if let Some(CliCommand::Skill(arguments)) = &cli.command {
        return run_skill(arguments);
    }
    run_inspect(cli.into_inspect_args())
}

fn run_skill(arguments: &SkillArgs) -> ExitCode {
    let SkillCommand::Install { path } = &arguments.command;
    match skill::install(path) {
        Ok(written) => {
            for document in written {
                println!("{}", document.display());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rust-doctor: the skill was not installed, {error}.");
            ExitCode::FAILURE
        }
    }
}

fn run_rules(arguments: &RulesArgs) -> ExitCode {
    let RulesCommand::List { json } = arguments.command;
    let catalog = rust_doctor::catalog();
    let mut out = io::stdout().lock();
    let written = if json {
        match serde_json::to_string_pretty(&catalog) {
            Ok(payload) => writeln!(out, "{payload}"),
            Err(_) => {
                eprintln!("rust-doctor: the catalog could not be serialized.");
                return ExitCode::FAILURE;
            }
        }
    } else {
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
    };
    // A closed pipe is how `| head` ends, not a failure to report.
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn run_inspect(arguments: InspectArgs) -> ExitCode {
    let scoped_base = match validate_scope(arguments.scope, arguments.base.as_deref()) {
        Ok(scope) => scope,
        Err((kind, message)) => return clap_error(kind, message),
    };
    let stdin_is_terminal = io::stdin().is_terminal();
    let stdout_is_terminal = io::stdout().is_terminal();
    let term = env::var_os("TERM");
    let interactive = interactions_allowed(
        arguments.json,
        arguments.yes,
        stdin_is_terminal,
        stdout_is_terminal,
        env::var_os("CI").as_deref(),
        term.as_deref() == Some(OsStr::new("dumb")),
    );
    if !arguments.json {
        eprintln!("Scanning Rust files...");
    }
    // The scope is what the invocation asked for, nothing else. A run that
    // names no scope scans the whole workspace rather than opening a menu:
    // narrowing to what changed is what `--scope files` and `--scope baseline`
    // are for, and a question asked before the scan is a question asked before
    // the reader has seen a single finding.
    let request = arguments.request(scoped_base.as_ref());
    let session = InspectionSession::prepare(request);
    let rescan_command = arguments.rescan_command(scoped_base.as_ref());

    let started = Instant::now();
    let (report, workspace_root) = match session {
        Ok(session) => {
            let workspace_root = session.workspace_root().to_path_buf();
            (session.inspect(), workspace_root)
        }
        Err(report) => (*report, PathBuf::from(".")),
    };
    let elapsed = started.elapsed();
    let scan_exit = report.exit_code();
    let presentation = ReportPresentation::derive_terminal(&report);
    let color = terminal_color_enabled(
        stdout_is_terminal,
        env::var_os("NO_COLOR").as_deref(),
        term.as_deref(),
    );
    let animate = terminal_animation_enabled(
        stdout_is_terminal,
        env::var_os("NO_COLOR").as_deref(),
        term.as_deref(),
        env::var_os("CI").as_deref(),
    );
    // `--verbose` means print everything, so it keeps the linear report; a
    // failed scan does too, because its errors are the only thing worth
    // reading and the interactive report has no room to carry them.
    let interactive_report =
        interactive && !arguments.json && !arguments.verbose && report.status != Status::Failed;
    let stdout = io::stdout();
    let render_result = if arguments.json {
        render_json(&report, stdout.lock())
    } else if interactive_report {
        Ok(())
    } else {
        render_terminal_with_presentation(
            &report,
            &presentation,
            stdout.lock(),
            TerminalOptions {
                workspace_root: &workspace_root,
                elapsed,
                verbose: arguments.verbose,
                width: terminal_width(
                    stdout_is_terminal,
                    term.as_deref(),
                    env::var_os("COLUMNS").as_deref(),
                ),
                color,
                animate,
            },
        )
    };
    if let Err(error) = render_result {
        if error.is_broken_pipe() {
            return ExitCode::from(scan_exit);
        }
        bounded_stderr(&format!("Failed to write report: {error}"));
        return ExitCode::from(2);
    }

    if interactive_report
        && let Err(error) = run_interactive_report(
            &report,
            &presentation,
            &workspace_root,
            rescan_command,
            color,
            animate,
        )
    {
        bounded_stderr(&error.to_string());
        if scan_exit == 0 {
            return ExitCode::from(2);
        }
    }
    ExitCode::from(scan_exit)
}

/// Draws the interactive report and carries out whatever the reader chose on
/// the way out. An agent is launched only once the loop has given the terminal
/// back, so it inherits a screen nothing else is writing to.
fn run_interactive_report(
    report: &InspectReport,
    presentation: &ReportPresentation,
    workspace_root: &Path,
    rescan_command: Result<RescanCommand, HandoffError>,
    color: bool,
    animate: bool,
) -> Result<(), HandoffError> {
    let agents = available_agents();
    let session = tui::Session {
        report,
        presentation,
        workspace_root,
        agents: &agents,
        color,
        animate,
    };
    let completed = match tui::run(&session) {
        Ok(completed) => completed,
        Err(error) => {
            bounded_stderr(&format!("Interactive report unavailable: {error}"));
            return Ok(());
        }
    };
    if let Some(path) = &completed.installed_workflow {
        eprintln!("Added {}.", path.display());
    }
    match completed.outcome {
        tui::Outcome::Quit => Ok(()),
        tui::Outcome::LaunchAgent(index) => {
            let Some(agent) = agents.get(index) else {
                return Ok(());
            };
            let payload = build_prompt(report, presentation, &rescan_command?)?;
            launch_agent(agent, &payload, workspace_root)
        }
        tui::Outcome::CopyPrompt => {
            let payload = build_prompt(report, presentation, &rescan_command?)?;
            copy_to_clipboard(&payload)?;
            eprintln!("Prompt copied to clipboard.");
            Ok(())
        }
    }
}

fn validate_scope(
    scope: Option<ScopeArgument>,
    base: Option<&str>,
) -> Result<Option<(ScopeArgument, String)>, (ErrorKind, &'static str)> {
    match (scope, base) {
        (None | Some(ScopeArgument::Full), None) => Ok(None),
        (Some(mode @ (ScopeArgument::Files | ScopeArgument::Baseline)), Some(base)) => {
            Ok(Some((mode, base.to_owned())))
        }
        (Some(ScopeArgument::Files), None) => Err((
            ErrorKind::MissingRequiredArgument,
            "--scope files requires --base <REF>",
        )),
        (Some(ScopeArgument::Baseline), None) => Err((
            ErrorKind::MissingRequiredArgument,
            "--scope baseline requires --base <REF>",
        )),
        (None | Some(ScopeArgument::Full), Some(_)) => Err((
            ErrorKind::ArgumentConflict,
            "--base <REF> requires --scope files or --scope baseline",
        )),
    }
}

fn interactions_allowed(
    json: bool,
    yes: bool,
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
    ci: Option<&OsStr>,
    terminal_is_dumb: bool,
) -> bool {
    stdin_is_terminal
        && stdout_is_terminal
        && !json
        && !yes
        && ci.is_none_or(OsStr::is_empty)
        && !terminal_is_dumb
}

fn terminal_width(
    stdout_is_terminal: bool,
    term: Option<&OsStr>,
    columns: Option<&OsStr>,
) -> usize {
    if !stdout_is_terminal || term == Some(OsStr::new("dumb")) {
        return 80;
    }
    columns
        .and_then(OsStr::to_str)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|width| *width >= 80)
        .unwrap_or(80)
}

fn terminal_color_enabled(
    stdout_is_terminal: bool,
    no_color: Option<&OsStr>,
    term: Option<&OsStr>,
) -> bool {
    stdout_is_terminal && no_color.is_none() && term != Some(OsStr::new("dumb"))
}

/// Animating the score requires everything colour requires, plus a session
/// outside CI: a runner captures the output, where intermediate frames would
/// be nothing but noise.
fn terminal_animation_enabled(
    stdout_is_terminal: bool,
    no_color: Option<&OsStr>,
    term: Option<&OsStr>,
    ci: Option<&OsStr>,
) -> bool {
    terminal_color_enabled(stdout_is_terminal, no_color, term) && ci.is_none()
}

fn bounded_stderr(message: &str) {
    let mut bounded: String = message
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    if bounded.len() > 1020 {
        let mut boundary = 1020;
        while !bounded.is_char_boundary(boundary) {
            boundary -= 1;
        }
        bounded.truncate(boundary);
        bounded.push('…');
    }
    eprintln!("{bounded}");
}

fn clap_error(kind: ErrorKind, message: &'static str) -> ExitCode {
    let mut command = Cli::command();
    let error = command.error(kind, message);
    let _ = error.print();
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_historical_subcommand_share_arguments() {
        let root = Cli::try_parse_from(["rust-doctor", "fixture", "--verbose", "--yes"])
            .unwrap()
            .into_inspect_args();
        let alias =
            Cli::try_parse_from(["rust-doctor", "inspect", "fixture", "--verbose", "--yes"])
                .unwrap()
                .into_inspect_args();
        assert_eq!(root.path, alias.path);
        assert_eq!(root.verbose, alias.verbose);
        assert_eq!(root.yes, alias.yes);
    }

    #[test]
    fn inspect_path_is_disambiguated_by_dot_slash() {
        let alias = Cli::try_parse_from(["rust-doctor", "inspect"])
            .unwrap()
            .into_inspect_args();
        let path = Cli::try_parse_from(["rust-doctor", "./inspect"])
            .unwrap()
            .into_inspect_args();
        assert_eq!(alias.path, Path::new("."));
        assert_eq!(path.path, Path::new("./inspect"));
    }

    #[test]
    fn interaction_gate_requires_both_terminals_and_no_quiet_mode() {
        assert!(interactions_allowed(false, false, true, true, None, false));
        for gated in [
            interactions_allowed(true, false, true, true, None, false),
            interactions_allowed(false, true, true, true, None, false),
            interactions_allowed(false, false, false, true, None, false),
            interactions_allowed(false, false, true, false, None, false),
            interactions_allowed(false, false, true, true, Some(OsStr::new("1")), false),
            interactions_allowed(false, false, true, true, None, true),
        ] {
            assert!(!gated);
        }
        assert!(interactions_allowed(
            false,
            false,
            true,
            true,
            Some(OsStr::new("")),
            false
        ));
    }

    #[test]
    fn invalid_scope_combinations_are_rejected_before_execution() {
        assert!(validate_scope(Some(ScopeArgument::Files), None).is_err());
        assert!(validate_scope(Some(ScopeArgument::Baseline), None).is_err());
        assert!(validate_scope(None, Some("HEAD")).is_err());
        assert!(validate_scope(Some(ScopeArgument::Full), Some("HEAD")).is_err());
    }

    #[test]
    fn rescan_command_preserves_policy_overrides() {
        let arguments = Cli::try_parse_from([
            "rust-doctor",
            ".",
            "--rule",
            "clippy::todo=off",
            "--category",
            "maintainability=error",
            "--blocking",
            "warning",
        ])
        .unwrap()
        .into_inspect_args();
        let command = arguments.rescan_command(None).unwrap();
        assert!(command.as_str().contains("--rule 'clippy::todo=off'"));
        assert!(
            command
                .as_str()
                .contains("--category 'maintainability=error'")
        );
        assert!(command.as_str().contains("--blocking warning"));
        assert!(command.as_str().ends_with("--yes"));
    }

    #[test]
    fn redirected_and_dumb_terminals_force_static_eighty_column_output() {
        assert_eq!(terminal_width(false, None, Some(OsStr::new("140"))), 80);
        assert_eq!(
            terminal_width(true, Some(OsStr::new("dumb")), Some(OsStr::new("140"))),
            80
        );
        assert_eq!(
            terminal_width(true, Some(OsStr::new("xterm")), Some(OsStr::new("140"))),
            140
        );
        assert_eq!(
            terminal_width(true, Some(OsStr::new("xterm")), Some(OsStr::new("79"))),
            80
        );
    }

    #[test]
    fn no_color_and_dumb_terminal_disable_ansi_without_mutating_environment() {
        assert!(terminal_color_enabled(true, None, None));
        assert!(!terminal_color_enabled(true, Some(OsStr::new("1")), None));
        assert!(!terminal_color_enabled(
            true,
            None,
            Some(OsStr::new("dumb"))
        ));
        assert!(!terminal_color_enabled(false, None, None));
    }
}
