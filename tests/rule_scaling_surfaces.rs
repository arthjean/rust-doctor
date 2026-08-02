#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

use rust_doctor::{
    GateStatus, InspectReport, InspectRequest, RuleLevel, RuleOverride, ScopeMode, Severity,
    Status, inspect,
};
use serde_json::Value;
use support::rule_scaling::{clippy_command_without_rules, oracle};

static TEST_LOCK: Mutex<()> = Mutex::new(());
static NEXT_REPOSITORY: AtomicUsize = AtomicUsize::new(0);
const DETERMINISM_RUNS: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceState {
    Clean,
    Expanded,
    ProjectionClean,
    ProjectionExpanded,
    ProjectionUnchanged,
}

#[derive(Clone, Copy)]
struct SourceRevision {
    path: &'static str,
    baseline: SourceState,
    current: SourceState,
}

struct Repository {
    root: PathBuf,
    target: PathBuf,
}

struct EnvironmentGuard {
    values: Vec<(&'static str, Option<OsString>)>,
}

impl EnvironmentGuard {
    fn set(values: &[(&'static str, &Path)]) -> Self {
        let mut previous = Vec::new();
        for (key, value) in values {
            previous.push((*key, env::var_os(key)));
            // The integration test serializes every in-process inspection.
            unsafe { env::set_var(key, value) };
        }
        previous.push(("CARGO_NET_OFFLINE", env::var_os("CARGO_NET_OFFLINE")));
        // The integration test serializes every in-process inspection.
        unsafe { env::set_var("CARGO_NET_OFFLINE", "true") };
        Self { values: previous }
    }
}

impl Drop for EnvironmentGuard {
    fn drop(&mut self) {
        for (key, value) in self.values.drain(..).rev() {
            match value {
                Some(value) => {
                    // The integration test serializes every in-process inspection.
                    unsafe { env::set_var(key, value) };
                }
                None => {
                    // The integration test serializes every in-process inspection.
                    unsafe { env::remove_var(key) };
                }
            }
        }
    }
}

fn surface_fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rule-scaling-kernel/surface")
        .join(path)
}

fn source(state: SourceState) -> String {
    fs::read_to_string(surface_fixture(match state {
        SourceState::Clean => "clean.rs",
        SourceState::Expanded => "expanded.rs",
        SourceState::ProjectionClean => "projection-clean.rs",
        SourceState::ProjectionExpanded => "projection-expanded.rs",
        SourceState::ProjectionUnchanged => "projection-unchanged.rs",
    }))
    .unwrap()
}

fn run(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git(root: &Path, arguments: &[&str]) -> Output {
    support::git_output(root, arguments)
}

fn repository_from_revisions(
    name: &str,
    revisions: &[SourceRevision],
    configuration: Option<&str>,
) -> Repository {
    let root = support::temporary_target("rule-scaling-surfaces", &NEXT_REPOSITORY).join(name);
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(root.join("src")).unwrap();
    fs::copy(surface_fixture("Cargo.toml"), root.join("Cargo.toml")).unwrap();
    fs::copy(surface_fixture("Cargo.lock"), root.join("Cargo.lock")).unwrap();
    fs::copy(surface_fixture("clippy.toml"), root.join("clippy.toml")).unwrap();
    for revision in revisions {
        fs::write(root.join(revision.path), source(revision.baseline)).unwrap();
    }
    if let Some(configuration) = configuration {
        fs::write(root.join("rust-doctor.toml"), configuration).unwrap();
    }

    git(&root, &["init", "--initial-branch=main", "--quiet"]);
    git(&root, &["config", "user.name", "Rust Doctor"]);
    git(
        &root,
        &["config", "user.email", "rust-doctor@example.invalid"],
    );
    git(&root, &["config", "commit.gpgsign", "false"]);
    git(&root, &["add", "."]);
    run(Command::new("git")
        .args(["commit", "--quiet", "-m", "baseline"])
        .current_dir(&root)
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z"));
    for revision in revisions {
        if revision.current != revision.baseline {
            fs::write(root.join(revision.path), source(revision.current)).unwrap();
        }
    }

    Repository {
        target: root.with_extension("target"),
        root,
    }
}

fn repository(
    name: &str,
    baseline: SourceState,
    current: SourceState,
    configuration: Option<&str>,
) -> Repository {
    repository_from_revisions(
        name,
        &[SourceRevision {
            path: "src/lib.rs",
            baseline,
            current,
        }],
        configuration,
    )
}

fn projection_repository() -> Repository {
    repository_from_revisions(
        "projection",
        &[
            SourceRevision {
                path: "src/lib.rs",
                baseline: SourceState::ProjectionClean,
                current: SourceState::ProjectionExpanded,
            },
            SourceRevision {
                path: "src/unchanged.rs",
                baseline: SourceState::ProjectionUnchanged,
                current: SourceState::ProjectionUnchanged,
            },
        ],
        None,
    )
}

fn inspect_api(repository: &Repository, request: InspectRequest) -> InspectReport {
    let _environment = EnvironmentGuard::set(&[("CARGO_TARGET_DIR", &repository.target)]);
    inspect(request)
}

fn inspect_cli(repository: &Repository, arguments: &[&str]) -> (Output, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--json")
        .args(arguments)
        .arg(&repository.root)
        .env("CARGO_TARGET_DIR", &repository.target)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap();
    (output, report)
}

fn candidate_codes<'a>(
    report: &'a InspectReport,
    candidates: &BTreeSet<&str>,
) -> BTreeSet<&'a str> {
    report
        .diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic.code.as_deref())
        .filter(|code| candidates.contains(code))
        .collect()
}

fn json_candidate_codes<'a>(report: &'a Value, candidates: &BTreeSet<&str>) -> BTreeSet<&'a str> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str())
        .filter(|code| candidates.contains(code))
        .collect()
}

fn terminal(report: &InspectReport) -> Vec<u8> {
    let mut output = Vec::new();
    rust_doctor::render::render_terminal(report, &mut output).unwrap();
    output
}

fn assert_private(bytes: &[u8]) {
    let rendered = String::from_utf8_lossy(bytes);
    for forbidden in [
        "credential=EP018_",
        "source=EP018_PRIVATE",
        "/home/ep018-private",
        "\u{1b}",
    ] {
        assert!(!rendered.contains(forbidden), "leaked {forbidden:?}");
    }
}

fn repository_evidence(repository: &Repository) -> support::GitRepositoryState {
    support::git_repository_state(&repository.root)
}

fn assert_delta(report: &InspectReport, introduced: usize, pre_existing: usize, fixed: usize) {
    let delta = report.delta.as_ref().unwrap();
    assert_eq!(delta.fingerprint_version, 1);
    assert_eq!(delta.summary.introduced, introduced);
    assert_eq!(delta.summary.pre_existing, pre_existing);
    assert_eq!(delta.summary.fixed, fixed);
}

fn deterministic_report(
    repository: &Repository,
    request: InspectRequest,
    expected_status: Status,
    candidates: &BTreeSet<&str>,
    expected_codes: &BTreeSet<&str>,
    scenario: &str,
) -> InspectReport {
    let before = repository_evidence(repository);
    let first = inspect_api(repository, request.clone());
    assert_eq!(&first.status, &expected_status, "{scenario}");
    assert_eq!(candidate_codes(&first, candidates), *expected_codes);
    let expected_json = serde_json::to_vec(&first).unwrap();
    let expected_terminal = terminal(&first);
    assert_private(&expected_json);
    assert_private(&expected_terminal);

    for _ in 1..DETERMINISM_RUNS {
        let report = inspect_api(repository, request.clone());
        assert_eq!(&report.status, &expected_status, "{scenario}");
        assert_eq!(candidate_codes(&report, candidates), *expected_codes);
        let json = serde_json::to_vec(&report).unwrap();
        let rendered = terminal(&report);
        assert_private(&json);
        assert_private(&rendered);
        assert_eq!(json, expected_json, "{scenario} JSON");
        assert_eq!(rendered, expected_terminal, "{scenario} terminal");
    }

    assert_eq!(
        repository_evidence(repository),
        before,
        "{scenario} mutated repository"
    );
    first
}

fn deterministic_cli(
    repository: &Repository,
    arguments: &[&str],
    candidates: &BTreeSet<&str>,
    expected_codes: &BTreeSet<&str>,
    scenario: &str,
) -> (Output, Value) {
    let before = repository_evidence(repository);
    let (first_output, first_report) = inspect_cli(repository, arguments);
    assert_eq!(
        json_candidate_codes(&first_report, candidates),
        *expected_codes
    );
    assert_private(&first_output.stdout);
    assert_private(&first_output.stderr);

    for _ in 1..DETERMINISM_RUNS {
        let (output, report) = inspect_cli(repository, arguments);
        assert_eq!(json_candidate_codes(&report, candidates), *expected_codes);
        assert_private(&output.stdout);
        assert_private(&output.stderr);
        assert_eq!(output.status, first_output.status, "{scenario} exit status");
        assert_eq!(output.stdout, first_output.stdout, "{scenario} stdout");
        assert_eq!(output.stderr, first_output.stderr, "{scenario} stderr");
    }

    assert_eq!(
        repository_evidence(repository),
        before,
        "{scenario} mutated repository"
    );
    (first_output, first_report)
}

fn candidate_projection(report: &Value, id: &str) -> Value {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == id)
        .map(|diagnostic| {
            serde_json::json!({
                "source": diagnostic["source"],
                "code": diagnostic["code"],
                "category": diagnostic["category"],
                "base_severity": diagnostic["base_severity"],
                "severity": diagnostic["severity"],
                "help": diagnostic["help"],
            })
        })
        .unwrap()
}

#[test]
fn full_and_files_scopes_are_deterministic_and_projection_is_discriminating() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let rule_scaling = oracle();
    let candidates = rule_scaling.candidate_ids();
    let projection = projection_repository();

    let full = deterministic_report(
        &projection,
        InspectRequest::new(&projection.root),
        Status::Complete,
        &candidates,
        &candidates,
        "full",
    );
    assert_eq!(full.scope.as_ref().unwrap().mode(), ScopeMode::Full);

    let mut projected_codes = candidates.clone();
    projected_codes.remove("clippy::mem_forget");
    let files = deterministic_report(
        &projection,
        InspectRequest::new(&projection.root).with_files_scope("HEAD"),
        Status::Complete,
        &candidates,
        &projected_codes,
        "files",
    );
    assert_eq!(files.scope.as_ref().unwrap().mode(), ScopeMode::Files);
    assert_eq!(
        files.scope.as_ref().unwrap().files(),
        Some(&["src/lib.rs".to_owned()][..])
    );

    let _ = fs::remove_dir_all(&projection.target);
    fs::remove_dir_all(&projection.root).unwrap();
}

#[test]
fn baseline_delta_policy_and_gate_cover_the_whole_pack() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let rule_scaling = oracle();
    let candidates = rule_scaling.candidate_ids();
    let introduced = repository(
        "introduced",
        SourceState::Clean,
        SourceState::Expanded,
        None,
    );
    let baseline = deterministic_report(
        &introduced,
        InspectRequest::new(&introduced.root).with_baseline_scope("HEAD"),
        Status::Complete,
        &candidates,
        &candidates,
        "baseline",
    );
    assert_delta(&baseline, 5, 0, 0);
    assert_eq!(baseline.delta.as_ref().unwrap().introduced.len(), 5);

    let persisting = repository(
        "persisting",
        SourceState::Expanded,
        SourceState::Expanded,
        None,
    );
    let default = deterministic_report(
        &persisting,
        InspectRequest::new(&persisting.root).with_baseline_scope("HEAD"),
        Status::Complete,
        &candidates,
        &candidates,
        "persisting-default",
    );
    assert_delta(&default, 0, 5, 0);
    let error = deterministic_report(
        &persisting,
        InspectRequest::new(&persisting.root)
            .with_baseline_scope("HEAD")
            .with_rule_override(RuleOverride::new("clippy::mem_forget", RuleLevel::Error)),
        Status::Complete,
        &candidates,
        &candidates,
        "persisting-error",
    );
    assert_delta(&error, 0, 5, 0);
    let warning = default
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_deref() == Some("clippy::mem_forget"))
        .unwrap();
    let elevated = error
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_deref() == Some("clippy::mem_forget"))
        .unwrap();
    assert_eq!(warning.id, elevated.id);
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(elevated.severity, Severity::Error);
    assert_eq!(error.gate.status, GateStatus::Passed);

    let resolved = repository("resolved", SourceState::Expanded, SourceState::Clean, None);
    let fixed = deterministic_report(
        &resolved,
        InspectRequest::new(&resolved.root).with_baseline_scope("HEAD"),
        Status::Complete,
        &candidates,
        &BTreeSet::new(),
        "resolved",
    );
    assert_delta(&fixed, 0, 0, 5);
    assert_eq!(candidate_codes(&fixed, &candidates).len(), 0);
    assert_eq!(
        fixed
            .delta
            .as_ref()
            .unwrap()
            .fixed
            .iter()
            .filter_map(|diagnostic| diagnostic.code.as_deref())
            .collect::<BTreeSet<_>>(),
        candidates
    );

    for id in rule_scaling.rules.iter().map(|rule| rule.id.as_str()) {
        let mut expected_codes = candidates.clone();
        expected_codes.remove(id);
        let report = deterministic_report(
            &introduced,
            InspectRequest::new(&introduced.root)
                .with_rule_override(RuleOverride::new(id, RuleLevel::Off)),
            Status::Complete,
            &candidates,
            &expected_codes,
            &format!("off-{id}"),
        );
        let active = candidate_codes(&report, &candidates);
        assert_eq!(active.len(), 4, "{id}");
        assert!(!active.contains(id), "{id}");
        assert_eq!(
            report.scan.command.as_ref().unwrap(),
            &clippy_command_without_rules(&rule_scaling.clippy_command, &[id.to_owned()]),
            "{id}"
        );
        for historical in ["clippy::dbg_macro", "clippy::todo", "clippy::unimplemented"] {
            assert!(
                report
                    .scan
                    .command
                    .as_ref()
                    .unwrap()
                    .contains(&historical.to_owned()),
                "{id} pruned {historical}"
            );
        }
    }

    for repository in [&introduced, &persisting, &resolved] {
        let _ = fs::remove_dir_all(&repository.target);
        fs::remove_dir_all(&repository.root).unwrap();
    }
}

#[test]
fn cli_configuration_and_api_policy_are_equivalent_and_failures_never_pass_the_gate() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let rule_scaling = oracle();
    let candidates = rule_scaling.candidate_ids();
    let request_repository = repository(
        "request-policy",
        SourceState::Expanded,
        SourceState::Expanded,
        None,
    );
    let configuration_repository = repository(
        "configuration-policy",
        SourceState::Expanded,
        SourceState::Expanded,
        Some("[rules]\n\"clippy::mem_forget\" = \"error\"\n"),
    );
    let api = deterministic_report(
        &request_repository,
        InspectRequest::new(&request_repository.root)
            .with_rule_override(RuleOverride::new("clippy::mem_forget", RuleLevel::Error)),
        Status::Complete,
        &candidates,
        &candidates,
        "api-policy",
    );
    let api_value = serde_json::to_value(&api).unwrap();
    let (cli_output, cli) = deterministic_cli(
        &request_repository,
        &["--rule", "clippy::mem_forget=error"],
        &candidates,
        &candidates,
        "cli-policy",
    );
    let (configuration_output, configuration) = deterministic_cli(
        &configuration_repository,
        &[],
        &candidates,
        &candidates,
        "configuration-policy",
    );
    assert_eq!(cli_output.status.code(), Some(api.exit_code().into()));
    assert_eq!(
        configuration_output.status.code(),
        Some(api.exit_code().into())
    );
    for report in [&cli, &configuration] {
        assert_eq!(report["scan"]["command"], api_value["scan"]["command"]);
        assert_eq!(report["gate"], api_value["gate"]);
        assert_eq!(
            candidate_projection(report, "clippy::mem_forget"),
            candidate_projection(&api_value, "clippy::mem_forget")
        );
        assert_eq!(json_candidate_codes(report, &candidates), candidates);
    }

    let invalid_configuration = repository(
        "invalid-configuration",
        SourceState::Expanded,
        SourceState::Expanded,
        Some("[rules]\n\"clippy::unknown-private\" = \"warn\"\n"),
    );
    let invalid = deterministic_report(
        &invalid_configuration,
        InspectRequest::new(&invalid_configuration.root),
        Status::Failed,
        &candidates,
        &BTreeSet::new(),
        "invalid-configuration",
    );
    assert_eq!(invalid.status, Status::Failed);
    assert_eq!(invalid.gate.status, GateStatus::NotEvaluated);
    assert!(invalid.scan.command.is_none());
    for rendered in [serde_json::to_vec(&invalid).unwrap(), terminal(&invalid)] {
        assert!(!String::from_utf8_lossy(&rendered).contains("unknown-private"));
    }

    let hostile_base = "credential=EP018_BASE\u{1b}";
    let invalid_scope = deterministic_report(
        &request_repository,
        InspectRequest::new(&request_repository.root).with_baseline_scope(hostile_base),
        Status::Failed,
        &candidates,
        &BTreeSet::new(),
        "invalid-scope",
    );
    assert_eq!(invalid_scope.status, Status::Failed);
    assert_eq!(invalid_scope.gate.status, GateStatus::NotEvaluated);
    assert!(invalid_scope.scan.command.is_none());
    for rendered in [
        serde_json::to_vec(&invalid_scope).unwrap(),
        terminal(&invalid_scope),
    ] {
        assert!(!String::from_utf8_lossy(&rendered).contains(hostile_base));
    }

    for repository in [
        &request_repository,
        &configuration_repository,
        &invalid_configuration,
    ] {
        let _ = fs::remove_dir_all(&repository.target);
        fs::remove_dir_all(&repository.root).unwrap();
    }
}
