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
use rust_doctor::presentation::{DiagnosticGroup, ReportPresentation, canonical_rule_help};
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
}

impl AvailableAgent {
    pub const fn label(&self) -> &'static str {
        self.target.label()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffError {
    UnsafePayload,
    PayloadTooLarge,
    AgentUnavailable(AgentTarget),
    AgentSpawn(AgentTarget, io::ErrorKind),
    AgentExited(AgentTarget),
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
            Self::AgentExited(target) => write!(
                formatter,
                "{} exited before handoff completed.",
                target.label()
            ),
            Self::ClipboardUnavailable => {
                formatter.write_str("Clipboard unavailable; choose another destination.")
            }
            Self::ClipboardFailed => {
                formatter.write_str("Clipboard command failed; choose another destination.")
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
        .push_line(&audit);

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
    // The catalogued help, never the toolchain's. This prompt is handed to an
    // agent with tool access, so it carries only text this crate wrote: a
    // diagnostic's own message and help are workspace-controlled, and
    // `hostile_fields_are_excluded_or_refused_before_delivery` is what keeps
    // them out. This is why the handoff does not go through
    // `DiagnosticGroup::resolved_help`, which prefers exactly that text.
    if let Some(help) = canonical_rule_help(rule_id.as_str()) {
        prompt.push_line(&format!("   Help: {help}"));
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

/// One catalogued rule, ready to be handed to an agent on its own.
///
/// The interactive report copies this rather than the whole handoff prompt: a
/// reader who selected one finding wants that finding fixed, not a plan for the
/// backlog. It is built here, next to [`build_prompt`], so both payloads leave
/// through the same gate and inherit the same bound.
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

/// The clipboard payload of one rule, the transposition of
/// `build-issue-prompt.ts`.
///
/// Unlike [`build_prompt`], this one carries the diagnostic's own message and
/// help, which is text the scanned workspace controls. That is the point: a
/// reader asked for one finding explained, and the catalogued help alone does
/// not explain it. It is safe here and not there because nothing executes this
/// payload; it lands on a clipboard for a person to read and paste. Every
/// field is sanitized on the way in, so a message carrying an escape sequence
/// cannot travel through the clipboard, and refusing rather than neutralizing
/// would give the reader an error they cannot act on.
///
/// The rule id is the one field that must be well formed, since it is what the
/// agent is told to fix and to re-verify.
pub fn build_issue_prompt(issue: &IssuePrompt<'_>) -> Result<HandoffPayload, HandoffError> {
    // Validated raw rather than sanitized first: sanitizing a rule id would
    // turn `clippy::a\nb` into the plausible-looking `clippy::ab` and send the
    // agent after a rule that does not exist. Refusing is the honest answer.
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

/// The payload under construction, and the first reason it cannot be
/// delivered.
///
/// The refusal is held rather than returned per line. A prompt is a template:
/// twenty literal lines returning a `Result` each meant twenty `?` in a
/// function with no branching in it, which is what made `build_issue_prompt`
/// one of the crate's own complexity hotspots at cyclomatic 26 and cognitive
/// 6. Nothing is appended once a refusal is held, so the bound and the control
/// character check stop the payload exactly where they did before, and
/// [`finish`](Self::finish) is the one place either is reported.
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

    /// A value that has to be well formed rather than merely safe, such as a
    /// rule id: the refusal is recorded the same way, so the caller stays a
    /// straight line.
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

impl RescanCommand {
    pub fn for_inspection(
        verbose: bool,
        blocking: Option<BlockingLevel>,
        rule_overrides: &[RuleOverride],
        category_overrides: &[CategoryOverride],
        scope: Option<(ScopeMode, &str)>,
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
        if let Some((scope, base)) = scope {
            validate_git_base(base)?;
            arguments.push("--scope".to_owned());
            arguments.push(
                match scope {
                    ScopeMode::Files => "files",
                    ScopeMode::Baseline => "baseline",
                    ScopeMode::Full => return Err(HandoffError::UnsafePayload),
                }
                .to_owned(),
            );
            arguments.push("--base".to_owned());
            arguments.push(base.to_owned());
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
    let status = Command::new(&agent.executable)
        .arg(payload.as_str())
        .current_dir(workspace_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| HandoffError::AgentSpawn(agent.target, error.kind()))?;
    if status.success() {
        Ok(())
    } else {
        Err(HandoffError::AgentExited(agent.target))
    }
}

pub fn copy_to_clipboard(payload: &HandoffPayload) -> Result<(), HandoffError> {
    copy_text(payload.as_str())
}

/// Hands a URL to the desktop, reporting whether the launcher accepted it.
///
/// Both streams go to the void: a browser launcher that writes to stdout would
/// land in the middle of an interactive frame. Only compiled-in URLs travel
/// through here, which is what makes the `cmd /C start` form on Windows safe.
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

/// Puts arbitrary already-sanitized text on the clipboard. The interactive
/// report copies one rule at a time rather than the whole handoff prompt; that
/// payload comes out of [`build_issue_prompt`].
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
