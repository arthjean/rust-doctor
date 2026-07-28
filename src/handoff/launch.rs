use crate::cli::HandoffTarget;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

struct AgentSpec {
    target: HandoffTarget,
    label: &'static str,
    binary: &'static str,
}

const AGENT_SPECS: &[AgentSpec] = &[
    AgentSpec {
        target: HandoffTarget::ClaudeCode,
        label: "Claude Code",
        binary: "claude",
    },
    AgentSpec {
        target: HandoffTarget::Codex,
        label: "Codex",
        binary: "codex",
    },
    AgentSpec {
        target: HandoffTarget::Cursor,
        label: "Cursor",
        binary: "cursor-agent",
    },
];

struct ResolvedCommand {
    program: PathBuf,
    leading_arguments: Vec<OsString>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum DeliveryError {
    #[error("{0} is not a launchable handoff target")]
    UnsupportedTarget(HandoffTarget),
    #[error("{0} is not available on PATH")]
    Unavailable(&'static str),
    #[error("failed to launch {binary}: {source}")]
    Launch {
        binary: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("clipboard delivery failed: {0}")]
    Clipboard(String),
}

pub(super) fn launchable_targets() -> Vec<(String, HandoffTarget)> {
    AGENT_SPECS
        .iter()
        .filter(|specification| resolve_agent_command(specification).is_some())
        .map(|specification| (specification.label.to_string(), specification.target))
        .collect()
}

pub(super) fn binary_name(target: HandoffTarget) -> Option<&'static str> {
    AGENT_SPECS
        .iter()
        .find(|specification| specification.target == target)
        .map(|specification| specification.binary)
}

pub(super) fn launch_agent(
    target: HandoffTarget,
    prompt: &str,
    working_directory: &Path,
) -> Result<ExitStatus, DeliveryError> {
    let specification = AGENT_SPECS
        .iter()
        .find(|specification| specification.target == target)
        .ok_or(DeliveryError::UnsupportedTarget(target))?;
    let resolved = resolve_agent_command(specification)
        .ok_or(DeliveryError::Unavailable(specification.binary))?;
    run_resolved_command(&resolved, prompt, working_directory).map_err(|source| {
        DeliveryError::Launch {
            binary: specification.binary,
            source,
        }
    })
}

fn run_resolved_command(
    resolved: &ResolvedCommand,
    prompt: &str,
    working_directory: &Path,
) -> Result<ExitStatus, std::io::Error> {
    Command::new(&resolved.program)
        .args(&resolved.leading_arguments)
        .arg(prompt)
        .current_dir(working_directory)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
}

pub(super) fn copy_to_clipboard(content: &str) -> Result<(), DeliveryError> {
    let commands: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };
    let mut failures = Vec::new();
    for (binary, arguments) in commands {
        let Some(program) = find_executable(binary) else {
            failures.push(format!("{binary}: not found"));
            continue;
        };
        let mut child = match Command::new(program)
            .args(*arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                failures.push(format!("{binary}: {error}"));
                continue;
            }
        };
        let Some(mut stdin) = child.stdin.take() else {
            failures.push(format!("{binary}: stdin unavailable"));
            let _ = child.kill();
            let _ = child.wait();
            continue;
        };
        let write_result = stdin.write_all(content.as_bytes());
        drop(stdin);
        let status = child.wait();
        if write_result.is_ok() && status.is_ok_and(|status| status.success()) {
            return Ok(());
        }
        failures.push(format!("{binary}: delivery failed"));
    }
    Err(DeliveryError::Clipboard(failures.join("; ")))
}

#[cfg(not(target_os = "windows"))]
fn resolve_agent_command(specification: &AgentSpec) -> Option<ResolvedCommand> {
    find_executable(specification.binary).map(|program| ResolvedCommand {
        program,
        leading_arguments: Vec::new(),
    })
}

#[cfg(target_os = "windows")]
fn resolve_agent_command(specification: &AgentSpec) -> Option<ResolvedCommand> {
    if specification.target == HandoffTarget::Cursor
        && let Some(resolved) = resolve_cursor_bundled_node()
    {
        return Some(resolved);
    }
    let program = find_executable(specification.binary)?;
    let extension = program
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "cmd" | "bat") {
        return resolve_windows_node_wrapper(&program);
    }
    Some(ResolvedCommand {
        program,
        leading_arguments: Vec::new(),
    })
}

fn find_executable(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_executable_in(binary, &path)
}

fn find_executable_in(binary: &str, path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .flat_map(|directory| executable_candidates(&directory, binary))
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(not(target_os = "windows"))]
fn executable_candidates(directory: &Path, binary: &str) -> Vec<PathBuf> {
    vec![directory.join(binary)]
}

#[cfg(target_os = "windows")]
fn executable_candidates(directory: &Path, binary: &str) -> Vec<PathBuf> {
    if Path::new(binary).extension().is_some() {
        return vec![directory.join(binary)];
    }
    let extensions =
        std::env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
    extensions
        .to_string_lossy()
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(|extension| directory.join(format!("{binary}{extension}")))
        .collect()
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(target_os = "windows")]
fn resolve_windows_node_wrapper(wrapper: &Path) -> Option<ResolvedCommand> {
    let content = std::fs::read_to_string(wrapper).ok()?;
    let parent = wrapper.parent()?;
    let script = content.split('"').find_map(|token| {
        let token = token.trim();
        let relative = token
            .strip_prefix("%~dp0\\")
            .or_else(|| token.strip_prefix("%dp0%\\"))?;
        let extension = Path::new(relative)
            .extension()
            .and_then(OsStr::to_str)?
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "js" | "mjs" | "cjs") {
            return None;
        }
        let normalized = relative.replace('\\', std::path::MAIN_SEPARATOR_STR);
        let candidate = parent.join(normalized);
        candidate.is_file().then_some(candidate)
    })?;
    let node = find_executable("node")?;
    Some(ResolvedCommand {
        program: node,
        leading_arguments: vec![script.into_os_string()],
    })
}

#[cfg(target_os = "windows")]
fn resolve_cursor_bundled_node() -> Option<ResolvedCommand> {
    let versions = PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
        .join("cursor-agent")
        .join("versions");
    let mut directories: Vec<_> = std::fs::read_dir(versions)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    directories.sort_by(|left, right| {
        version_key(&right.file_name()).cmp(&version_key(&left.file_name()))
    });
    directories.into_iter().find_map(|entry| {
        let node = entry.path().join("node.exe");
        let script = entry.path().join("index.js");
        (node.is_file() && script.is_file()).then_some(ResolvedCommand {
            program: node,
            leading_arguments: vec![script.into_os_string()],
        })
    })
}

#[cfg(target_os = "windows")]
fn version_key(value: &OsStr) -> Vec<u64> {
    value
        .to_string_lossy()
        .split(|character: char| !character.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_contract_matches_react_doctor_order_without_approval_bypasses() {
        let targets: Vec<_> = AGENT_SPECS
            .iter()
            .map(|specification| {
                (
                    specification.target,
                    specification.label,
                    specification.binary,
                )
            })
            .collect();
        assert_eq!(
            targets,
            vec![
                (HandoffTarget::ClaudeCode, "Claude Code", "claude"),
                (HandoffTarget::Codex, "Codex", "codex"),
                (HandoffTarget::Cursor, "Cursor", "cursor-agent"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_resolution_requires_an_executable_regular_file() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("codex");
        std::fs::write(&binary, "#!/bin/sh\n").unwrap();
        let path = std::env::join_paths([directory.path()]).unwrap();
        assert!(find_executable_in("codex", &path).is_none());

        let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&binary, permissions).unwrap();
        assert_eq!(find_executable_in("codex", &path), Some(binary));
    }

    #[cfg(unix)]
    #[test]
    fn resolved_agent_receives_only_the_prompt_and_runs_in_the_project() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("agent");
        let capture = directory.path().join("capture");
        std::fs::write(
            &script,
            "#!/bin/sh\ncapture=$1\nshift\nprintf '%s\\n%s\\n%s\\n' \"$#\" \"$1\" \"$PWD\" > \"$capture\"\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();
        let resolved = ResolvedCommand {
            program: script,
            leading_arguments: vec![capture.clone().into_os_string()],
        };

        let status = run_resolved_command(&resolved, "Fix the issue.", directory.path()).unwrap();

        assert!(status.success());
        let captured = std::fs::read_to_string(capture).unwrap();
        let canonical_directory = directory.path().canonicalize().unwrap();
        assert_eq!(
            captured,
            format!("1\nFix the issue.\n{}\n", canonical_directory.display())
        );
    }
}
