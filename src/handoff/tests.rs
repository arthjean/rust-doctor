//! Tests for the agent handoff.
//!
//! They live in a file of their own so that every file of the module stays
//! under the thousand lines `oversized_unit` reports at.

/// The handoff passes the rule the report it builds from publishes. It was one
/// file of 1032 lines with no such test.
#[test]
fn the_handoff_holds_the_size_bound_the_report_reports_for() {
    for own in [
        include_str!("../handoff.rs"),
        include_str!("tests.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < rust_doctor::OVERSIZED_UNIT_FILE_LINES,
            "a file of the handoff is {lines} lines long, over the {} it reports",
            rust_doctor::OVERSIZED_UNIT_FILE_LINES
        );
    }
}


use super::*;
use crate::test_scratch::scratch;
use rust_doctor::presentation::ReportPresentation;
use rust_doctor::{
    Audit, BlockingLevel, Diagnostic, DiagnosticSource, DiagnosticSpan, GateReport, GateStatus,
    InspectReport, ScanReport, Severity, Status, Summary, ToolchainReport,
};



fn report() -> InspectReport {
    let diagnostics = vec![Diagnostic {
        context: None,
        id: "one".to_owned(),
        source: DiagnosticSource::Clippy,
        code: Some("clippy::todo".to_owned()),
        base_severity: Severity::Warning,
        severity: Severity::Warning,
        category: Some("correctness".to_owned()),
        message: "source is deliberately excluded".to_owned(),
        help: Some("Replace the placeholder.".to_owned()),
        package: None,
        target: None,
        path: Some("src/lib.rs".to_owned()),
        span: Some(DiagnosticSpan {
            line_start: 4,
            column_start: 5,
            line_end: 4,
            column_end: 10,
        }),
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        occurrences: 2,
    }];
    let summary = Summary::from_diagnostics(&diagnostics);
    InspectReport {
        schema_version: rust_doctor::SCHEMA_VERSION,
        audit: Audit::build(1, 100, Status::Complete, &diagnostics),
        status: Status::Complete,
        complete: true,
        policy: None,
        scope: None,
        project: None,
        toolchain: ToolchainReport {
            rustc: None,
            cargo: None,
            clippy: None,
        },
        scan: ScanReport {
            command: None,
            exit_code: Some(0),
            build_finished: Some(true),
            noise_lines: Some(0),
        },
        diagnostics,
        delta: None,
        errors: Vec::new(),
        summary,
        gate: GateReport {
            blocking: BlockingLevel::Error,
            status: GateStatus::Passed,
            blocking_diagnostics: Some(0),
        },
    }
}

fn rescan_command() -> RescanCommand {
    RescanCommand::for_inspection(false, None, &[], &[], None).unwrap()
}

fn payload() -> HandoffPayload {
    let report = report();
    let presentation = ReportPresentation::derive(&report);
    build_prompt(&report, &presentation, &rescan_command()).unwrap()
}

fn issue(sites: &[String]) -> IssuePrompt<'_> {
    IssuePrompt {
        project_name: "workspace",
        rule_id: "clippy::unwrap_used",
        title: "Unwrap used",
        category: "Bugs",
        is_error: false,
        site_count: 12,
        message: "called `unwrap` on a value",
        help: Some("Use `?` or `unwrap_or`."),
        rule_url: "https://rust-doctor.com/rules/clippy",
        sites,
    }
}

fn ten_sites() -> Vec<String> {
    (1..=10).map(|line| format!("src/lib.rs:{line}")).collect()
}

#[test]
fn the_issue_prompt_names_one_rule_bounds_its_sites_and_ends_on_the_rescan() {
    let sites = ten_sites();
    let prompt = build_issue_prompt(&issue(&sites)).unwrap();
    let prompt = prompt.as_str();
    assert!(prompt.starts_with("Fix exactly one Rust Doctor rule in workspace:"));
    assert!(prompt.contains("WARN Bugs: Unwrap used (clippy::unwrap_used, ×12)"));
    assert!(prompt.contains("- Fix only clippy::unwrap_used."));
    assert_eq!(prompt.matches("- src/lib.rs:").count(), MAX_ISSUE_SITES);
    assert!(prompt.contains("- +2 more sites"));
    assert!(
        prompt
            .trim_end()
            .ends_with("confirm clippy::unwrap_used is gone before moving on.")
    );
}

/// Report text is neutralized on its way to the clipboard, and the payload
/// carries the same bound as the agent handoff rather than growing with
/// whatever the toolchain produced.
#[test]
fn the_issue_prompt_sanitizes_report_text_and_stays_inside_the_handoff_bound() {
    let sites = ten_sites();
    let mut hostile = issue(&sites);
    hostile.message = "safe\u{1b}[2Jwiped";
    hostile.title = "title\nsecond line";
    let prompt = build_issue_prompt(&hostile).unwrap();
    assert!(prompt.as_str().contains("safewiped"));
    assert!(prompt.as_str().contains("titlesecond line"));
    assert!(!prompt.as_str().contains('\u{1b}'));

    let flood = "x".repeat(MAX_HANDOFF_BYTES);
    let mut oversized = issue(&sites);
    oversized.message = &flood;
    assert_eq!(
        build_issue_prompt(&oversized),
        Err(HandoffError::PayloadTooLarge)
    );

    let mut unsafe_rule = issue(&sites);
    unsafe_rule.rule_id = "clippy::unwrap\nsource";
    assert_eq!(
        build_issue_prompt(&unsafe_rule),
        Err(HandoffError::UnsafePayload)
    );
}

#[test]
fn prompt_uses_bounded_groups_relative_locations_and_rescan_command() {
    let report = report();
    let presentation = ReportPresentation::derive(&report);
    let prompt = build_prompt(&report, &presentation, &rescan_command()).unwrap();

    assert!(prompt.as_str().contains("Audit:"));
    assert!(prompt.as_str().contains("clippy::todo"));
    assert!(prompt.as_str().contains("src/lib.rs:4:5"));
    assert!(
        prompt
            .as_str()
            .contains("Validate with: rust-doctor . --yes")
    );
    assert!(!prompt.as_str().contains("source is deliberately excluded"));
    assert!(prompt.as_str().len() <= MAX_HANDOFF_BYTES);
}

#[test]
fn hostile_fields_are_excluded_or_refused_before_delivery() {
    for hostile in [
        "/home/private/source.rs",
        "file:///home/private/source.rs",
        "see:/home/private/source.rs",
        "C:\\private\\source.rs",
        "$HOME/private/source.rs",
        "bad\nargument",
    ] {
        assert_eq!(
            RescanCommand::for_inspection(
                false,
                None,
                &[],
                &[],
                Some((ScopeMode::Files, hostile))
            )
            .map(|_| ()),
            Err(HandoffError::UnsafePayload),
            "{hostile}"
        );
    }

    let mut report = report();
    report.diagnostics[0].message = "fn main() { unsafe { source(); } }".to_owned();
    report.diagnostics[0].help = Some("let secret = include_str!(\"config.toml\");".to_owned());
    let presentation = ReportPresentation::derive(&report);
    let prompt = build_prompt(&report, &presentation, &rescan_command()).unwrap();
    assert!(!prompt.as_str().contains("unsafe { source"));
    assert!(!prompt.as_str().contains("include_str"));
    assert!(
        prompt
            .as_str()
            .contains("Replace todo! with the intended implementation")
    );

    let mut hostile_presentation = presentation.clone();
    hostile_presentation.groups[0].rule_id = "clippy::todo\nsource".to_owned();
    assert_eq!(
        build_prompt(&report, &hostile_presentation, &rescan_command()),
        Err(HandoffError::UnsafePayload)
    );

    for hostile_path in [
        "/home/private/source.rs",
        "C:/private/source.rs",
        "C:\\private\\source.rs",
        "src\\private.rs",
    ] {
        let mut hostile_presentation = presentation.clone();
        hostile_presentation.groups[0].diagnostics[0].path = Some(hostile_path.to_owned());
        assert_eq!(
            build_prompt(&report, &hostile_presentation, &rescan_command()),
            Err(HandoffError::UnsafePayload),
            "{hostile_path}"
        );
    }

    let mut oversized = ReportPresentation::derive(&report);
    oversized.groups[0].diagnostics[0].path =
        Some(format!("{}.rs", "x".repeat(MAX_HANDOFF_BYTES)));
    assert_eq!(
        build_prompt(&report, &oversized, &rescan_command()),
        Err(HandoffError::PayloadTooLarge)
    );
}

#[test]
fn agent_order_is_closed_and_stable() {
    let targets = [
        AgentTarget::ClaudeCode,
        AgentTarget::Codex,
        AgentTarget::Cursor,
    ];
    assert_eq!(
        targets.map(AgentTarget::label),
        ["Claude Code", "Codex", "Cursor"]
    );
    assert_eq!(
        targets.map(AgentTarget::executable),
        ["claude", "codex", "cursor-agent"]
    );
}

#[cfg(unix)]
#[test]
fn agent_launch_passes_one_argument_and_uses_the_workspace_cwd() {
    use std::os::unix::fs::PermissionsExt;

    let root = scratch("handoff-tests", "agent-success");
    let executable = root.join("codex");
    fs::write(
        &executable,
        "#!/bin/sh\nprintf '%s' \"$#\" > argument-count\nprintf '%s' \"$1\" > payload\npwd > cwd\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let agent = AvailableAgent {
        target: AgentTarget::Codex,
        executable,
    };
    let payload = payload();

    launch_agent(&agent, &payload, &root).unwrap();

    assert_eq!(
        fs::read_to_string(root.join("argument-count")).unwrap(),
        "1"
    );
    assert_eq!(
        fs::read_to_string(root.join("payload")).unwrap(),
        payload.as_str()
    );
    assert_eq!(
        PathBuf::from(fs::read_to_string(root.join("cwd")).unwrap().trim()),
        root.canonicalize().unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}

/// Runs `attempt` until the freshly written script stops being reported
/// busy.
///
/// These tests write a shell script and execute it immediately. The kernel
/// answers `ETXTBSY` when any process still holds that file open for
/// writing, and a sibling test thread doing its own `fork` inherits every
/// descriptor this process had open at that instant, including one this
/// test just closed (rust-lang/rust#114554). The window is real on a loaded
/// two-core runner and says nothing about the handoff, so it is waited out
/// rather than asserted. A failure that is not the race still surfaces,
/// because it is returned unchanged on the first attempt that shows it.
#[cfg(unix)]
fn without_text_busy(
    mut attempt: impl FnMut() -> Result<(), HandoffError>,
) -> Result<(), HandoffError> {
    for _ in 0..20u8 {
        let outcome = attempt();
        let busy = matches!(
            outcome,
            Err(HandoffError::AgentSpawn(_, io::ErrorKind::ExecutableFileBusy))
                | Err(HandoffError::ClipboardFailed)
        );
        if !busy {
            return outcome;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    attempt()
}

#[cfg(unix)]
#[test]
fn non_zero_agent_and_clipboard_fallback_are_typed() {
    use std::os::unix::fs::PermissionsExt;

    let root = scratch("handoff-tests", "process-failures");
    let failing_agent = root.join("codex");
    fs::write(&failing_agent, "#!/bin/sh\nexit 7\n").unwrap();
    fs::set_permissions(&failing_agent, fs::Permissions::from_mode(0o700)).unwrap();
    let agent = AvailableAgent {
        target: AgentTarget::Codex,
        executable: failing_agent,
    };
    let payload = payload();
    assert_eq!(
        without_text_busy(|| launch_agent(&agent, &payload, &root)),
        Err(HandoffError::AgentExited(AgentTarget::Codex))
    );

    let first = root.join("first-copy");
    fs::write(&first, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&first, fs::Permissions::from_mode(0o700)).unwrap();
    let capture = root.join("clipboard-payload");
    let second = root.join("second-copy");
    fs::write(
        &second,
        format!("#!/bin/sh\ncat > '{}'\n", capture.display()),
    )
    .unwrap();
    fs::set_permissions(&second, fs::Permissions::from_mode(0o700)).unwrap();
    without_text_busy(|| {
        copy_to_clipboard_in(
            payload.as_str(),
            Some(root.as_os_str()),
            &[("first-copy", &[]), ("second-copy", &[])],
        )
    })
    .unwrap();
    assert_eq!(fs::read_to_string(capture).unwrap(), payload.as_str());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_agent_is_rejected_before_spawn() {
    let agent = AvailableAgent {
        target: AgentTarget::Codex,
        executable: PathBuf::from("/path/that/does/not/exist/codex"),
    };
    assert_eq!(
        launch_agent(&agent, &payload(), Path::new(".")),
        Err(HandoffError::AgentUnavailable(AgentTarget::Codex))
    );
}
