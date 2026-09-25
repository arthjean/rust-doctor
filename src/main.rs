#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod handoff;
mod hook;
mod progress_line;
mod skill;
#[cfg(test)]
#[path = "test_scratch.rs"]
mod test_scratch;
mod tui;
mod workspace_write;

use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::{Duration, Instant};

use clap::builder::TypedValueParser;
use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use progress_line::ProgressLine;
use handoff::{
    HandoffError, RescanCommand, RescanScope, available_agents, build_prompt, copy_to_clipboard,
    launch_agent,
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
            Some(CliCommand::Rules(_) | CliCommand::Skill(_) | CliCommand::Hook(_)) | None => {
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
    ))]
    Inspect(InspectArgs),
    #[command(about = "Print the rule catalog")]
    Rules(RulesArgs),
    #[command(about = "Install the agent skill")]
    Skill(SkillArgs),
    #[command(about = "Install or run the hooks that rescan a commit or an agent's turn")]
    Hook(HookArgs),
}

/// The hooks that turn a rescan into a habit. Only `install` writes, and only
/// what it prints.
#[derive(Debug, Clone, Args)]
struct HookArgs {
    #[command(subcommand)]
    command: HookCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum HookCommand {
    #[command(about = "Install a git pre-commit hook or an agent's end-of-turn hook")]
    Install {
        #[command(subcommand)]
        target: HookTarget,
    },
    #[command(about = "Run an agent's end-of-turn hook: rescan the lines the turn changed")]
    Run {
        #[arg(value_enum)]
        agent: hook::AgentHook,
    },
}

#[derive(Debug, Clone, Subcommand)]
enum HookTarget {
    #[command(
        about = "Write a pre-commit hook running `rust-doctor . --yes --staged --scope lines`"
    )]
    Git {
        #[arg(default_value = ".", value_name = "PATH")]
        path: PathBuf,
        /// Print the target and the content, and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    #[command(about = "Add a Stop hook to .claude/settings.local.json")]
    Claude {
        #[arg(default_value = ".", value_name = "PATH")]
        path: PathBuf,
        /// Write .claude/settings.json, the file the repository shares.
        #[arg(long)]
        shared: bool,
        /// Print the entry, and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    #[command(about = "Add a stop hook to .cursor/hooks.json")]
    Cursor {
        #[arg(default_value = ".", value_name = "PATH")]
        path: PathBuf,
        /// Print the entry, and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
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
    /// What to judge: the whole workspace, the files or the lines changed
    /// since a base, or what the change introduced against a scan of the base.
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
        selection: Option<&ScopeSelection>,
    ) -> Result<RescanCommand, HandoffError> {
        RescanCommand::for_inspection(
            self.verbose,
            self.blocking,
            &self.rule,
            &self.category,
            selection.map(|selection| RescanScope {
                mode: selection.mode,
                base: selection.base.as_deref(),
                staged: selection.staged,
                include_untracked: selection.include_untracked,
            }),
        )
    }
}

/// A changed-work scope the invocation asked for, checked for combinations
/// that mean nothing before any process starts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopeSelection {
    mode: ScopeMode,
    base: Option<String>,
    staged: bool,
    include_untracked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ScopeArgument {
    Full,
    Files,
    Lines,
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
    if let Some(CliCommand::Hook(arguments)) = &cli.command {
        return run_hook(arguments);
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
    match skill::install(path, *agent, options) {
        Ok(installed) => {
            for copy in installed {
                if let (Some(replaced), Some(directory)) = (
                    &copy.replaced,
                    copy.written.first().and_then(|path| path.parent()),
                ) {
                    println!(
                        "{} {} with {} in {}.",
                        if *dry_run { "Would replace" } else { "Replaced" },
                        replaced.as_deref().map_or_else(
                            || "a copy that recorded no version".to_owned(),
                            |version| format!("rust-doctor {version}")
                        ),
                        env!("CARGO_PKG_VERSION"),
                        directory.display()
                    );
                }
                for document in copy.written {
                    if *dry_run {
                        println!("Would write {}", document.display());
                    } else {
                        println!("{}", document.display());
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rust-doctor: the skill was not installed, {error}.");
            ExitCode::FAILURE
        }
    }
}

fn run_hook(arguments: &HookArgs) -> ExitCode {
    match &arguments.command {
        HookCommand::Run { agent } => hook::run_agent_hook(*agent),
        HookCommand::Install { target } => match target {
            HookTarget::Git { path, dry_run } => hook::install_git(path, *dry_run),
            HookTarget::Claude {
                path,
                shared,
                dry_run,
            } => hook::install_agent(hook::AgentHook::Claude, path, *shared, *dry_run),
            HookTarget::Cursor { path, dry_run } => {
                hook::install_agent(hook::AgentHook::Cursor, path, false, *dry_run)
            }
        },
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
    let selection = match validate_scope(&arguments) {
        Ok(selection) => selection,
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
        |name| env::var_os(name),
        term.as_deref() == Some(OsStr::new("dumb")),
    );
    // The phases replace the static line this used to print. A JSON reader
    // gets no progress at all, as it got no line before.
    let progress = ProgressLine::new(
        io::stderr().is_terminal() && term.as_deref() != Some(OsStr::new("dumb")),
    );
    // The scope is what the invocation asked for, nothing else. A run that
    // names no scope scans the whole workspace rather than opening a menu:
    // narrowing to what changed is what `--scope files` and `--scope baseline`
    // are for, and a question asked before the scan is a question asked before
    // the reader has seen a single finding.
    let mut request = arguments.request(selection.as_ref());
    if !arguments.json {
        request = request.with_progress(progress.sink());
    }
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
    arguments: &InspectArgs,
) -> Result<Option<ScopeSelection>, (ErrorKind, &'static str)> {
    let mode = match arguments.scope {
        None | Some(ScopeArgument::Full) => {
            return match (
                arguments.base.is_some(),
                arguments.staged,
                arguments.include_untracked,
            ) {
                (false, false, false) => Ok(None),
                (true, _, _) => Err((
                    ErrorKind::ArgumentConflict,
                    "--base <REF> requires --scope files, --scope lines or --scope baseline",
                )),
                (_, true, _) => Err((
                    ErrorKind::ArgumentConflict,
                    "--staged cannot judge the full codebase: pass --scope files, lines or baseline",
                )),
                (_, _, true) => Err((
                    ErrorKind::ArgumentConflict,
                    "--include-untracked requires --scope files or --scope lines",
                )),
            };
        }
        Some(ScopeArgument::Files) => ScopeMode::Files,
        Some(ScopeArgument::Lines) => ScopeMode::Lines,
        Some(ScopeArgument::Baseline) => ScopeMode::Baseline,
    };
    if arguments.include_untracked && mode == ScopeMode::Baseline {
        return Err((
            ErrorKind::ArgumentConflict,
            "--include-untracked requires --scope files or --scope lines",
        ));
    }
    if arguments.include_untracked && arguments.staged {
        return Err((
            ErrorKind::ArgumentConflict,
            "--include-untracked cannot be combined with --staged: the index holds no untracked file",
        ));
    }
    Ok(Some(ScopeSelection {
        mode,
        base: arguments.base.clone(),
        staged: arguments.staged,
        include_untracked: arguments.include_untracked,
    }))
}

/// The variables that say nobody can drive an interactive report, whatever the
/// terminals look like: an agent's shell, a git hook, or a CI provider. An
/// agent may run the tool under a PTY, so two terminals prove nothing about who
/// reads them. `skills/rust-doctor/SKILL.md` names this list.
const NON_INTERACTIVE_MARKERS: [&str; 12] = [
    // Coding agents.
    "CLAUDECODE",
    "CODEX_SANDBOX",
    "CURSOR_AGENT",
    // Git hooks.
    "GIT_DIR",
    "GIT_INDEX_FILE",
    // CI providers.
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "BUILDKITE",
    "CIRCLECI",
    "TF_BUILD",
    "JENKINS_URL",
    "TEAMCITY_VERSION",
];

/// Whether the interactive report may open, reading the environment through
/// `variable` so the decision is tested without mutating the process's own.
///
/// A marker set to the empty string counts as unset. `CI` is read by value
/// rather than presence, since `CI=false` is how a developer says the shell is
/// not one.
fn interactions_allowed(
    json: bool,
    yes: bool,
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
    variable: impl Fn(&str) -> Option<std::ffi::OsString>,
    terminal_is_dumb: bool,
) -> bool {
    stdin_is_terminal
        && stdout_is_terminal
        && !json
        && !yes
        && !terminal_is_dumb
        && variable("CI").as_deref().is_none_or(ci_is_off)
        && NON_INTERACTIVE_MARKERS
            .iter()
            .all(|marker| variable(marker).as_deref().is_none_or(OsStr::is_empty))
}

/// `CI` left empty, or set to `false` or `0` in any case, is not a CI run.
fn ci_is_off(value: &OsStr) -> bool {
    value
        .to_str()
        .is_some_and(|value| value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false"))
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

    fn environment(
        pairs: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| std::ffi::OsString::from(value))
        }
    }

    #[test]
    fn interaction_gate_requires_both_terminals_and_no_quiet_mode() {
        let clean = || environment(&[]);
        assert!(interactions_allowed(false, false, true, true, clean(), false));
        for gated in [
            interactions_allowed(true, false, true, true, clean(), false),
            interactions_allowed(false, true, true, true, clean(), false),
            interactions_allowed(false, false, false, true, clean(), false),
            interactions_allowed(false, false, true, false, clean(), false),
            interactions_allowed(false, false, true, true, clean(), true),
        ] {
            assert!(!gated);
        }
    }

    #[test]
    fn every_agent_hook_and_ci_marker_closes_the_interactive_report() {
        for marker in NON_INTERACTIVE_MARKERS {
            let set = move |name: &str| (name == marker).then(|| "1".into());
            assert!(
                !interactions_allowed(false, false, true, true, set, false),
                "{marker}=1 left the interactive report open"
            );
            // A variable set to the empty string counts as unset.
            let empty = move |name: &str| (name == marker).then(std::ffi::OsString::new);
            assert!(
                interactions_allowed(false, false, true, true, empty, false),
                "{marker}= closed the interactive report"
            );
        }
    }

    #[test]
    fn ci_set_to_false_zero_or_nothing_is_not_ci() {
        for off in ["false", "FALSE", "False", "0", ""] {
            let ci = move |name: &str| (name == "CI").then(|| off.into());
            assert!(interactions_allowed(false, false, true, true, ci, false), "CI={off:?}");
        }
        for on in ["1", "true", "yes", "off", "no"] {
            let ci = move |name: &str| (name == "CI").then(|| on.into());
            assert!(!interactions_allowed(false, false, true, true, ci, false), "CI={on:?}");
        }
    }

    #[test]
    fn the_skill_names_every_non_interactive_marker() {
        let skill = include_str!("../skills/rust-doctor/SKILL.md");
        assert!(skill.contains("NON_INTERACTIVE_MARKERS"));
        for marker in NON_INTERACTIVE_MARKERS {
            assert!(skill.contains(&format!("`{marker}`")), "SKILL.md omits `{marker}`");
        }
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
