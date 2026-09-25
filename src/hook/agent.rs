//! The end-of-turn hook of a coding agent: installed into the agent's settings
//! file as one merged entry, and run by the agent when its turn ends.
//!
//! Claude Code runs a `Stop` hook and keeps the turn going when it exits 2,
//! feeding its stderr back to the model; `stop_hook_active` in the hook's
//! input says the turn is already continuing because of one, which is the
//! loop guard (<https://code.claude.com/docs/en/hooks>). Cursor's `stop` hook
//! cannot block: it can only submit a follow-up message, capped at five in a
//! row by default (<https://cursor.com/docs/agent/hooks>).

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

use super::FAILED;
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

    /// The entry merged into the settings file.
    fn entry(self) -> Value {
        match self {
            Self::Claude => json!({
                "hooks": [{
                    "type": "command",
                    "command": self.command(),
                    "timeout": HOOK_TIMEOUT_SECONDS,
                }]
            }),
            Self::Cursor => json!({ "command": self.command() }),
        }
    }

    const fn event(self) -> &'static str {
        match self {
            Self::Claude => "Stop",
            Self::Cursor => "stop",
        }
    }

    /// Whether an entry of the event already runs this hook's command.
    fn installed(self, entries: &[Value]) -> bool {
        let runs = |entry: &Value| {
            entry.get("command").and_then(Value::as_str) == Some(self.command())
        };
        entries.iter().any(|entry| match self {
            Self::Claude => entry
                .get("hooks")
                .and_then(Value::as_array)
                .is_some_and(|hooks| hooks.iter().any(runs)),
            Self::Cursor => runs(entry),
        })
    }
}

/// `rust-doctor hook install claude|cursor`.
pub fn install(agent: AgentHook, workspace: &Path, shared: bool, dry_run: bool) -> ExitCode {
    let relative = agent.settings(shared);
    if let Some(link) = symlinked_component(workspace, Path::new(relative)) {
        eprintln!(
            "rust-doctor: {} is a symlink, and nothing is written through one.",
            link.display()
        );
        return ExitCode::from(FAILED);
    }
    let path = workspace.join(relative);
    let existing = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            eprintln!("rust-doctor: {relative} could not be read: {error}. Nothing was written.");
            return ExitCode::from(FAILED);
        }
    };
    let mut settings = match existing.as_deref().map(serde_json::from_str::<Value>) {
        None => Value::Object(Map::new()),
        Some(Ok(settings)) => settings,
        Some(Err(error)) => {
            eprintln!(
                "Could not parse {relative} at line {}, column {}: nothing was written.",
                error.line(),
                error.column()
            );
            return ExitCode::from(FAILED);
        }
    };
    let Some(entries) = event_entries(&mut settings, agent) else {
        eprintln!(
            "rust-doctor: {relative} does not hold the shape its agent reads: nothing was written."
        );
        return ExitCode::from(FAILED);
    };
    if agent.installed(entries) {
        println!(
            "The rust-doctor {} hook is already in {relative}.",
            agent.event()
        );
        return ExitCode::SUCCESS;
    }
    let entry = agent.entry();
    entries.push(entry.clone());
    let verb = if dry_run { "Would add" } else { "Added" };
    println!("{verb} a {} hook to {relative}: {entry}", agent.event());
    if dry_run {
        return ExitCode::SUCCESS;
    }
    let serialized = match serde_json::to_string_pretty(&settings) {
        Ok(serialized) => serialized + "\n",
        Err(_) => return ExitCode::from(FAILED),
    };
    let written = if existing.is_some() {
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
pub fn run(agent: AgentHook) -> ExitCode {
    let mut input = String::new();
    // The input is advisory: a hook whose input cannot be read still scans,
    // and a reader at a terminal is not made to type one.
    if !io::stdin().is_terminal() {
        let _ = io::stdin().take(STDIN_LIMIT).read_to_string(&mut input);
    }
    let input: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    if agent == AgentHook::Claude && input.get("stop_hook_active") == Some(&Value::Bool(true)) {
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

    #[test]
    fn a_merge_keeps_every_other_key_and_adds_one_entry() {
        let mut settings = json!({
            "permissions": { "allow": ["Bash(ls)"] },
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "other" }] }] }
        });
        let entries = event_entries(&mut settings, AgentHook::Claude).unwrap();
        assert!(!AgentHook::Claude.installed(entries));
        entries.push(AgentHook::Claude.entry());
        assert!(AgentHook::Claude.installed(entries));
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
