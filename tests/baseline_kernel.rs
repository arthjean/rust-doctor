#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::thread;

use serde_json::{Value, json};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());
const SELECTOR: &str = "private-base";

struct Fixture {
    root: PathBuf,
    repository: PathBuf,
    project: PathBuf,
    target: PathBuf,
    detail_log: PathBuf,
    processes: support::ProcessHarness,
    real_cargo: PathBuf,
    real_git: PathBuf,
    real_rustc: PathBuf,
    real_rustup: PathBuf,
}

fn run(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git(root: &Path, arguments: &[&str]) -> Output {
    run(Command::new("git").args(arguments).current_dir(root))
}

fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn commit(root: &Path, message: &str) {
    run(Command::new("git")
        .args(["commit", "--quiet", "-m", message])
        .current_dir(root)
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z"));
}

fn base_manifest() -> &'static str {
    concat!(
        "[package]\n",
        "name = \"baseline-kernel\"\n",
        "version = \"0.0.0\"\n",
        "edition = \"2024\"\n",
        "publish = false\n",
    )
}

fn fixture(valid_baseline: bool) -> Fixture {
    fixture_with_symlink(valid_baseline, "staged.rs")
}

fn fixture_with_symlink(valid_baseline: bool, symlink_target: &str) -> Fixture {
    fixture_with_baseline_source(
        valid_baseline,
        symlink_target,
        "mod debt;\nmod staged;\nmod unstaged;\npub fn baseline() -> bool { true }\n",
    )
}

fn fixture_with_baseline_source(
    valid_baseline: bool,
    symlink_target: &str,
    baseline_source: &str,
) -> Fixture {
    let root = support::temporary_target("baseline-kernel", &NEXT_FIXTURE);
    let _ = fs::remove_dir_all(&root);
    let repository = root.join("repository");
    let project = repository.join("workspace");
    fs::create_dir_all(project.join("src")).unwrap();
    write(
        project.join("Cargo.toml"),
        if valid_baseline {
            base_manifest()
        } else {
            "this is not a Cargo manifest\n"
        },
    );
    write(project.join("src/lib.rs"), baseline_source);
    write(
        project.join("src/debt.rs"),
        "pub fn debt() -> bool { todo!() }\n",
    );
    write(
        project.join("src/staged.rs"),
        "pub fn staged() -> bool { true }\n",
    );
    write(
        project.join("src/unstaged.rs"),
        "pub fn unstaged() -> bool { true }\n",
    );
    write(
        project.join("rust-doctor.toml"),
        "historical_unknown = \"must be ignored\"\n",
    );
    write(
        project.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.95.0\"\n",
    );
    symlink(symlink_target, project.join("src/internal-link.rs")).unwrap();
    write(project.join(".gitignore"), "/target\n");

    git(&repository, &["init", "--initial-branch=main", "--quiet"]);
    git(&repository, &["config", "user.name", "Rust Doctor"]);
    git(
        &repository,
        &["config", "user.email", "rust-doctor@example.invalid"],
    );
    git(&repository, &["add", "."]);
    commit(&repository, "baseline");
    git(&repository, &["branch", SELECTOR]);

    write(project.join("Cargo.toml"), base_manifest());
    write(
        project.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"stable\"\n",
    );
    run(Command::new(env!("CARGO"))
        .args(["generate-lockfile", "--offline"])
        .current_dir(&project));
    if valid_baseline {
        git(&project, &["add", "Cargo.lock"]);
        run(Command::new("git")
            .args(["commit", "--amend", "--no-edit", "--quiet"])
            .current_dir(&project)
            .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z"));
        git(&project, &["branch", "-f", SELECTOR, "HEAD"]);
    }
    write(
        project.join("rust-doctor.toml"),
        "[rules]\n\"clippy::todo\" = \"warn\"\n",
    );
    write(
        project.join("src/staged.rs"),
        "pub fn staged() -> bool { todo!() }\n",
    );
    git(&project, &["add", "src/staged.rs"]);
    write(
        project.join("src/unstaged.rs"),
        "pub fn unstaged() -> bool { todo!() }\n",
    );
    write(
        project.join("src/lib.rs"),
        concat!(
            "mod staged;\n",
            "mod unstaged;\n",
            "mod untracked;\n",
            "mod debt;\n",
            "pub fn baseline() -> bool { true }\n",
        ),
    );
    write(
        project.join("src/untracked.rs"),
        "pub fn untracked() -> bool { todo!() }\n",
    );

    let processes = support::ProcessHarness::install_with_git(&root);
    let detail_log = root.join("process-detail.log");
    fs::write(&detail_log, []).unwrap();
    Fixture {
        target: root.join("current-target"),
        real_cargo: support::resolve_program("cargo"),
        real_git: support::resolve_program("git"),
        real_rustc: support::resolve_program("rustc"),
        real_rustup: support::resolve_program("rustup"),
        root,
        repository,
        project,
        detail_log,
        processes,
    }
}

fn inspect_command(fixture: &Fixture, arguments: &[&str], target: &Path) -> Command {
    fixture.processes.reset();
    fs::write(&fixture.detail_log, []).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rust-doctor"));
    command
        .arg("inspect")
        .arg("--json")
        .args(arguments)
        .arg(&fixture.project)
        .env("PATH", fixture.processes.command_path())
        .env("RUST_DOCTOR_PROCESS_LOG", fixture.processes.log_path())
        .env("RUST_DOCTOR_PROCESS_DETAIL_LOG", &fixture.detail_log)
        .env("RUST_DOCTOR_REAL_CARGO", &fixture.real_cargo)
        .env("RUST_DOCTOR_REAL_GIT", &fixture.real_git)
        .env("RUST_DOCTOR_REAL_RUSTC", &fixture.real_rustc)
        .env("RUST_DOCTOR_REAL_RUSTUP", &fixture.real_rustup)
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_NET_OFFLINE", "true")
        .env("GIT_DIR", "/private/hostile/repository")
        .env("GIT_WORK_TREE", "/private/hostile/worktree")
        .env("GIT_INDEX_FILE", "/private/hostile/index")
        .env("GIT_OBJECT_DIRECTORY", "/private/hostile/objects")
        .env(
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "/private/hostile/alternates",
        )
        .env("GIT_COMMON_DIR", "/private/hostile/common")
        .env("GIT_CONFIG", "/private/hostile/config")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.pager")
        .env("GIT_CONFIG_VALUE_0", "/private/hostile/pager")
        .env("GIT_EXTERNAL_DIFF", "/private/hostile/diff")
        .env("GIT_PAGER", "/private/hostile/pager")
        .env("GIT_NO_LAZY_FETCH", "0")
        .env_remove("RUSTUP_TOOLCHAIN");
    command
}

fn inspect(fixture: &Fixture, arguments: &[&str], target: &Path) -> Output {
    inspect_command(fixture, arguments, target)
        .output()
        .unwrap()
}

fn report(output: &Output) -> Value {
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn temporary_snapshots() -> BTreeSet<PathBuf> {
    fs::read_dir(env::temp_dir())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rust-doctor-baseline-"))
        })
        .collect()
}

fn counts(events: &[String]) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for event in events {
        *counts.entry(event.as_str()).or_insert(0) += 1;
    }
    counts
}

#[derive(Debug, PartialEq, Eq)]
struct RepositoryState {
    commands: Vec<Vec<u8>>,
    config: Vec<u8>,
    objects: BTreeMap<String, (blake3::Hash, u64)>,
}

fn repository_state(root: &Path) -> RepositoryState {
    let commands = [
        vec!["--no-optional-locks", "status", "--porcelain=v1", "-z"],
        vec!["rev-parse", "HEAD"],
        vec!["show-ref"],
        vec!["hash-object", ".git/index"],
    ]
    .into_iter()
    .map(|arguments| git(root, &arguments).stdout)
    .collect();
    RepositoryState {
        commands,
        config: fs::read(root.join(".git/config")).unwrap(),
        objects: support::content_states(&root.join(".git/objects")),
    }
}

#[test]
fn baseline_runs_two_identical_sides_without_mutation_or_leak() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = fixture(true);
    assert!(
        String::from_utf8(
            git(
                &fixture.repository,
                &["show", "private-base:workspace/rust-toolchain.toml"]
            )
            .stdout
        )
        .unwrap()
        .contains("1.95.0")
    );
    assert!(
        fs::read_to_string(fixture.project.join("rust-toolchain.toml"))
            .unwrap()
            .contains("stable")
    );
    let before_git = repository_state(&fixture.repository);
    let before_files = support::file_states(&fixture.project);
    let before_temp = temporary_snapshots();
    let output = inspect(
        &fixture,
        &["--scope", "baseline", "--base", SELECTOR],
        &fixture.target,
    );
    assert_eq!(output.status.code(), Some(0));
    let baseline = report(&output);
    let baseline_events = fixture.processes.events();

    assert_eq!(baseline["schema_version"], 13);
    assert_eq!(baseline["status"], "complete");
    assert_eq!(baseline["complete"], true);
    assert_eq!(baseline["scope"]["mode"], "baseline");
    assert_eq!(baseline["scope"]["execution_scope"], "workspace");
    assert!(baseline["scope"]["files"].is_null());
    let oid = baseline["scope"]["comparison_base"].as_str().unwrap();
    assert!(matches!(oid.len(), 40 | 64));
    assert!(oid.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(
        baseline["scope"],
        json!({
            "mode": "baseline",
            "execution_scope": "workspace",
            "comparison_base": oid,
            "files": null,
        })
    );
    assert_eq!(baseline["gate"]["status"], "passed");
    assert_eq!(baseline["gate"]["blocking_diagnostics"], 0);
    assert_eq!(baseline["delta"]["fingerprint_version"], 1);
    assert_eq!(baseline["delta"]["base_diagnostics"], 4);
    assert_eq!(baseline["delta"]["current_diagnostics"], 8);
    assert_eq!(baseline["delta"]["summary"]["introduced"], 4);
    assert_eq!(baseline["delta"]["summary"]["pre_existing"], 4);
    assert_eq!(baseline["delta"]["summary"]["fixed"], 0);
    assert_eq!(baseline["policy"]["config_file"], "rust-doctor.toml");
    let audit_issues: u64 = baseline["audit"]["categories"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|category| ["errors", "warnings", "info", "unknown"].map(|key| &category[key]))
        .map(|count| count.as_u64().unwrap())
        .sum();
    let introduced: BTreeSet<_> = baseline["delta"]["introduced"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    // No introduced diagnostic disappears from the categories, including the
    // one no catalog category covers: it falls into `Other`.
    let introduced_diagnostics: Vec<_> = baseline["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| introduced.contains(diagnostic["id"].as_str().unwrap()))
        .collect();
    let introduced_occurrences: u64 = introduced_diagnostics
        .iter()
        .map(|diagnostic| diagnostic["occurrences"].as_u64().unwrap())
        .sum();
    let audit_distinct: u64 = baseline["audit"]["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| category["distinct"]["total"].as_u64().unwrap())
        .sum();
    assert_eq!(audit_issues, introduced_occurrences);
    assert_eq!(audit_distinct, introduced_diagnostics.len() as u64);

    let paths: BTreeSet<_> = baseline["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "clippy::todo")
        .map(|diagnostic| diagnostic["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        BTreeSet::from([
            "src/debt.rs",
            "src/staged.rs",
            "src/unstaged.rs",
            "src/untracked.rs"
        ])
    );

    assert_eq!(
        counts(&baseline_events),
        BTreeMap::from([
            ("cargo-version", 1),
            ("clippy", 2),
            ("clippy-version", 1),
            ("git-checkout-index", 1),
            ("git-ls-tree", 1),
            ("git-merge-base", 1),
            ("git-read-tree", 1),
            ("git-rev-parse", 1),
            ("metadata", 2),
            ("rustc-version", 1),
            ("rustup-toolchain", 1),
        ])
    );

    let details = fs::read_to_string(&fixture.detail_log).unwrap();
    let cargo_calls: Vec<Vec<&str>> = details
        .lines()
        .map(|line| line.split('\t').collect())
        .collect();
    let metadata: Vec<_> = cargo_calls
        .iter()
        .filter(|fields| fields[0] == "metadata")
        .collect();
    let clippy: Vec<_> = cargo_calls
        .iter()
        .filter(|fields| fields[0] == "clippy")
        .collect();
    assert_eq!(metadata.len(), 2);
    assert_eq!(clippy.len(), 2);
    assert_eq!(Path::new(metadata[0][1]), fixture.project);
    assert_eq!(Path::new(metadata[0][2]), fixture.target);
    assert!(metadata[1][1].contains("rust-doctor-baseline-"));
    assert!(metadata[1][2].contains("rust-doctor-baseline-"));
    assert_eq!(Path::new(clippy[1][1]), fixture.project);
    assert_eq!(Path::new(clippy[1][2]), fixture.target);
    assert_eq!(clippy[0][3], clippy[1][3]);
    assert!(clippy[0][3].contains("-W clippy::todo"));
    assert!(!clippy[0][4].is_empty());
    assert_eq!(clippy[0][4], clippy[1][4]);

    let full_output = inspect(&fixture, &[], &fixture.target);
    assert_eq!(full_output.status.code(), Some(0));
    let full = report(&full_output);
    for field in [
        "status",
        "complete",
        "policy",
        "project",
        "toolchain",
        "scan",
        "diagnostics",
        "errors",
        "summary",
    ] {
        assert_eq!(baseline[field], full[field], "{field}");
    }

    assert_eq!(support::file_states(&fixture.project), before_files);
    assert_eq!(repository_state(&fixture.repository), before_git);
    assert_eq!(temporary_snapshots(), before_temp);
    let mut rendered = String::from_utf8_lossy(&output.stdout).into_owned();
    rendered.push_str(&String::from_utf8_lossy(&output.stderr));
    for private in [
        SELECTOR,
        fixture.root.to_string_lossy().as_ref(),
        "credential=secret",
        "historical_unknown",
    ] {
        assert!(!rendered.contains(private), "leaked {private:?}");
    }
}

#[test]
fn baseline_gate_counts_only_introduced_diagnostics() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = fixture(true);
    let output = inspect(
        &fixture,
        &[
            "--blocking",
            "warning",
            "--scope",
            "baseline",
            "--base",
            SELECTOR,
        ],
        &fixture.target,
    );
    assert_eq!(output.status.code(), Some(1));
    let report = report(&output);
    assert_eq!(report["status"], "complete");
    assert_eq!(report["delta"]["summary"]["introduced"], 4);
    assert_eq!(report["delta"]["summary"]["pre_existing"], 4);
    assert_eq!(report["gate"]["status"], "failed");
    assert_eq!(report["gate"]["blocking_diagnostics"], 4);
}

#[test]
fn invalid_baseline_metadata_fails_closed_before_versions_or_current_scan() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = fixture(false);
    let before_temp = temporary_snapshots();
    let output = inspect(
        &fixture,
        &["--scope", "baseline", "--base", SELECTOR],
        &fixture.target,
    );
    assert_eq!(output.status.code(), Some(2));
    let report = report(&output);
    assert_eq!(report["status"], "failed");
    assert_eq!(report["errors"][0]["stage"], "baseline");
    assert_eq!(report["errors"][0]["code"], "baseline-scan-incomplete");
    assert!(report["delta"].is_null());
    assert_eq!(report["gate"]["status"], "not-evaluated");
    assert_eq!(report["scope"]["mode"], "baseline");
    assert!(report["toolchain"]["cargo"].is_null());
    assert!(report["scan"]["command"].is_null());
    let events = fixture.processes.events();
    assert_eq!(
        events.iter().filter(|event| *event == "metadata").count(),
        2
    );
    assert!(!events.iter().any(|event| event.contains("version")));
    assert!(!events.iter().any(|event| event == "clippy"));
    assert_eq!(temporary_snapshots(), before_temp);
}

#[test]
fn unresolved_active_toolchain_fails_closed_before_baseline_metadata_or_scans() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = fixture(true);
    let before_temp = temporary_snapshots();
    let output = inspect_command(
        &fixture,
        &["--scope", "baseline", "--base", SELECTOR],
        &fixture.target,
    )
    .env("RUST_DOCTOR_FAIL_ACTIVE_TOOLCHAIN", "1")
    .output()
    .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let report = report(&output);
    assert_eq!(report["errors"][0]["code"], "baseline-scan-incomplete");
    assert!(report["delta"].is_null());
    assert_eq!(report["gate"]["status"], "not-evaluated");
    let events = fixture.processes.events();
    assert_eq!(
        events.iter().filter(|event| *event == "metadata").count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| *event == "rustup-toolchain")
            .count(),
        1
    );
    assert!(!events.iter().any(|event| event.contains("version")));
    assert!(!events.iter().any(|event| event == "clippy"));
    assert_eq!(temporary_snapshots(), before_temp);
}

#[test]
fn escaping_baseline_symlink_fails_before_baseline_metadata() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = fixture_with_symlink(true, "../outside.rs");
    let before_temp = temporary_snapshots();
    let output = inspect(
        &fixture,
        &["--scope", "baseline", "--base", SELECTOR],
        &fixture.target,
    );
    assert_eq!(output.status.code(), Some(2));
    let report = report(&output);
    assert_eq!(report["errors"][0]["stage"], "baseline");
    assert_eq!(report["errors"][0]["code"], "baseline-entry-invalid");
    assert!(report["delta"].is_null());
    let events = fixture.processes.events();
    assert_eq!(
        events.iter().filter(|event| *event == "metadata").count(),
        1
    );
    assert!(!events.iter().any(|event| event == "clippy"));
    assert_eq!(temporary_snapshots(), before_temp);
}

#[test]
fn incomplete_baseline_clippy_stops_before_the_current_clippy_scan() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = fixture_with_baseline_source(true, "staged.rs", "pub fn broken(\n");
    let before_temp = temporary_snapshots();
    let output = inspect(
        &fixture,
        &["--scope", "baseline", "--base", SELECTOR],
        &fixture.target,
    );
    assert_eq!(output.status.code(), Some(2));
    let report = report(&output);
    assert_eq!(report["errors"][0]["code"], "baseline-scan-incomplete");
    assert!(report["delta"].is_null());
    assert!(report["toolchain"]["cargo"].is_string());
    let events = fixture.processes.events();
    assert_eq!(
        events.iter().filter(|event| *event == "metadata").count(),
        2
    );
    assert_eq!(events.iter().filter(|event| *event == "clippy").count(), 1);
    assert_eq!(temporary_snapshots(), before_temp);
}

#[test]
fn incomplete_current_preserves_current_errors_after_a_complete_baseline() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = fixture(true);
    write(
        fixture.project.join("src/lib.rs"),
        "mod staged;\nmod unstaged;\npub fn broken(\n",
    );
    let before_temp = temporary_snapshots();
    let output = inspect(
        &fixture,
        &["--scope", "baseline", "--base", SELECTOR],
        &fixture.target,
    );
    assert_eq!(output.status.code(), Some(1));
    let report = report(&output);
    assert_eq!(report["status"], "incomplete");
    assert_eq!(report["complete"], false);
    assert_eq!(report["gate"]["status"], "not-evaluated");
    assert!(report["delta"].is_null());
    let codes: BTreeSet<_> = report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains("clippy-exit"));
    assert!(!codes.contains("baseline-scan-incomplete"));
    assert_eq!(
        fixture
            .processes
            .events()
            .iter()
            .filter(|event| *event == "clippy")
            .count(),
        2
    );
    assert_eq!(temporary_snapshots(), before_temp);
}

#[test]
fn disabled_clippy_producer_starts_no_scan_on_either_side() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = fixture(true);
    let before_temp = temporary_snapshots();
    let output = inspect(
        &fixture,
        &[
            "--scope",
            "baseline",
            "--base",
            SELECTOR,
            // Turning off every category turns off every producer, without
            // tying the test to the catalog volume.
            "--category",
            "correctness=off",
            "--category",
            "dependencies=off",
            "--category",
            "maintainability=off",
            "--category",
            "performance=off",
            "--category",
            "reliability=off",
            "--category",
            "security=off",
        ],
        &fixture.target,
    );

    assert_eq!(output.status.code(), Some(0));
    let report = report(&output);
    assert_eq!(report["status"], "complete");
    assert!(report["scan"]["command"].is_null());
    assert!(report["toolchain"]["clippy"].is_string());
    assert_eq!(
        fixture
            .processes
            .events()
            .iter()
            .filter(|event| *event == "clippy")
            .count(),
        0
    );
    assert_eq!(temporary_snapshots(), before_temp);
}

#[test]
fn concurrent_baseline_scans_are_isolated_and_deterministic() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = fixture(true);
    let before_temp = temporary_snapshots();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_rust-doctor"));
    let project = fixture.project.clone();
    let root = fixture.root.clone();
    let run = |suffix: &'static str| {
        let binary = binary.clone();
        let project = project.clone();
        let target = root.join(format!("concurrent-target-{suffix}"));
        thread::spawn(move || {
            Command::new(binary)
                .arg("inspect")
                .arg("--json")
                .args(["--scope", "baseline", "--base", SELECTOR])
                .arg(project)
                .env("CARGO_TARGET_DIR", target)
                .env("CARGO_NET_OFFLINE", "true")
                .output()
                .unwrap()
        })
    };
    let left = run("left").join().unwrap();
    let right = run("right").join().unwrap();
    assert_eq!(left.status.code(), Some(0));
    assert_eq!(right.status.code(), Some(0));
    assert_eq!(left.stdout, right.stdout);
    assert_eq!(temporary_snapshots(), before_temp);
}

#[test]
fn promisor_clone_fails_closed_without_fetching_or_writing_git_objects() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = support::temporary_target("baseline-promisor", &NEXT_FIXTURE);
    let _ = fs::remove_dir_all(&root);
    let origin = root.join("origin");
    fs::create_dir_all(origin.join("src")).unwrap();
    write(
        origin.join("Cargo.toml"),
        "[package]\nname = \"promisor-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    );
    write(
        origin.join("src/lib.rs"),
        "pub fn historical() -> bool { false }\n",
    );
    git(&origin, &["init", "--initial-branch=main", "--quiet"]);
    git(&origin, &["config", "user.name", "Rust Doctor"]);
    git(
        &origin,
        &["config", "user.email", "rust-doctor@example.invalid"],
    );
    git(&origin, &["config", "uploadpack.allowFilter", "true"]);
    git(&origin, &["add", "."]);
    commit(&origin, "baseline");
    let base = String::from_utf8(git(&origin, &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    write(
        origin.join("src/lib.rs"),
        "pub fn historical() -> bool { true }\n",
    );
    git(&origin, &["add", "."]);
    commit(&origin, "current");

    let clone = root.join("clone");
    run(Command::new("git")
        .args([
            "-c",
            "protocol.file.allow=always",
            "clone",
            "--quiet",
            "--filter=blob:none",
        ])
        .arg(format!("file://{}", origin.display()))
        .arg(&clone));
    let missing = Command::new("git")
        .args([
            "--no-lazy-fetch",
            "cat-file",
            "-e",
            &format!("{base}:src/lib.rs"),
        ])
        .current_dir(&clone)
        .output()
        .unwrap();
    assert!(
        !missing.status.success(),
        "the historical blob must be absent locally"
    );

    let before = repository_state(&clone);
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--json")
        .args(["--scope", "baseline", "--base", &base])
        .arg(&clone)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", root.join("current-target"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report = report(&output);
    assert_eq!(report["status"], "failed");
    assert_eq!(report["errors"][0]["code"], "baseline-inventory-failed");
    assert_eq!(report["gate"]["status"], "not-evaluated");
    assert!(report["delta"].is_null());
    assert_eq!(repository_state(&clone), before);
    fs::remove_dir_all(root).unwrap();
}

#[path = "baseline_kernel/delta.rs"]
mod delta;
