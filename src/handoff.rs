use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rust_doctor::InspectReport;
use rust_doctor::presentation::{
    DiagnosticGroup, ReportPresentation, canonical_not_a_defect, canonical_rule_help,
};
use rust_doctor::terminal_text::sanitize;
use rust_doctor::{BlockingLevel, CategoryOverride, RuleOverride, ScopeMode};

pub const MAX_HANDOFF_BYTES: usize = 12 * 1024;
const MAX_GROUPS: usize = 3;
const MAX_LOCATIONS: usize = 24;
const MAX_ISSUE_SITES: usize = 8;
const CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTarget {
    ClaudeCode,
    Codex,
    Cursor,
}

impl AgentTarget {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::Cursor => "Cursor",
        }
    }

    const fn executable(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor-agent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableAgent {
    target: AgentTarget,
    executable: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffPayload {
    body: String,
}

impl HandoffPayload {
    pub fn as_str(&self) -> &str {
        &self.body
    }

    pub fn preceded_by(&self, line: &str) -> Result<Self, HandoffError> {
        let mut prompt = PromptBuilder::new();
        prompt.push_line(line);
        for existing in self.body.lines() {
            prompt.push_line(existing);
        }
        prompt.finish()
    }
}

impl AvailableAgent {
    pub const fn label(&self) -> &'static str {
        self.target.label()
    }

    pub const fn skill_agent(&self) -> crate::skill::Agent {
        match self.target {
            AgentTarget::ClaudeCode => crate::skill::Agent::Claude,
            AgentTarget::Codex => crate::skill::Agent::Codex,
            AgentTarget::Cursor => crate::skill::Agent::Cursor,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffError {
    UnsafePayload,
    PayloadTooLarge,
    AgentUnavailable(AgentTarget),
    AgentSpawn(AgentTarget, io::ErrorKind),
    ClipboardUnavailable,
    ClipboardFailed,
}

impl fmt::Display for HandoffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafePayload => formatter.write_str("Handoff prompt contains unsafe content."),
            Self::PayloadTooLarge => {
                formatter.write_str("Handoff prompt exceeds the 12 KiB limit.")
            }
            Self::AgentUnavailable(target) => {
                write!(
                    formatter,
                    "{} is no longer available on PATH.",
                    target.label()
                )
            }
            Self::AgentSpawn(target, kind) => write!(
                formatter,
                "{} handoff could not start ({}).",
                target.label(),
                bounded_io_kind(*kind)
            ),
            Self::ClipboardUnavailable => {
                formatter.write_str("Clipboard unavailable: no clipboard command on PATH.")
            }
            Self::ClipboardFailed => {
                formatter.write_str("Clipboard command failed.")
            }
        }
    }
}

impl std::error::Error for HandoffError {}

pub fn available_agents() -> Vec<AvailableAgent> {
    available_agents_in(env::var_os("PATH").as_deref())
}

fn available_agents_in(path: Option<&OsStr>) -> Vec<AvailableAgent> {
    [
        AgentTarget::ClaudeCode,
        AgentTarget::Codex,
        AgentTarget::Cursor,
    ]
    .into_iter()
    .filter_map(|target| {
        resolve_executable(target.executable(), path)
            .map(|executable| AvailableAgent { target, executable })
    })
    .collect()
}

pub fn build_prompt(
    report: &InspectReport,
    presentation: &ReportPresentation,
    rescan_command: &RescanCommand,
) -> Result<HandoffPayload, HandoffError> {
    let audit = match &report.audit.score {
        Some(score) => format!(
            "Audit: {}/100 {} ({}).",
            score.value,
            score.label,
            if score.authoritative {
                "authoritative"
            } else {
                "partial"
            }
        ),
        None => "Audit: score unavailable.".to_owned(),
    };

    let mut prompt = PromptBuilder::new();
    prompt
        .push_line(
            "Fix the highest-priority Rust Doctor findings. Preserve unrelated changes and rescan after the fixes.",
        )
        .push_line(
            "Before editing a site, apply its rule's \"Not a defect when\" check: a site it describes is left as it is.",
        )
        .push_line(&audit);
    push_targets(&mut prompt, report);

    let mut locations = 0usize;
    for (index, group) in presentation.groups.iter().take(MAX_GROUPS).enumerate() {
        push_group(&mut prompt, index + 1, group, &mut locations);
    }
    for advisory in presentation.migration_advisories.iter().take(MAX_GROUPS) {
        let Some(rule_id) = prompt.require(RuleId::new(&advisory.rule_id)) else {
            break;
        };
        let line = format!(
            "Migration advisory: {} has {} occurrences across {} files.",
            rule_id.as_str(),
            advisory.occurrences,
            advisory.files
        );
        prompt.push_line(&line);
    }
    prompt.push_line(&format!("Validate with: {}", rescan_command.as_str()));
    prompt.finish()
}

fn push_targets(prompt: &mut PromptBuilder, report: &InspectReport) {
    let Some(score) = &report.audit.score else {
        return;
    };
    if let Some(overall) = score.ceilings.iter().find(|ceiling| ceiling.dimension == "overall") {
        let holding: BTreeSet<&str> = rust_doctor::catalog()
            .into_iter()
            .filter(|rule| rule.tier == overall.tier)
            .map(|rule| rule.id)
            .collect();
        let named: Vec<&str> = overall
            .rule_ids
            .iter()
            .map(String::as_str)
            .filter(|id| holding.contains(id))
            .take(MAX_GROUPS)
            .collect();
        if named.iter().any(|id| prompt.require(RuleId::new(id)).is_none()) {
            return;
        }
        prompt.push_line(&format!(
            "Ceiling: the score is held at {}/100 by {} findings; repair {} before anything else.",
            overall.ceiling,
            overall.tier.as_str(),
            named.join(", ")
        ));
    }
    if let Some(projected) = score
        .projected_after_top_three
        .filter(|projected| *projected > score.value)
    {
        prompt.push_line(&format!(
            "Target: repairing the rules below reaches a projected {projected}/100."
        ));
    }
}

fn push_group(
    prompt: &mut PromptBuilder,
    index: usize,
    group: &DiagnosticGroup,
    locations: &mut usize,
) {
    let Some(rule_id) = prompt.require(RuleId::new(&group.rule_id)) else {
        return;
    };
    let heading = format!(
        "{index}. {} ({}, {} occurrences)",
        rule_id.as_str(),
        group.severity,
        group.occurrences
    );
    prompt.push_line(&heading);
    if let Some(help) = canonical_rule_help(rule_id.as_str()) {
        prompt.push_line(&format!("   Help: {help}"));
    }
    if let Some(check) = canonical_not_a_defect(rule_id.as_str()) {
        prompt.push_line(&format!("   Not a defect when: {check}"));
    }
    for diagnostic in &group.diagnostics {
        if *locations >= MAX_LOCATIONS {
            break;
        }
        let Some(location) = diagnostic.location() else {
            continue;
        };
        let Some(path) = prompt.require(RelativeLocation::new(&location.path)) else {
            return;
        };
        let site = format!(
            "   - {}:{}:{}",
            path.as_str(),
            location.span.line_start.max(1),
            location.span.column_start.max(1)
        );
        prompt.push_line(&site);
        *locations += 1;
    }
}

pub struct IssuePrompt<'a> {
    pub project_name: &'a str,
    pub rule_id: &'a str,
    pub title: &'a str,
    pub category: &'a str,
    pub is_error: bool,
    pub site_count: usize,
    pub message: &'a str,
    pub help: Option<&'a str>,
    pub rule_url: &'a str,
    pub sites: &'a [String],
}

pub fn build_issue_prompt(issue: &IssuePrompt<'_>) -> Result<HandoffPayload, HandoffError> {
    let rule_id = RuleId::new(issue.rule_id)?.as_str().to_owned();
    let severity = if issue.is_error { "ERROR" } else { "WARN" };

    let mut prompt = PromptBuilder::new();
    prompt
        .push_line(&format!(
            "Fix exactly one Rust Doctor rule in {}:",
            sanitize(issue.project_name)
        ))
        .push_line("")
        .push_line(&format!(
            "{severity} {}: {} ({rule_id}, ×{})",
            sanitize(issue.category),
            sanitize(issue.title),
            issue.site_count
        ))
        .push_line(&sanitize(issue.message));
    if let Some(help) = issue.help {
        prompt
            .push_line("")
            .push_line(&format!("Suggested fix: {}", sanitize(help)));
    }
    if let Some(check) = canonical_not_a_defect(&rule_id) {
        prompt
            .push_line("")
            .push_line(&format!("Not a defect when: {check} Check this before editing."));
    }

    prompt
        .push_line("")
        .push_line("Scope:")
        .push_line(&format!("- Fix only {rule_id}."))
        .push_line("- Fix the root cause; do not suppress, disable, or silence the rule.")
        .push_line("- Keep unrelated refactors out of this pass.")
        .push_line("")
        .push_line("Affected sites:");
    for site in issue.sites.iter().take(MAX_ISSUE_SITES) {
        prompt.push_line(&format!("- {}", sanitize(site)));
    }
    let remaining = issue.sites.len().saturating_sub(MAX_ISSUE_SITES);
    if remaining > 0 {
        prompt.push_line(&format!("- +{remaining} more sites"));
    }

    if !issue.rule_url.is_empty() {
        prompt
            .push_line("")
            .push_line(&format!("Learn more: {}", sanitize(issue.rule_url)));
    }

    prompt.push_line("").push_line(&format!(
        "Verify with `rust-doctor . --yes --verbose` and confirm {rule_id} is gone before moving on."
    ));
    prompt.finish()
}

struct PromptBuilder {
    body: String,
    refused: Option<HandoffError>,
}

impl PromptBuilder {
    fn new() -> Self {
        Self {
            body: String::new(),
            refused: None,
        }
    }

    fn push_line(&mut self, line: &str) -> &mut Self {
        if self.refused.is_some() {
            return self;
        }
        if line.chars().any(char::is_control) {
            self.refused = Some(HandoffError::UnsafePayload);
            return self;
        }
        if self.body.len().saturating_add(line.len()).saturating_add(1) > MAX_HANDOFF_BYTES {
            self.refused = Some(HandoffError::PayloadTooLarge);
            return self;
        }
        self.body.push_str(line);
        self.body.push('\n');
        self
    }

    fn require<T>(&mut self, checked: Result<T, HandoffError>) -> Option<T> {
        match checked {
            Ok(value) => Some(value),
            Err(error) => {
                self.refused.get_or_insert(error);
                None
            }
        }
    }

    fn finish(self) -> Result<HandoffPayload, HandoffError> {
        if let Some(refused) = self.refused {
            return Err(refused);
        }
        if self.body.len() > MAX_HANDOFF_BYTES {
            return Err(HandoffError::PayloadTooLarge);
        }
        Ok(HandoffPayload { body: self.body })
    }
}

struct RuleId<'a>(&'a str);

impl<'a> RuleId<'a> {
    fn new(value: &'a str) -> Result<Self, HandoffError> {
        if value.is_empty()
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-' | b'.')
            })
        {
            return Err(HandoffError::UnsafePayload);
        }
        Ok(Self(value))
    }

    const fn as_str(&self) -> &'a str {
        self.0
    }
}

struct RelativeLocation<'a>(&'a str);

impl<'a> RelativeLocation<'a> {
    fn new(value: &'a str) -> Result<Self, HandoffError> {
        let path = Path::new(value);
        if value.is_empty()
            || value.chars().any(char::is_control)
            || value.contains('\\')
            || has_windows_drive_prefix(value)
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(HandoffError::UnsafePayload);
        }
        Ok(Self(value))
    }

    const fn as_str(&self) -> &'a str {
        self.0
    }
}

fn has_windows_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RescanCommand(String);

#[derive(Debug, Clone, Copy)]
pub struct RescanScope<'a> {
    pub mode: ScopeMode,
    pub base: Option<&'a str>,
    pub staged: bool,
    pub include_untracked: bool,
}

impl RescanCommand {
    pub fn for_inspection(
        verbose: bool,
        blocking: Option<BlockingLevel>,
        rule_overrides: &[RuleOverride],
        category_overrides: &[CategoryOverride],
        packages: &[String],
        scope: Option<RescanScope<'_>>,
    ) -> Result<Self, HandoffError> {
        let mut arguments = vec!["rust-doctor".to_owned(), ".".to_owned()];
        if verbose {
            arguments.push("--verbose".to_owned());
        }
        if let Some(blocking) = blocking {
            arguments.push("--blocking".to_owned());
            arguments.push(blocking.to_string());
        }
        for rule_override in rule_overrides {
            arguments.push("--rule".to_owned());
            arguments.push(rule_override.to_string());
        }
        for category_override in category_overrides {
            arguments.push("--category".to_owned());
            arguments.push(category_override.to_string());
        }
        for package in packages {
            arguments.push("--package".to_owned());
            arguments.push(package.clone());
        }
        if let Some(scope) = scope {
            arguments.push("--scope".to_owned());
            arguments.push(
                match scope.mode {
                    ScopeMode::Files => "files",
                    ScopeMode::Lines => "lines",
                    ScopeMode::Contextual => "contextual",
                    ScopeMode::Baseline => "baseline",
                    ScopeMode::Full => return Err(HandoffError::UnsafePayload),
                }
                .to_owned(),
            );
            if let Some(base) = scope.base {
                validate_git_base(base)?;
                arguments.push("--base".to_owned());
                arguments.push(base.to_owned());
            }
            if scope.staged {
                arguments.push("--staged".to_owned());
            }
            if scope.include_untracked {
                arguments.push("--include-untracked".to_owned());
            }
        }
        arguments.push("--yes".to_owned());

        let mut command = String::new();
        for argument in &arguments {
            if !command.is_empty() {
                command.push(' ');
            }
            command.push_str(&shell_argument(argument));
        }
        Ok(Self(command))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_git_base(base: &str) -> Result<(), HandoffError> {
    if !(1..=256).contains(&base.len())
        || Path::new(base).is_absolute()
        || !base.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'-' | b'_' | b'.' | b'/' | b'~' | b'^' | b'@' | b'{' | b'}'
                )
        })
    {
        return Err(HandoffError::UnsafePayload);
    }
    Ok(())
}

fn shell_argument(argument: &str) -> String {
    if argument
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-._/".contains(character))
    {
        argument.to_owned()
    } else {
        format!("'{0}'", argument.replace('\'', "'\\''"))
    }
}

pub fn launch_agent(
    agent: &AvailableAgent,
    payload: &HandoffPayload,
    workspace_root: &Path,
) -> Result<(), HandoffError> {
    if !is_executable_file(&agent.executable) {
        return Err(HandoffError::AgentUnavailable(agent.target));
    }
    Command::new(&agent.executable)
        .arg(payload.as_str())
        .current_dir(workspace_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map(drop)
        .map_err(|error| HandoffError::AgentSpawn(agent.target, error.kind()))
}

pub fn copy_to_clipboard(payload: &HandoffPayload) -> Result<(), HandoffError> {
    copy_text(payload.as_str())
}

pub const PROMPT_RULER: &str = "---- Agent prompt ----";

pub fn launch_or_print(agent: &AvailableAgent, payload: &HandoffPayload, workspace_root: &Path) {
    if let Err(error) = launch_agent(agent, payload, workspace_root) {
        let reason = format!(
            "Warning: could not launch {}: {error}",
            agent.executable.display()
        );
        print_instead(&reason, payload, &mut io::stdout().lock(), &mut io::stderr().lock());
    }
}

pub fn copy_or_print(payload: &HandoffPayload) {
    match copy_to_clipboard(payload) {
        Ok(()) => eprintln!("Prompt copied to clipboard."),
        Err(error) => print_instead(
            &error.to_string(),
            payload,
            &mut io::stdout().lock(),
            &mut io::stderr().lock(),
        ),
    }
}

pub(crate) fn print_instead(
    reason: &str,
    payload: &HandoffPayload,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) {
    let _ = writeln!(stderr, "{reason} The prompt follows.");
    let _ = writeln!(stdout, "{PROMPT_RULER}\n{}\n{PROMPT_RULER}", payload.as_str());
}

pub fn open_url(url: &str) -> bool {
    let (program, arguments): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };
    Command::new(program)
        .args(arguments)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn copy_text(text: &str) -> Result<(), HandoffError> {
    let path = env::var_os("PATH");
    copy_to_clipboard_in(text, path.as_deref(), clipboard_candidates())
}

fn clipboard_candidates() -> &'static [(&'static str, &'static [&'static str])] {
    if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    }
}

fn copy_to_clipboard_in(
    payload: &str,
    path: Option<&OsStr>,
    candidates: &[(&str, &[&str])],
) -> Result<(), HandoffError> {
    let mut found = false;
    for (program, arguments) in candidates {
        let Some(executable) = resolve_executable(program, path) else {
            continue;
        };
        found = true;
        if run_clipboard(&executable, arguments, payload).is_ok() {
            return Ok(());
        }
    }
    if found {
        Err(HandoffError::ClipboardFailed)
    } else {
        Err(HandoffError::ClipboardUnavailable)
    }
}

fn run_clipboard(executable: &Path, arguments: &[&str], payload: &str) -> io::Result<()> {
    let mut child = Command::new(executable)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("clipboard stdin unavailable"))?
        .write_all(payload.as_bytes());
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let deadline = Instant::now() + CLIPBOARD_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            return status
                .success()
                .then_some(())
                .ok_or_else(|| io::Error::other("clipboard command failed"));
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "clipboard command timed out",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn resolve_executable(program: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    let path = path?;
    for directory in env::split_paths(path) {
        for candidate in executable_candidates(&directory, program) {
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn executable_candidates(directory: &Path, program: &str) -> Vec<PathBuf> {
    if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .into_iter()
            .map(|extension| directory.join(format!("{program}.{extension}")))
            .collect()
    } else {
        vec![directory.join(program)]
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

const fn bounded_io_kind(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::NotFound => "not found",
        io::ErrorKind::PermissionDenied => "permission denied",
        io::ErrorKind::Interrupted => "interrupted",
        _ => "process error",
    }
}

#[cfg(test)]
mod tests;
