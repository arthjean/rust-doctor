use super::*;

fn rich_delta_fixture() -> Fixture {
    let fixture = fixture(true);
    git(&fixture.project, &["add", "."]);
    commit(&fixture.project, "seed current state");

    write(
        fixture.project.join("src/lib.rs"),
        concat!(
            "pub mod changed;\n",
            "pub mod copy_original;\n",
            "pub mod debt;\n",
            "pub mod fixed;\n",
            "pub mod keep;\n",
            "pub mod move_old;\n",
            "pub mod staged;\n",
            "pub mod unstaged;\n",
        ),
    );
    write(
        fixture.project.join("src/changed.rs"),
        "pub fn changed() -> bool { todo!(\"old\") }\n",
    );
    write(
        fixture.project.join("src/copy_original.rs"),
        "pub fn copy_original() -> bool { todo!(\"copy\") }\n",
    );
    write(
        fixture.project.join("src/fixed.rs"),
        "pub fn fixed() -> bool { todo!(\"fixed\") }\n",
    );
    write(
        fixture.project.join("src/keep.rs"),
        "pub fn keep() -> bool { todo!(\"keep\") }\n",
    );
    write(
        fixture.project.join("src/move_old.rs"),
        "pub fn moved() -> bool { todo!(\"move\") }\n",
    );
    write(
        fixture.project.join("src/staged.rs"),
        "pub fn staged() -> bool { true }\n",
    );
    write(
        fixture.project.join("src/unstaged.rs"),
        "pub fn unstaged() -> bool { true }\n",
    );
    git(&fixture.project, &["rm", "--quiet", "src/untracked.rs"]);
    git(&fixture.project, &["add", "."]);
    commit(&fixture.project, "rich delta baseline");
    git(&fixture.project, &["branch", "-f", SELECTOR, "HEAD"]);
    fixture
}

fn current_id_hash(report: &Value) -> String {
    let ids = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["id"].as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    blake3::hash(ids.as_bytes()).to_hex().to_string()
}

fn introduced_id_hash(report: &Value) -> String {
    let ids = report["delta"]["introduced"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    blake3::hash(ids.as_bytes()).to_hex().to_string()
}

fn run_delta_state(fixture: &Fixture, name: &str, expected_exit: i32) -> (Value, Value) {
    let before_files = support::file_states(&fixture.project);
    let before_git = repository_state(&fixture.repository);
    let before_temp = temporary_snapshots();
    let mut expected_output = None;
    let mut observed_counts = None;
    for _ in 0..30 {
        let output = inspect(
            fixture,
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
        assert_eq!(output.status.code(), Some(expected_exit), "{name}");
        let counts = fixture.processes.counts();
        assert_eq!(
            counts,
            BTreeMap::from([
                ("cargo-version".to_owned(), 1),
                ("clippy".to_owned(), 2),
                ("clippy-version".to_owned(), 1),
                ("git-checkout-index".to_owned(), 1),
                ("git-ls-tree".to_owned(), 1),
                ("git-merge-base".to_owned(), 1),
                ("git-read-tree".to_owned(), 1),
                ("git-rev-parse".to_owned(), 1),
                ("metadata".to_owned(), 2),
                ("rustc-version".to_owned(), 1),
                ("rustup-toolchain".to_owned(), 1),
            ]),
            "{name}"
        );
        observed_counts.get_or_insert(counts);
        if let Some(expected) = &expected_output {
            assert_eq!(&output.stdout, expected, "{name}");
        } else {
            expected_output = Some(output.stdout);
        }
    }
    assert_eq!(
        support::file_states(&fixture.project),
        before_files,
        "{name}"
    );
    assert_eq!(repository_state(&fixture.repository), before_git, "{name}");
    assert_eq!(temporary_snapshots(), before_temp, "{name}");

    let output = expected_output.unwrap();
    let rendered = String::from_utf8_lossy(&output);
    for private in [
        SELECTOR,
        fixture.root.to_string_lossy().as_ref(),
        "credential=secret",
        "todo!(",
    ] {
        assert!(!rendered.contains(private), "{name} leaked {private:?}");
    }
    let report: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(report["schema_version"], 8);
    assert_eq!(report["status"], "complete");
    assert_eq!(report["scope"]["mode"], "baseline");
    assert_eq!(report["delta"]["fingerprint_version"], 1);
    let evaluation = json!({
        "name": name,
        "delta": report["delta"]["summary"],
        "summary": report["summary"],
        "gate": report["gate"],
        "exit_code": expected_exit,
        "current_id_hash": current_id_hash(&report),
        "introduced_id_hash": introduced_id_hash(&report),
        "processes_per_run": observed_counts.unwrap(),
        "runs": 30,
    });
    (report, evaluation)
}

fn process_counts(entries: &[(&str, usize)]) -> BTreeMap<String, usize> {
    entries
        .iter()
        .map(|(name, count)| ((*name).to_owned(), *count))
        .collect()
}

struct ErrorExpectation<'a> {
    name: &'a str,
    base: &'a str,
    code: &'a str,
    status: &'a str,
    exit_code: i32,
    processes: BTreeMap<String, usize>,
}

fn run_error_state(
    fixture: &Fixture,
    expectation: ErrorExpectation<'_>,
    configure: impl FnOnce(&mut Command),
) -> Value {
    let before_files = support::content_states(&fixture.repository);
    let before_git = repository_state(&fixture.repository);
    let mut command = inspect_command(
        fixture,
        &["--scope", "baseline", "--base", expectation.base],
        &fixture.target,
    );
    configure(&mut command);
    let output = command.output().unwrap();
    let report = report(&output);
    let observed_processes = fixture.processes.counts();

    assert_eq!(
        output.status.code(),
        Some(expectation.exit_code),
        "{}",
        expectation.name
    );
    assert_eq!(report["status"], expectation.status, "{}", expectation.name);
    assert_eq!(
        report["gate"]["status"], "not-evaluated",
        "{}",
        expectation.name
    );
    assert!(report["delta"].is_null(), "{}", expectation.name);
    assert!(
        report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["code"] == expectation.code),
        "{} did not return {}",
        expectation.name,
        expectation.code,
    );
    assert_eq!(
        observed_processes, expectation.processes,
        "{} process oracle",
        expectation.name,
    );
    assert_eq!(
        support::content_states(&fixture.repository),
        before_files,
        "{} repository content",
        expectation.name,
    );
    assert_eq!(
        repository_state(&fixture.repository),
        before_git,
        "{} Git state",
        expectation.name,
    );

    json!({
        "name": expectation.name,
        "error_code": expectation.code,
        "status": expectation.status,
        "gate": "not-evaluated",
        "exit_code": expectation.exit_code,
        "maximum_processes": observed_processes.values().sum::<usize>(),
        "processes": observed_processes,
    })
}

fn error_evaluation() -> Value {
    let cases = [
        (
            ErrorExpectation {
                name: "missing-ref",
                base: "missing-baseline",
                code: "base-unavailable",
                status: "failed",
                exit_code: 2,
                processes: process_counts(&[("git-rev-parse", 1), ("metadata", 1)]),
            },
            None,
        ),
        (
            ErrorExpectation {
                name: "shallow-history",
                base: SELECTOR,
                code: "merge-base-unavailable",
                status: "failed",
                exit_code: 2,
                processes: process_counts(&[
                    ("git-merge-base", 1),
                    ("git-rev-parse", 1),
                    ("metadata", 1),
                ]),
            },
            Some("merge-base-unavailable"),
        ),
        (
            ErrorExpectation {
                name: "ambiguous-merge-base",
                base: SELECTOR,
                code: "merge-base-ambiguous",
                status: "failed",
                exit_code: 2,
                processes: process_counts(&[
                    ("git-merge-base", 1),
                    ("git-rev-parse", 1),
                    ("metadata", 1),
                ]),
            },
            Some("merge-base-ambiguous"),
        ),
        (
            ErrorExpectation {
                name: "snapshot-limit",
                base: SELECTOR,
                code: "baseline-limit-exceeded",
                status: "failed",
                exit_code: 2,
                processes: process_counts(&[
                    ("git-ls-tree", 1),
                    ("git-merge-base", 1),
                    ("git-rev-parse", 1),
                    ("metadata", 1),
                ]),
            },
            Some("baseline-limit"),
        ),
        (
            ErrorExpectation {
                name: "gitlink",
                base: SELECTOR,
                code: "baseline-entry-invalid",
                status: "failed",
                exit_code: 2,
                processes: process_counts(&[
                    ("git-ls-tree", 1),
                    ("git-merge-base", 1),
                    ("git-rev-parse", 1),
                    ("metadata", 1),
                ]),
            },
            Some("baseline-gitlink"),
        ),
    ]
    .into_iter()
    .map(|(expectation, fault)| {
        let case_fixture = fixture(true);
        let result = run_error_state(&case_fixture, expectation, |command| {
            if let Some(fault) = fault {
                command.env("RUST_DOCTOR_GIT_FAULT", fault);
            }
        });
        fs::remove_dir_all(&case_fixture.root).unwrap();
        result
    })
    .collect::<Vec<_>>();

    let mut cases = cases;
    let case_fixture = fixture(false);
    cases.push(run_error_state(
        &case_fixture,
        ErrorExpectation {
            name: "baseline-scan-incomplete",
            base: SELECTOR,
            code: "baseline-scan-incomplete",
            status: "failed",
            exit_code: 2,
            processes: process_counts(&[
                ("git-checkout-index", 1),
                ("git-ls-tree", 1),
                ("git-merge-base", 1),
                ("git-read-tree", 1),
                ("git-rev-parse", 1),
                ("metadata", 2),
                ("rustup-toolchain", 1),
            ]),
        },
        |_| {},
    ));
    fs::remove_dir_all(&case_fixture.root).unwrap();

    let case_fixture = fixture(true);
    write(
        case_fixture.project.join("src/lib.rs"),
        "mod staged;\nmod unstaged;\npub fn broken(\n",
    );
    cases.push(run_error_state(
        &case_fixture,
        ErrorExpectation {
            name: "current-scan-incomplete",
            base: SELECTOR,
            code: "clippy-exit",
            status: "incomplete",
            exit_code: 1,
            processes: process_counts(&[
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
            ]),
        },
        |_| {},
    ));
    fs::remove_dir_all(&case_fixture.root).unwrap();

    let case_fixture = fixture(true);
    let temporary = case_fixture.root.join("cleanup-parent");
    fs::create_dir(&temporary).unwrap();
    cases.push(run_error_state(
        &case_fixture,
        ErrorExpectation {
            name: "cleanup-failure",
            base: SELECTOR,
            code: "baseline-cleanup-failed",
            status: "failed",
            exit_code: 2,
            processes: process_counts(&[
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
            ]),
        },
        |command| {
            command
                .env("TMPDIR", &temporary)
                .env("RUST_DOCTOR_GIT_FAULT", "cleanup-failure")
                .env("RUST_DOCTOR_CLEANUP_PARENT", &temporary);
        },
    ));
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir_all(&case_fixture.root).unwrap();

    Value::Array(cases)
}

fn remove_modules(source: &str, modules: &[&str]) -> String {
    source
        .lines()
        .filter(|line| {
            !modules
                .iter()
                .any(|module| line == &format!("pub mod {module};"))
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn local_evaluation() -> Value {
    let fixture = rich_delta_fixture();
    let mut cases = Vec::new();

    let (initial, evaluation) = run_delta_state(&fixture, "baseline", 0);
    assert_eq!(initial["delta"]["summary"]["introduced"], 0);
    assert_eq!(initial["delta"]["summary"]["fixed"], 0);
    cases.push(evaluation);

    write(
        fixture.project.join("src/staged.rs"),
        "pub fn staged() -> bool { todo!(\"staged\") }\n",
    );
    git(&fixture.project, &["add", "src/staged.rs"]);
    let (_, evaluation) = run_delta_state(&fixture, "staged", 1);
    cases.push(evaluation);

    write(
        fixture.project.join("src/unstaged.rs"),
        "pub fn unstaged() -> bool { todo!(\"unstaged\") }\n",
    );
    let (_, evaluation) = run_delta_state(&fixture, "unstaged", 1);
    cases.push(evaluation);

    let lib = fs::read_to_string(fixture.project.join("src/lib.rs")).unwrap();
    write(
        fixture.project.join("src/lib.rs"),
        &format!("{lib}pub mod untracked;\n"),
    );
    write(
        fixture.project.join("src/untracked.rs"),
        "pub fn untracked() -> bool { todo!(\"untracked\") }\n",
    );
    let (_, evaluation) = run_delta_state(&fixture, "untracked", 1);
    cases.push(evaluation);

    git(
        &fixture.project,
        &["mv", "src/move_old.rs", "src/move_new.rs"],
    );
    let lib = fs::read_to_string(fixture.project.join("src/lib.rs"))
        .unwrap()
        .replace("pub mod move_old;", "pub mod move_new;");
    write(fixture.project.join("src/lib.rs"), &lib);
    let (moved, evaluation) = run_delta_state(&fixture, "moved", 1);
    assert_eq!(moved["delta"]["summary"]["cross_file_matches"], 1);
    cases.push(evaluation);

    let lib = fs::read_to_string(fixture.project.join("src/lib.rs")).unwrap();
    write(
        fixture.project.join("src/lib.rs"),
        &format!("{lib}pub mod copy_new;\n"),
    );
    write(
        fixture.project.join("src/copy_new.rs"),
        "pub fn copy_new() -> bool { todo!(\"copy\") }\n",
    );
    let (copied, evaluation) = run_delta_state(&fixture, "copied", 1);
    assert!(
        copied["delta"]["introduced"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| {
                let id = id.as_str().unwrap();
                copied["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|diagnostic| {
                        diagnostic["id"] == id && diagnostic["path"] == "src/copy_new.rs"
                    })
            })
    );
    cases.push(evaluation);

    write(
        fixture.project.join("src/changed.rs"),
        "pub fn changed() -> bool { todo!(\"new\") }\n",
    );
    write(
        fixture.project.join("src/fixed.rs"),
        "pub fn fixed() -> bool { true }\n",
    );
    let lib = fs::read_to_string(fixture.project.join("src/lib.rs")).unwrap();
    write(
        fixture.project.join("src/lib.rs"),
        &format!("{lib}pub mod introduced;\n"),
    );
    write(
        fixture.project.join("src/introduced.rs"),
        "pub fn introduced() -> bool { todo!(\"introduced\") }\n",
    );
    let (changed, evaluation) = run_delta_state(&fixture, "changed-and-fixed", 1);
    assert_eq!(changed["delta"]["summary"]["fixed"], 2);
    let full = inspect(&fixture, &["--blocking", "warning"], &fixture.target);
    let full: Value = serde_json::from_slice(&full.stdout).unwrap();
    assert_eq!(full["delta"], Value::Null);
    assert_eq!(changed["diagnostics"], full["diagnostics"]);
    let untouched_ids = changed["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic["path"].as_str(),
                Some("src/debt.rs" | "src/keep.rs" | "src/copy_original.rs" | "src/move_new.rs")
            )
        })
        .map(|diagnostic| diagnostic["id"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    cases.push(evaluation);

    write(
        fixture.project.join("src/staged.rs"),
        "pub fn staged() -> bool { true }\n",
    );
    git(&fixture.project, &["add", "src/staged.rs"]);
    write(
        fixture.project.join("src/unstaged.rs"),
        "pub fn unstaged() -> bool { true }\n",
    );
    write(
        fixture.project.join("src/changed.rs"),
        "pub fn changed() -> bool { todo!(\"old\") }\n",
    );
    for path in ["src/untracked.rs", "src/copy_new.rs", "src/introduced.rs"] {
        fs::remove_file(fixture.project.join(path)).unwrap();
    }
    let lib = remove_modules(
        &fs::read_to_string(fixture.project.join("src/lib.rs")).unwrap(),
        &["untracked", "copy_new", "introduced"],
    );
    write(fixture.project.join("src/lib.rs"), &lib);
    let (corrected, evaluation) = run_delta_state(&fixture, "corrected", 0);
    assert_eq!(corrected["delta"]["summary"]["introduced"], 0);
    assert_eq!(corrected["delta"]["summary"]["fixed"], 1);
    assert_eq!(corrected["gate"]["status"], "passed");
    let corrected_ids = corrected["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["id"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    assert!(untouched_ids.is_subset(&corrected_ids));
    cases.push(evaluation);

    let evaluation = json!({
        "schema_version": 1,
        "epic": "EP-016",
        "stories": ["US-045", "US-046", "US-047"],
        "git_version": String::from_utf8(git(&fixture.repository, &["--version"]).stdout)
            .unwrap()
            .trim()
            .to_owned(),
        "comparison_base": corrected["scope"]["comparison_base"],
        "toolchain": corrected["toolchain"],
        "limits": {
            "diagnostics_per_side": 50_000,
            "proof_bytes": 65_536,
            "source_file_bytes": 8_388_608,
            "evidence_bytes_per_side": 67_108_864,
        },
        "matrix": cases,
        "determinism": {
            "states": 8,
            "runs_per_state": 30,
            "total_reports": 240,
        },
        "error_matrix": error_evaluation(),
        "privacy": "pass",
        "non_mutation": "working-content-size-mtime-plus-head-refs-index-config-and-objects",
        "ordinary_temp_residue": 0,
    });
    fs::remove_dir_all(&fixture.root).unwrap();
    evaluation
}

fn artifact_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tasks/rust-doctor-baseline-delta-kernel-evaluation.json")
}

fn evaluation_artifact() -> Value {
    serde_json::from_str(&fs::read_to_string(artifact_path()).unwrap()).unwrap()
}

#[test]
fn baseline_delta_product_matrix_is_deterministic_private_and_non_mutating() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let local = local_evaluation();
    let artifact = evaluation_artifact();

    for (key, value) in local.as_object().unwrap() {
        assert_eq!(
            artifact.get(key),
            Some(value),
            "measured local artifact field {key:?} differs",
        );
    }
    assert_eq!(artifact["public_repositories"].as_array().unwrap().len(), 3);
    assert_eq!(artifact["performance"]["measured"], true);
    assert_eq!(artifact["performance"]["verdict"], "pass");
    assert_eq!(
        artifact["reconstruction"]["public_repositories"],
        "measured"
    );
    assert_eq!(artifact["reconstruction"]["performance"], "measured");
    assert_eq!(artifact["verdict"], "pass");

    let serialized = serde_json::to_string(&artifact).unwrap();
    for forbidden in [
        "http://",
        "https://",
        "file://",
        "/home/",
        "/tmp/",
        "credential=",
        "\u{1b}",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "evaluation artifact leaked {forbidden:?}",
        );
    }
}

const PUBLIC_REPOSITORIES: [(&str, &str, &str); 3] = [
    (
        "anyhow",
        "18c2598afa0f996f56217ef128aa3a20ea1e9512",
        "f2719888cb2f4f033c441cf6723cea1c532c0c87",
    ),
    (
        "serde_json",
        "efa66e3a1d61459ab2d325f92ebe3acbd6ca18b1",
        "23679e2b9d7e4dcaef797ca7c51a4ffb6fce9f36",
    ),
    (
        "hexyl",
        "abc20a380c8c2d9d76c1976222725d3211cef809",
        "dd89c22327f388a4ca07108ab18bd7afcb64f7ba",
    ),
];

fn resolved_commit(repository: &Path, revision: &str) -> String {
    String::from_utf8(git(repository, &["rev-parse", revision]).stdout)
        .unwrap()
        .trim()
        .to_owned()
}

fn prepare_public_lockfile(repository: &Path) -> &'static str {
    let committed = !git(
        repository,
        &["ls-tree", "--name-only", "HEAD", "--", "Cargo.lock"],
    )
    .stdout
    .is_empty();
    if repository.join("Cargo.lock").is_file() {
        return if committed {
            "committed"
        } else {
            "generated-before-measurement"
        };
    }
    let output = Command::new(env!("CARGO"))
        .args(["generate-lockfile", "--offline"])
        .current_dir(repository)
        .env("CARGO_NET_OFFLINE", "true")
        .env("RUSTUP_TOOLCHAIN", "1.97.1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "public corpus lockfile preparation failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(!committed, "committed public corpus lockfile is missing");
    "generated-before-measurement"
}

fn measure_public_repositories(corpus_root: &Path, target_root: &Path) -> Value {
    let binary = Path::new(env!("CARGO_BIN_EXE_rust-doctor"));
    Value::Array(
        PUBLIC_REPOSITORIES
            .into_iter()
            .map(|(name, current_commit, base_commit)| {
                let repository = corpus_root.join(name);
                assert_eq!(
                    resolved_commit(&repository, "HEAD"),
                    current_commit,
                    "{name}"
                );
                assert_eq!(
                    resolved_commit(&repository, &format!("{base_commit}^{{commit}}")),
                    base_commit,
                    "{name}",
                );
                let lockfile_input = prepare_public_lockfile(&repository);
                let before_git = repository_state(&repository);
                let before_files = support::content_states(&repository);
                assert!(
                    before_git.commands[0].is_empty(),
                    "{name} corpus repository is dirty"
                );

                let output = Command::new(binary)
                    .arg("inspect")
                    .arg("--json")
                    .args(["--scope", "baseline", "--base", base_commit])
                    .arg(&repository)
                    .env("CARGO_TARGET_DIR", target_root.join(name))
                    .env("CARGO_NET_OFFLINE", "true")
                    .env("RUSTUP_TOOLCHAIN", "1.97.1")
                    .output()
                    .unwrap();
                let report: Value = serde_json::from_slice(&output.stdout).unwrap();
                let after_files = support::content_states(&repository);
                let changed_paths = before_files
                    .keys()
                    .chain(after_files.keys())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .filter(|path| before_files.get(*path) != after_files.get(*path))
                    .cloned()
                    .collect::<Vec<_>>();
                let after_git = repository_state(&repository);
                let unchanged = changed_paths.is_empty() && after_git == before_git;
                assert!(
                    unchanged,
                    "{name} was mutated by baseline inspection: {changed_paths:?}",
                );
                let error_codes = report["errors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|error| error["code"].clone())
                    .collect::<Vec<_>>();
                json!({
                    "name": name,
                    "current_commit": current_commit,
                    "base_commit": base_commit,
                    "status": report["status"],
                    "delta": report["delta"]["summary"],
                    "error_codes": error_codes,
                    "gate": report["gate"]["status"],
                    "exit_code": output.status.code(),
                    "offline": true,
                    "lockfile_input": lockfile_input,
                    "repository_unchanged": unchanged,
                })
            })
            .collect(),
    )
}

fn measure_performance() -> Value {
    use std::time::{Duration, Instant};

    let fixture = rich_delta_fixture();
    let run = |arguments: &[&str]| {
        let started = Instant::now();
        let output = inspect(&fixture, arguments, &fixture.target);
        assert_eq!(output.status.code(), Some(0));
        started.elapsed()
    };
    for _ in 0..3 {
        run(&[]);
        run(&["--scope", "baseline", "--base", SELECTOR]);
    }
    let mut full = Vec::new();
    let mut baseline = Vec::new();
    for _ in 0..30 {
        full.push(run(&[]));
        baseline.push(run(&["--scope", "baseline", "--base", SELECTOR]));
    }
    full.sort();
    baseline.sort();
    let percentile = |durations: &[Duration]| durations[28];
    let full_p95 = percentile(&full);
    let baseline_p95 = percentile(&baseline);
    assert!(
        baseline_p95.as_nanos() * 10 <= full_p95.as_nanos() * 25,
        "baseline P95 {:.3} ms exceeds 2.5 times full P95 {:.3} ms",
        baseline_p95.as_secs_f64() * 1_000.0,
        full_p95.as_secs_f64() * 1_000.0,
    );
    fs::remove_dir_all(&fixture.root).unwrap();

    json!({
        "fixture": "rich-delta",
        "warmups": 3,
        "runs": 30,
        "maximum_ratio": 2.5,
        "measured": true,
        "verdict": "pass",
    })
}

#[test]
#[ignore = "requires the trusted pinned corpus and performs the complete EP-016 evaluation"]
fn baseline_delta_evaluation_is_reconstructed_from_measured_inputs() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let corpus_root = env::var_os("RUST_DOCTOR_BASELINE_DELTA_CORPUS_ROOT")
        .map(PathBuf::from)
        .expect("RUST_DOCTOR_BASELINE_DELTA_CORPUS_ROOT must name the trusted pinned corpus");
    let target_root = support::temporary_target("baseline-delta-evaluation", &NEXT_FIXTURE);
    let _ = fs::remove_dir_all(&target_root);
    fs::create_dir_all(&target_root).unwrap();

    let mut evaluation = local_evaluation();
    let public_repositories = measure_public_repositories(&corpus_root, &target_root);
    let performance = measure_performance();
    let document = evaluation.as_object_mut().unwrap();
    document.insert("public_repositories".to_owned(), public_repositories);
    document.insert("performance".to_owned(), performance);
    document.insert(
        "reconstruction".to_owned(),
        json!({
            "test": "baseline_delta_evaluation_is_reconstructed_from_measured_inputs",
            "public_repositories": "measured",
            "performance": "measured",
        }),
    );
    document.insert("verdict".to_owned(), Value::String("pass".to_owned()));

    if env::var_os("RUST_DOCTOR_UPDATE_BASELINE_DELTA_EVALUATION").is_some() {
        fs::write(
            artifact_path(),
            format!("{}\n", serde_json::to_string_pretty(&evaluation).unwrap()),
        )
        .unwrap();
    }
    let expected = evaluation_artifact();
    fs::remove_dir_all(target_root).unwrap();
    assert_eq!(
        evaluation,
        expected,
        "evaluation artifact differs:\n{}",
        serde_json::to_string_pretty(&evaluation).unwrap(),
    );
}
