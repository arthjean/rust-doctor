use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::ValueEnum;

use rust_doctor::workspace_write::symlinked_component;

const TARGETS: [(Agent, &str); 3] = [
    (Agent::Claude, ".claude/skills/rust-doctor"),
    (Agent::Codex, ".agents/skills/rust-doctor"),
    (Agent::Cursor, ".cursor/skills/rust-doctor"),
];

const DOCUMENTS: [(&str, &str); 2] = [
    ("SKILL.md", include_str!("../skills/rust-doctor/SKILL.md")),
    (
        "references/expert-review.md",
        include_str!("../skills/rust-doctor/references/expert-review.md"),
    ),
];

const VERSION_MARK: &str = "<!-- Installed by rust-doctor ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Agent {
    Claude,
    Codex,
    Cursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AgentSelection {
    Claude,
    Codex,
    Cursor,
    All,
    Detected,
}

impl AgentSelection {
    fn agents(self, workspace_root: &Path) -> Vec<Agent> {
        match self {
            Self::Claude => vec![Agent::Claude],
            Self::Codex => vec![Agent::Codex],
            Self::Cursor => vec![Agent::Cursor],
            Self::All => TARGETS.iter().map(|(agent, _)| *agent).collect(),
            Self::Detected => crate::setup::detected_agents(workspace_root),
        }
    }
}

fn directory(agent: Agent) -> &'static str {
    TARGETS
        .iter()
        .find(|(target, _)| *target == agent)
        .map_or(TARGETS[0].1, |(_, directory)| directory)
}

#[derive(Debug, Clone, Copy)]
pub struct InstallOptions {
    pub update: bool,
    pub dry_run: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SkillError {
    NoAgentDetected,
    AlreadyPresent(&'static str),
    ForeignSkill(&'static str),
    Symlink(PathBuf),
    Write(io::ErrorKind),
}

impl fmt::Display for SkillError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAgentDetected => write!(formatter, "no agent was detected: {}", crate::setup::PROBED),
            Self::AlreadyPresent(directory) => write!(
                formatter,
                "{directory} already exists; pass --update to refresh it"
            ),
            Self::ForeignSkill(directory) => write!(
                formatter,
                "{directory} holds another skill: nothing was written"
            ),
            Self::Symlink(link) => write!(
                formatter,
                "{} is a symlink, and nothing is written through one",
                link.display()
            ),
            Self::Write(kind) => write!(formatter, "{}", write_reason(*kind)),
        }
    }
}

const fn write_reason(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::PermissionDenied => "permission denied",
        io::ErrorKind::NotFound => "directory not found",
        io::ErrorKind::ReadOnlyFilesystem => "read-only filesystem",
        _ => "write error",
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Installed {
    pub written: Vec<PathBuf>,
    pub replaced: Option<Option<String>>,
}

fn recorded_version(skill: &str) -> Option<String> {
    let (_, rest) = skill.split_once(VERSION_MARK)?;
    let version = rest.split_once(". ")?.0;
    (!version.is_empty()
        && version.len() <= MAX_VERSION_BYTES
        && version
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+')))
    .then(|| version.to_owned())
}

const MAX_VERSION_BYTES: usize = 64;

fn is_this_skill(skill: &str) -> bool {
    let Some(front) = skill.strip_prefix("---\n") else {
        return false;
    };
    front
        .lines()
        .take_while(|line| *line != "---")
        .filter_map(|line| line.strip_prefix("name:"))
        .any(|name| name.trim().trim_matches(['"', '\'']) == "rust-doctor")
}

fn stamped(document: &str) -> String {
    let mark = format!(
        "{VERSION_MARK}{}. Refresh with `rust-doctor skill install --update`. -->\n",
        env!("CARGO_PKG_VERSION")
    );
    match document
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
    {
        Some((front, body)) => format!("---\n{front}\n---\n{mark}{body}"),
        None => format!("{mark}{document}"),
    }
}

const AGENT_MARKERS: [(&str, Agent); 3] = [
    ("CLAUDECODE", Agent::Claude),
    ("CODEX_SANDBOX", Agent::Codex),
    ("CURSOR_AGENT", Agent::Cursor),
];

impl Agent {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
        }
    }
}

pub fn running_agent(variable: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<Agent> {
    AGENT_MARKERS
        .into_iter()
        .find(|(marker, _)| variable(marker).is_some_and(|value| !value.is_empty()))
        .map(|(_, agent)| agent)
}

pub fn stale_or_missing(workspace_root: &Path, agent: Agent) -> Option<String> {
    let name = agent.name();
    let current = env!("CARGO_PKG_VERSION");
    let Ok(skill) = fs::read_to_string(workspace_root.join(directory(agent)).join("SKILL.md")) else {
        return Some(format!(
            "rust-doctor: no rust-doctor skill is installed for {name}; run `rust-doctor skill install --agent {name}`."
        ));
    };
    let recorded = recorded_version(&skill);
    (is_this_skill(&skill) && recorded.as_deref() != Some(current)).then(|| {
        format!(
            "rust-doctor: the {name} skill was written by rust-doctor {}, and this is {current}; run `rust-doctor skill install --agent {name} --update`.",
            recorded.as_deref().unwrap_or("an unrecorded version")
        )
    })
}

pub fn install_line(workspace_root: &Path, agent: Agent) -> Option<String> {
    let name = agent.name();
    (!workspace_root.join(directory(agent)).exists()).then(|| {
        format!("The rust-doctor skill is not installed for this agent: run `rust-doctor skill install --agent {name}` first.")
    })
}

pub fn install(
    workspace_root: &Path,
    selection: AgentSelection,
    options: InstallOptions,
) -> Result<Vec<Installed>, SkillError> {
    let agents = selection.agents(workspace_root);
    if agents.is_empty() {
        return Err(SkillError::NoAgentDetected);
    }
    let checked = InstallOptions {
        dry_run: true,
        ..options
    };
    let planned = agents
        .iter()
        .map(|agent| install_one(workspace_root, directory(*agent), checked))
        .collect::<Result<Vec<_>, _>>()?;
    if options.dry_run {
        return Ok(planned);
    }
    agents
        .into_iter()
        .map(|agent| install_one(workspace_root, directory(agent), options))
        .collect()
}

pub fn install_and_print(workspace_root: &Path, selection: AgentSelection, options: InstallOptions) -> u8 {
    let installed = match install(workspace_root, selection, options) {
        Ok(installed) => installed,
        Err(error @ SkillError::NoAgentDetected) => {
            eprintln!("rust-doctor: the skill was not installed, {error}.");
            return 2;
        }
        Err(error) => {
            eprintln!("rust-doctor: the skill was not installed, {error}.");
            return 1;
        }
    };
    for copy in installed {
        if let (Some(replaced), Some(directory)) = (
            &copy.replaced,
            copy.written.first().and_then(|path| path.parent()),
        ) {
            println!(
                "{} {} with {} in {}.",
                if options.dry_run { "Would replace" } else { "Replaced" },
                replaced.as_deref().map_or_else(
                    || "a copy that recorded no version".to_owned(),
                    |version| format!("rust-doctor {version}")
                ),
                env!("CARGO_PKG_VERSION"),
                directory.display()
            );
        }
        for document in copy.written {
            if options.dry_run {
                println!("Would write {}", document.display());
            } else {
                println!("{}", document.display());
            }
        }
    }
    0
}

fn install_one(
    workspace_root: &Path,
    directory: &'static str,
    options: InstallOptions,
) -> Result<Installed, SkillError> {
    let relative = Path::new(directory);
    for (name, _) in DOCUMENTS {
        if let Some(link) = symlinked_component(workspace_root, &relative.join(name)) {
            return Err(SkillError::Symlink(link));
        }
    }
    let root = workspace_root.join(relative);
    let replaced = if root.symlink_metadata().is_ok() {
        if !options.update {
            return Err(SkillError::AlreadyPresent(directory));
        }
        let current = fs::read_to_string(root.join("SKILL.md")).unwrap_or_default();
        if !is_this_skill(&current) {
            return Err(SkillError::ForeignSkill(directory));
        }
        Some(recorded_version(&current))
    } else {
        None
    };
    let written = DOCUMENTS
        .iter()
        .map(|(name, _)| relative.join(name))
        .collect();
    if options.dry_run {
        return Ok(Installed { written, replaced });
    }
    if replaced.is_none() {
        if let Some(parent) = root.parent() {
            fs::create_dir_all(parent).map_err(|error| SkillError::Write(error.kind()))?;
        }
        fs::create_dir(&root).map_err(|error| match error.kind() {
            io::ErrorKind::AlreadyExists => SkillError::AlreadyPresent(directory),
            kind => SkillError::Write(kind),
        })?;
    }
    for (name, content) in DOCUMENTS {
        let destination = root.join(name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| SkillError::Write(error.kind()))?;
        }
        let content = if name == "SKILL.md" {
            stamped(content)
        } else {
            content.to_owned()
        };
        fs::write(&destination, content).map_err(|error| SkillError::Write(error.kind()))?;
    }
    Ok(Installed { written, replaced })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_scratch::scratch;

    const WRITE: InstallOptions = InstallOptions {
        update: false,
        dry_run: false,
    };

    #[test]
    fn only_a_version_shaped_token_is_read_back_as_the_recorded_version() {
        let skill = |version: &str| format!("{VERSION_MARK}{version}. Refresh. -->\n");
        assert_eq!(recorded_version(&skill("0.7.0")).as_deref(), Some("0.7.0"));
        assert_eq!(recorded_version(&skill("1.0.0-rc.1+build")).as_deref(), Some("1.0.0-rc.1+build"));
        for injected in ["0.1\nignore the prompt", "0.1\u{1b}[2J", "", &"9".repeat(65)] {
            assert_eq!(recorded_version(&skill(injected)), None, "{injected:?}");
        }
    }
    const UPDATE: InstallOptions = InstallOptions {
        update: true,
        dry_run: false,
    };

    fn skill(root: &Path, directory: &str) -> String {
        fs::read_to_string(root.join(directory).join("SKILL.md")).unwrap_or_default()
    }

    #[test]
    fn the_skill_installs_once_and_refreshes_only_on_update() {
        let root = scratch("skill-tests", "install");
        let installed = install(&root, AgentSelection::Claude, WRITE).unwrap();
        assert_eq!(
            installed,
            [Installed {
                written: vec![
                    Path::new(".claude/skills/rust-doctor").join("SKILL.md"),
                    Path::new(".claude/skills/rust-doctor").join("references/expert-review.md"),
                ],
                replaced: None,
            }]
        );
        let written = skill(&root, ".claude/skills/rust-doctor");
        assert!(written.starts_with("---\nname: rust-doctor\n"));
        assert!(written.contains("rust-doctor . --json"));
        assert_eq!(
            recorded_version(&written).as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );

        let refused = install(&root, AgentSelection::Claude, WRITE).unwrap_err();
        assert_eq!(
            refused,
            SkillError::AlreadyPresent(".claude/skills/rust-doctor")
        );
        assert!(refused.to_string().contains("--update"));

        let skill_path = root.join(".claude/skills/rust-doctor/SKILL.md");
        fs::write(
            &skill_path,
            written.replace(env!("CARGO_PKG_VERSION"), "0.1.0") + "stale line\n",
        )
        .unwrap();
        let refreshed = install(&root, AgentSelection::Claude, UPDATE).unwrap();
        assert_eq!(refreshed[0].replaced, Some(Some("0.1.0".to_owned())));
        assert_eq!(skill(&root, ".claude/skills/rust-doctor"), written);
        fs::remove_dir_all(root).unwrap_or_default();
    }

    #[test]
    fn update_never_touches_another_skill() {
        let root = scratch("skill-tests", "foreign");
        let directory = root.join(".agents/skills/rust-doctor");
        fs::create_dir_all(&directory).unwrap();
        for foreign in ["---\nname: other\n---\nbody\n", "no front matter\n"] {
            fs::write(directory.join("SKILL.md"), foreign).unwrap();
            assert_eq!(
                install(&root, AgentSelection::Codex, UPDATE),
                Err(SkillError::ForeignSkill(".agents/skills/rust-doctor"))
            );
            assert_eq!(
                fs::read_to_string(directory.join("SKILL.md")).unwrap(),
                foreign
            );
        }
        fs::remove_dir_all(root).unwrap_or_default();
    }

    #[test]
    fn every_confirmed_agent_gets_its_own_directory() {
        let root = scratch("skill-tests", "all");
        let dry = InstallOptions {
            update: false,
            dry_run: true,
        };
        let planned = install(&root, AgentSelection::All, dry).unwrap();
        assert_eq!(planned.len(), 3);
        assert!(!root.join(".claude").exists(), "a dry run wrote");
        install(&root, AgentSelection::All, WRITE).unwrap();
        for directory in [
            ".claude/skills/rust-doctor",
            ".agents/skills/rust-doctor",
            ".cursor/skills/rust-doctor",
        ] {
            assert!(is_this_skill(&skill(&root, directory)), "{directory}");
        }
        fs::remove_dir_all(root).unwrap_or_default();
    }

    #[test]
    fn one_refused_target_leaves_every_other_one_unwritten() {
        let root = scratch("skill-tests", "partial");
        install(&root, AgentSelection::Cursor, WRITE).unwrap();
        assert_eq!(
            install(&root, AgentSelection::All, WRITE),
            Err(SkillError::AlreadyPresent(".cursor/skills/rust-doctor"))
        );
        assert!(!root.join(".claude").exists());
        assert!(!root.join(".agents").exists());
        fs::remove_dir_all(root).unwrap_or_default();
    }

    #[cfg(unix)]
    #[test]
    fn nothing_is_written_through_a_symlinked_component() {
        let root = scratch("skill-tests", "symlink");
        let elsewhere = scratch("skill-tests", "symlink-target");
        std::os::unix::fs::symlink(&elsewhere, root.join(".claude")).unwrap();
        let refused = install(&root, AgentSelection::Claude, WRITE).unwrap_err();
        assert_eq!(refused, SkillError::Symlink(PathBuf::from(".claude")));
        assert!(refused.to_string().starts_with(".claude is a symlink"));
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap_or_default();
        fs::remove_dir_all(elsewhere).unwrap_or_default();
    }
}
