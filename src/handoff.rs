//! Bounded, deterministic diagnostic dumps and optional local agent handoffs.

mod launch;

use crate::cli::HandoffTarget;
use crate::diagnostics::{CanonicalDiagnostic, DiagnosticLocation, ReportV1, Severity};
use dialoguer::Select;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

const MAX_DIAGNOSTICS: usize = 1_000;
const MAX_MESSAGE_CHARS: usize = 500;
const MAX_INLINE_GROUPS: usize = 3;
const MAX_INLINE_FINDINGS: usize = 5;
const COPY_PROMPT_LABEL: &str = "Copy prompt to clipboard";
const SKIP_LABEL: &str = "Skip";

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
}

#[derive(Debug, Serialize)]
struct DiagnosticDump {
    schema_version: &'static str,
    score: Option<u32>,
    score_label: Option<crate::diagnostics::ScoreLabel>,
    score_authoritative: bool,
    score_reasons: Vec<String>,
    total_diagnostics: usize,
    included_diagnostics: usize,
    truncated: bool,
    /// What truncation dropped, by priority and category. An agent that reads
    /// this dump must never mistake a bounded head for the whole repository
    /// (US-015 AC-5).
    #[serde(skip_serializing_if = "crate::ordering::TruncationSummary::is_empty")]
    omitted: crate::ordering::TruncationSummary,
    root_causes: Vec<crate::diagnostics::RootCauseGroup>,
    diagnostics: Vec<DumpDiagnostic>,
}

#[derive(Clone, Debug, Serialize)]
struct DumpDiagnostic {
    site_id: String,
    rule: String,
    title: String,
    severity: Severity,
    category: crate::diagnostics::Category,
    location: DiagnosticLocation,
    message: String,
    help: Option<String>,
    url: String,
    fix_group_ids: Vec<String>,
    /// Canonical decision metadata, so an agent ranks the dump the way every
    /// other Rust Doctor surface does.
    priority: Option<String>,
    root_cause_key: Option<String>,
    score_impact: crate::diagnostics::ScoreImpact,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TargetSelection {
    Chosen(HandoffTarget),
    Cancelled,
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

    let prompted = request.target.is_none() && request.interactive;
    let selection = select_target(request)?;
    let TargetSelection::Chosen(target) = selection else {
        return Ok(None);
    };
    if request.remember_target {
        store_preference(target)?;
    } else if prompted {
        let _ = store_preference(target);
    }
    if target == HandoffTarget::None && request.output_dir.is_none() {
        return Ok(None);
    }

    let directory = output_directory(request.output_dir.as_deref())?;
    let dump = bounded_dump(report);
    let working_directory = handoff_working_directory(report);
    let project_name = project_name(&working_directory);
    let handoff = render_handoff(&dump, &project_name, &directory);
    write_dump(&directory, &dump, &handoff)?;
    match target {
        HandoffTarget::Clipboard => deliver_to_clipboard(&handoff),
        HandoffTarget::None => {}
        _ => deliver_to_agent(target, &handoff, &working_directory),
    }
    Ok(Some(HandoffOutcome {
        directory,
        target: (target != HandoffTarget::None).then_some(target),
    }))
}

fn bounded_dump(report: &ReportV1) -> DiagnosticDump {
    // The dump inherits the canonical order and the canonical bound: the top
    // root-cause groups survive truncation, and what was dropped is reported.
    let mut ordered: Vec<&CanonicalDiagnostic> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.visible_on.iter().any(|surface| surface == "mcp"))
        .collect();
    let impact = crate::ordering::RootCauseImpact::measure(ordered.iter().copied());
    crate::ordering::sort_refs(&mut ordered, &impact);
    let root_causes = crate::ordering::root_cause_groups(ordered.iter().copied());
    let total_diagnostics = ordered.len();
    let (kept, omitted) = crate::ordering::truncate(&ordered, &root_causes, MAX_DIAGNOSTICS);

    let mut diagnostics: Vec<_> = kept
        .iter()
        .map(|diagnostic| DumpDiagnostic {
            site_id: diagnostic.site_id.clone(),
            rule: diagnostic.rule.clone(),
            title: diagnostic.title.clone(),
            severity: diagnostic.severity,
            category: diagnostic.category.clone(),
            location: diagnostic.location.clone(),
            message: redact_and_bound(&diagnostic.message),
            help: diagnostic.help.as_deref().map(redact_and_bound),
            url: diagnostic.url.clone(),
            fix_group_ids: diagnostic
                .fixes
                .iter()
                .filter_map(|fix| fix.group_id.clone())
                .collect(),
            priority: diagnostic.priority.clone(),
            root_cause_key: diagnostic.root_cause_key.clone(),
            score_impact: diagnostic.score_impact,
        })
        .collect();
    for diagnostic in &mut diagnostics {
        diagnostic.fix_group_ids.sort();
        diagnostic.fix_group_ids.dedup();
    }
    DiagnosticDump {
        schema_version: "1.0",
        score: report.summary.score,
        score_label: report.summary.score_label,
        score_authoritative: report.summary.score_authoritative,
        score_reasons: report.summary.score_reasons.clone(),
        total_diagnostics,
        included_diagnostics: diagnostics.len(),
        truncated: diagnostics.len() < total_diagnostics,
        omitted,
        root_causes,
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
                diagnostic.title
            );
            let _ = writeln!(content, "  {}", diagnostic.message);
            if let Some(help) = &diagnostic.help {
                let _ = writeln!(content, "  Suggested fix: {help}");
            }
            if !diagnostic.url.is_empty() {
                let _ = writeln!(content, "  Learn more: {}", diagnostic.url);
            }
        }
        atomic_write(
            &canonical_rules.join(format!("{}.txt", group_filename(&rule))),
            content.as_bytes(),
        )?;
    }
    Ok(())
}

fn handoff_score_status(dump: &DiagnosticDump) -> String {
    (dump.score_authoritative.then_some(dump.score).flatten()).map_or_else(
        || {
            dump.score_reasons.first().map_or_else(
                || "Authoritative Core Score unavailable.".to_string(),
                |reason| format!("Authoritative Core Score unavailable ({reason})."),
            )
        },
        |score| {
            format!(
                "Authoritative Core Score: {score}/100 ({}).",
                dump.score_label
                    .map_or_else(|| "unlabeled".to_string(), |label| label.to_string())
            )
        },
    )
}

fn render_handoff(dump: &DiagnosticDump, project_name: &str, directory: &Path) -> String {
    let groups = priority_groups(&dump.diagnostics);
    let shown_group_count = groups.len().min(MAX_INLINE_GROUPS);
    let issue_label = if shown_group_count == 1 {
        "issue"
    } else {
        "issues"
    };
    let migration_scale =
        crate::ordering::use_migration_grouping(dump.total_diagnostics, &dump.root_causes);
    let mut output = if migration_scale {
        // Above the canonical threshold the handoff leads with root causes, not
        // with individual sites (US-015 AC-3).
        format!(
            "Rust Doctor found {} findings in {project_name} across {} root causes. This is migration-scale work: fix the top {shown_group_count} root {issue_label} below on this pass, sampling before you sweep.\n\n",
            dump.total_diagnostics,
            dump.root_causes.len()
        )
    } else {
        format!(
            "Fix the top {shown_group_count} Rust Doctor {issue_label} in {project_name} on this pass. Leave the rest for a follow-up.\n\n"
        )
    };
    output.insert_str(0, &format!("{}\n\n", handoff_score_status(dump)));
    for (index, (rule, diagnostics)) in groups.iter().take(MAX_INLINE_GROUPS).enumerate() {
        let Some(representative) = diagnostics.first() else {
            continue;
        };
        let count_badge = shared_fix_site_count(diagnostics).map_or_else(
            || format!("x{}", diagnostics.len()),
            |site_count| format!("one fix, {site_count} sites"),
        );
        let _ = writeln!(
            output,
            "{}. {} {}: {} ({rule}, {count_badge})",
            index + 1,
            severity_label(representative.severity),
            representative.category,
            representative.title
        );
        let _ = writeln!(output, "   {}", representative.message);
        if let Some(help) = &representative.help {
            let _ = writeln!(output, "   Suggested fix: {help}");
        }
        if !representative.url.is_empty() {
            let _ = writeln!(output, "   Learn more: {}", representative.url);
        }
        let mut locations = BTreeSet::new();
        for diagnostic in diagnostics {
            locations.insert(display_location(&diagnostic.location));
        }
        for location in locations.iter().take(MAX_INLINE_FINDINGS) {
            let _ = writeln!(output, "   - {location}");
        }
        let remaining_locations = locations.len().saturating_sub(MAX_INLINE_FINDINGS);
        if remaining_locations > 0 {
            let _ = writeln!(output, "   - +{remaining_locations} more sites");
        }
        let migration_file_count = migration_file_count(diagnostics);
        if migration_scale && migration_file_count >= crate::ordering::MIGRATION_FILE_THRESHOLD {
            let _ = writeln!(
                output,
                "   Migration-scale ({migration_file_count} files): fix a representative sample, confirm the recipe holds, and get the code owner's sign-off before changing the rest in one pass."
            );
        }
        output.push('\n');
    }

    if !dump.omitted.is_empty() {
        let _ = writeln!(output, "Truncated: {}.\n", dump.omitted.describe());
    }
    let included_label = if dump.truncated {
        format!(
            "{} included of {} total diagnostics",
            dump.included_diagnostics, dump.total_diagnostics
        )
    } else {
        format!("all {} diagnostics", dump.total_diagnostics)
    };
    let _ = writeln!(
        output,
        "Full results for {included_label} (diagnostics.json plus one file per rule): {}",
        directory.display()
    );
    output.push_str(
        "\nRead each file and fix the root cause. Do not suppress, disable, or silence the rule.\n",
    );
    if dump
        .diagnostics
        .iter()
        .any(|diagnostic| !diagnostic.fix_group_ids.is_empty())
    {
        output.push_str(
            "\nFindings that share a fix_group_id in diagnostics.json are one root cause. Treat them as one task, not one task per site.\n",
        );
    }
    output.push_str(
        "\nVerify against the real tool: re-run `rust-doctor . --verbose` and confirm each issue is gone before moving on.\n",
    );
    output.push_str(
        "\nFor every issue you touch, explain plainly what was wrong, its real-world impact, and why the fix addresses the root cause.\n",
    );
    append_deferred_group_guidance(&mut output, &groups);
    output
}

fn append_deferred_group_guidance(output: &mut String, groups: &[(String, Vec<&DumpDiagnostic>)]) {
    if groups.len() <= MAX_INLINE_GROUPS {
        return;
    }
    let migration_count = groups
        .iter()
        .skip(MAX_INLINE_GROUPS)
        .filter(|(_, diagnostics)| {
            migration_file_count(diagnostics) >= crate::ordering::MIGRATION_FILE_THRESHOLD
        })
        .count();
    if migration_count > 0 {
        let group_label = if migration_count == 1 {
            "group"
        } else {
            "groups"
        };
        let _ = writeln!(
            output,
            "\nThe remaining results include {migration_count} migration-scale {group_label}. For each one, fix a representative sample, confirm the recipe holds, and get the code owner's sign-off before changing the rest in one pass."
        );
    }
    output.push_str("\nThen work through the rest from the full results above.\n");
}

/// Groups in canonical root-cause order.
///
/// The dump is already sorted by the canonical comparator, so first appearance
/// is the canonical rank: no second ordering heuristic is applied here
/// (US-015 AC-1).
fn priority_groups(diagnostics: &[DumpDiagnostic]) -> Vec<(String, Vec<&DumpDiagnostic>)> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, Vec<&DumpDiagnostic>> = BTreeMap::new();
    for diagnostic in diagnostics {
        let key = group_key(diagnostic);
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(diagnostic);
    }
    order
        .into_iter()
        .filter_map(|key| groups.remove(&key).map(|value| (key, value)))
        .collect()
}

/// Root cause when the rule declares one, rule identity otherwise. An unmapped
/// rule keeps its own group rather than borrowing another rule's root cause.
fn group_key(diagnostic: &DumpDiagnostic) -> String {
    diagnostic
        .root_cause_key
        .clone()
        .unwrap_or_else(|| diagnostic.rule.clone())
}

fn group_diagnostics(diagnostics: &[DumpDiagnostic]) -> BTreeMap<String, Vec<&DumpDiagnostic>> {
    let mut groups = BTreeMap::new();
    for diagnostic in diagnostics {
        groups
            .entry(group_key(diagnostic))
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

fn select_target(request: &HandoffRequest) -> Result<TargetSelection, HandoffError> {
    if let Some(target) = request.target {
        return Ok(TargetSelection::Chosen(target));
    }
    if !request.interactive {
        return Ok(TargetSelection::Chosen(HandoffTarget::None));
    }

    let mut targets = launch::launchable_targets();
    targets.push((COPY_PROMPT_LABEL.to_string(), HandoffTarget::Clipboard));
    targets.push((SKIP_LABEL.to_string(), HandoffTarget::None));
    let choices: Vec<_> = targets
        .iter()
        .map(|(label, target)| choice_label(label, *target))
        .collect();
    let remembered = if home_directory().is_some() {
        load_preference().unwrap_or(None)
    } else {
        None
    };
    let initial = remembered_choice_index(&targets, remembered);
    // React Doctor separates the footer from the picker with a blank line.
    println!();
    let selection = Select::with_theme(&crate::output::PromptTheme)
        .with_prompt("What would you like to do next?")
        .items(&choices)
        .default(initial)
        .interact_on_opt(&dialoguer::console::Term::stdout())?;
    Ok(selection.map_or(TargetSelection::Cancelled, |selection| {
        TargetSelection::Chosen(
            targets
                .get(selection)
                .map_or(HandoffTarget::None, |(_, target)| *target),
        )
    }))
}

/// Pair a picker label with the one-line description React Doctor shows.
fn choice_label(label: &str, target: HandoffTarget) -> String {
    let description = match target {
        HandoffTarget::Clipboard => "Paste into any agent or chat".to_string(),
        HandoffTarget::None => "Don't hand off".to_string(),
        _ => match launch::binary_name(target) {
            Some(binary) => format!("Open {binary} here with the top issues as a prompt"),
            None => return label.to_string(),
        },
    };
    format!("{label}\n  {description}")
}

fn remembered_choice_index(
    targets: &[(String, HandoffTarget)],
    remembered: Option<HandoffTarget>,
) -> usize {
    remembered
        .and_then(|target| {
            targets
                .iter()
                .position(|(_, candidate)| *candidate == target)
        })
        .unwrap_or_default()
}

fn handoff_working_directory(report: &ReportV1) -> PathBuf {
    for candidate in [
        report.resolved_root.as_deref(),
        Some(report.requested_root.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(path) = Path::new(candidate).canonicalize()
            && path.is_dir()
        {
            return path;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn project_name(working_directory: &Path) -> String {
    working_directory
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("this Rust project")
        .to_string()
}

fn deliver_to_clipboard(handoff: &str) {
    match launch::copy_to_clipboard(handoff) {
        Ok(()) => eprintln!("Copied the prompt to your clipboard."),
        Err(error) => {
            eprintln!("Warning: could not copy the prompt to the clipboard: {error}");
            print_prompt(handoff);
        }
    }
}

fn deliver_to_agent(target: HandoffTarget, handoff: &str, working_directory: &Path) {
    install_skill_best_effort(target, working_directory);
    let binary = launch::binary_name(target).unwrap_or("selected agent");
    eprintln!("Handing off to {target}...");
    match launch::launch_agent(target, handoff, working_directory) {
        Ok(status) if status.success() => {}
        Ok(status) => {
            eprintln!("{binary} exited with status {status}. Here is the prompt instead:");
            print_prompt(handoff);
        }
        Err(error) => {
            eprintln!("Could not launch {binary}: {error}. Here is the prompt instead:");
            print_prompt(handoff);
        }
    }
}

fn install_skill_best_effort(target: HandoffTarget, working_directory: &Path) {
    let Some(agent) = setup_agent(target) else {
        return;
    };
    let Some(home) = home_directory() else {
        return;
    };
    let mut request = crate::setup::SetupRequest::install(home, working_directory.to_path_buf());
    request.yes = true;
    request.agents = vec![agent];
    request.skill = true;
    request.mcp = false;
    if crate::setup::execute(&request).is_ok_and(|report| report.changed()) {
        eprintln!("Installed the Rust Doctor skill for {target}.");
    }
}

const fn setup_agent(target: HandoffTarget) -> Option<crate::setup::AgentId> {
    match target {
        HandoffTarget::ClaudeCode => Some(crate::setup::AgentId::Claude),
        HandoffTarget::Cursor => Some(crate::setup::AgentId::Cursor),
        HandoffTarget::Codex => Some(crate::setup::AgentId::Codex),
        HandoffTarget::OpenCode
        | HandoffTarget::Windsurf
        | HandoffTarget::Clipboard
        | HandoffTarget::None => None,
    }
}

fn print_prompt(handoff: &str) {
    eprintln!("──── Agent prompt ────");
    eprintln!("{handoff}");
    eprintln!("──────────────────────");
}

fn shared_fix_site_count(diagnostics: &[&DumpDiagnostic]) -> Option<usize> {
    if diagnostics.len() < 2 {
        return None;
    }
    let first = diagnostics.first()?;
    first
        .fix_group_ids
        .iter()
        .find(|group_id| {
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix_group_ids.contains(group_id))
        })
        .map(|_| diagnostics.len())
}

fn migration_file_count(diagnostics: &[&DumpDiagnostic]) -> usize {
    diagnostics
        .iter()
        .filter_map(|diagnostic| match &diagnostic.location {
            DiagnosticLocation::Source { path, .. } => Some(path.as_str()),
            DiagnosticLocation::Project => None,
        })
        .collect::<BTreeSet<_>>()
        .len()
}

const fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "ERROR",
        Severity::Warning => "WARN",
        Severity::Info => "INFO",
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

    fn canonical(rule: &str, priority: &str, surfaces: &[&str]) -> CanonicalDiagnostic {
        serde_json::from_value(serde_json::json!({
            "provider": "rust-doctor",
            "rule": rule,
            "title": rule,
            "category": "correctness",
            "severity": "warning",
            "message": format!("{rule} fired"),
            "url": "",
            "tags": [],
            "analysis_kind": "syn_ast",
            "confidence": "medium",
            "original_level": "warning",
            "ownership": {"kind": "workspace"},
            "source_surface": "library",
            "location": {"kind": "project"},
            "related_locations": [],
            "fixes": [],
            "visible_on": surfaces,
            "site_id": format!("site:{rule}"),
            "baseline_key": format!("baseline:{rule}"),
            "namespace_fallback": false,
            "advisory": false,
            "priority": priority,
            "trust_tier": "compiler-proven",
            "score_eligible": true,
            "score_impact": "scored",
            "aggregation_policy": "bounded-occurrence",
            "root_cause_key": format!("rule:{rule}"),
            "evidence_summary": "",
            "limitations": [],
            "suppressed": false
        }))
        .unwrap()
    }

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
    fn handoff_uses_agent_surface_and_canonical_order() {
        let mut report = ReportV1::failure(
            Path::new("/project"),
            crate::diagnostics::ScanMode::Full,
            "test",
            "",
        );
        report.diagnostics = vec![
            canonical("later", "p2", &["mcp"]),
            canonical("hidden", "p0", &["terminal"]),
            canonical("first", "p0", &["mcp"]),
        ];
        let dump = bounded_dump(&report);
        assert_eq!(
            dump.diagnostics
                .iter()
                .map(|diagnostic| diagnostic.rule.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "later"]
        );
    }

    #[test]
    fn remembered_target_value_contains_no_project_identity() {
        assert_eq!(parse_target("codex"), Some(HandoffTarget::Codex));
        assert!(parse_target("/project/codex").is_none());
    }

    #[test]
    fn interactive_picker_matches_react_doctor_labels() {
        let targets = [
            ("Claude Code".to_string(), HandoffTarget::ClaudeCode),
            ("Codex".to_string(), HandoffTarget::Codex),
            ("Cursor".to_string(), HandoffTarget::Cursor),
        ];
        assert_eq!(
            targets
                .iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>(),
            vec!["Claude Code", "Codex", "Cursor"]
        );
        assert_eq!(COPY_PROMPT_LABEL, "Copy prompt to clipboard");
        assert_eq!(SKIP_LABEL, "Skip");
    }

    #[test]
    fn interactive_picker_describes_every_choice_like_react_doctor() {
        assert_eq!(
            choice_label("Claude Code", HandoffTarget::ClaudeCode),
            "Claude Code\n  Open claude here with the top issues as a prompt"
        );
        assert_eq!(
            choice_label("Cursor", HandoffTarget::Cursor),
            "Cursor\n  Open cursor-agent here with the top issues as a prompt"
        );
        assert_eq!(
            choice_label(COPY_PROMPT_LABEL, HandoffTarget::Clipboard),
            "Copy prompt to clipboard\n  Paste into any agent or chat"
        );
        assert_eq!(
            choice_label(SKIP_LABEL, HandoffTarget::None),
            "Skip\n  Don't hand off"
        );
    }

    #[test]
    fn remembered_choice_only_changes_the_initial_focus_when_still_available() {
        let targets = vec![
            ("Claude Code".to_string(), HandoffTarget::ClaudeCode),
            (COPY_PROMPT_LABEL.to_string(), HandoffTarget::Clipboard),
            (SKIP_LABEL.to_string(), HandoffTarget::None),
        ];
        assert_eq!(
            remembered_choice_index(&targets, Some(HandoffTarget::None)),
            2
        );
        assert_eq!(
            remembered_choice_index(&targets, Some(HandoffTarget::OpenCode)),
            0
        );
        assert_eq!(remembered_choice_index(&targets, None), 0);
    }

    #[test]
    fn agent_prompt_is_actionable_and_points_to_the_complete_dump() {
        let diagnostics = vec![
            DumpDiagnostic {
                site_id: "site-1".to_string(),
                rule: "unwrap-in-production".to_string(),
                title: "Production unwrap".to_string(),
                severity: Severity::Error,
                category: crate::diagnostics::Category::ErrorHandling,
                location: serde_json::from_value(serde_json::json!({
                    "kind": "source",
                    "path": "src/lib.rs",
                    "range": {
                        "start": {"line": 12, "column": 4},
                        "end": {"line": 12, "column": 10}
                    }
                }))
                .unwrap(),
                message: "This can panic.".to_string(),
                help: Some("Return a typed error.".to_string()),
                url: "https://rust-doctor.vercel.app/docs/rules/unwrap-in-production".to_string(),
                fix_group_ids: vec!["shared-fix".to_string()],
                priority: Some("p2".to_string()),
                root_cause_key: Some("rule:unwrap-in-production".to_string()),
                score_impact: crate::diagnostics::ScoreImpact::Scored,
            },
            DumpDiagnostic {
                site_id: "site-2".to_string(),
                rule: "unwrap-in-production".to_string(),
                title: "Production unwrap".to_string(),
                severity: Severity::Error,
                category: crate::diagnostics::Category::ErrorHandling,
                location: serde_json::from_value(serde_json::json!({
                    "kind": "source",
                    "path": "src/main.rs",
                    "range": {
                        "start": {"line": 20, "column": 2},
                        "end": {"line": 20, "column": 8}
                    }
                }))
                .unwrap(),
                message: "This can panic.".to_string(),
                help: Some("Return a typed error.".to_string()),
                url: String::new(),
                fix_group_ids: vec!["shared-fix".to_string()],
                priority: Some("p2".to_string()),
                root_cause_key: Some("rule:unwrap-in-production".to_string()),
                score_impact: crate::diagnostics::ScoreImpact::Scored,
            },
        ];
        let dump = DiagnosticDump {
            schema_version: "1.0",
            score: Some(87),
            score_label: Some(crate::diagnostics::ScoreLabel::Great),
            score_authoritative: true,
            score_reasons: Vec::new(),
            total_diagnostics: diagnostics.len(),
            included_diagnostics: diagnostics.len(),
            truncated: false,
            omitted: crate::ordering::TruncationSummary::default(),
            root_causes: Vec::new(),
            diagnostics,
        };
        assert_eq!(shared_fix_site_count(&[&dump.diagnostics[0]]), None);
        let prompt = render_handoff(&dump, "demo", Path::new("/tmp/rust-doctor-demo"));
        assert!(prompt.contains("Fix the top 1 Rust Doctor issue in demo"));
        assert!(prompt.contains("one fix, 2 sites"));
        assert!(prompt.contains("Suggested fix: Return a typed error."));
        assert!(prompt.contains("/tmp/rust-doctor-demo"));
        assert!(prompt.contains("Do not suppress, disable, or silence the rule."));
        assert!(prompt.contains("rust-doctor . --verbose"));
    }

    #[test]
    fn deferred_migration_groups_keep_the_safety_instruction() {
        let diagnostic = |rule: &str, path: String| DumpDiagnostic {
            site_id: format!("{rule}:{path}"),
            rule: rule.to_string(),
            title: "Migration finding".to_string(),
            severity: Severity::Warning,
            category: crate::diagnostics::Category::Architecture,
            location: serde_json::from_value(serde_json::json!({
                "kind": "source",
                "path": path,
                "range": {
                    "start": {"line": 1, "column": 1},
                    "end": {"line": 1, "column": 2}
                }
            }))
            .unwrap(),
            message: "Update this site.".to_string(),
            help: None,
            url: String::new(),
            fix_group_ids: Vec::new(),
            priority: Some("p2".to_string()),
            root_cause_key: Some(format!("rule:{rule}")),
            score_impact: crate::diagnostics::ScoreImpact::Scored,
        };
        let mut diagnostics = ["a-first", "b-second", "c-third"]
            .into_iter()
            .map(|rule| diagnostic(rule, format!("src/{rule}.rs")))
            .collect::<Vec<_>>();
        diagnostics.extend(
            (0..crate::ordering::MIGRATION_FILE_THRESHOLD * 4)
                .map(|index| diagnostic("z-deferred-migration", format!("src/file_{index}.rs"))),
        );
        let root_causes = Vec::new();
        let dump = DiagnosticDump {
            schema_version: "1.0",
            score: Some(87),
            score_label: Some(crate::diagnostics::ScoreLabel::Great),
            score_authoritative: true,
            score_reasons: Vec::new(),
            total_diagnostics: diagnostics.len(),
            included_diagnostics: diagnostics.len(),
            truncated: false,
            omitted: crate::ordering::TruncationSummary::default(),
            root_causes,
            diagnostics,
        };

        let prompt = render_handoff(&dump, "demo", Path::new("/tmp/rust-doctor-demo"));

        assert!(prompt.contains("remaining results include 1 migration-scale group"));
        assert!(prompt.contains("fix a representative sample"));
        assert!(prompt.contains("code owner's sign-off"));
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
            score: None,
            score_label: None,
            score_authoritative: false,
            score_reasons: vec!["required_analysis_failed:clippy".to_string()],
            total_diagnostics: 0,
            included_diagnostics: 0,
            truncated: false,
            omitted: crate::ordering::TruncationSummary::default(),
            root_causes: Vec::new(),
            diagnostics: Vec::new(),
        };
        assert!(write_dump(directory.path(), &dump, "# handoff\n").is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
