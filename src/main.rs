#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod handoff;

use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, IsTerminal};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Instant;

use clap::builder::TypedValueParser;
use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use console::{Key, Term};
use handoff::{
    HandoffError, RescanCommand, available_agents, build_prompt, copy_to_clipboard, launch_agent,
};
use rust_doctor::presentation::ReportPresentation;
use rust_doctor::render::{TerminalOptions, render_json, render_terminal_with_presentation};
use rust_doctor::{
    BlockingLevel, CategoryOverride, InspectRequest, InspectionSession, RuleOverride, ScopeMode,
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
            None => self.inspect,
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
    run_inspect(Cli::parse().into_inspect_args())
}

fn run_inspect(arguments: InspectArgs) -> ExitCode {
    let scope_was_explicit = arguments.scope.is_some();
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
    let request = arguments.request(scoped_base.as_ref());
    let mut session = InspectionSession::prepare(request);
    let scoped_base = if scoped_base.is_none() && !scope_was_explicit && interactive {
        match session
            .as_mut()
            .ok()
            .and_then(InspectionSession::preview_uncommitted_rust_files)
        {
            Some(count) if count > 0 => match choose_scope(count) {
                Ok(Some(ScopeArgument::Files)) => {
                    if let Ok(session) = &mut session {
                        session.select_uncommitted_files();
                    }
                    Some((ScopeArgument::Files, "HEAD".to_owned()))
                }
                Ok(Some(ScopeArgument::Full)) => None,
                Ok(Some(ScopeArgument::Baseline)) => None,
                Ok(None) | Err(_) => {
                    eprintln!("Scan cancelled.");
                    return ExitCode::from(130);
                }
            },
            _ => None,
        }
    } else {
        scoped_base
    };

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
    let stdout = io::stdout();
    let render_result = if arguments.json {
        render_json(&report, stdout.lock())
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
                color: terminal_color_enabled(
                    stdout_is_terminal,
                    env::var_os("NO_COLOR").as_deref(),
                    term.as_deref(),
                ),
                animate: terminal_animation_enabled(
                    stdout_is_terminal,
                    env::var_os("NO_COLOR").as_deref(),
                    term.as_deref(),
                    env::var_os("CI").as_deref(),
                ),
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

    if interactive
        && !arguments.json
        && report.audit.source_files > 0
        && !report.diagnostics.is_empty()
        && !presentation.groups.is_empty()
    {
        match rescan_command
            .and_then(|command| build_prompt(&report, &presentation, &command))
            .and_then(|payload| run_handoff_menu(&payload, &workspace_root))
        {
            Ok(()) => {}
            Err(error) => {
                bounded_stderr(&error.to_string());
                if scan_exit == 0 {
                    return ExitCode::from(2);
                }
            }
        }
    }
    ExitCode::from(scan_exit)
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

fn choose_scope(changed_files: usize) -> io::Result<Option<ScopeArgument>> {
    let items = [
        "Full codebase".to_owned(),
        format!("Uncommitted changes ({changed_files})"),
    ];
    select_item("Choose what to scan", &items)
        .map(|selection| selection.map(|index| [ScopeArgument::Full, ScopeArgument::Files][index]))
}

fn run_handoff_menu(
    payload: &handoff::HandoffPayload,
    workspace_root: &Path,
) -> Result<(), HandoffError> {
    let agents = available_agents();
    loop {
        let mut items: Vec<_> = agents
            .iter()
            .map(|agent| agent.label().to_owned())
            .collect();
        items.push("Copy prompt to clipboard".to_owned());
        items.push("Skip".to_owned());
        let selection = match select_item("What would you like to do next?", &items) {
            Ok(selection) => selection,
            Err(_) => return Ok(()),
        };
        let Some(selection) = selection else {
            return Ok(());
        };
        if selection < agents.len() {
            return launch_agent(&agents[selection], payload, workspace_root);
        }
        if selection == agents.len() {
            match copy_to_clipboard(payload) {
                Ok(()) => {
                    eprintln!("Prompt copied to clipboard.");
                    return Ok(());
                }
                Err(error) => {
                    bounded_stderr(&error.to_string());
                    continue;
                }
            }
        }
        return Ok(());
    }
}

fn select_item(prompt: &str, items: &[String]) -> io::Result<Option<usize>> {
    let terminal = Term::stdout();
    terminal.hide_cursor()?;
    let result = select_item_on(&terminal, prompt, items);
    let _ = terminal.show_cursor();
    let _ = terminal.flush();
    result
}

fn select_item_on(terminal: &Term, prompt: &str, items: &[String]) -> io::Result<Option<usize>> {
    let mut selected = 0usize;
    loop {
        terminal.write_line(&format!("{prompt}:"))?;
        for (index, item) in items.iter().enumerate() {
            let marker = if index == selected { ">" } else { " " };
            terminal.write_line(&format!("{marker} {item}"))?;
        }
        terminal.flush()?;

        match terminal.read_key_raw()? {
            Key::ArrowDown | Key::Tab | Key::Char('j') => {
                selected = (selected + 1) % items.len();
            }
            Key::ArrowUp | Key::BackTab | Key::Char('k') => {
                selected = (selected + items.len() - 1) % items.len();
            }
            Key::Enter | Key::Char(' ') => {
                terminal.clear_last_lines(items.len() + 1)?;
                terminal.write_line(&format!("{prompt}: {}", items[selected]))?;
                return Ok(Some(selected));
            }
            Key::Escape | Key::CtrlC | Key::Char('q') => {
                terminal.clear_last_lines(items.len() + 1)?;
                return Ok(None);
            }
            _ => {}
        }
        terminal.clear_last_lines(items.len() + 1)?;
    }
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

/// L'animation du score exige tout ce qu'exige la couleur, et en plus une
/// session hors CI: un runner capture la sortie, où les frames intermédiaires
/// ne seraient que du bruit.
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
