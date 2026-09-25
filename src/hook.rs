//! The hooks that make a rescan a habit: a git pre-commit hook that judges what
//! the commit records, and an end-of-turn hook that keeps an agent from
//! declaring its work done over findings it introduced.
//!
//! Only `hook install` writes, only under the workspace it is given (or, for a
//! git hook, where the repository runs its hooks), only the files it prints,
//! and never over a file rust-doctor did not write: the one exception is an
//! agent's settings file, into which it merges one entry and keeps every other
//! key. A scan never installs anything.

mod agent;

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use rust_doctor::git_hook::{self, HookSetup};

use crate::workspace_write::{create_new, on_path, symlinked_component};

pub use agent::{AgentHook, install as install_agent, run as run_agent_hook};

/// The comment every file rust-doctor writes as a hook carries, and the one a
/// second install recognizes.
const MARKER: &str = "# rust-doctor pre-commit hook";

/// Exit code of an install that could not run at all: not a repository, an
/// unreadable settings file, a failed write.
const FAILED: u8 = 2;

/// The command a pre-commit hook runs, with the workspace spelled relative to
/// the top of the working tree, where git starts it. No `--blocking` is passed,
/// so the workspace's configured level decides.
fn pre_commit_command(workspace: &str) -> String {
    format!(
        "rust-doctor {} --yes --staged --scope lines",
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
        "#!/bin/sh\n{MARKER}: judges what this commit records.\n# Written by `rust-doctor hook install git`; remove this file to uninstall.\nexec {command}\n"
    )
}

/// `rust-doctor hook install git`.
pub fn install_git(workspace: &Path, dry_run: bool) -> ExitCode {
    let Some(location) = git_hook::locate(workspace) else {
        eprintln!("rust-doctor: not a git repository, so no pre-commit hook was installed.");
        return ExitCode::from(FAILED);
    };
    let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let toplevel = canonical(&location.toplevel);
    let relative = canonical(workspace)
        .strip_prefix(&toplevel)
        .map(|relative| relative.to_string_lossy().into_owned())
        .unwrap_or_default();
    let command = pre_commit_command(if relative.is_empty() { "." } else { &relative });
    let directory = match location.setup {
        HookSetup::Manager(config) => {
            println!("{config} runs this repository's hooks, and rust-doctor never edits it.");
            println!(
                "Add this hook to it:\n\n{}",
                manager_snippet(config, &command)
            );
            return ExitCode::SUCCESS;
        }
        HookSetup::HooksPath(directory)
        | HookSetup::CargoHusky(directory)
        | HookSetup::Default(directory) => canonical(&directory),
    };
    let target = directory.join("pre-commit");
    // A hooks directory outside the working tree is shared: with other
    // repositories through a global `core.hooksPath`, or with the main working
    // tree of a linked one. This command writes only inside the tree it was
    // pointed at.
    let Ok(shown) = target.strip_prefix(&toplevel) else {
        println!("The hooks directory is outside this working tree, so nothing was written.");
        println!("Add this line to its pre-commit hook:\n\n{command}");
        return ExitCode::FAILURE;
    };
    if let Some(link) = symlinked_component(&toplevel, shown) {
        eprintln!(
            "rust-doctor: {} is a symlink, and nothing is written through one.",
            link.display()
        );
        return ExitCode::from(FAILED);
    }
    let shown = shown.display();
    if target.symlink_metadata().is_ok() {
        let existing = fs::read_to_string(&target).unwrap_or_default();
        if existing.contains(MARKER) {
            println!("The rust-doctor pre-commit hook is already installed in {shown}.");
            return ExitCode::SUCCESS;
        }
        println!("{shown} exists and was not written by rust-doctor, so nothing was written.");
        println!("Add this line to it:\n\n{command}");
        return ExitCode::FAILURE;
    }
    let script = hook_script(&command);
    if dry_run {
        print!("Would write {shown}:\n\n{script}");
        return ExitCode::SUCCESS;
    }
    if let Err(error) =
        create_new(&target, script.as_bytes()).and_then(|()| make_executable(&target))
    {
        eprintln!("rust-doctor: {shown} could not be written: {error}.");
        return ExitCode::from(FAILED);
    }
    print!("Wrote {shown}:\n\n{script}");
    if !on_path("rust-doctor") {
        eprintln!("Warning: `rust-doctor` is not on PATH, and the hook runs it by name.");
    }
    ExitCode::SUCCESS
}

/// The exact entry for a hook manager's own file, printed rather than merged:
/// no YAML is edited.
fn manager_snippet(config: &str, command: &str) -> String {
    let mut snippet = String::new();
    if config == "lefthook.yml" {
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
        assert_eq!(
            pre_commit_command("."),
            "rust-doctor . --yes --staged --scope lines"
        );
        assert_eq!(
            pre_commit_command("crates/my app"),
            "rust-doctor 'crates/my app' --yes --staged --scope lines"
        );
        let script = hook_script(&pre_commit_command("."));
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains(MARKER));
        assert!(script.ends_with("exec rust-doctor . --yes --staged --scope lines\n"));
        for config in [".pre-commit-config.yaml", "lefthook.yml"] {
            assert!(manager_snippet(config, "rust-doctor .").contains("rust-doctor ."));
        }
    }
}
