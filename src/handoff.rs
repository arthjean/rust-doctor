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
    let mut prompt = PromptBuilder::new();
    prompt.push_line(
        "Fix the highest-priority Rust Doctor findings. Preserve unrelated changes and rescan after the fixes.",
    )?;
    match &report.audit.score {
        Some(score) => {
            let authority = if score.authoritative {
                "authoritative"
            } else {
                "partial"
            };
            prompt.push_line(&format!(
                "Audit: {}/100 {} ({authority}).",
                score.value, score.label
            ))?;
        }
        None => prompt.push_line("Audit: score unavailable.")?,
    }

    let mut locations = 0usize;
    for (index, group) in presentation.groups.iter().take(MAX_GROUPS).enumerate() {
        push_group(&mut prompt, index + 1, group, &mut locations)?;
    }
    for advisory in presentation.migration_advisories.iter().take(MAX_GROUPS) {
        let rule_id = RuleId::new(&advisory.rule_id)?;
        prompt.push_line(&format!(
            "Migration advisory: {} has {} occurrences across {} files.",
            rule_id.as_str(),
            advisory.occurrences,
            advisory.files
        ))?;
    }
    prompt.push_line(&format!("Validate with: {}", rescan_command.as_str()))?;
    prompt.finish()
}

fn push_group(
    prompt: &mut PromptBuilder,
    index: usize,
    group: &DiagnosticGroup,
    locations: &mut usize,
) -> Result<(), HandoffError> {
    let rule_id = RuleId::new(&group.rule_id)?;
    prompt.push_line(&format!(
        "{index}. {} ({}, {} occurrences)",
        rule_id.as_str(),
        group.severity,
        group.occurrences
    ))?;
    // The catalogued help, never the toolchain's. This prompt is handed to an
    // agent with tool access, so it carries only text this crate wrote: a
    // diagnostic's own message and help are workspace-controlled, and
    // `hostile_fields_are_excluded_or_refused_before_delivery` is what keeps
    // them out. This is why the handoff does not go through
    // `DiagnosticGroup::resolved_help`, which prefers exactly that text.
    if let Some(help) = canonical_rule_help(rule_id.as_str()) {
        prompt.push_line(&format!("   Help: {help}"))?;
    }
    for diagnostic in &group.diagnostics {
        if *locations >= MAX_LOCATIONS {
            break;
        }
        let Some(location) = diagnostic.location() else {
            continue;
        };
        let path = RelativeLocation::new(&location.path)?;
        prompt.push_line(&format!(
            "   - {}:{}:{}",
            path.as_str(),
            location.span.line_start.max(1),
            location.span.column_start.max(1)
        ))?;
        *locations += 1;
    }
    Ok(())
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
    prompt.push_line(&format!(
        "Fix exactly one Rust Doctor rule in {}:",
        sanitize(issue.project_name)
    ))?;
    prompt.push_line("")?;
    prompt.push_line(&format!(
        "{severity} {}: {} ({rule_id}, ×{})",
        sanitize(issue.category),
        sanitize(issue.title),
        issue.site_count
    ))?;
    prompt.push_line(&sanitize(issue.message))?;
    if let Some(help) = issue.help {
        prompt.push_line("")?;
        prompt.push_line(&format!("Suggested fix: {}", sanitize(help)))?;
    }

    prompt.push_line("")?;
    prompt.push_line("Scope:")?;
    prompt.push_line(&format!("- Fix only {rule_id}."))?;
    prompt.push_line("- Fix the root cause; do not suppress, disable, or silence the rule.")?;
    prompt.push_line("- Keep unrelated refactors out of this pass.")?;

    prompt.push_line("")?;
    prompt.push_line("Affected sites:")?;
    for site in issue.sites.iter().take(MAX_ISSUE_SITES) {
        prompt.push_line(&format!("- {}", sanitize(site)))?;
    }
    let remaining = issue.sites.len().saturating_sub(MAX_ISSUE_SITES);
    if remaining > 0 {
        prompt.push_line(&format!("- +{remaining} more sites"))?;
    }

    if !issue.rule_url.is_empty() {
        prompt.push_line("")?;
        prompt.push_line(&format!("Learn more: {}", sanitize(issue.rule_url)))?;
    }

    prompt.push_line("")?;
    prompt.push_line(&format!(
        "Verify with `rust-doctor . --yes --verbose` and confirm {rule_id} is gone before moving on."
    ))?;
    prompt.finish()
}

struct PromptBuilder {
    body: String,
}

impl PromptBuilder {
    fn new() -> Self {
        Self {
            body: String::new(),
        }
    }

    fn push_line(&mut self, line: &str) -> Result<(), HandoffError> {
        if line.chars().any(char::is_control) {
            return Err(HandoffError::UnsafePayload);
        }
        if self.body.len().saturating_add(line.len()).saturating_add(1) > MAX_HANDOFF_BYTES {
            return Err(HandoffError::PayloadTooLarge);
        }
        self.body.push_str(line);
        self.body.push('\n');
        Ok(())
    }

    fn finish(self) -> Result<HandoffPayload, HandoffError> {
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
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use rust_doctor::presentation::ReportPresentation;
    use rust_doctor::{
        Audit, BlockingLevel, Diagnostic, DiagnosticSource, DiagnosticSpan, GateReport, GateStatus,
        InspectReport, ScanReport, Severity, Status, Summary, ToolchainReport,
    };

    static TEMPORARY: AtomicUsize = AtomicUsize::new(0);

    fn temporary_root(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/handoff-tests")
            .join(format!(
                "{}-{name}-{}",
                std::process::id(),
                TEMPORARY.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn report() -> InspectReport {
        let diagnostics = vec![Diagnostic {
            context: None,
            id: "one".to_owned(),
            source: DiagnosticSource::Clippy,
            code: Some("clippy::todo".to_owned()),
            base_severity: Severity::Warning,
            severity: Severity::Warning,
            category: Some("correctness".to_owned()),
            message: "source is deliberately excluded".to_owned(),
            help: Some("Replace the placeholder.".to_owned()),
            package: None,
            target: None,
            path: Some("src/lib.rs".to_owned()),
            span: Some(DiagnosticSpan {
                line_start: 4,
                column_start: 5,
                line_end: 4,
                column_end: 10,
            }),
            related: Vec::new(),
            similarity_basis_points: None,
            complexity: None,
            occurrences: 2,
        }];
        let summary = Summary::from_diagnostics(&diagnostics);
        InspectReport {
            schema_version: rust_doctor::SCHEMA_VERSION,
            audit: Audit::build(1, Status::Complete, &diagnostics),
            status: Status::Complete,
            complete: true,
            policy: None,
            scope: None,
            project: None,
            toolchain: ToolchainReport {
                rustc: None,
                cargo: None,
                clippy: None,
            },
            scan: ScanReport {
                command: None,
                exit_code: Some(0),
                build_finished: Some(true),
                noise_lines: Some(0),
            },
            diagnostics,
            delta: None,
            errors: Vec::new(),
            summary,
            gate: GateReport {
                blocking: BlockingLevel::Error,
                status: GateStatus::Passed,
                blocking_diagnostics: Some(0),
            },
        }
    }

    fn rescan_command() -> RescanCommand {
        RescanCommand::for_inspection(false, None, &[], &[], None).unwrap()
    }

    fn payload() -> HandoffPayload {
        let report = report();
        let presentation = ReportPresentation::derive(&report);
        build_prompt(&report, &presentation, &rescan_command()).unwrap()
    }

    fn issue(sites: &[String]) -> IssuePrompt<'_> {
        IssuePrompt {
            project_name: "workspace",
            rule_id: "clippy::unwrap_used",
            title: "Unwrap used",
            category: "Bugs",
            is_error: false,
            site_count: 12,
            message: "called `unwrap` on a value",
            help: Some("Use `?` or `unwrap_or`."),
            rule_url: "https://rust-doctor.com/rules/clippy",
            sites,
        }
    }

    fn ten_sites() -> Vec<String> {
        (1..=10).map(|line| format!("src/lib.rs:{line}")).collect()
    }

    #[test]
    fn the_issue_prompt_names_one_rule_bounds_its_sites_and_ends_on_the_rescan() {
        let sites = ten_sites();
        let prompt = build_issue_prompt(&issue(&sites)).unwrap();
        let prompt = prompt.as_str();
        assert!(prompt.starts_with("Fix exactly one Rust Doctor rule in workspace:"));
        assert!(prompt.contains("WARN Bugs: Unwrap used (clippy::unwrap_used, ×12)"));
        assert!(prompt.contains("- Fix only clippy::unwrap_used."));
        assert_eq!(prompt.matches("- src/lib.rs:").count(), MAX_ISSUE_SITES);
        assert!(prompt.contains("- +2 more sites"));
        assert!(
            prompt
                .trim_end()
                .ends_with("confirm clippy::unwrap_used is gone before moving on.")
        );
    }

    /// Report text is neutralized on its way to the clipboard, and the payload
    /// carries the same bound as the agent handoff rather than growing with
    /// whatever the toolchain produced.
    #[test]
    fn the_issue_prompt_sanitizes_report_text_and_stays_inside_the_handoff_bound() {
        let sites = ten_sites();
        let mut hostile = issue(&sites);
        hostile.message = "safe\u{1b}[2Jwiped";
        hostile.title = "title\nsecond line";
        let prompt = build_issue_prompt(&hostile).unwrap();
        assert!(prompt.as_str().contains("safewiped"));
        assert!(prompt.as_str().contains("titlesecond line"));
        assert!(!prompt.as_str().contains('\u{1b}'));

        let flood = "x".repeat(MAX_HANDOFF_BYTES);
        let mut oversized = issue(&sites);
        oversized.message = &flood;
        assert_eq!(
            build_issue_prompt(&oversized),
            Err(HandoffError::PayloadTooLarge)
        );

        let mut unsafe_rule = issue(&sites);
        unsafe_rule.rule_id = "clippy::unwrap\nsource";
        assert_eq!(
            build_issue_prompt(&unsafe_rule),
            Err(HandoffError::UnsafePayload)
        );
    }

    #[test]
    fn prompt_uses_bounded_groups_relative_locations_and_rescan_command() {
        let report = report();
        let presentation = ReportPresentation::derive(&report);
        let prompt = build_prompt(&report, &presentation, &rescan_command()).unwrap();

        assert!(prompt.as_str().contains("Audit:"));
        assert!(prompt.as_str().contains("clippy::todo"));
        assert!(prompt.as_str().contains("src/lib.rs:4:5"));
        assert!(
            prompt
                .as_str()
                .contains("Validate with: rust-doctor . --yes")
        );
        assert!(!prompt.as_str().contains("source is deliberately excluded"));
        assert!(prompt.as_str().len() <= MAX_HANDOFF_BYTES);
    }

    #[test]
    fn hostile_fields_are_excluded_or_refused_before_delivery() {
        for hostile in [
            "/home/private/source.rs",
            "file:///home/private/source.rs",
            "see:/home/private/source.rs",
            "C:\\private\\source.rs",
            "$HOME/private/source.rs",
            "bad\nargument",
        ] {
            assert_eq!(
                RescanCommand::for_inspection(
                    false,
                    None,
                    &[],
                    &[],
                    Some((ScopeMode::Files, hostile))
                )
                .map(|_| ()),
                Err(HandoffError::UnsafePayload),
                "{hostile}"
            );
        }

        let mut report = report();
        report.diagnostics[0].message = "fn main() { unsafe { source(); } }".to_owned();
        report.diagnostics[0].help = Some("let secret = include_str!(\"config.toml\");".to_owned());
        let presentation = ReportPresentation::derive(&report);
        let prompt = build_prompt(&report, &presentation, &rescan_command()).unwrap();
        assert!(!prompt.as_str().contains("unsafe { source"));
        assert!(!prompt.as_str().contains("include_str"));
        assert!(
            prompt
                .as_str()
                .contains("Replace todo! with the intended implementation")
        );

        let mut hostile_presentation = presentation.clone();
        hostile_presentation.groups[0].rule_id = "clippy::todo\nsource".to_owned();
        assert_eq!(
            build_prompt(&report, &hostile_presentation, &rescan_command()),
            Err(HandoffError::UnsafePayload)
        );

        for hostile_path in [
            "/home/private/source.rs",
            "C:/private/source.rs",
            "C:\\private\\source.rs",
            "src\\private.rs",
        ] {
            let mut hostile_presentation = presentation.clone();
            hostile_presentation.groups[0].diagnostics[0].path = Some(hostile_path.to_owned());
            assert_eq!(
                build_prompt(&report, &hostile_presentation, &rescan_command()),
                Err(HandoffError::UnsafePayload),
                "{hostile_path}"
            );
        }

        let mut oversized = ReportPresentation::derive(&report);
        oversized.groups[0].diagnostics[0].path =
            Some(format!("{}.rs", "x".repeat(MAX_HANDOFF_BYTES)));
        assert_eq!(
            build_prompt(&report, &oversized, &rescan_command()),
            Err(HandoffError::PayloadTooLarge)
        );
    }

    #[test]
    fn agent_order_is_closed_and_stable() {
        let targets = [
            AgentTarget::ClaudeCode,
            AgentTarget::Codex,
            AgentTarget::Cursor,
        ];
        assert_eq!(
            targets.map(AgentTarget::label),
            ["Claude Code", "Codex", "Cursor"]
        );
        assert_eq!(
            targets.map(AgentTarget::executable),
            ["claude", "codex", "cursor-agent"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn agent_launch_passes_one_argument_and_uses_the_workspace_cwd() {
        use std::os::unix::fs::PermissionsExt;

        let root = temporary_root("agent-success");
        let executable = root.join("codex");
        fs::write(
            &executable,
            "#!/bin/sh\nprintf '%s' \"$#\" > argument-count\nprintf '%s' \"$1\" > payload\npwd > cwd\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let agent = AvailableAgent {
            target: AgentTarget::Codex,
            executable,
        };
        let payload = payload();

        launch_agent(&agent, &payload, &root).unwrap();

        assert_eq!(
            fs::read_to_string(root.join("argument-count")).unwrap(),
            "1"
        );
        assert_eq!(
            fs::read_to_string(root.join("payload")).unwrap(),
            payload.as_str()
        );
        assert_eq!(
            PathBuf::from(fs::read_to_string(root.join("cwd")).unwrap().trim()),
            root.canonicalize().unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Runs `attempt` until the freshly written script stops being reported
    /// busy.
    ///
    /// These tests write a shell script and execute it immediately. The kernel
    /// answers `ETXTBSY` when any process still holds that file open for
    /// writing, and a sibling test thread doing its own `fork` inherits every
    /// descriptor this process had open at that instant, including one this
    /// test just closed (rust-lang/rust#114554). The window is real on a loaded
    /// two-core runner and says nothing about the handoff, so it is waited out
    /// rather than asserted. A failure that is not the race still surfaces,
    /// because it is returned unchanged on the first attempt that shows it.
    #[cfg(unix)]
    fn without_text_busy(
        mut attempt: impl FnMut() -> Result<(), HandoffError>,
    ) -> Result<(), HandoffError> {
        for _ in 0..20u8 {
            let outcome = attempt();
            let busy = matches!(
                outcome,
                Err(HandoffError::AgentSpawn(_, io::ErrorKind::ExecutableFileBusy))
                    | Err(HandoffError::ClipboardFailed)
            );
            if !busy {
                return outcome;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        attempt()
    }

    #[cfg(unix)]
    #[test]
    fn non_zero_agent_and_clipboard_fallback_are_typed() {
        use std::os::unix::fs::PermissionsExt;

        let root = temporary_root("process-failures");
        let failing_agent = root.join("codex");
        fs::write(&failing_agent, "#!/bin/sh\nexit 7\n").unwrap();
        fs::set_permissions(&failing_agent, fs::Permissions::from_mode(0o700)).unwrap();
        let agent = AvailableAgent {
            target: AgentTarget::Codex,
            executable: failing_agent,
        };
        let payload = payload();
        assert_eq!(
            without_text_busy(|| launch_agent(&agent, &payload, &root)),
            Err(HandoffError::AgentExited(AgentTarget::Codex))
        );

        let first = root.join("first-copy");
        fs::write(&first, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&first, fs::Permissions::from_mode(0o700)).unwrap();
        let capture = root.join("clipboard-payload");
        let second = root.join("second-copy");
        fs::write(
            &second,
            format!("#!/bin/sh\ncat > '{}'\n", capture.display()),
        )
        .unwrap();
        fs::set_permissions(&second, fs::Permissions::from_mode(0o700)).unwrap();
        without_text_busy(|| {
            copy_to_clipboard_in(
                payload.as_str(),
                Some(root.as_os_str()),
                &[("first-copy", &[]), ("second-copy", &[])],
            )
        })
        .unwrap();
        assert_eq!(fs::read_to_string(capture).unwrap(), payload.as_str());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_agent_is_rejected_before_spawn() {
        let agent = AvailableAgent {
            target: AgentTarget::Codex,
            executable: PathBuf::from("/path/that/does/not/exist/codex"),
        };
        assert_eq!(
            launch_agent(&agent, &payload(), Path::new(".")),
            Err(HandoffError::AgentUnavailable(AgentTarget::Codex))
        );
    }
}
