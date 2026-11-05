mod agent;

pub(crate) use agent::{AgentHook, install as install_agent};

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use rust_doctor::git_hook::{self, HookSetup};
use rust_doctor::terminal_text::sanitize;
use rust_doctor::workspace_write::{create_new, on_path, symlinked_component};


#[derive(Debug, Clone, Args)]
#[command(
    about = "The hooks that turn a rescan into a habit. Only `install` writes, and only what it prints"
)]
pub struct HookArgs {
    #[command(subcommand)]
    command: HookCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum HookCommand {
    #[command(about = "Install a git pre-commit hook or an agent's end-of-turn hook")]
    Install {
        #[command(subcommand)]
        target: HookTarget,
    },
    #[command(
        about = "Run an agent's end-of-turn hook: rescan the lines the turn changed",
        long_about = format!(
            "Run an agent's end-of-turn hook: rescan the lines the turn changed.\n\n{}",
            crate::TRUST_WARNING
        )
    )]
    Run {
        #[arg(value_enum)]
        agent: AgentHook,
        #[command(flatten)]
        clippy: ClippyArgs,
    },
}

#[derive(Debug, Clone, Copy, Args)]
#[command(about = "Whether an agent hook compiles the tree its agent wrote")]
pub(crate) struct ClippyArgs {
    #[arg(
        long,
        help = "Turn every Clippy rule off, whatever rust-doctor.toml says: the hook then compiles nothing, so no build script or procedural macro of the tree the agent wrote runs under your account"
    )]
    pub(crate) no_clippy: bool,
}

#[derive(Debug, Clone, Subcommand)]
enum HookTarget {
    #[command(
        about = "Write a pre-commit hook running `rust-doctor . --yes --staged --scope lines --blocking warning`"
    )]
    Git {
        #[arg(default_value = ".", value_name = "PATH")]
        path: PathBuf,
        #[command(flatten)]
        write: WriteArgs,
    },
    #[command(about = "Add a Stop hook to .claude/settings.local.json")]
    Claude {
        #[arg(default_value = ".", value_name = "PATH")]
        path: PathBuf,
        #[arg(long, help = "Write .claude/settings.json, the file the repository shares")]
        shared: bool,
        #[command(flatten)]
        clippy: ClippyArgs,
        #[command(flatten)]
        write: WriteArgs,
    },
    #[command(about = "Add a stop hook to .cursor/hooks.json")]
    Cursor {
        #[arg(default_value = ".", value_name = "PATH")]
        path: PathBuf,
        #[command(flatten)]
        clippy: ClippyArgs,
        #[command(flatten)]
        write: WriteArgs,
    },
}

#[derive(Debug, Clone, Copy, Args)]
#[command(about = "How an install treats what it would write")]
pub(crate) struct WriteArgs {
    #[arg(
        long,
        help = "Rewrite a hook rust-doctor installed before that differs from the current one"
    )]
    pub(crate) update: bool,
    #[arg(long, help = "Print what would be written, and write nothing")]
    pub(crate) dry_run: bool,
}

pub fn run(arguments: &HookArgs) -> ExitCode {
    match &arguments.command {
        HookCommand::Run { agent, clippy } => agent::run(*agent, *clippy),
        HookCommand::Install { target } => {
            let (HookTarget::Git { path, .. }
            | HookTarget::Claude { path, .. }
            | HookTarget::Cursor { path, .. }) = target;
            let path = match crate::path_argument::resolve("PATH", path) {
                Ok(path) => path,
                Err(code) => return ExitCode::from(code),
            };
            ExitCode::from(match target {
                HookTarget::Git { write, .. } => install_git(&path, *write),
                HookTarget::Claude { shared, clippy, write, .. } => {
                    agent::install(AgentHook::Claude, &path, *shared, *clippy, *write)
                }
                HookTarget::Cursor { clippy, write, .. } => {
                    agent::install(AgentHook::Cursor, &path, false, *clippy, *write)
                }
            })
        }
    }
}

const MARKER: &str = "# rust-doctor pre-commit hook";

const FAILED: u8 = 2;

fn pre_commit_command(workspace: &str) -> String {
    format!(
        "rust-doctor {} --yes --staged --scope lines --blocking warning",
        shell_word(workspace)
    )
}

fn shell_word(word: &str) -> String {
    if word
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"./_-".contains(&byte))
    {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

fn hook_script(command: &str) -> String {
    format!(
        "#!/bin/sh\n{MARKER}: judges what this commit records.\n# Written by `rust-doctor hook install git`; remove this file to uninstall.\nif ! command -v rust-doctor >/dev/null 2>&1; then\n  echo \"rust-doctor is not on PATH: this commit was not judged.\" >&2\n  exit 0\nfi\n{command}\nstatus=$?\nif [ \"$status\" -eq 2 ]; then\n  echo \"rust-doctor could not scan: this commit was not judged.\" >&2\n  exit 0\nfi\nexit \"$status\"\n"
    )
}

fn script_command(script: &str) -> &str {
    script
        .lines()
        .map(str::trim_start)
        .find_map(|line| {
            let command = line.strip_prefix("exec ").unwrap_or(line);
            command.starts_with("rust-doctor ").then_some(command)
        })
        .unwrap_or("(none)")
}

pub(crate) fn install_git(workspace: &Path, write: WriteArgs) -> u8 {
    let WriteArgs { update, dry_run } = write;
    let Some(location) = git_hook::locate(workspace) else {
        eprintln!("rust-doctor: not a git repository, so no pre-commit hook was installed.");
        return FAILED;
    };
    let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let confined = match git_hook::confine(workspace, &location.toplevel) {
        Ok(confined) => confined,
        Err(refusal) => {
            eprintln!("rust-doctor: refusing to install: {refusal}.");
            return FAILED;
        }
    };
    let relative = confined.workspace.to_string_lossy().into_owned();
    let toplevel = confined.toplevel;
    if relative.chars().any(char::is_control) {
        eprintln!(
            "rust-doctor: refusing to install: the workspace's path from the repository's top holds a control character."
        );
        return FAILED;
    }
    let command = pre_commit_command(if relative.is_empty() { "." } else { &relative });
    let directory = match location.setup {
        HookSetup::Manager(config) => {
            println!("{config} runs this repository's hooks, and rust-doctor never edits it.");
            println!(
                "Add this hook to it:\n\n{}",
                manager_snippet(config, &command)
            );
            return 0;
        }
        HookSetup::HooksPath(directory)
        | HookSetup::CargoHusky(directory)
        | HookSetup::Default(directory) => canonical(&directory),
    };
    let target = directory.join("pre-commit");
    let Ok(shown) = target.strip_prefix(&toplevel) else {
        println!("The hooks directory is outside this working tree, so nothing was written.");
        println!("Add this line to its pre-commit hook:\n\n{command}");
        return 1;
    };
    if let Some(link) = symlinked_component(&toplevel, shown) {
        eprintln!(
            "rust-doctor: {} is a symlink, and nothing is written through one.",
            sanitize(&link.display().to_string())
        );
        return FAILED;
    }
    let shown = sanitize(&shown.display().to_string());
    let script = hook_script(&command);
    let mut replaced = false;
    if target.symlink_metadata().is_ok() {
        let existing = fs::read_to_string(&target).unwrap_or_default();
        if !existing.contains(MARKER) {
            println!("{shown} exists and was not written by rust-doctor, so nothing was written.");
            println!("Add this line to it:\n\n{command}");
            return if update {
                FAILED
            } else {
                1
            };
        }
        if existing == script {
            println!("The rust-doctor pre-commit hook is already installed in {shown}.");
            return 0;
        }
        if !update {
            println!(
                "{shown} holds an outdated rust-doctor pre-commit hook: pass --update to rewrite it."
            );
            return 1;
        }
        println!(
            "{} {shown}: `{}` becomes `{command}`.",
            if dry_run { "Would update" } else { "Updating" },
            sanitize(script_command(&existing))
        );
        replaced = true;
    }
    if dry_run {
        print!("Would write {shown}:\n\n{script}");
        return 0;
    }
    let written = if replaced {
        fs::write(&target, script.as_bytes())
    } else {
        create_new(&target, script.as_bytes())
    };
    if let Err(error) = written.and_then(|()| make_executable(&target)) {
        eprintln!("rust-doctor: {shown} could not be written: {error}.");
        return FAILED;
    }
    print!("Wrote {shown}:\n\n{script}");
    if !on_path("rust-doctor") {
        eprintln!("Warning: `rust-doctor` is not on PATH, and the hook runs it by name.");
    }
    0
}

fn manager_snippet(config: &str, command: &str) -> String {
    let mut snippet = String::new();
    if config.contains("lefthook") {
        let _ = write!(
            snippet,
            "pre-commit:\n  commands:\n    rust-doctor:\n      run: {command}\n"
        );
    } else {
        let _ = write!(
            snippet,
            "- repo: local\n  hooks:\n    - id: rust-doctor\n      name: rust-doctor\n      entry: {command}\n      language: system\n      pass_filenames: false\n"
        );
    }
    snippet
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_runs_a_staged_lines_scan_of_the_workspace() {
        let command = "rust-doctor . --yes --staged --scope lines --blocking warning";
        assert_eq!(pre_commit_command("."), command);
        assert_eq!(
            pre_commit_command("crates/my app"),
            "rust-doctor 'crates/my app' --yes --staged --scope lines --blocking warning"
        );
        let script = hook_script(&pre_commit_command("."));
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains(MARKER));
        assert!(script.contains(&format!("\nfi\n{command}\nstatus=$?\n")));
        assert_eq!(script_command(&script), command);
        assert_eq!(
            script_command(&format!("#!/bin/sh\n{MARKER}\nexec rust-doctor .\n")),
            "rust-doctor ."
        );
        for config in [".pre-commit-config.yaml", "lefthook.yml", ".lefthook.yaml"] {
            assert!(manager_snippet(config, "rust-doctor .").contains("rust-doctor ."));
        }
        assert!(manager_snippet(".lefthook.yaml", "rust-doctor .").starts_with("pre-commit:"));
    }
}
