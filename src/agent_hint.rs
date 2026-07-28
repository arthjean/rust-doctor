//! Once-per-project setup hint for supported coding-agent environments.

use crate::cli::Cli;
use crate::diagnostics::{ReportOutcome, ReportV1, ScanMode};
use crate::discovery::ProjectInfo;
use crate::setup::{AgentId, detect_agents_in};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

const STATE_DIRECTORY: &str = "agent-hints";
const MARKER_CONTENT: &[u8] = b"shown-v1\n";
const HINT_LINES: &[&str] = &[
    "Rust Doctor is not installed for this coding agent.",
    "Ask the user if they would like to set it up, then run:",
    "",
    "  npx rust-doctor@latest install --yes",
    "",
    "This adds Rust Doctor skill files for detected coding agents.",
];

#[derive(Clone, Copy)]
struct HintEligibility {
    completed_normal_scan: bool,
    machine_output: bool,
    staged: bool,
    active_agent: Option<AgentId>,
    integration_installed: bool,
    marker_recorded: bool,
}

impl HintEligibility {
    const fn should_show(self) -> bool {
        self.completed_normal_scan
            && !self.machine_output
            && !self.staged
            && self.active_agent.is_some()
            && !self.integration_installed
            && !self.marker_recorded
    }
}

/// Print the Rust Doctor install hint when the completed scan qualifies.
///
/// Detection and persistence are best-effort. Missing or unreadable state
/// intentionally allows the hint to appear again and never fails the scan.
pub fn emit_if_eligible(cli: &Cli, report: &ReportV1, project: &ProjectInfo) {
    let active_agent = detect_active_agent();
    let home = home_directory();
    let state_root = crate::telemetry::config_root().ok();
    let mut write_line = |line: &str| println!("{line}");

    let _ = emit_if_eligible_with(
        cli,
        report_completed_normally(report),
        report.mode == ScanMode::Staged,
        &project.root_dir,
        active_agent,
        home.as_deref(),
        state_root.as_deref(),
        &mut write_line,
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "injected environment, state, and writer keep hint eligibility deterministic in tests"
)]
fn emit_if_eligible_with(
    cli: &Cli,
    completed_normal_scan: bool,
    report_is_staged: bool,
    project_root: &Path,
    active_agent: Option<AgentId>,
    home: Option<&Path>,
    state_root: Option<&Path>,
    write_line: &mut dyn FnMut(&str),
) -> bool {
    let Some(canonical_root) = project_root.canonicalize().ok() else {
        return false;
    };
    let integration_installed = active_agent
        .zip(home)
        .is_some_and(|(agent, home)| active_agent_has_integration(home, agent));
    let marker_recorded =
        state_root.is_some_and(|state_root| marker_is_recorded(state_root, &canonical_root));
    let eligibility = HintEligibility {
        completed_normal_scan,
        machine_output: cli.score
            || cli.wants_json()
            || cli.sarif
            || cli.mcp
            || cli.lsp
            || cli.command.is_some(),
        staged: cli.staged || report_is_staged,
        active_agent,
        integration_installed,
        marker_recorded,
    };
    if !eligibility.should_show() {
        return false;
    }

    write_line("");
    for line in HINT_LINES {
        write_line(line);
    }
    if let Some(state_root) = state_root {
        let _ = record_marker(state_root, &canonical_root);
    }
    true
}

const fn report_completed_normally(report: &ReportV1) -> bool {
    scan_completed(
        report.report_constructed,
        report.error.is_some(),
        matches!(report.outcome, ReportOutcome::NothingToScan),
    )
}

const fn scan_completed(report_constructed: bool, has_error: bool, nothing_to_scan: bool) -> bool {
    report_constructed && !has_error && !nothing_to_scan
}

fn detect_active_agent() -> Option<AgentId> {
    detect_active_agent_with(|name| std::env::var_os(name))
}

fn detect_active_agent_with(mut read: impl FnMut(&str) -> Option<OsString>) -> Option<AgentId> {
    const MARKERS: &[(&str, AgentId)] = &[
        ("CLAUDECODE", AgentId::Claude),
        ("CLAUDE_CODE", AgentId::Claude),
        ("CURSOR_AGENT", AgentId::Cursor),
        ("CODEX_CI", AgentId::Codex),
        ("CODEX_SANDBOX", AgentId::Codex),
        ("CODEX_SANDBOX_NETWORK_DISABLED", AgentId::Codex),
        ("OPENCODE", AgentId::OpenCode),
    ];
    MARKERS.iter().find_map(|(name, agent)| {
        read(name)
            .is_some_and(|value| !value.is_empty())
            .then_some(*agent)
    })
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn active_agent_has_integration(home: &Path, active_agent: AgentId) -> bool {
    detect_agents_in(home)
        .into_iter()
        .find(|agent| agent.id == active_agent)
        .is_some_and(|agent| agent.skill_already_installed)
}

fn project_key(canonical_root: &Path) -> String {
    let digest = Sha256::digest(canonical_root.to_string_lossy().as_bytes());
    let mut key = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(key, "{byte:02x}");
    }
    key
}

fn marker_path(state_root: &Path, canonical_root: &Path) -> PathBuf {
    state_root
        .join(STATE_DIRECTORY)
        .join(project_key(canonical_root))
}

fn marker_is_recorded(state_root: &Path, canonical_root: &Path) -> bool {
    std::fs::read(marker_path(state_root, canonical_root))
        .is_ok_and(|content| content == MARKER_CONTENT)
}

fn record_marker(state_root: &Path, canonical_root: &Path) -> std::io::Result<()> {
    let directory = state_root.join(STATE_DIRECTORY);
    std::fs::create_dir_all(&directory)?;

    let destination = marker_path(state_root, canonical_root);
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(MARKER_CONTENT)?;
    temporary.as_file_mut().sync_all()?;

    match temporary.persist_noclobber(destination) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::collections::BTreeMap;

    fn cli(arguments: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("rust-doctor").chain(arguments.iter().copied()))
            .unwrap()
    }

    fn emitted_lines(
        cli: &Cli,
        completed: bool,
        staged: bool,
        project_root: &Path,
        agent: Option<AgentId>,
        home: Option<&Path>,
        state_root: Option<&Path>,
    ) -> (bool, Vec<String>) {
        let mut lines = Vec::new();
        let emitted = emit_if_eligible_with(
            cli,
            completed,
            staged,
            project_root,
            agent,
            home,
            state_root,
            &mut |line| lines.push(line.to_string()),
        );
        (emitted, lines)
    }

    #[test]
    fn pure_gate_requires_every_eligibility_condition() {
        let eligible = HintEligibility {
            completed_normal_scan: true,
            machine_output: false,
            staged: false,
            active_agent: Some(AgentId::Codex),
            integration_installed: false,
            marker_recorded: false,
        };
        assert!(eligible.should_show());

        assert!(
            !HintEligibility {
                completed_normal_scan: false,
                ..eligible
            }
            .should_show()
        );
        assert!(
            !HintEligibility {
                machine_output: true,
                ..eligible
            }
            .should_show()
        );
        assert!(
            !HintEligibility {
                staged: true,
                ..eligible
            }
            .should_show()
        );
        assert!(
            !HintEligibility {
                active_agent: None,
                ..eligible
            }
            .should_show()
        );
        assert!(
            !HintEligibility {
                integration_installed: true,
                ..eligible
            }
            .should_show()
        );
        assert!(
            !HintEligibility {
                marker_recorded: true,
                ..eligible
            }
            .should_show()
        );
    }

    #[test]
    fn any_constructed_error_free_report_counts_as_a_completed_scan() {
        assert!(scan_completed(true, false, false));
        assert!(!scan_completed(false, false, false));
        assert!(!scan_completed(true, true, false));
        assert!(!scan_completed(true, false, true));
    }

    #[test]
    fn detects_only_supported_active_agent_markers() {
        let variables = BTreeMap::from([
            ("CODEX_SANDBOX", OsString::from("1")),
            ("OPENCODE", OsString::from("1")),
        ]);
        assert_eq!(
            detect_active_agent_with(|name| variables.get(name).cloned()),
            Some(AgentId::Codex)
        );

        let unsupported = BTreeMap::from([("GOOSE_TERMINAL", OsString::from("1"))]);
        assert_eq!(
            detect_active_agent_with(|name| unsupported.get(name).cloned()),
            None
        );
    }

    #[test]
    fn prints_exact_hint_once_and_persists_only_a_project_hash() {
        let fixture = tempfile::tempdir().unwrap();
        let project = fixture.path().join("secret-project-name");
        let home = fixture.path().join("home");
        let state = fixture.path().join("state");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();

        let (emitted, lines) = emitted_lines(
            &cli(&[]),
            true,
            false,
            &project,
            Some(AgentId::Codex),
            Some(&home),
            Some(&state),
        );
        assert!(emitted);
        assert_eq!(lines.first().map(String::as_str), Some(""));
        assert_eq!(&lines[1..], HINT_LINES);

        let markers: Vec<_> = std::fs::read_dir(state.join(STATE_DIRECTORY))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(markers.len(), 1);
        let marker_name = markers[0].file_name().unwrap().to_string_lossy();
        assert_eq!(marker_name.len(), 64);
        assert!(!marker_name.contains("secret-project-name"));
        assert_eq!(std::fs::read(&markers[0]).unwrap(), MARKER_CONTENT);

        let (emitted_again, lines_again) = emitted_lines(
            &cli(&[]),
            true,
            false,
            &project,
            Some(AgentId::Codex),
            Some(&home),
            Some(&state),
        );
        assert!(!emitted_again);
        assert!(lines_again.is_empty());
    }

    #[test]
    fn active_agent_integration_suppresses_unrelated_agent_state() {
        let fixture = tempfile::tempdir().unwrap();
        let project = fixture.path().join("project");
        let home = fixture.path().join("home");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(home.join(".codex/skills/rust-doctor")).unwrap();
        std::fs::write(
            home.join(".codex/skills/rust-doctor/SKILL.md"),
            b"installed\n",
        )
        .unwrap();

        let (codex_emitted, _) = emitted_lines(
            &cli(&[]),
            true,
            false,
            &project,
            Some(AgentId::Codex),
            Some(&home),
            None,
        );
        assert!(!codex_emitted);

        let (claude_emitted, _) = emitted_lines(
            &cli(&[]),
            true,
            false,
            &project,
            Some(AgentId::Claude),
            Some(&home),
            None,
        );
        assert!(claude_emitted);
    }

    #[test]
    fn mcp_configuration_without_the_installed_skill_does_not_suppress() {
        let fixture = tempfile::tempdir().unwrap();
        let project = fixture.path().join("project");
        let home = fixture.path().join("home");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex/config.toml"),
            "[mcp_servers.rust-doctor]\ncommand = \"rust-doctor\"\n",
        )
        .unwrap();

        let (emitted, _) = emitted_lines(
            &cli(&[]),
            true,
            false,
            &project,
            Some(AgentId::Codex),
            Some(&home),
            None,
        );
        assert!(emitted);
    }

    #[test]
    fn machine_and_staged_modes_never_emit() {
        let fixture = tempfile::tempdir().unwrap();
        let project = fixture.path().join("project");
        let home = fixture.path().join("home");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&home).unwrap();

        for arguments in [
            &["--json"][..],
            &["--score"][..],
            &["--sarif"][..],
            &["--staged"][..],
        ] {
            let (emitted, lines) = emitted_lines(
                &cli(arguments),
                true,
                false,
                &project,
                Some(AgentId::Codex),
                Some(&home),
                None,
            );
            assert!(!emitted);
            assert!(lines.is_empty());
        }
    }

    #[test]
    fn unreadable_marker_state_shows_again_without_overwriting_it() {
        let fixture = tempfile::tempdir().unwrap();
        let project = fixture.path().join("project");
        let home = fixture.path().join("home");
        let state = fixture.path().join("state");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let marker = marker_path(&state, &project.canonicalize().unwrap());
        std::fs::create_dir_all(&marker).unwrap();

        for _ in 0..2 {
            let (emitted, lines) = emitted_lines(
                &cli(&[]),
                true,
                false,
                &project,
                Some(AgentId::Codex),
                Some(&home),
                Some(&state),
            );
            assert!(emitted);
            assert!(!lines.is_empty());
        }
        assert!(marker.is_dir());
    }
}
