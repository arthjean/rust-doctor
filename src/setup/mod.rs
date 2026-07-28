//! Reversible installation of rust-doctor agent integrations.

mod detect;
mod hook;
mod mcp_config;
mod skill;
mod transaction;

pub use detect::{AgentId, DetectedAgent, detect_agents_in};

use crate::error::SetupError;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, MultiSelect, Select};
use owo_colors::{OwoColorize, Stream};
use std::collections::BTreeSet;
use std::fs;
use std::io::IsTerminal;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use transaction::{DesiredState, FileState, Mutation, TransactionError};

/// Installer operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupAction {
    Install,
    Uninstall,
}

impl SetupAction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Uninstall => "uninstall",
        }
    }
}

/// Severity at which a generated pre-commit hook blocks a commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockingLevel {
    Error,
    Warning,
    None,
}

impl BlockingLevel {
    /// Stable CLI spelling used in generated hooks.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::None => "none",
        }
    }
}

/// Optional staged pre-commit hook configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StagedHook {
    pub blocking: BlockingLevel,
}

/// MCP process written to supported agent configuration files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpLaunch {
    pub command: String,
    pub args: Vec<String>,
}

impl Default for McpLaunch {
    fn default() -> Self {
        Self {
            command: "rust-doctor".to_owned(),
            args: vec!["--mcp".to_owned()],
        }
    }
}

/// Typed request shared by interactive and scripted setup entry points.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupRequest {
    pub action: SetupAction,
    pub dry_run: bool,
    pub yes: bool,
    /// Set only after a caller has completed its own interactive confirmation.
    pub interactive: bool,
    pub home_dir: PathBuf,
    pub project_root: PathBuf,
    /// Empty means all detected agents.
    pub agents: Vec<AgentId>,
    pub skill: bool,
    pub mcp: bool,
    /// `Some` installs or removes the namespace-scoped staged hook.
    pub hook: Option<StagedHook>,
    pub mcp_launch: McpLaunch,
}

impl SetupRequest {
    /// Build an install request. The caller must set `yes`, `interactive`, or
    /// `dry_run` before execution.
    #[must_use]
    pub fn install(home_dir: PathBuf, project_root: PathBuf) -> Self {
        Self::new(SetupAction::Install, home_dir, project_root)
    }

    /// Build an uninstall request. The caller must set `yes`, `interactive`, or
    /// `dry_run` before execution.
    #[must_use]
    pub fn uninstall(home_dir: PathBuf, project_root: PathBuf) -> Self {
        Self::new(SetupAction::Uninstall, home_dir, project_root)
    }

    fn new(action: SetupAction, home_dir: PathBuf, project_root: PathBuf) -> Self {
        Self {
            action,
            dry_run: false,
            yes: false,
            interactive: false,
            home_dir,
            project_root,
            agents: Vec::new(),
            skill: true,
            mcp: true,
            hook: None,
            mcp_launch: McpLaunch::default(),
        }
    }
}

/// Integration represented by a report entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupComponent {
    Skill,
    Mcp,
    PreCommitHook,
}

impl SetupComponent {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Mcp => "MCP config",
            Self::PreCommitHook => "pre-commit hook",
        }
    }
}

/// Planned or completed mutation kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Create,
    Update,
    Remove,
    Unchanged,
}

impl ChangeKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Remove => "remove",
            Self::Unchanged => "unchanged",
        }
    }
}

/// One deterministic setup result entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupChange {
    pub agent: Option<AgentId>,
    pub component: SetupComponent,
    pub kind: ChangeKind,
    pub path: PathBuf,
}

/// Result returned after planning or applying a setup request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupReport {
    pub action: SetupAction,
    pub dry_run: bool,
    pub detected_agents: Vec<AgentId>,
    pub changes: Vec<SetupChange>,
    pub backups: Vec<PathBuf>,
}

impl SetupReport {
    /// Whether the plan contains at least one file mutation.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.changes
            .iter()
            .any(|change| change.kind != ChangeKind::Unchanged)
    }
}

/// Plan, validate and atomically execute an install or uninstall request.
///
/// All configuration files are parsed before any mutation. Existing same-name
/// entries that are not exact rust-doctor managed entries are refused. A
/// multi-file failure restores every earlier target and removes its temporary
/// backups.
///
/// # Errors
///
/// Returns a typed setup error for invalid authorization, missing agents,
/// malformed or conflicting content, path containment failures, I/O failures,
/// or rollback failures.
pub fn execute(request: &SetupRequest) -> Result<SetupReport, SetupError> {
    validate_request(request)?;
    let home = canonical_root(&request.home_dir, "home directory")?;
    let project = canonical_root(&request.project_root, "project root")?;
    let detected = detect_agents_in(&home);
    let selected = select_requested_agents(request, &detected, &home)?;

    let mut plan = Plan::default();
    for agent in &selected {
        if request.skill {
            plan_skill(request, agent, &home, &mut plan)?;
        }
        if request.mcp {
            plan_mcp(request, agent, &home, &mut plan)?;
        }
    }
    if let Some(hook) = request.hook {
        plan_hook(request.action, hook, &project, &mut plan)?;
    }

    let backups = if request.dry_run || plan.mutations.is_empty() {
        Vec::new()
    } else {
        transaction::apply(&plan.mutations).map_err(transaction_error)?
    };

    Ok(SetupReport {
        action: request.action,
        dry_run: request.dry_run,
        detected_agents: detected.iter().map(|agent| agent.id).collect(),
        changes: plan.changes,
        backups,
    })
}

/// Render a deterministic, color-free report suitable for either stdout or
/// stderr.
#[must_use]
pub fn render_report(report: &SetupReport) -> String {
    let mut output = String::new();
    if report.dry_run {
        output.push_str("Dry run: no files changed.\n");
    }
    output.push_str(report.action.as_str());
    output.push_str(" plan:\n");
    for change in &report.changes {
        output.push_str("  ");
        output.push_str(change.kind.as_str());
        output.push(' ');
        output.push_str(change.component.as_str());
        if let Some(agent) = change.agent {
            output.push_str(" [");
            output.push_str(agent.as_str());
            output.push(']');
        }
        output.push_str(": ");
        output.push_str(&change.path.display().to_string());
        output.push('\n');
    }
    for backup in &report.backups {
        output.push_str("  backup: ");
        output.push_str(&backup.display().to_string());
        output.push('\n');
    }
    output
}

/// Run the compatibility interactive setup wizard.
///
/// # Errors
///
/// Returns an error when no terminal is available, a prompt fails, validation
/// refuses the plan, or the transaction cannot be completed.
pub fn run_setup() -> Result<(), SetupError> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(SetupError::NotInteractive(
            "`rust-doctor setup` requires an interactive terminal. Use `rust-doctor install --yes` for scripts."
                .to_owned(),
        ));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| install_error(None, "neither HOME nor USERPROFILE is set"))?;
    let project = std::env::current_dir().map_err(|error| {
        install_error(
            None,
            format!("failed to resolve current directory: {error}"),
        )
    })?;
    let agents = detect_agents_in(&home);
    if agents.is_empty() {
        return Err(no_agents_error(&home, &AgentId::ALL));
    }

    print_banner();
    eprintln!("\n  Detected agents:\n");
    for agent in &agents {
        eprintln!(
            "    {} {} - {}",
            "\u{2713}".if_supports_color(Stream::Stderr, |text| text.green()),
            agent
                .name
                .if_supports_color(Stream::Stderr, |text| text.bold()),
            agent
                .description
                .if_supports_color(Stream::Stderr, |text| text.dimmed())
        );
    }

    let selected_agents = prompt_agents(&agents)?;
    if selected_agents.is_empty() {
        eprintln!("No agents selected. Exiting.");
        return Ok(());
    }
    let component_labels = ["Agent skill", "MCP configuration", "Staged pre-commit hook"];
    let components = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("  Integrations to install")
        .items(&component_labels)
        .defaults(&[true, true, false])
        .interact()?;
    if components.is_empty() {
        eprintln!("No integrations selected. Exiting.");
        return Ok(());
    }
    let hook = if components.contains(&2) {
        Some(StagedHook {
            blocking: prompt_blocking_level()?,
        })
    } else {
        None
    };

    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("  Apply this installation?")
        .default(true)
        .interact()?
    {
        eprintln!("Cancelled.");
        return Ok(());
    }

    let mut request = SetupRequest::install(home, project);
    request.interactive = true;
    request.agents = selected_agents;
    request.skill = components.contains(&0);
    request.mcp = components.contains(&1);
    request.hook = hook;
    let report = execute(&request)?;
    eprint!("{}", render_report(&report));
    Ok(())
}

#[derive(Default)]
struct Plan {
    changes: Vec<SetupChange>,
    mutations: Vec<Mutation>,
}

fn validate_request(request: &SetupRequest) -> Result<(), SetupError> {
    if !request.dry_run && !request.yes && !request.interactive {
        return Err(install_error(
            None,
            "non-interactive setup requires `yes = true`; use dry-run to inspect without authorization",
        ));
    }
    if !request.skill && !request.mcp && request.hook.is_none() {
        return Err(install_error(None, "no setup integration was selected"));
    }
    if request.mcp {
        if request.mcp_launch.command.trim().is_empty() {
            return Err(install_error(None, "MCP command cannot be empty"));
        }
        if !request
            .mcp_launch
            .args
            .iter()
            .any(|argument| argument == "--mcp")
        {
            return Err(install_error(
                None,
                "MCP command arguments must contain `--mcp`",
            ));
        }
    }
    Ok(())
}

fn canonical_root(path: &Path, label: &str) -> Result<PathBuf, SetupError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        install_error(Some(path), format!("failed to resolve {label}: {error}"))
    })?;
    let metadata = fs::metadata(&canonical).map_err(|error| {
        install_error(
            Some(&canonical),
            format!("failed to inspect {label}: {error}"),
        )
    })?;
    if !metadata.is_dir() {
        return Err(install_error(
            Some(&canonical),
            format!("{label} is not a directory"),
        ));
    }
    Ok(canonical)
}

fn select_requested_agents(
    request: &SetupRequest,
    detected: &[DetectedAgent],
    home: &Path,
) -> Result<Vec<DetectedAgent>, SetupError> {
    let detected_ids: BTreeSet<_> = detected.iter().map(|agent| agent.id).collect();
    let requested: BTreeSet<_> = if request.agents.is_empty() {
        detected_ids.clone()
    } else {
        request.agents.iter().copied().collect()
    };
    if requested.is_empty() {
        return Err(no_agents_error(home, &AgentId::ALL));
    }
    for id in &requested {
        if !detected_ids.contains(id) {
            return Err(no_agents_error(home, &[*id]));
        }
    }

    Ok(requested
        .into_iter()
        .filter_map(|id| detect::resolve_agent(id, home))
        .collect())
}

fn no_agents_error(home: &Path, ids: &[AgentId]) -> SetupError {
    let checked = ids
        .iter()
        .flat_map(|id| detect::probe_paths(*id, home))
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    install_error(
        Some(home),
        format!("no selected supported agent was detected; checked: {checked}"),
    )
}

fn plan_skill(
    request: &SetupRequest,
    agent: &DetectedAgent,
    home: &Path,
    plan: &mut Plan,
) -> Result<(), SetupError> {
    let path = agent.skills_dir.join("rust-doctor/SKILL.md");
    validate_target(home, &path)?;
    let before = transaction::read_state(&path).map_err(transaction_error)?;
    let edit = match request.action {
        SetupAction::Install => {
            skill::install(before.as_ref().map(|state| state.contents.as_slice()))
        }
        SetupAction::Uninstall => {
            skill::uninstall(before.as_ref().map(|state| state.contents.as_slice()))
        }
    }
    .map_err(|message| install_error(Some(&path), message))?;

    match edit {
        skill::SkillEdit::Unchanged => {
            push_unchanged(plan, Some(agent.id), SetupComponent::Skill, path);
        }
        skill::SkillEdit::Write(contents) => push_write(
            plan,
            Some(agent.id),
            SetupComponent::Skill,
            path,
            before,
            contents,
            false,
        ),
        skill::SkillEdit::Delete => {
            push_delete(plan, Some(agent.id), SetupComponent::Skill, path, before);
        }
    }
    Ok(())
}

fn plan_mcp(
    request: &SetupRequest,
    agent: &DetectedAgent,
    home: &Path,
    plan: &mut Plan,
) -> Result<(), SetupError> {
    let path = agent.mcp_config_path.clone();
    validate_target(home, &path)?;
    let before = transaction::read_state(&path).map_err(transaction_error)?;
    let content = before
        .as_ref()
        .map(|state| {
            std::str::from_utf8(&state.contents)
                .map_err(|_| install_error(Some(&path), "MCP config is not valid UTF-8"))
        })
        .transpose()?;
    let format = detect::mcp_format(agent.id, &path).ok_or_else(|| {
        install_error(
            Some(&path),
            format!("no MCP format registered for {}", agent.id),
        )
    })?;
    let rendered = match request.action {
        SetupAction::Install => mcp_config::install(content, format, &request.mcp_launch),
        SetupAction::Uninstall => mcp_config::uninstall(content, format, &request.mcp_launch),
    }
    .map_err(|message| install_error(Some(&path), message))?;

    if let Some(rendered) = rendered {
        push_write(
            plan,
            Some(agent.id),
            SetupComponent::Mcp,
            path,
            before,
            rendered.into_bytes(),
            false,
        );
    } else {
        push_unchanged(plan, Some(agent.id), SetupComponent::Mcp, path);
    }
    Ok(())
}

fn plan_hook(
    action: SetupAction,
    hook_options: StagedHook,
    project: &Path,
    plan: &mut Plan,
) -> Result<(), SetupError> {
    let path = resolve_hook_path(project)?;
    validate_target(project, &path)?;
    let before = transaction::read_state(&path).map_err(transaction_error)?;
    let edit = match action {
        SetupAction::Install => hook::install(
            before.as_ref().map(|state| state.contents.as_slice()),
            hook_options.blocking,
        ),
        SetupAction::Uninstall => {
            hook::uninstall(before.as_ref().map(|state| state.contents.as_slice()))
        }
    }
    .map_err(|message| install_error(Some(&path), message))?;

    match edit {
        hook::HookEdit::Unchanged => {
            push_unchanged(plan, None, SetupComponent::PreCommitHook, path);
        }
        hook::HookEdit::Write(contents) => push_write(
            plan,
            None,
            SetupComponent::PreCommitHook,
            path,
            before,
            contents,
            true,
        ),
        hook::HookEdit::Delete => {
            push_delete(plan, None, SetupComponent::PreCommitHook, path, before);
        }
    }
    Ok(())
}

fn resolve_hook_path(project: &Path) -> Result<PathBuf, SetupError> {
    let repository_root = git_output(
        project,
        &["rev-parse", "--path-format=absolute", "--show-toplevel"],
    )?;
    let repository_root = canonical_root(Path::new(&repository_root), "Git repository root")?;
    if repository_root != project {
        return Err(install_error(
            Some(project),
            format!(
                "project root must be the Git repository root `{}`",
                repository_root.display()
            ),
        ));
    }

    let configured = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["config", "--path", "--get", "core.hooksPath"])
        .output()
        .map_err(|error| {
            install_error(
                Some(project),
                format!("failed to inspect Git hooks path: {error}"),
            )
        })?;
    if configured.status.success() {
        let configured = output_text(project, "Git hooks path", &configured.stdout)?;
        let hooks = PathBuf::from(configured);
        return Ok(if hooks.is_absolute() {
            hooks.join("pre-commit")
        } else {
            project.join(hooks).join("pre-commit")
        });
    }
    if configured.status.code() != Some(1) {
        return Err(install_error(
            Some(project),
            format!(
                "failed to inspect Git hooks path: {}",
                String::from_utf8_lossy(&configured.stderr).trim()
            ),
        ));
    }

    let default_hook = git_output(
        project,
        &[
            "rev-parse",
            "--path-format=relative",
            "--git-path",
            "hooks/pre-commit",
        ],
    )?;
    Ok(project.join(default_hook))
}

fn git_output(project: &Path, args: &[&str]) -> Result<String, SetupError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(args)
        .output()
        .map_err(|error| install_error(Some(project), format!("failed to invoke Git: {error}")))?;
    if !output.status.success() {
        return Err(install_error(
            Some(project),
            format!(
                "Git could not resolve `{}`: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    output_text(project, "Git output", &output.stdout)
}

fn output_text(path: &Path, label: &str, bytes: &[u8]) -> Result<String, SetupError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| install_error(Some(path), format!("{label} is not valid UTF-8")))?;
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

fn validate_target(root: &Path, target: &Path) -> Result<(), SetupError> {
    let relative = target.strip_prefix(root).map_err(|_| {
        install_error(
            Some(target),
            format!("destination escapes allowed root `{}`", root.display()),
        )
    })?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(install_error(
            Some(target),
            format!("destination escapes allowed root `{}`", root.display()),
        ));
    }

    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            cursor.push(part);
        }
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(install_error(
                    Some(&cursor),
                    format!(
                        "symbolic links are not allowed in setup destinations under `{}`",
                        root.display()
                    ),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(install_error(
                    Some(&cursor),
                    format!("failed to validate destination containment: {error}"),
                ));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_write(
    plan: &mut Plan,
    agent: Option<AgentId>,
    component: SetupComponent,
    path: PathBuf,
    before: Option<FileState>,
    contents: Vec<u8>,
    executable: bool,
) {
    let kind = if before.is_some() {
        ChangeKind::Update
    } else {
        ChangeKind::Create
    };
    plan.changes.push(SetupChange {
        agent,
        component,
        kind,
        path: path.clone(),
    });
    plan.mutations.push(Mutation {
        path,
        before,
        desired: DesiredState::Write {
            contents,
            executable,
        },
    });
}

fn push_delete(
    plan: &mut Plan,
    agent: Option<AgentId>,
    component: SetupComponent,
    path: PathBuf,
    before: Option<FileState>,
) {
    plan.changes.push(SetupChange {
        agent,
        component,
        kind: ChangeKind::Remove,
        path: path.clone(),
    });
    plan.mutations.push(Mutation {
        path,
        before,
        desired: DesiredState::Delete,
    });
}

fn push_unchanged(
    plan: &mut Plan,
    agent: Option<AgentId>,
    component: SetupComponent,
    path: PathBuf,
) {
    plan.changes.push(SetupChange {
        agent,
        component,
        kind: ChangeKind::Unchanged,
        path,
    });
}

fn prompt_agents(agents: &[DetectedAgent]) -> Result<Vec<AgentId>, dialoguer::Error> {
    if agents.len() == 1 {
        return Ok(vec![agents[0].id]);
    }
    let labels: Vec<_> = agents
        .iter()
        .map(|agent| format!("{} - {}", agent.name, agent.description))
        .collect();
    let selected = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("  Agents to configure")
        .items(&labels)
        .defaults(&vec![true; labels.len()])
        .interact()?;
    Ok(selected
        .into_iter()
        .filter_map(|index| agents.get(index).map(|agent| agent.id))
        .collect())
}

fn prompt_blocking_level() -> Result<BlockingLevel, dialoguer::Error> {
    let labels = ["warning", "error", "none"];
    let selected = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("  Hook blocking level")
        .items(&labels)
        .default(0)
        .interact()?;
    Ok(match selected {
        1 => BlockingLevel::Error,
        2 => BlockingLevel::None,
        _ => BlockingLevel::Warning,
    })
}

fn print_banner() {
    eprintln!(
        "\n  {}",
        "rust-doctor setup".if_supports_color(Stream::Stderr, |text| text.bold())
    );
    eprintln!(
        "  {}",
        "Configure rust-doctor for your coding agents"
            .if_supports_color(Stream::Stderr, |text| text.dimmed())
    );
}

fn transaction_error(error: TransactionError) -> SetupError {
    SetupError::Install {
        path: error.path,
        message: error.message,
    }
}

fn install_error(path: Option<&Path>, message: impl Into<String>) -> SetupError {
    SetupError::Install {
        path: path.map(Path::to_path_buf),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _temp: tempfile::TempDir,
        home: PathBuf,
        project: PathBuf,
    }

    impl Fixture {
        fn codex() -> Self {
            let temp = tempfile::tempdir().expect("temporary fixture");
            let home = temp.path().join("home");
            let project = temp.path().join("project");
            fs::create_dir_all(home.join(".codex")).expect("Codex probe");
            fs::create_dir_all(&project).expect("project directory");
            assert!(
                Command::new("git")
                    .args(["init", "--quiet"])
                    .arg(&project)
                    .status()
                    .expect("git init")
                    .success()
            );
            Self {
                _temp: temp,
                home,
                project,
            }
        }

        fn request(&self) -> SetupRequest {
            let mut request = SetupRequest::install(self.home.clone(), self.project.clone());
            request.yes = true;
            request.agents = vec![AgentId::Codex];
            request.hook = Some(StagedHook {
                blocking: BlockingLevel::Warning,
            });
            request
        }
    }

    #[test]
    fn dry_run_reports_exact_targets_without_writing() {
        let fixture = Fixture::codex();
        let mut request = fixture.request();
        request.yes = false;
        request.dry_run = true;

        let report = execute(&request).expect("dry-run plan");
        assert!(report.dry_run);
        assert_eq!(report.changes.len(), 3);
        assert!(
            report
                .changes
                .iter()
                .all(|change| change.kind == ChangeKind::Create)
        );
        assert!(!fixture.home.join(".codex/config.toml").exists());
        assert!(
            !fixture
                .home
                .join(".codex/skills/rust-doctor/SKILL.md")
                .exists()
        );
        assert!(!fixture.project.join(".git/hooks/pre-commit").exists());
    }

    #[test]
    fn install_is_idempotent_and_uninstall_removes_only_managed_surfaces() {
        let fixture = Fixture::codex();
        let request = fixture.request();
        let installed = execute(&request).expect("install");
        assert!(installed.changed());

        let repeated = execute(&request).expect("repeated install");
        assert!(!repeated.changed());
        assert!(repeated.backups.is_empty());

        let mut uninstall = request;
        uninstall.action = SetupAction::Uninstall;
        let removed = execute(&uninstall).expect("uninstall");
        assert!(removed.changed());
        assert!(
            !fixture
                .home
                .join(".codex/skills/rust-doctor/SKILL.md")
                .exists()
        );
        assert!(!fixture.project.join(".git/hooks/pre-commit").exists());
        let config = fs::read_to_string(fixture.home.join(".codex/config.toml"))
            .expect("preserved Codex config");
        assert!(!config.contains("mcp_servers.rust-doctor"));
    }

    #[test]
    fn malformed_later_config_prevents_all_planned_writes() {
        let temp = tempfile::tempdir().expect("temporary fixture");
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        fs::create_dir_all(home.join(".claude")).expect("Claude probe");
        fs::create_dir_all(home.join(".codex")).expect("Codex probe");
        fs::create_dir_all(&project).expect("project directory");
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(&project)
                .status()
                .expect("git init")
                .success()
        );
        fs::write(home.join(".codex/config.toml"), "not = [valid")
            .expect("malformed Codex fixture");

        let mut request = SetupRequest::install(home.clone(), project);
        request.yes = true;
        request.agents = vec![AgentId::Claude, AgentId::Codex];
        let error = execute(&request).expect_err("malformed config must fail");
        assert!(error.to_string().contains("config.toml"));
        assert!(!home.join(".claude/skills/rust-doctor/SKILL.md").exists());
        assert!(!home.join(".claude.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_agent_destination_is_refused() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::codex();
        let outside = fixture._temp.path().join("outside");
        fs::create_dir_all(&outside).expect("outside directory");
        symlink(&outside, fixture.home.join(".codex/skills")).expect("skills symlink");
        let mut request = fixture.request();
        request.mcp = false;
        request.hook = None;

        let error = execute(&request).expect_err("symlink must fail");
        assert!(error.to_string().contains("symbolic links"));
        assert!(!outside.join("rust-doctor/SKILL.md").exists());
    }

    #[test]
    fn report_renderer_is_deterministic() {
        let report = SetupReport {
            action: SetupAction::Install,
            dry_run: true,
            detected_agents: vec![AgentId::Codex],
            changes: vec![SetupChange {
                agent: Some(AgentId::Codex),
                component: SetupComponent::Skill,
                kind: ChangeKind::Create,
                path: PathBuf::from("/tmp/SKILL.md"),
            }],
            backups: Vec::new(),
        };
        assert_eq!(
            render_report(&report),
            "Dry run: no files changed.\ninstall plan:\n  create skill [codex]: /tmp/SKILL.md\n"
        );
    }
}
