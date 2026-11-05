use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::ValueEnum;
use rust_doctor::{
    BlockingLevel, Diagnostic, GateStatus, InspectReport, InspectRequest, RuleLevel, RuleOverride,
    ScopeMode, Severity, Status, catalog, inspect,
};
use serde_json::{Map, Value, json};

use super::{ClippyArgs, FAILED, WriteArgs};
use rust_doctor::terminal_text::sanitize;
use rust_doctor::workspace_write::{create_new, on_path, symlinked_component};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AgentHook {
    Claude,
    Cursor,
}

const HOOK_TIMEOUT_SECONDS: u64 = 600;
const SCAN_DEADLINE_SECONDS: u64 = 540;
const LISTED_FINDINGS: usize = 10;
const STDIN_LIMIT: u64 = 1_048_576;
const MAX_FOLLOWUPS: u64 = 1;

const RESCAN: &str =
    "rust-doctor . --scope lines --base HEAD --include-untracked --blocking warning";

pub(crate) const COMPILE_AUTHORITY: &str = "Note: this hook runs `cargo clippy`, which executes the \
build scripts and procedural macros of the tree the agent wrote, at the end of every turn and with \
your account's authority. `--no-clippy` installs a hook that compiles nothing.";

impl AgentHook {
    fn command(self, clippy: ClippyArgs) -> String {
        let agent = match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
        };
        let flag = if clippy.no_clippy { " --no-clippy" } else { "" };
        format!("rust-doctor hook run {agent}{flag}")
    }

    fn settings(self, shared: bool) -> &'static str {
        match (self, shared) {
            (Self::Claude, false) => ".claude/settings.local.json",
            (Self::Claude, true) => ".claude/settings.json",
            (Self::Cursor, _) => ".cursor/hooks.json",
        }
    }

    fn hook(self, clippy: ClippyArgs) -> Value {
        match self {
            Self::Claude => json!({
                "type": "command",
                "command": self.command(clippy),
                "timeout": HOOK_TIMEOUT_SECONDS,
            }),
            Self::Cursor => json!({ "command": self.command(clippy) }),
        }
    }

    fn entry(self, clippy: ClippyArgs) -> Value {
        match self {
            Self::Claude => json!({ "hooks": [self.hook(clippy)] }),
            Self::Cursor => self.hook(clippy),
        }
    }

    const fn event(self) -> &'static str {
        match self {
            Self::Claude => "Stop",
            Self::Cursor => "stop",
        }
    }

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

fn current(hook: &Value, template: &Value) -> bool {
    template.as_object().is_some_and(|keys| {
        keys.iter()
            .all(|(key, value)| hook.get(key) == Some(value))
    })
}

pub(crate) fn install(
    agent: AgentHook,
    workspace: &Path,
    shared: bool,
    clippy: ClippyArgs,
    write: WriteArgs,
) -> u8 {
    let relative = agent.settings(shared);
    if let Some(link) = symlinked_component(workspace, Path::new(relative)) {
        eprintln!(
            "rust-doctor: {} is a symlink, and nothing is written through one.",
            link.display()
        );
        return FAILED;
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
        return FAILED;
    };
    if let Some(code) = merge(agent, entries, relative, clippy, write) {
        return code;
    }
    if write.dry_run {
        return 0;
    }
    if !clippy.no_clippy {
        eprintln!("{COMPILE_AUTHORITY}");
    }
    let Ok(serialized) = serde_json::to_string_pretty(&settings) else {
        return FAILED;
    };
    let serialized = serialized + "\n";
    let written = if existed {
        fs::write(&path, serialized)
    } else {
        create_new(&path, serialized.as_bytes())
    };
    if let Err(error) = written {
        eprintln!("rust-doctor: {relative} could not be written: {error}.");
        return FAILED;
    }
    if !on_path("rust-doctor") {
        eprintln!("Warning: `rust-doctor` is not on PATH, and the hook runs it by name.");
    }
    0
}

fn read_settings(path: &Path, relative: &str) -> Result<(bool, Value), u8> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((false, Value::Object(Map::new())));
        }
        Err(error) => {
            eprintln!("rust-doctor: {relative} could not be read: {error}. Nothing was written.");
            return Err(FAILED);
        }
    };
    serde_json::from_str(&text).map(|settings| (true, settings)).map_err(|error| {
        eprintln!(
            "Could not parse {relative} at line {}, column {}: nothing was written.",
            error.line(),
            error.column()
        );
        FAILED
    })
}

fn merge(
    agent: AgentHook,
    entries: &mut Vec<Value>,
    relative: &str,
    clippy: ClippyArgs,
    write: WriteArgs,
) -> Option<u8> {
    let event = agent.event();
    let template = agent.hook(clippy);
    let mut ours = agent.ours(entries);
    if ours.iter().any(|hook| current(hook, &template)) {
        println!("The rust-doctor {event} hook is already in {relative}.");
        return Some(0);
    }
    if let Some(hook) = ours.first_mut() {
        if !write.update {
            println!(
                "The rust-doctor {event} hook in {relative} is outdated: pass --update to rewrite it."
            );
            return Some(1);
        }
        let previous = sanitize(&hook.to_string());
        if let (Some(hook), Some(keys)) = (hook.as_object_mut(), template.as_object()) {
            hook.extend(keys.clone());
        }
        let verb = if write.dry_run { "Would update" } else { "Updated" };
        let hook = sanitize(&hook.to_string());
        println!("{verb} the {event} hook in {relative}: {previous} becomes {hook}");
        return None;
    }
    let entry = agent.entry(clippy);
    let verb = if write.dry_run { "Would add" } else { "Added" };
    println!("{verb} a {event} hook to {relative}: {entry}");
    entries.push(entry);
    None
}

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

pub(super) fn run(agent: AgentHook, clippy: ClippyArgs) -> ExitCode {
    let mut input = String::new();
    if !io::stdin().is_terminal() {
        let _ = io::stdin().take(STDIN_LIMIT).read_to_string(&mut input);
    }
    let input: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    if guarded(agent, &input) {
        return ExitCode::SUCCESS;
    }
    let mut request = InspectRequest::new(workspace(agent))
        .with_scope(ScopeMode::Lines, Some("HEAD".to_owned()))
        .with_untracked()
        .with_blocking(BlockingLevel::Warning)
        .with_max_duration(Duration::from_secs(SCAN_DEADLINE_SECONDS));
    if clippy.no_clippy {
        for rule in catalog().iter().filter(|rule| rule.id.starts_with("clippy::")) {
            request = request.with_rule_override(RuleOverride::new(rule.id, RuleLevel::Off));
        }
    }
    let report = inspect(request);
    let verdict = verdict(&report);
    match (agent, verdict) {
        (_, Verdict::Pass) => ExitCode::SUCCESS,
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

    const COMPILING: ClippyArgs = ClippyArgs { no_clippy: false };
    const COMPILE_FREE: ClippyArgs = ClippyArgs { no_clippy: true };

    #[test]
    fn every_hook_command_parses_as_the_cli_stands() {
        for command in [
            crate::hook::pre_commit_command("."),
            AgentHook::Claude.command(COMPILING),
            AgentHook::Cursor.command(COMPILING),
            AgentHook::Claude.command(COMPILE_FREE),
            AgentHook::Cursor.command(COMPILE_FREE),
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
        entries.push(AgentHook::Claude.entry(COMPILING));
        let template = AgentHook::Claude.hook(COMPILING);
        assert!(current(AgentHook::Claude.ours(entries)[0], &template));
        assert!(!current(&json!({"command": "rust-doctor hook run claude"}), &template));
        assert_eq!(settings["permissions"]["allow"][0], "Bash(ls)");
        assert_eq!(settings["hooks"]["Stop"][0]["hooks"][0]["command"], "other");
        assert_eq!(settings["hooks"]["Stop"][1]["hooks"][0]["timeout"], 600);

        let mut cursor = json!({});
        event_entries(&mut cursor, AgentHook::Cursor)
            .unwrap()
            .push(AgentHook::Cursor.entry(COMPILING));
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

    #[test]
    fn an_update_sets_or_drops_the_compile_free_flag() {
        let mut settings = json!({});
        let entries = event_entries(&mut settings, AgentHook::Claude).unwrap();
        entries.push(AgentHook::Claude.entry(COMPILING));
        let write = WriteArgs { update: true, dry_run: false };
        for (clippy, command) in [
            (COMPILE_FREE, "rust-doctor hook run claude --no-clippy"),
            (COMPILING, "rust-doctor hook run claude"),
        ] {
            assert_eq!(merge(AgentHook::Claude, entries, "settings", clippy, write), None);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0]["hooks"][0]["command"], command);
        }
        let outdated = WriteArgs { update: false, dry_run: false };
        assert_eq!(merge(AgentHook::Claude, entries, "settings", COMPILE_FREE, outdated), Some(1));
    }
}
