//! Bounded, deterministic diagnostic dumps and optional local agent handoffs.

use crate::cli::HandoffTarget;
use crate::diagnostics::{CanonicalDiagnostic, DiagnosticLocation, ReportV1, Severity};
use dialoguer::Select;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MAX_DIAGNOSTICS: usize = 1_000;
const MAX_MESSAGE_CHARS: usize = 500;
const MAX_INLINE_GROUPS: usize = 3;
const MAX_INLINE_FINDINGS: usize = 5;

#[derive(Debug)]
pub struct HandoffRequest {
    pub output_dir: Option<PathBuf>,
    pub target: Option<HandoffTarget>,
    pub remember_target: bool,
    pub reset_target: bool,
    pub interactive: bool,
}

#[derive(Debug)]
pub struct HandoffOutcome {
    pub directory: PathBuf,
    pub target: Option<HandoffTarget>,
}

#[derive(Debug, thiserror::Error)]
pub enum HandoffError {
    #[error("handoff state directory is unavailable")]
    StateDirectoryUnavailable,
    #[error("failed to access handoff path '{}': {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize diagnostic dump: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("handoff target selection failed: {0}")]
    Prompt(#[from] dialoguer::Error),
    #[error("clipboard delivery failed: {0}")]
    Clipboard(String),
}

#[derive(Debug, Serialize)]
struct DiagnosticDump {
    schema_version: &'static str,
    total_diagnostics: usize,
    included_diagnostics: usize,
    truncated: bool,
    diagnostics: Vec<DumpDiagnostic>,
}

#[derive(Clone, Debug, Serialize)]
struct DumpDiagnostic {
    site_id: String,
    rule: String,
    severity: Severity,
    category: crate::diagnostics::Category,
    location: DiagnosticLocation,
    message: String,
    help: Option<String>,
}

pub fn execute(
    report: &ReportV1,
    request: &HandoffRequest,
) -> Result<Option<HandoffOutcome>, HandoffError> {
    if request.reset_target {
        reset_preference()?;
    }
    let should_dump =
        request.output_dir.is_some() || request.target.is_some() || request.interactive;
    if !should_dump || (report.diagnostics.is_empty() && request.output_dir.is_none()) {
        return Ok(None);
    }

    let directory = output_directory(request.output_dir.as_deref())?;
    let dump = bounded_dump(&report.diagnostics);
    let target = match select_target(request) {
        Ok(target) => target,
        Err(error) => {
            write_dump(&directory, &dump, &render_handoff(&dump, None))?;
            return Err(error);
        }
    };
    let handoff = render_handoff(&dump, target);
    write_dump(&directory, &dump, &handoff)?;
    if request.remember_target
        && let Some(target) = target
    {
        store_preference(target)?;
    }
    if target == Some(HandoffTarget::Clipboard) {
        copy_to_clipboard(&handoff)?;
    }
    Ok(Some(HandoffOutcome { directory, target }))
}

fn bounded_dump(diagnostics: &[CanonicalDiagnostic]) -> DiagnosticDump {
    let mut diagnostics: Vec<_> = diagnostics
        .iter()
        .map(|diagnostic| DumpDiagnostic {
            site_id: diagnostic.site_id.clone(),
            rule: diagnostic.rule.clone(),
            severity: diagnostic.severity,
            category: diagnostic.category.clone(),
            location: diagnostic.location.clone(),
            message: redact_and_bound(&diagnostic.message),
            help: diagnostic.help.as_deref().map(redact_and_bound),
        })
        .collect();
    diagnostics.sort_by(|left, right| {
        severity_rank(right.severity)
            .cmp(&severity_rank(left.severity))
            .then(left.rule.cmp(&right.rule))
            .then(left.site_id.cmp(&right.site_id))
    });
    let total_diagnostics = diagnostics.len();
    diagnostics.truncate(MAX_DIAGNOSTICS);
    DiagnosticDump {
        schema_version: "1.0",
        total_diagnostics,
        included_diagnostics: diagnostics.len(),
        truncated: diagnostics.len() < total_diagnostics,
        diagnostics,
    }
}

fn write_dump(directory: &Path, dump: &DiagnosticDump, handoff: &str) -> Result<(), HandoffError> {
    std::fs::create_dir_all(directory).map_err(|source| HandoffError::Io {
        path: directory.to_path_buf(),
        source,
    })?;
    atomic_write(
        &directory.join("diagnostics.json"),
        &serde_json::to_vec_pretty(dump)?,
    )?;
    atomic_write(&directory.join("handoff.md"), handoff.as_bytes())?;

    let groups = group_diagnostics(&dump.diagnostics);
    let rules_dir = directory.join("rules");
    match std::fs::symlink_metadata(&rules_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(HandoffError::Io {
                path: rules_dir,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "handoff rules path must be a real directory",
                ),
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&rules_dir).map_err(|source| HandoffError::Io {
                path: rules_dir.clone(),
                source,
            })?;
        }
        Err(source) => {
            return Err(HandoffError::Io {
                path: rules_dir,
                source,
            });
        }
    }
    let canonical_rules = rules_dir
        .canonicalize()
        .map_err(|source| HandoffError::Io {
            path: rules_dir.clone(),
            source,
        })?;
    if !canonical_rules.starts_with(directory) {
        return Err(HandoffError::Io {
            path: rules_dir,
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "handoff rules path escapes the output directory",
            ),
        });
    }
    for entry in std::fs::read_dir(&canonical_rules).map_err(|source| HandoffError::Io {
        path: canonical_rules.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| HandoffError::Io {
            path: canonical_rules.clone(),
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("rust-doctor-") && name.ends_with(".txt") {
            std::fs::remove_file(entry.path()).map_err(|source| HandoffError::Io {
                path: entry.path(),
                source,
            })?;
        }
    }
    for (rule, diagnostics) in groups {
        let mut content = String::new();
        let _ = writeln!(content, "# {rule}\n");
        for diagnostic in diagnostics {
            let _ = writeln!(
                content,
                "- [{}] {}: {}",
                diagnostic.severity,
                display_location(&diagnostic.location),
                diagnostic.message
            );
        }
        atomic_write(
            &canonical_rules.join(format!("{}.txt", group_filename(&rule))),
            content.as_bytes(),
        )?;
    }
    Ok(())
}

fn render_handoff(dump: &DiagnosticDump, target: Option<HandoffTarget>) -> String {
    let groups = priority_groups(&dump.diagnostics);
    let mut output = String::from("# Rust Doctor diagnostic handoff\n\n");
    let _ = writeln!(
        output,
        "Target: {}\nDiagnostics: {} included of {}\nComplete dump: diagnostics.json\n",
        target.map_or_else(|| "none".to_string(), |value| value.to_string()),
        dump.included_diagnostics,
        dump.total_diagnostics
    );
    for (rule, diagnostics) in groups.into_iter().take(MAX_INLINE_GROUPS) {
        let _ = writeln!(output, "## {rule}\n");
        for diagnostic in diagnostics.into_iter().take(MAX_INLINE_FINDINGS) {
            let _ = writeln!(
                output,
                "- [{}] {}: {}",
                diagnostic.severity,
                display_location(&diagnostic.location),
                diagnostic.message
            );
        }
        output.push('\n');
    }
    output
}

fn priority_groups(diagnostics: &[DumpDiagnostic]) -> Vec<(String, Vec<&DumpDiagnostic>)> {
    let mut groups: Vec<_> = group_diagnostics(diagnostics).into_iter().collect();
    groups.sort_by(|(left_rule, left), (right_rule, right)| {
        let left_rank = left
            .iter()
            .map(|diagnostic| severity_rank(diagnostic.severity))
            .max()
            .unwrap_or_default();
        let right_rank = right
            .iter()
            .map(|diagnostic| severity_rank(diagnostic.severity))
            .max()
            .unwrap_or_default();
        right_rank.cmp(&left_rank).then(left_rule.cmp(right_rule))
    });
    groups
}

fn group_diagnostics(diagnostics: &[DumpDiagnostic]) -> BTreeMap<String, Vec<&DumpDiagnostic>> {
    let mut groups = BTreeMap::new();
    for diagnostic in diagnostics {
        groups
            .entry(diagnostic.rule.clone())
            .or_insert_with(Vec::new)
            .push(diagnostic);
    }
    groups
}

fn output_directory(explicit: Option<&Path>) -> Result<PathBuf, HandoffError> {
    if let Some(directory) = explicit {
        std::fs::create_dir_all(directory).map_err(|source| HandoffError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
        return directory.canonicalize().map_err(|source| HandoffError::Io {
            path: directory.to_path_buf(),
            source,
        });
    }
    tempfile::Builder::new()
        .prefix("rust-doctor-")
        .tempdir()
        .map(tempfile::TempDir::keep)
        .map_err(|source| HandoffError::Io {
            path: std::env::temp_dir(),
            source,
        })
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), HandoffError> {
    let parent = path.parent().ok_or_else(|| HandoffError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent"),
    })?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| HandoffError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temporary
        .write_all(content)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|source| HandoffError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temporary.persist(path).map_err(|error| HandoffError::Io {
        path: path.to_path_buf(),
        source: error.error,
    })?;
    Ok(())
}

fn select_target(request: &HandoffRequest) -> Result<Option<HandoffTarget>, HandoffError> {
    if let Some(target) = request.target {
        return Ok((target != HandoffTarget::None).then_some(target));
    }
    if !request.interactive {
        return Ok(None);
    }
    if home_directory().is_some()
        && let Some(preference) = load_preference()?
    {
        return Ok((preference != HandoffTarget::None).then_some(preference));
    }

    let detected = home_directory()
        .as_deref()
        .map(crate::setup::detect_agents_in)
        .unwrap_or_default();
    let mut targets: Vec<_> = detected
        .iter()
        .map(|agent| (agent.name.to_string(), setup_target(agent.id)))
        .collect();
    targets.push(("Clipboard".to_string(), HandoffTarget::Clipboard));
    targets.push(("No handoff".to_string(), HandoffTarget::None));
    let labels: Vec<_> = targets.iter().map(|(label, _)| label.as_str()).collect();
    let selection = Select::new()
        .with_prompt("Diagnostic handoff target")
        .items(&labels)
        .default(labels.len().saturating_sub(1))
        .interact_on(&dialoguer::console::Term::stdout())?;
    Ok(targets
        .get(selection)
        .map(|(_, target)| *target)
        .filter(|target| *target != HandoffTarget::None))
}

const fn setup_target(agent: crate::setup::AgentId) -> HandoffTarget {
    match agent {
        crate::setup::AgentId::Claude => HandoffTarget::ClaudeCode,
        crate::setup::AgentId::Cursor => HandoffTarget::Cursor,
        crate::setup::AgentId::Codex => HandoffTarget::Codex,
        crate::setup::AgentId::OpenCode => HandoffTarget::OpenCode,
        crate::setup::AgentId::Windsurf => HandoffTarget::Windsurf,
    }
}

fn preference_path() -> Result<PathBuf, HandoffError> {
    let home = home_directory().ok_or(HandoffError::StateDirectoryUnavailable)?;
    Ok(home
        .join(".config")
        .join("rust-doctor")
        .join("handoff-target"))
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn load_preference() -> Result<Option<HandoffTarget>, HandoffError> {
    let path = preference_path()?;
    match std::fs::read_to_string(&path) {
        Ok(value) => Ok(parse_target(value.trim())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(HandoffError::Io { path, source }),
    }
}

fn store_preference(target: HandoffTarget) -> Result<(), HandoffError> {
    let path = preference_path()?;
    let parent = path
        .parent()
        .ok_or(HandoffError::StateDirectoryUnavailable)?;
    std::fs::create_dir_all(parent).map_err(|source| HandoffError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    atomic_write(&path, target.to_string().as_bytes())
}

fn reset_preference() -> Result<(), HandoffError> {
    let path = preference_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(HandoffError::Io { path, source }),
    }
}

fn parse_target(value: &str) -> Option<HandoffTarget> {
    match value {
        "claude-code" => Some(HandoffTarget::ClaudeCode),
        "cursor" => Some(HandoffTarget::Cursor),
        "codex" => Some(HandoffTarget::Codex),
        "open-code" => Some(HandoffTarget::OpenCode),
        "windsurf" => Some(HandoffTarget::Windsurf),
        "clipboard" => Some(HandoffTarget::Clipboard),
        "none" => Some(HandoffTarget::None),
        _ => None,
    }
}

fn copy_to_clipboard(content: &str) -> Result<(), HandoffError> {
    let commands: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        &[("wl-copy", &[])]
    };
    let mut failures = Vec::new();
    for (program, arguments) in commands {
        let mut child = match Command::new(program)
            .args(*arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                failures.push(format!("{program}: {error}"));
                continue;
            }
        };
        let write_result = child
            .stdin
            .take()
            .ok_or_else(|| HandoffError::Clipboard(format!("{program}: stdin unavailable")))?
            .write_all(content.as_bytes());
        let status = child.wait();
        if write_result.is_ok() && status.is_ok_and(|status| status.success()) {
            return Ok(());
        }
        failures.push(format!("{program}: delivery failed"));
    }
    Err(HandoffError::Clipboard(failures.join("; ")))
}

fn redact_and_bound(value: &str) -> String {
    let mut sanitized = value
        .split_whitespace()
        .map(|token| {
            let upper = token.to_ascii_uppercase();
            if token.contains('=')
                && ["TOKEN", "SECRET", "PASSWORD", "API_KEY", "PRIVATE_KEY"]
                    .iter()
                    .any(|marker| upper.starts_with(marker))
            {
                token.split_once('=').map_or_else(
                    || "<redacted>".to_string(),
                    |(key, _)| format!("{key}=<redacted>"),
                )
            } else if ["AKIA", "GHP_", "GHU_", "SK-"]
                .iter()
                .any(|prefix| upper.starts_with(prefix))
            {
                "<redacted>".to_string()
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if sanitized.chars().count() > MAX_MESSAGE_CHARS {
        sanitized = sanitized.chars().take(MAX_MESSAGE_CHARS).collect();
        sanitized.push_str("...");
    }
    sanitized
}

fn group_filename(rule: &str) -> String {
    let readable: String = rule
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let digest = Sha256::digest(rule.as_bytes());
    format!(
        "rust-doctor-{readable}-{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

const fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Error => 3,
        Severity::Warning => 2,
        Severity::Info => 1,
    }
}

fn display_location(location: &DiagnosticLocation) -> String {
    match location {
        DiagnosticLocation::Source { path, range } => {
            format!("{path}:{}:{}", range.start.line, range.start.column)
        }
        DiagnosticLocation::Project => "project".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_text_redacts_secrets_and_is_bounded() {
        let value = format!("TOKEN=secret ghp_abcdef {}", "x".repeat(600));
        let redacted = redact_and_bound(&value);
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("ghp_abcdef"));
        assert!(redacted.chars().count() <= MAX_MESSAGE_CHARS + 3);
    }

    #[test]
    fn group_names_are_deterministic_and_path_safe() {
        let left = group_filename("clippy::future/unsafe");
        assert_eq!(left, group_filename("clippy::future/unsafe"));
        assert!(!left.contains('/'));
        assert!(!left.contains(':'));
    }

    #[test]
    fn remembered_target_value_contains_no_project_identity() {
        assert_eq!(parse_target("codex"), Some(HandoffTarget::Codex));
        assert!(parse_target("/project/codex").is_none());
    }

    #[test]
    fn explicit_output_writes_an_empty_schema_valid_dump() {
        let directory = tempfile::tempdir().unwrap();
        let report: ReportV1 =
            serde_json::from_str(include_str!("../tests/fixtures/report-v1/failure.json")).unwrap();
        let outcome = execute(
            &report,
            &HandoffRequest {
                output_dir: Some(directory.path().to_path_buf()),
                target: None,
                remember_target: false,
                reset_target: false,
                interactive: false,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(outcome.directory, directory.path().canonicalize().unwrap());
        let dump: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("diagnostics.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(dump["schema_version"], "1.0");
        assert_eq!(dump["included_diagnostics"], 0);
        assert!(directory.path().join("handoff.md").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_rules_directory_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), directory.path().join("rules")).unwrap();
        let dump = DiagnosticDump {
            schema_version: "1.0",
            total_diagnostics: 0,
            included_diagnostics: 0,
            truncated: false,
            diagnostics: Vec::new(),
        };
        assert!(write_dump(directory.path(), &dump, "# handoff\n").is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
