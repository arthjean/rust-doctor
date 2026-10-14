#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod ci;
mod handoff;
mod hook;
mod progress_line;
mod report_command;
mod rules_command;
mod scope_argument;
mod setup;
mod skill;
mod terminal_profile;
#[cfg(test)]
#[path = "test_scratch.rs"]
mod test_scratch;
mod tui;
mod why_command;
mod workspace_write;

use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::{Duration, Instant};

use clap::builder::TypedValueParser;
use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand};
use progress_line::ProgressLine;
use terminal_profile::TerminalProfile;
use scope_argument::{ScopeArgument, ScopeSelection, validate_scope};
use handoff::{
    HandoffError, RescanCommand, RescanScope, available_agents, build_prompt, copy_or_print,
    launch_or_print,
};
use rust_doctor::presentation::ReportPresentation;
use rust_doctor::render::{TerminalOptions, render_json, render_terminal_with_presentation};
use rust_doctor::{
    AdvisoryDatabase, BlockingLevel, CategoryOverride, InspectReport, InspectRequest, InspectionSession,
    RuleOverride, Status,
};

const TRUST_WARNING: &str = "Cargo may execute build.rs files and procedural macros. Inspect trusted local repositories only.";

const EXIT_STATUS: &str = "Exit status:
  0       the scan completed and the gate passed
  1       the gate failed, or the scan is incomplete
  2       the scan failed, ran past --max-duration, or the arguments are invalid
  3-125   --warning-exit-code: the gate passed with a finding --blocking warning would block
  130     a SIGINT or SIGTERM stopped the scan
The precedence is 2, 1, the warning code, then 0.";

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
#[command(name = "rust-doctor", version, after_help = EXIT_STATUS)]
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
            Some(
                CliCommand::Rules(_)
                | CliCommand::Why(_)
                | CliCommand::Setup(_)
                | CliCommand::Skill(_)
                | CliCommand::Hook(_)
                | CliCommand::Ci(_)
                | CliCommand::Report(_),
            )
            | None => {
                self.inspect
            }
        }
    }
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    #[command(about = "Inspect a trusted local Cargo workspace")]
    #[command(long_about = format!(
        "Inspect a trusted local Cargo workspace.\n\n{TRUST_WARNING}\n\nUse `rust-doctor ./inspect` when the path itself is named `inspect`."
    ), after_help = EXIT_STATUS)]
    Inspect(InspectArgs),
    #[command(about = "Print the rule catalog, or explain one rule")]
    Rules(rules_command::RulesArgs),
    #[command(about = "Explain why a line is reported, and how to silence it")]
    Why(why_command::WhyArgs),
    #[command(about = "Install the agent skill")]
    Skill(SkillArgs),
    #[command(about = "Install the skill, the git hook and the end-of-turn hooks for the agents detected")]
    Setup(setup::SetupArgs),
    #[command(about = "Install or run the hooks that rescan a commit or an agent's turn")]
    Hook(hook::HookArgs),
    #[command(about = "Write the GitHub Actions workflow that scans every pull request")]
    Ci(ci::CiArgs),
    #[command(about = "Render a report a scan already saved, without scanning")]
    Report(report_command::ReportArgs),
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
    #[command(about = "Write the skill where an agent reads it, never over another one")]
    Install {
        #[arg(default_value = ".", value_name = "PATH")]
        path: PathBuf,
        /// The agent to install for: its project skill directory is
        /// .claude/skills, .agents/skills or .cursor/skills.
        #[arg(long, value_enum, default_value = "claude")]
        agent: skill::AgentSelection,
        /// Rewrite an installed copy of this skill with the one this binary
        /// ships.
        #[arg(long)]
        update: bool,
        /// Print what would be written, and write nothing.
        #[arg(long)]
        dry_run: bool,
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
    /// What to judge: the whole workspace, the files or the lines changed
    /// since a base, those lines plus the functions holding them for a rule
    /// whose cause can sit above its site (contextual), or what the change
    /// introduced against a scan of the base.
    #[arg(long, value_enum)]
    scope: Option<ScopeArgument>,
    /// The ref a files, lines or baseline scope compares against. Without it,
    /// the default branch the repository answers for (`origin/HEAD`, then
    /// `origin/main`, `origin/master`, `main`, `master`), or `HEAD` when that
    /// branch is checked out or the scan is `--staged`.
    #[arg(long, value_name = "REF")]
    base: Option<String>,
    /// Judge the index rather than the working tree: what `git commit` would
    /// record. Takes a files, lines or baseline scope.
    #[arg(long)]
    staged: bool,
    /// Add untracked files to a files or lines scope, as changed whole.
    #[arg(long)]
    include_untracked: bool,
    /// Stop the scan after SECONDS of wall-clock time, killing every process
    /// it started. Without it the scan has no deadline.
    #[arg(
        long,
        value_name = "SECONDS",
        value_parser = clap::value_parser!(u64).range(1..=86_400)
    )]
    max_duration: Option<u64>,
    /// Scan and score this workspace member only. Repeat it to select several.
    #[arg(long, value_name = "NAME")]
    package: Vec<String>,
    /// RustSec database to read, else the one cargo audit or cargo deny left; never fetched.
    #[arg(long, value_name = "PATH")]
    advisory_db: Option<PathBuf>,
    /// Publish the milliseconds of each phase as `timings` in --json.
    #[arg(long)]
    timings: bool,
    /// Exit with CODE when the gate passed but a finding `--blocking warning`
    /// would block; ignored under `--blocking none`.
    #[arg(
        long,
        value_name = "CODE",
        value_parser = clap::value_parser!(u8).range(3..=125)
    )]
    warning_exit_code: Option<u8>,
}

impl InspectArgs {
    fn request(&self, selection: Option<&ScopeSelection>) -> InspectRequest {
        let mut request = InspectRequest::new(&self.path);
        if let Some(selection) = selection {
            request = request.with_scope(selection.mode, selection.base.clone());
            if selection.staged {
                request = request.with_staged();
            }
            if selection.include_untracked {
                request = request.with_untracked();
            }
        }
        if let Some(blocking) = self.blocking {
            request = request.with_blocking(blocking);
        }
        if let Some(seconds) = self.max_duration {
            request = request.with_max_duration(Duration::from_secs(seconds));
        }
        if self.timings || (self.verbose && !self.json) {
            request = request.with_timings();
        }
        for rule_override in &self.rule {
            request = request.with_rule_override(rule_override.clone());
        }
        for category_override in &self.category {
            request = request.with_category_override(category_override.clone());
        }
        for package in &self.package {
            request = request.with_package(package.clone());
        }
        AdvisoryDatabase::locate(self.advisory_db.clone())
            .into_iter()
            .fold(request, InspectRequest::with_advisory_db)
    }

    fn rescan_command(
        &self,
        selection: Option<&ScopeSelection>,
    ) -> Result<RescanCommand, HandoffError> {
        RescanCommand::for_inspection(
            self.verbose,
            self.blocking,
            &self.rule,
            &self.category,
            &self.package,
            selection.map(|selection| RescanScope {
                mode: selection.mode,
                base: selection.base.as_deref(),
                staged: selection.staged,
                include_untracked: selection.include_untracked,
            }),
        )
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(CliCommand::Rules(arguments)) = &cli.command {
        return rules_command::run(arguments);
    }
    if let Some(CliCommand::Why(arguments)) = &cli.command {
        return why_command::run(arguments);
    }
    if let Some(CliCommand::Skill(arguments)) = &cli.command {
        return run_skill(arguments);
    }
    if let Some(CliCommand::Setup(arguments)) = &cli.command {
        return setup::run(arguments);
    }
    if let Some(CliCommand::Hook(arguments)) = &cli.command {
        return hook::run(arguments);
    }
    if let Some(CliCommand::Ci(arguments)) = &cli.command {
        return ci::run(arguments);
    }
    if let Some(CliCommand::Report(arguments)) = &cli.command {
        return report_command::run(arguments);
    }
    run_inspect(cli.into_inspect_args())
}

fn run_skill(arguments: &SkillArgs) -> ExitCode {
    let SkillCommand::Install {
        path,
        agent,
        update,
        dry_run,
    } = &arguments.command;
    let options = skill::InstallOptions {
        update: *update,
        dry_run: *dry_run,
    };
    ExitCode::from(skill::install_and_print(path, *agent, options))
}

fn run_inspect(arguments: InspectArgs) -> ExitCode {
    let selection = match validate_scope(&arguments) {
        Ok(selection) => selection,
        Err((kind, message)) => return clap_error(kind, message),
    };
    let profile = TerminalProfile::detect(arguments.json, arguments.yes);
    // The phases replace the static line this used to print. A JSON reader
    // gets none of them, only where discovery found the workspace.
    let progress = ProgressLine::new(profile.rewrite_progress);
    // The scope is what the invocation asked for, nothing else. A run that
    // names no scope scans the whole workspace rather than opening a menu:
    // narrowing to what changed is what `--scope files` and `--scope baseline`
    // are for, and a question asked before the scan is a question asked before
    // the reader has seen a single finding.
    let request = arguments
        .request(selection.as_ref())
        .with_progress(progress.sink(!arguments.json));
    rust_doctor::handle_interrupts();
    let session = InspectionSession::prepare(request);
    let rescan_command = arguments.rescan_command(selection.as_ref());

    let started = Instant::now();
    let (report, workspace_root) = match session {
        Ok(session) => {
            let workspace_root = session.workspace_root().to_path_buf();
            (session.inspect(), workspace_root)
        }
        Err(report) => (*report, PathBuf::from(".")),
    };
    progress.clear();
    let elapsed = started.elapsed();
    let scan_exit = if rust_doctor::interrupted() {
        rust_doctor::INTERRUPTED_EXIT_STATUS
    } else {
        report.exit_code_with_warning(arguments.warning_exit_code)
    };
    let presentation = ReportPresentation::derive_terminal(&report);
    // `--verbose` means print everything, so it keeps the linear report; a
    // failed scan does too, because its errors are the only thing worth
    // reading and the interactive report has no room to carry them.
    let interactive_report =
        profile.interactive && !arguments.verbose && report.status != Status::Failed;
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
                width: profile.width,
                color: profile.color,
                animate: profile.animate,
            },
        )
    };
    // An agent told its skill is missing or stale never follows instructions
    // for flags this binary no longer has. Nothing is written, and a JSON
    // reader's stderr stays clean.
    if !arguments.json
        && let Some(notice) = skill::running_agent(|name| env::var_os(name))
            .and_then(|agent| skill::stale_or_missing(&workspace_root, agent))
    {
        eprintln!("{notice}");
    }
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
            profile.color,
            profile.animate,
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
            let mut payload = build_prompt(report, presentation, &rescan_command?)?;
            if let Some(line) = skill::install_line(workspace_root, agent.skill_agent()) {
                payload = payload.preceded_by(&line)?;
            }
            launch_or_print(agent, &payload, workspace_root);
            Ok(())
        }
        tui::Outcome::CopyPrompt => {
            copy_or_print(&build_prompt(report, presentation, &rescan_command?)?);
            Ok(())
        }
    }
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
    use rust_doctor::ScopeMode;

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

    fn selection(arguments: &[&str]) -> Result<Option<ScopeSelection>, (ErrorKind, &'static str)> {
        let mut command = vec!["rust-doctor", "."];
        command.extend_from_slice(arguments);
        let arguments = Cli::try_parse_from(command)
            .map_err(|_| (ErrorKind::InvalidValue, "the test arguments did not parse"))?;
        validate_scope(&arguments.into_inspect_args())
    }

    #[test]
    fn invalid_scope_combinations_are_rejected_before_execution() {
        for refused in [
            &["--base", "HEAD"][..],
            &["--scope", "full", "--base", "HEAD"],
            &["--staged"],
            &["--scope", "full", "--staged"],
            &["--include-untracked"],
            &["--scope", "baseline", "--include-untracked"],
            &["--scope", "lines", "--staged", "--include-untracked"],
        ] {
            assert!(selection(refused).is_err(), "{refused:?}");
        }
        let (_, message) = selection(&["--scope", "full", "--staged"]).unwrap_err();
        assert!(message.contains("--staged") && message.contains("full"), "{message}");
    }

    #[test]
    fn a_changed_scope_needs_no_base() {
        for (mode, scope) in [
            ("files", ScopeMode::Files),
            ("lines", ScopeMode::Lines),
            ("contextual", ScopeMode::Contextual),
            ("baseline", ScopeMode::Baseline),
        ] {
            assert_eq!(
                selection(&["--scope", mode]).unwrap(),
                Some(ScopeSelection {
                    mode: scope,
                    base: None,
                    staged: false,
                    include_untracked: false,
                })
            );
            assert!(selection(&["--scope", mode, "--staged"]).unwrap().unwrap().staged);
        }
        assert!(
            selection(&["--scope", "lines", "--include-untracked", "--base", "HEAD"])
                .unwrap()
                .unwrap()
                .include_untracked
        );
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
}
