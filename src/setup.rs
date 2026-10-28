//! `rust-doctor setup`: the skill and the end-of-turn hooks for the agents
//! this machine and this workspace actually have, and the git pre-commit
//! hook, in one command.
//!
//! An agent is detected by its command on `PATH`, the one the handoff
//! launches, or by its project directory in the workspace. Every step reuses
//! its own installer and that installer's refusals, and every step is checked
//! as a dry run before any is written, so a refusal of the last leaves the
//! first unwritten.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;

use crate::handoff::{AvailableAgent, available_agents};
use crate::hook::{AgentHook, ClippyArgs, WriteArgs, install_agent, install_git};
use crate::skill::{self, Agent, AgentSelection, InstallOptions};

/// What detection looks at, named when it finds nothing.
pub const PROBED: &str = "the `claude`, `codex` and `cursor-agent` commands on PATH, and the \
                          .claude/, .agents/, .codex/ and .cursor/ directories of the workspace";

/// The project directory each agent reads.
const PROJECT_DIRECTORIES: [(&str, Agent); 4] = [
    (".claude", Agent::Claude),
    (".agents", Agent::Codex),
    (".codex", Agent::Codex),
    (".cursor", Agent::Cursor),
];

#[derive(Debug, Clone, Args)]
pub struct SetupArgs {
    #[arg(default_value = ".", value_name = "PATH")]
    path: PathBuf,
    /// Print the plan, and write nothing.
    #[arg(long)]
    dry_run: bool,
}

/// The agents whose command is on `PATH` or whose project directory exists,
/// in the order the skill table lists them.
pub fn detected_agents(workspace_root: &Path) -> Vec<Agent> {
    let on_path: Vec<Agent> = available_agents()
        .iter()
        .map(AvailableAgent::skill_agent)
        .collect();
    [Agent::Claude, Agent::Codex, Agent::Cursor]
        .into_iter()
        .filter(|agent| {
            on_path.contains(agent)
                || PROJECT_DIRECTORIES
                    .iter()
                    .any(|(directory, owner)| owner == agent && workspace_root.join(directory).is_dir())
        })
        .collect()
}

pub fn run(arguments: &SetupArgs) -> ExitCode {
    let root = &match crate::path_argument::resolve("PATH", &arguments.path) {
        Ok(root) => root,
        Err(code) => return ExitCode::from(code),
    };
    let agents = detected_agents(root);
    if agents.is_empty() {
        eprintln!("rust-doctor: no agent detected, so nothing was set up. Probed {PROBED}.");
        return ExitCode::from(2);
    }
    let names: Vec<&str> = agents.iter().map(|agent| agent.name()).collect();
    println!("Detected: {}.", names.join(", "));
    // Checked, then written: the first pass is every installer's dry run,
    // which is also the plan this prints.
    println!("Plan:");
    let checked = step_all(root, &agents, true);
    if checked != 0 || arguments.dry_run {
        println!("{NO_CI}");
        return ExitCode::from(checked);
    }
    println!("Writing:");
    let written = step_all(root, &agents, false);
    println!("{NO_CI}");
    ExitCode::from(written)
}

const NO_CI: &str =
    "setup never adds a CI workflow: run `rust-doctor ci install` for GitHub Actions.";

/// Every step in order, stopping at the first that does not succeed.
fn step_all(root: &Path, agents: &[Agent], dry_run: bool) -> u8 {
    let write = WriteArgs {
        update: true,
        dry_run,
    };
    let skills = skill::install_and_print(
        root,
        AgentSelection::Detected,
        InstallOptions {
            update: true,
            dry_run,
        },
    );
    if skills != 0 {
        return skills;
    }
    let git = install_git(root, write);
    if git != 0 {
        return git;
    }
    for hook in agents.iter().filter_map(|agent| match agent {
        Agent::Claude => Some(AgentHook::Claude),
        Agent::Cursor => Some(AgentHook::Cursor),
        Agent::Codex => None,
    }) {
        let status = install_agent(hook, root, false, ClippyArgs { no_clippy: false }, write);
        if status != 0 {
            return status;
        }
    }
    0
}
