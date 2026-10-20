//! EP-001 US-002: the base side of a baseline runs the repository pass
//! through a git view of its snapshot, the repository's own `.git` with the
//! snapshot's index and the snapshot as work tree, so the five
//! `rust_doctor::repo::*` rules pair across the two sides. Before it, the
//! snapshot had no `.git`, the base side held no repository finding, and a
//! branch that changed nothing introduced every one of them.
//!
//! Key-shaped values are assembled at runtime so no source file of this
//! repository carries one.

use super::*;

static NEXT_REPOSITORY: AtomicUsize = AtomicUsize::new(0);

const CREDENTIAL: &str = "rust_doctor::repo::hardcoded_credential";
const WORKFLOW: &str = "name: release\non: push\njobs:\n  publish:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - env:\n          CARGO_REGISTRY_TOKEN: ${{ secrets.CRATES_IO_TOKEN }}\n        run: cargo publish --locked\n";

/// Not the key AWS documents, which the rule rightly reads as an example.
fn key(tail: &str) -> String {
    format!("AKIA{tail}")
}

/// A binary whose base commit tracks a key in `src/key.rs`, under `repository`,
/// with a scan target beside it that every run starts without.
fn repository() -> (PathBuf, PathBuf) {
    let root = support::temporary_target("baseline-repository-view", &NEXT_REPOSITORY);
    let _ = fs::remove_dir_all(&root);
    let repository = root.join("repository");
    write(
        repository.join("Cargo.toml"),
        "[package]\nname = \"repository-view\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n",
    );
    write(repository.join(".gitignore"), "/target\n");
    write(repository.join("src/main.rs"), "mod key;\nfn main() {\n    let _ = key::KEY;\n}\n");
    write(
        repository.join("src/key.rs"),
        &format!("pub const KEY: &str = \"{}\";\n", key("Q3VJ7RNYH2KXW4PZ")),
    );
    run(Command::new(env!("CARGO"))
        .args(["generate-lockfile", "--offline"])
        .current_dir(&repository));
    git(&repository, &["init", "--initial-branch=main", "--quiet"]);
    git(&repository, &["config", "user.name", "Rust Doctor"]);
    git(&repository, &["config", "user.email", "rust-doctor@example.invalid"]);
    git(&repository, &["add", "-A"]);
    commit(&repository, "base");
    (repository, root.join("scan-target"))
}

fn scan(repository: &Path, target: &Path, arguments: &[&str]) -> (Option<i32>, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(repository)
        .args(["--yes", "--json"])
        .args(arguments)
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("RUSTFLAGS")
        .output()
        .unwrap();
    (output.status.code(), report(&output))
}

/// `(code, path, line)` of every repository finding in `diagnostics`.
fn repository_findings(diagnostics: &Value) -> BTreeSet<(String, String, Option<u64>)> {
    diagnostics
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| {
            diagnostic["code"].as_str().is_some_and(|code| code.starts_with("rust_doctor::repo::"))
        })
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap().to_owned(),
                diagnostic["path"].as_str().unwrap_or_default().to_owned(),
                diagnostic["span"]["line_start"].as_u64(),
            )
        })
        .collect()
}

#[test]
fn a_branch_with_no_change_introduces_no_repository_finding() {
    // A baseline scan here creates a snapshot the locked tests would count.
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let (repository, target) = repository();
    let (code, report) = scan(
        &repository,
        &target,
        &["--scope", "baseline", "--base", "HEAD", "--blocking", "warning"],
    );
    assert_eq!(code, Some(0), "{report:#}");
    assert_eq!(report["delta"]["summary"]["introduced"], 0, "{report:#}");
    assert_eq!(report["delta"]["summary"]["fixed"], 0, "{report:#}");
    let credential = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == CREDENTIAL)
        .unwrap();
    assert!(
        report["delta"]["pre_existing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|matched| matched["current_id"] == credential["id"]),
        "{report:#}"
    );
}

#[test]
fn a_removed_key_is_fixed_and_an_added_one_introduced() {
    // A baseline scan here creates a snapshot the locked tests would count.
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let (repository, target) = repository();
    write(repository.join("src/key.rs"), "pub const KEY: &str = \"\";\n");
    let (code, report) = scan(&repository, &target, &["--scope", "baseline", "--base", "HEAD"]);
    assert_eq!(code, Some(0), "{report:#}");
    assert_eq!(report["delta"]["summary"]["fixed"], 1, "{report:#}");
    assert_eq!(report["delta"]["fixed"][0]["code"], CREDENTIAL);

    git(&repository, &["checkout", "--quiet", "--", "src/key.rs"]);
    write(
        repository.join("src/second.rs"),
        &format!("pub const SECOND: &str = \"{}\";\n", key("W4PZQ3VJ7RNYH2KX")),
    );
    git(&repository, &["add", "src/second.rs"]);
    let (_, report) = scan(&repository, &target, &["--scope", "baseline", "--base", "HEAD"]);
    let introduced: Vec<&Value> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| {
            report["delta"]["introduced"].as_array().unwrap().contains(&diagnostic["id"])
        })
        .filter(|diagnostic| diagnostic["code"] == CREDENTIAL)
        .collect();
    assert_eq!(introduced.len(), 1, "{report:#}");
    assert_eq!(introduced[0]["path"], "src/second.rs");
}

/// The view answers what a real checkout of the base commit answers: every
/// repository rule fires on the base side, and a scan of a clone checked out
/// at that commit names the same codes, paths and lines. The clone is built
/// once, as the working tree was, since an ignored lockfile is in no commit
/// and only a build writes it.
#[test]
fn the_base_side_finds_what_a_checkout_of_the_base_commit_finds() {
    // A baseline scan here creates a snapshot the locked tests would count.
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let (repository, target) = repository();
    write(repository.join(".gitignore"), "Cargo.lock\n");
    write(repository.join(".env"), "TOKEN=1\n");
    write(repository.join(".github/workflows/release.yml"), WORKFLOW);
    git(&repository, &["rm", "--cached", "--quiet", "Cargo.lock"]);
    git(&repository, &["add", "-A"]);
    commit(&repository, "leaky base");

    let checkout = repository.with_file_name("checkout");
    run(Command::new("git").arg("clone").arg("--quiet").arg(&repository).arg(&checkout));
    run(Command::new(env!("CARGO"))
        .args(["generate-lockfile", "--offline"])
        .current_dir(&checkout));
    let (_, full) = scan(&checkout, &target.with_file_name("checkout-target"), &[]);
    let expected = repository_findings(&full["diagnostics"]);
    let codes: BTreeSet<&str> = expected.iter().map(|(code, _, _)| code.as_str()).collect();
    assert_eq!(
        codes,
        BTreeSet::from([
            CREDENTIAL,
            "rust_doctor::repo::ignored_lockfile",
            "rust_doctor::repo::secret_in_build_step",
            "rust_doctor::repo::tracked_secret_file",
            "rust_doctor::repo::unignored_build_output",
        ]),
        "{full:#}"
    );

    // The branch repairs all of it, so every base-side finding is fixed.
    write(repository.join(".gitignore"), "/target\n");
    write(repository.join("src/key.rs"), "pub const KEY: &str = \"\";\n");
    git(&repository, &["rm", "--quiet", ".env", ".github/workflows/release.yml"]);
    git(&repository, &["add", "-A"]);
    let (code, report) = scan(&repository, &target, &["--scope", "baseline", "--base", "HEAD"]);
    assert_eq!(code, Some(0), "{report:#}");
    assert_eq!(repository_findings(&report["delta"]["fixed"]), expected, "{report:#}");
    assert!(repository_findings(&report["diagnostics"]).is_empty(), "{report:#}");
}

#[test]
fn an_unreadable_snapshot_index_fails_the_comparison_instead_of_blaming_the_branch() {
    // A baseline scan here creates a snapshot the locked tests would count.
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let (repository, target) = repository();
    let processes = support::ProcessHarness::install_with_git(repository.parent().unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(&repository)
        .args(["--yes", "--json", "--scope", "baseline", "--base", "HEAD"])
        .env("PATH", processes.command_path())
        .env("RUST_DOCTOR_PROCESS_LOG", processes.log_path())
        .env("RUST_DOCTOR_REAL_CARGO", support::resolve_program("cargo"))
        .env("RUST_DOCTOR_REAL_GIT", support::resolve_program("git"))
        .env("RUST_DOCTOR_REAL_RUSTC", support::resolve_program("rustc"))
        .env("RUST_DOCTOR_REAL_RUSTUP", support::resolve_program("rustup"))
        .env("RUST_DOCTOR_GIT_FAULT", "snapshot-index-unreadable")
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("RUSTFLAGS")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report = report(&output);
    assert_eq!(report["errors"][0]["code"], "baseline-scan-incomplete", "{report:#}");
    assert!(report["delta"].is_null());
}
