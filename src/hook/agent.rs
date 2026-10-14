//! The end-of-turn hook of a coding agent: installed into the agent's settings
//! file as one merged entry, and run by the agent when its turn ends.
//!
//! Claude Code runs a `Stop` hook and keeps the turn going when it exits 2,
//! feeding its stderr back to the model; `stop_hook_active` in the hook's
//! input says the turn is already continuing because of one, which is the
//! loop guard (<https://code.claude.com/docs/en/hooks>). Cursor's `stop` hook
//! cannot block: it can only submit a follow-up message, capped at five in a
//! row by default (<https://cursor.com/docs/agent/hooks>). Its input carries
//! two guards of its own: a `status` other than `completed` means the user
//! aborted the turn or it errored, and a `loop_count` of at least
//! [`MAX_FOLLOWUPS`] means this hook already sent the turn back. Either one
//! ends the hook silently, before any scan. An input missing a field, or
//! holding it with another type, scans as if the guard were absent.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::ValueEnum;
use rust_doctor::{
    BlockingLevel, Diagnostic, GateStatus, InspectReport, InspectRequest, ScopeMode, Severity,
    Status, inspect,
};
use serde_json::{Map, Value, json};

use super::{FAILED, WriteArgs};
use crate::workspace_write::{create_new, on_path, symlinked_component};

/// The agents with an end-of-turn hook this tool installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AgentHook {
    Claude,
    Cursor,
}

/// Seconds Claude Code gives the hook. The scan's own deadline sits under it,
/// so a slow build ends as a scan that stopped, not as a hook that was killed.
const HOOK_TIMEOUT_SECONDS: u64 = 600;
const SCAN_DEADLINE_SECONDS: u64 = 540;
const LISTED_FINDINGS: usize = 10;
const STDIN_LIMIT: u64 = 1_048_576;
/// Follow-ups a Cursor turn receives from this hook before it is let end.
const MAX_FOLLOWUPS: u64 = 1;

/// The scan the hook runs, spelled as a reader would rerun it.
const RESCAN: &str =
    "rust-doctor . --scope lines --base HEAD --include-untracked --blocking warning";

impl AgentHook {
    const fn command(self) -> &'static str {
        match self {
            Self::Claude => "rust-doctor hook run claude",
            Self::Cursor => "rust-doctor hook run cursor",
        }
    }

    fn settings(self, shared: bool) -> &'static str {
        match (self, shared) {
            (Self::Claude, false) => ".claude/settings.local.json",
            (Self::Claude, true) => ".claude/settings.json",
            (Self::Cursor, _) => ".cursor/hooks.json",
        }
    }

    /// The hook object the agent runs: the entry itself for Cursor, the one
    /// hook of a matcher group for Claude.
    fn hook(self) -> Value {
        match self {
            Self::Claude => json!({
                "type": "command",
                "command": self.command(),
                "timeout": HOOK_TIMEOUT_SECONDS,
            }),
            Self::Cursor => json!({ "command": self.command() }),
        }
    }

    /// The entry merged into the settings file.
    fn entry(self) -> Value {
        match self {
            Self::Claude => json!({ "hooks": [self.hook()] }),
            Self::Cursor => self.hook(),
        }
    }

    const fn event(self) -> &'static str {
        match self {
            Self::Claude => "Stop",
            Self::Cursor => "stop",
        }
    }

    /// The hook objects of the event's entries that rust-doctor wrote: those
    /// whose command runs `rust-doctor hook run`, the marker of this install.
    fn ours(self, entries: &mut [Value]) -> Vec<&mut Value> {
        let hooks: Vec<&mut Value> = match self {
            Self::Claude => entries
                .iter_mut()
                .filter_map(|entry| entry.get_mut("hooks").and_then(Value::as_array_mut))
                .flatten()
                .collect(),
            Self::Cursor => entries.iter_mut().collect(),
        };
        hooks
            .into_iter()
            .filter(|hook| {
                hook.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| command.starts_with("rust-doctor hook run"))
            })
            .collect()
    }
}

/// Whether a hook object holds every key of the current template, any other
/// key the user added aside.
fn current(hook: &Value, template: &Value) -> bool {
    template.as_object().is_some_and(|keys| {
        keys.iter()
            .all(|(key, value)| hook.get(key) == Some(value))
    })
}

/// `rust-doctor hook install claude|cursor`.
pub(super) fn install(
    agent: AgentHook,
    workspace: &Path,
    shared: bool,
    write: WriteArgs,
) -> ExitCode {
    let relative = agent.settings(shared);
    if let Some(link) = symlinked_component(workspace, Path::new(relative)) {
        eprintln!(
            "rust-doctor: {} is a symlink, and nothing is written through one.",
            link.display()
        );
        return ExitCode::from(FAILED);
    }
    let path = workspace.join(relative);
    let (existed, mut settings) = match read_settings(&path, relative) {
        Ok(read) => read,
        Err(code) => return code,
    };
    let Some(entries) = event_entries(&mut settings, agent) else {
        eprintln!(
            "rust-doctor: {relative} does not hold the shape its agent reads: nothing was written."
        );
        return ExitCode::from(FAILED);
    };
    if let Some(code) = merge(agent, entries, relative, write) {
        return code;
    }
    if write.dry_run {
        return ExitCode::SUCCESS;
    }
    let Ok(serialized) = serde_json::to_string_pretty(&settings) else {
        return ExitCode::from(FAILED);
    };
    let serialized = serialized + "\n";
    let written = if existed {
        fs::write(&path, serialized)
    } else {
        create_new(&path, serialized.as_bytes())
    };
    if let Err(error) = written {
        eprintln!("rust-doctor: {relative} could not be written: {error}.");
        return ExitCode::from(FAILED);
    }
    if !on_path("rust-doctor") {
        eprintln!("Warning: `rust-doctor` is not on PATH, and the hook runs it by name.");
    }
    ExitCode::SUCCESS
}

/// The settings file as it stands, an empty object when absent, and whether
/// it existed; or the exit code of a file that could not be read or parsed.
fn read_settings(path: &Path, relative: &str) -> Result<(bool, Value), ExitCode> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((false, Value::Object(Map::new())));
        }
        Err(error) => {
            eprintln!("rust-doctor: {relative} could not be read: {error}. Nothing was written.");
            return Err(ExitCode::from(FAILED));
        }
    };
    serde_json::from_str(&text).map(|settings| (true, settings)).map_err(|error| {
        eprintln!(
            "Could not parse {relative} at line {}, column {}: nothing was written.",
            error.line(),
            error.column()
        );
        ExitCode::from(FAILED)
    })
}

/// Adds the hook to the event's entries, or refreshes the one an older release
/// wrote under `--update`, printing what changes. Answers the exit code when
/// nothing is to be written.
fn merge(
    agent: AgentHook,
    entries: &mut Vec<Value>,
    relative: &str,
    write: WriteArgs,
) -> Option<ExitCode> {
    let event = agent.event();
    let template = agent.hook();
    let mut ours = agent.ours(entries);
    if ours.iter().any(|hook| current(hook, &template)) {
        println!("The rust-doctor {event} hook is already in {relative}.");
        return Some(ExitCode::SUCCESS);
    }
    if let Some(hook) = ours.first_mut() {
        if !write.update {
            println!(
                "The rust-doctor {event} hook in {relative} is outdated: pass --update to rewrite it."
            );
            return Some(ExitCode::FAILURE);
        }
        let previous = hook.to_string();
        if let (Some(hook), Some(keys)) = (hook.as_object_mut(), template.as_object()) {
            hook.extend(keys.clone());
        }
        let verb = if write.dry_run { "Would update" } else { "Updated" };
        println!("{verb} the {event} hook in {relative}: {previous} becomes {hook}");
        return None;
    }
    let entry = agent.entry();
    let verb = if write.dry_run { "Would add" } else { "Added" };
    println!("{verb} a {event} hook to {relative}: {entry}");
    entries.push(entry);
    None
}

/// The array of entries the agent reads for its end-of-turn event, created
/// when absent, or `None` when a key on the way holds something else.
fn event_entries(settings: &mut Value, agent: AgentHook) -> Option<&mut Vec<Value>> {
    let root = settings.as_object_mut()?;
    if agent == AgentHook::Cursor {
        root.entry("version").or_insert(json!(1));
    }
    root.entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()?
        .entry(agent.event())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
}

/// `rust-doctor hook run claude|cursor`: rescans what the turn changed.
pub(super) fn run(agent: AgentHook) -> ExitCode {
    let mut input = String::new();
    // The input is advisory: a hook whose input cannot be read still scans,
    // and a reader at a terminal is not made to type one.
    if !io::stdin().is_terminal() {
        let _ = io::stdin().take(STDIN_LIMIT).read_to_string(&mut input);
    }
    let input: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    if guarded(agent, &input) {
        return ExitCode::SUCCESS;
    }
    let report = inspect(
        InspectRequest::new(workspace(agent))
            .with_scope(ScopeMode::Lines, Some("HEAD".to_owned()))
            .with_untracked()
            .with_blocking(BlockingLevel::Warning)
            .with_max_duration(Duration::from_secs(SCAN_DEADLINE_SECONDS)),
    );
    let verdict = verdict(&report);
    match (agent, verdict) {
        (_, Verdict::Pass) => ExitCode::SUCCESS,
        // A broken toolchain must never trap the agent in its turn.
        (_, Verdict::Unscanned(reason)) => {
            eprintln!("rust-doctor could not scan ({reason}); not blocking this turn.");
            ExitCode::SUCCESS
        }
        (AgentHook::Claude, Verdict::Blocked(message)) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
        (AgentHook::Cursor, Verdict::Blocked(message)) => {
            println!("{}", json!({ "followup_message": message }));
            ExitCode::SUCCESS
        }
    }
}

/// Whether the agent's input says this turn must end without a scan: Claude
/// is already continuing because of this hook, or Cursor's turn did not
/// complete or was already sent back [`MAX_FOLLOWUPS`] times.
fn guarded(agent: AgentHook, input: &Value) -> bool {
    match agent {
        AgentHook::Claude => input.get("stop_hook_active") == Some(&Value::Bool(true)),
        AgentHook::Cursor => {
            input
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| status != "completed")
                || input
                    .get("loop_count")
                    .and_then(Value::as_u64)
                    .is_some_and(|count| count >= MAX_FOLLOWUPS)
        }
    }
}

/// The workspace the agent's turn ran in: Claude Code names its project root
/// in `CLAUDE_PROJECT_DIR`, and Cursor starts project hooks in the project
/// root.
fn workspace(agent: AgentHook) -> PathBuf {
    match agent {
        AgentHook::Claude => env::var_os("CLAUDE_PROJECT_DIR")
            .filter(|directory| !directory.is_empty())
            .map_or_else(|| PathBuf::from("."), PathBuf::from),
        AgentHook::Cursor => PathBuf::from("."),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Pass,
    Unscanned(String),
    Blocked(String),
}

fn verdict(report: &InspectReport) -> Verdict {
    if report.gate.status == GateStatus::Failed {
        return Verdict::Blocked(blocked_message(report));
    }
    if report.status == Status::Complete {
        return Verdict::Pass;
    }
    Verdict::Unscanned(report.errors.first().map_or_else(
        || "the scan did not complete".to_owned(),
        |error| format!("{}: {}", error.stage, error.code),
    ))
}

fn blocks(diagnostic: &Diagnostic) -> bool {
    diagnostic.context.is_none()
        && diagnostic.unscored.is_none()
        && matches!(diagnostic.severity, Severity::Warning | Severity::Error)
}

fn blocked_message(report: &InspectReport) -> String {
    let blocking: Vec<&Diagnostic> = report.diagnostics.iter().filter(|d| blocks(d)).collect();
    let mut message = format!(
        "rust-doctor found {} finding{} in the lines this turn changed:\n",
        blocking.len(),
        if blocking.len() == 1 { "" } else { "s" }
    );
    for diagnostic in blocking.iter().take(LISTED_FINDINGS) {
        let location = match (&diagnostic.path, &diagnostic.span) {
            (Some(path), Some(span)) => format!("{path}:{}", span.line_start),
            (Some(path), None) => path.clone(),
            _ => "workspace".to_owned(),
        };
        message.push_str(&format!(
            "- {} {location} {}\n",
            diagnostic.code.as_deref().unwrap_or("rust-doctor"),
            diagnostic.message.lines().next().unwrap_or_default()
        ));
    }
    if blocking.len() > LISTED_FINDINGS {
        message.push_str(&format!(
            "- and {} more\n",
            blocking.len() - LISTED_FINDINGS
        ));
    }
    message.push_str(&format!("Fix them, then rescan: {RESCAN}"));
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command a hook runs or prints parses as the CLI stands, so a
    /// renamed flag fails here rather than in someone's commit or turn.
    #[test]
    fn every_hook_command_parses_as_the_cli_stands() {
        for command in [
            crate::hook::pre_commit_command("."),
            AgentHook::Claude.command().to_owned(),
            AgentHook::Cursor.command().to_owned(),
            RESCAN.to_owned(),
        ] {
            let parsed = <crate::Cli as clap::Parser>::try_parse_from(command.split_whitespace());
            assert!(parsed.is_ok(), "{command}: {parsed:?}");
        }
    }

    #[test]
    fn each_agent_reads_only_its_own_guards() {
        for (input, guarded_turn) in [
            (json!({"status": "completed", "loop_count": 0}), false),
            (json!({"status": "aborted", "loop_count": 0}), true),
            (json!({"status": "error"}), true),
            (json!({"status": "completed", "loop_count": 1}), true),
            (json!({"status": 3, "loop_count": "1"}), false),
            (json!({"loop_count": -1}), false),
            (json!({"stop_hook_active": true}), false),
            (Value::Null, false),
        ] {
            assert_eq!(guarded(AgentHook::Cursor, &input), guarded_turn, "{input}");
        }
        assert!(guarded(AgentHook::Claude, &json!({"stop_hook_active": true})));
        assert!(!guarded(AgentHook::Claude, &json!({"status": "aborted", "loop_count": 3})));
    }

    #[test]
    fn a_merge_keeps_every_other_key_and_adds_one_entry() {
        let mut settings = json!({
            "permissions": { "allow": ["Bash(ls)"] },
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "other" }] }] }
        });
        let entries = event_entries(&mut settings, AgentHook::Claude).unwrap();
        assert!(AgentHook::Claude.ours(entries).is_empty());
        entries.push(AgentHook::Claude.entry());
        let template = AgentHook::Claude.hook();
        assert!(current(AgentHook::Claude.ours(entries)[0], &template));
        assert!(!current(&json!({"command": "rust-doctor hook run claude"}), &template));
        assert_eq!(settings["permissions"]["allow"][0], "Bash(ls)");
        assert_eq!(settings["hooks"]["Stop"][0]["hooks"][0]["command"], "other");
        assert_eq!(settings["hooks"]["Stop"][1]["hooks"][0]["timeout"], 600);

        let mut cursor = json!({});
        event_entries(&mut cursor, AgentHook::Cursor)
            .unwrap()
            .push(AgentHook::Cursor.entry());
        assert_eq!(
            cursor,
            json!({ "version": 1, "hooks": { "stop": [{ "command": "rust-doctor hook run cursor" }] } })
        );

        for wrong in [
            json!([]),
            json!({ "hooks": [] }),
            json!({ "hooks": { "Stop": {} } }),
        ] {
            let mut wrong = wrong;
            assert!(event_entries(&mut wrong, AgentHook::Claude).is_none());
        }
    }
}
