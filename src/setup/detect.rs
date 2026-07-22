//! Data-driven detection of supported AI coding agents.

use super::mcp_config::McpFormat;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Stable identifier used by the non-interactive setup API.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AgentId {
    /// Anthropic Claude Code.
    Claude,
    /// Cursor.
    Cursor,
    /// OpenAI Codex.
    Codex,
    /// OpenCode.
    OpenCode,
    /// Windsurf.
    Windsurf,
}

impl AgentId {
    /// Every supported agent, in deterministic display order.
    pub const ALL: [Self; 5] = [
        Self::Claude,
        Self::Cursor,
        Self::Codex,
        Self::OpenCode,
        Self::Windsurf,
    ];

    /// Stable lowercase CLI spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Windsurf => "windsurf",
        }
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AgentId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Ok(Self::Claude),
            "cursor" => Ok(Self::Cursor),
            "codex" => Ok(Self::Codex),
            "opencode" | "open-code" => Ok(Self::OpenCode),
            "windsurf" => Ok(Self::Windsurf),
            _ => Err(format!("unsupported agent `{value}`")),
        }
    }
}

/// An AI coding agent detected below a supplied home directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetectedAgent {
    /// Stable identifier.
    pub id: AgentId,
    /// Human-readable name.
    pub name: &'static str,
    /// Short product description.
    pub description: &'static str,
    /// Resolved MCP configuration file.
    pub mcp_config_path: PathBuf,
    /// Resolved global skills directory.
    pub skills_dir: PathBuf,
    /// Whether a `rust-doctor` MCP namespace is present.
    pub mcp_already_configured: bool,
    /// Whether a rust-doctor skill file is present.
    pub skill_already_installed: bool,
}

#[derive(Clone, Copy)]
struct McpTargetDef {
    path: &'static str,
    format: McpFormat,
}

/// Internal registry entry. All paths are static, home-relative paths so callers
/// cannot inject a destination through an agent identifier.
struct AgentDef {
    id: AgentId,
    name: &'static str,
    description: &'static str,
    probe_paths: &'static [&'static str],
    mcp_targets: &'static [McpTargetDef],
    skills_dir: &'static str,
}

const STANDARD_JSON: McpFormat = McpFormat::StandardJson;
const OPENCODE_JSON: McpFormat = McpFormat::OpenCodeJson;
const CODEX_TOML: McpFormat = McpFormat::CodexToml;

const AGENTS: &[AgentDef] = &[
    AgentDef {
        id: AgentId::Claude,
        name: "Claude Code",
        description: "Anthropic's coding agent",
        probe_paths: &[".claude", ".claude.json"],
        mcp_targets: &[McpTargetDef {
            path: ".claude.json",
            format: STANDARD_JSON,
        }],
        skills_dir: ".claude/skills",
    },
    AgentDef {
        id: AgentId::Cursor,
        name: "Cursor",
        description: "AI-first code editor",
        probe_paths: &[".cursor"],
        mcp_targets: &[McpTargetDef {
            path: ".cursor/mcp.json",
            format: STANDARD_JSON,
        }],
        skills_dir: ".cursor/skills",
    },
    AgentDef {
        id: AgentId::Codex,
        name: "Codex",
        description: "OpenAI coding agent",
        probe_paths: &[".codex"],
        mcp_targets: &[McpTargetDef {
            path: ".codex/config.toml",
            format: CODEX_TOML,
        }],
        skills_dir: ".codex/skills",
    },
    AgentDef {
        id: AgentId::OpenCode,
        name: "OpenCode",
        description: "Open source coding agent",
        probe_paths: &[".config/opencode", ".opencode"],
        mcp_targets: &[McpTargetDef {
            path: ".config/opencode/opencode.json",
            format: OPENCODE_JSON,
        }],
        skills_dir: ".config/opencode/skills",
    },
    AgentDef {
        id: AgentId::Windsurf,
        name: "Windsurf",
        description: "AI-powered editor by Codeium",
        probe_paths: &[".codeium/windsurf", ".windsurf"],
        mcp_targets: &[McpTargetDef {
            path: ".codeium/windsurf/mcp_config.json",
            format: STANDARD_JSON,
        }],
        skills_dir: ".codeium/windsurf/skills",
    },
];

/// Detect every supported agent below `home`.
#[must_use]
pub fn detect_agents_in(home: &Path) -> Vec<DetectedAgent> {
    AGENTS
        .iter()
        .filter(|definition| is_detected(definition, home))
        .map(|definition| resolve(definition, home))
        .collect()
}

/// Resolve a known agent's static destinations even when it was not detected.
pub(super) fn resolve_agent(id: AgentId, home: &Path) -> Option<DetectedAgent> {
    AGENTS
        .iter()
        .find(|definition| definition.id == id)
        .map(|definition| resolve(definition, home))
}

/// Return every path used to detect an agent. Error messages use this to make
/// a missing explicit selection actionable.
pub(super) fn probe_paths(id: AgentId, home: &Path) -> Vec<PathBuf> {
    AGENTS
        .iter()
        .find(|definition| definition.id == id)
        .map_or_else(Vec::new, |definition| {
            definition
                .probe_paths
                .iter()
                .map(|path| home.join(path))
                .collect()
        })
}

pub(super) fn mcp_format(id: AgentId, path: &Path) -> Option<McpFormat> {
    AGENTS
        .iter()
        .find(|definition| definition.id == id)
        .and_then(|definition| {
            definition
                .mcp_targets
                .iter()
                .find(|target| path.ends_with(target.path))
                .map(|target| target.format)
        })
}

fn is_detected(definition: &AgentDef, home: &Path) -> bool {
    definition
        .probe_paths
        .iter()
        .any(|path| home.join(path).exists())
        || definition
            .mcp_targets
            .iter()
            .any(|target| home.join(target.path).exists())
        || home.join(definition.skills_dir).exists()
}

fn resolve(definition: &AgentDef, home: &Path) -> DetectedAgent {
    let mcp_target = definition
        .mcp_targets
        .iter()
        .find(|target| home.join(target.path).exists())
        .or_else(|| definition.mcp_targets.first());

    let (mcp_config_path, format) = mcp_target.map_or_else(
        || (home.to_path_buf(), STANDARD_JSON),
        |target| (home.join(target.path), target.format),
    );
    let skills_dir = home.join(definition.skills_dir);
    let mcp_already_configured = super::mcp_config::contains_namespace(&mcp_config_path, format);

    DetectedAgent {
        id: definition.id,
        name: definition.name,
        description: definition.description,
        mcp_config_path,
        skill_already_installed: skills_dir.join("rust-doctor/SKILL.md").exists(),
        skills_dir,
        mcp_already_configured,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_all_five_agents() {
        let ids: Vec<_> = AGENTS.iter().map(|agent| agent.id).collect();
        assert_eq!(ids, AgentId::ALL);
    }

    #[test]
    fn registry_paths_are_relative_and_contained() {
        for agent in AGENTS {
            for path in agent
                .probe_paths
                .iter()
                .copied()
                .chain(agent.mcp_targets.iter().map(|target| target.path))
                .chain(std::iter::once(agent.skills_dir))
            {
                let path = Path::new(path);
                assert!(path.is_relative(), "{} has an absolute path", agent.name);
                assert!(
                    !path.components().any(|component| matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )),
                    "{} has an escaping path",
                    agent.name
                );
            }
        }
    }

    #[test]
    fn detects_each_registered_agent() {
        let home = tempfile::tempdir().expect("temporary home");
        for agent in AGENTS {
            let probe = home.path().join(agent.probe_paths[0]);
            std::fs::create_dir_all(&probe).expect("agent probe directory");
        }

        let detected = detect_agents_in(home.path());
        let ids: Vec<_> = detected.iter().map(|agent| agent.id).collect();
        assert_eq!(ids, AgentId::ALL);
    }

    #[test]
    fn opencode_uses_the_canonical_json_config() {
        let home = tempfile::tempdir().expect("temporary home");
        let config = home.path().join(".config/opencode/opencode.json");
        std::fs::create_dir_all(config.parent().expect("config parent"))
            .expect("OpenCode directory");
        std::fs::write(&config, "{}\n").expect("OpenCode config");

        let agent = detect_agents_in(home.path())
            .into_iter()
            .find(|agent| agent.id == AgentId::OpenCode)
            .expect("OpenCode detection");
        assert_eq!(agent.mcp_config_path, config);
    }
}
