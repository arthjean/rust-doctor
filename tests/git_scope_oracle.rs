#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

static NEXT_REPOSITORY: AtomicUsize = AtomicUsize::new(0);
const AUTHOR_NAME: &str = "Rust Doctor Oracle";
const AUTHOR_EMAIL: &str = "rust-doctor-oracle@example.invalid";

struct OracleRepository {
    root: PathBuf,
    workspace: PathBuf,
    base_commit: String,
    baseline_tip: String,
}

fn run(command: &mut Command) -> Output {
    command.output().unwrap()
}

fn successful(command: &mut Command) -> Output {
    let output = run(command);
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn successful_git(root: &Path, arguments: &[&str]) -> Output {
    let mut command = Command::new("git");
    command.args(arguments).current_dir(root);
    successful(&mut command)
}

fn commit(root: &Path, message: &str, timestamp: &str) -> String {
    let mut command = Command::new("git");
    command
        .args(["commit", "--quiet", "-m", message])
        .current_dir(root)
        .env("GIT_AUTHOR_DATE", timestamp)
        .env("GIT_COMMITTER_DATE", timestamp);
    successful(&mut command);
    String::from_utf8(successful_git(root, &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned()
}

fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn repository() -> OracleRepository {
    let root = support::temporary_target("git-scope-oracle", &NEXT_REPOSITORY);
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    let workspace = root.join("workspace");
    write(
        workspace.join("Cargo.toml"),
        concat!(
            "[package]\n",
            "name = \"git-scope-oracle\"\n",
            "version = \"0.0.0\"\n",
            "edition = \"2024\"\n",
            "publish = false\n",
        ),
    );
    for (path, contents) in [
        ("src/lib.rs", "pub fn value() -> usize { 1 }\n"),
        ("src/deleted.rs", "pub const DELETED: bool = true;\n"),
        ("src/old.rs", "pub const OLD: bool = true;\n"),
        ("src/space name.rs", "pub const SPACE: bool = true;\n"),
        ("src/tab\tname.rs", "pub const TAB: bool = true;\n"),
        ("src/line\nname.rs", "pub const LINE: bool = true;\n"),
    ] {
        write(workspace.join(path), contents);
    }
    write(root.join("sibling.txt"), "base\n");

    successful_git(&root, &["init", "--quiet"]);
    successful_git(&root, &["config", "user.name", AUTHOR_NAME]);
    successful_git(&root, &["config", "user.email", AUTHOR_EMAIL]);
    successful_git(&root, &["add", "."]);
    let base_commit = commit(&root, "base", "2000-01-01T00:00:00Z");
    successful_git(&root, &["branch", "-M", "head"]);
    successful_git(&root, &["branch", "baseline"]);
    successful_git(&root, &["checkout", "--quiet", "baseline"]);
    write(root.join("sibling.txt"), "baseline\n");
    successful_git(&root, &["add", "sibling.txt"]);
    let baseline_tip = commit(&root, "baseline tip", "2000-01-02T00:00:00Z");
    successful_git(
        &root,
        &["update-ref", "refs/remotes/origin/baseline", &baseline_tip],
    );
    let mut tag = Command::new("git");
    tag.args(["tag", "-a", "baseline-tag", "-m", "baseline", &baseline_tip])
        .current_dir(&root)
        .env("GIT_COMMITTER_DATE", "2000-01-03T00:00:00Z");
    successful(&mut tag);

    successful_git(&root, &["checkout", "--quiet", "head"]);
    write(
        workspace.join("src/committed.rs"),
        "pub const COMMITTED: bool = true;\n",
    );
    successful_git(&root, &["add", "workspace/src/committed.rs"]);
    commit(&root, "head change", "2000-01-04T00:00:00Z");

    write(
        workspace.join("src/staged.rs"),
        "pub const STAGED: bool = true;\n",
    );
    write(
        workspace.join("src/added.rs"),
        "pub const ADDED: bool = true;\n",
    );
    successful_git(
        &root,
        &["add", "workspace/src/staged.rs", "workspace/src/added.rs"],
    );
    successful_git(
        &root,
        &["mv", "workspace/src/old.rs", "workspace/src/renamed.rs"],
    );
    successful_git(&root, &["rm", "--quiet", "workspace/src/deleted.rs"]);
    write(
        workspace.join("src/lib.rs"),
        "pub fn value() -> usize { 2 }\n",
    );
    write(
        workspace.join("src/space name.rs"),
        "pub const SPACE: bool = false;\n",
    );
    write(
        workspace.join("src/tab\tname.rs"),
        "pub const TAB: bool = false;\n",
    );
    write(
        workspace.join("src/line\nname.rs"),
        "pub const LINE: bool = false;\n",
    );
    write(workspace.join("src/untracked.rs"), "untracked\n");
    write(root.join("sibling.txt"), "outside workspace\n");

    OracleRepository {
        root,
        workspace,
        base_commit,
        baseline_tip,
    }
}

fn normative_git(workspace: &Path, operation: &[&OsStr]) -> Output {
    let mut command = Command::new("git");
    command
        .args(["-c", "color.ui=false", "-c", "core.fsmonitor=false"])
        .arg("--no-pager")
        .arg("-C")
        .arg(workspace)
        .args(operation)
        .current_dir(workspace)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    run(&mut command)
}

fn resolve(workspace: &Path, selector: &str) -> Output {
    let revision = format!("{selector}^{{commit}}");
    normative_git(
        workspace,
        &[
            OsStr::new("rev-parse"),
            OsStr::new("--verify"),
            OsStr::new("--quiet"),
            OsStr::new("--end-of-options"),
            OsStr::new(&revision),
        ],
    )
}

fn merge_base(workspace: &Path, base_commit: &str, head: &str) -> Output {
    normative_git(
        workspace,
        &[
            OsStr::new("merge-base"),
            OsStr::new("--all"),
            OsStr::new(base_commit),
            OsStr::new(head),
        ],
    )
}

fn diff(workspace: &Path, comparison_base: &str) -> Output {
    normative_git(
        workspace,
        &[
            OsStr::new("diff"),
            OsStr::new("--no-ext-diff"),
            OsStr::new("--no-renames"),
            OsStr::new("--relative"),
            OsStr::new("--name-only"),
            OsStr::new("-z"),
            OsStr::new("--diff-filter=ACMR"),
            OsStr::new(comparison_base),
            OsStr::new("--"),
            OsStr::new("."),
        ],
    )
}

fn oid(output: Output, length: usize) -> String {
    assert!(output.status.success());
    let oid = String::from_utf8(output.stdout).unwrap();
    let oid = oid.trim();
    assert_eq!(oid.len(), length);
    assert!(oid.bytes().all(|byte| byte.is_ascii_hexdigit()));
    oid.to_owned()
}

fn nul_paths(output: &[u8]) -> Vec<String> {
    if output.is_empty() {
        return Vec::new();
    }
    assert_eq!(output.last(), Some(&0));
    output[..output.len() - 1]
        .split(|byte| *byte == 0)
        .map(|path| String::from_utf8(path.to_vec()).unwrap())
        .collect()
}

fn commit_tree(root: &Path, tree: &str, parents: &[&str], message: &str, day: u8) -> String {
    let mut command = Command::new("git");
    command.arg("commit-tree").arg(tree);
    for parent in parents {
        command.args(["-p", parent]);
    }
    let timestamp = format!("2000-02-{day:02}T00:00:00Z");
    command
        .arg("-m")
        .arg(message)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", AUTHOR_NAME)
        .env("GIT_AUTHOR_EMAIL", AUTHOR_EMAIL)
        .env("GIT_COMMITTER_NAME", AUTHOR_NAME)
        .env("GIT_COMMITTER_EMAIL", AUTHOR_EMAIL)
        .env("GIT_AUTHOR_DATE", &timestamp)
        .env("GIT_COMMITTER_DATE", &timestamp);
    String::from_utf8(successful(&mut command).stdout)
        .unwrap()
        .trim()
        .to_owned()
}

#[test]
fn git_2_55_oracle_covers_the_twenty_four_normative_cases() {
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/git-scope/oracle.json")).unwrap();
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 24);
    assert_eq!(
        oracle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| case["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        (1..=24).collect::<Vec<_>>()
    );
    // Provenance, not an expectation of the machine: see the same reasoning in
    // tests/baseline_oracle.rs. The twenty-four cases below are what this
    // oracle pins, and they do not move with a git patch release.
    let recorded_git = oracle["git_version"].as_str().unwrap();
    assert!(
        recorded_git.starts_with("git version "),
        "the oracle should record the git it was measured under, found {recorded_git:?}"
    );

    let repository = repository();
    assert_eq!(repository.base_commit, oracle["base_commit"]);
    for selector in [
        "baseline",
        "refs/remotes/origin/baseline",
        "baseline-tag",
        repository.baseline_tip.as_str(),
    ] {
        assert_eq!(
            oid(resolve(&repository.workspace, selector), 40),
            repository.baseline_tip
        );
    }

    let comparison_base = oid(
        merge_base(&repository.workspace, &repository.baseline_tip, "HEAD"),
        40,
    );
    assert_eq!(comparison_base, repository.base_commit);
    assert_eq!(comparison_base, oracle["merge_base"]);
    let changed = diff(&repository.workspace, &comparison_base);
    assert!(changed.status.success());
    let paths = nul_paths(&changed.stdout);
    let expected: Vec<String> = serde_json::from_value(oracle["diff_paths"].clone()).unwrap();
    assert_eq!(paths, expected);
    assert_eq!(paths.iter().filter(|path| *path == "src/lib.rs").count(), 1);
    for excluded in [
        "src/deleted.rs",
        "src/old.rs",
        "src/untracked.rs",
        "sibling.txt",
        "../sibling.txt",
    ] {
        assert!(!paths.iter().any(|path| path == excluded));
    }
    assert_eq!(
        paths.iter().cloned().collect::<BTreeSet<_>>().len(),
        paths.len()
    );

    let clean = support::temporary_target("git-scope-empty-oracle", &NEXT_REPOSITORY);
    if clean.exists() {
        fs::remove_dir_all(&clean).unwrap();
    }
    fs::create_dir_all(&clean).unwrap();
    successful_git(&clean, &["init", "--quiet"]);
    successful_git(&clean, &["config", "user.name", AUTHOR_NAME]);
    successful_git(&clean, &["config", "user.email", AUTHOR_EMAIL]);
    write(clean.join("tracked"), "clean\n");
    successful_git(&clean, &["add", "tracked"]);
    let clean_base = commit(&clean, "clean", "2000-01-05T00:00:00Z");
    let empty = diff(&clean, &clean_base);
    assert!(empty.status.success());
    assert!(empty.stdout.is_empty());

    let sha256 = support::temporary_target("git-scope-sha256-oracle", &NEXT_REPOSITORY);
    if sha256.exists() {
        fs::remove_dir_all(&sha256).unwrap();
    }
    fs::create_dir_all(&sha256).unwrap();
    successful_git(&sha256, &["init", "--quiet", "--object-format=sha256"]);
    successful_git(&sha256, &["config", "user.name", AUTHOR_NAME]);
    successful_git(&sha256, &["config", "user.email", AUTHOR_EMAIL]);
    write(sha256.join("tracked"), "sha256\n");
    successful_git(&sha256, &["add", "tracked"]);
    let sha256_oid = commit(&sha256, "sha256", "2000-01-06T00:00:00Z");
    assert_eq!(oid(resolve(&sha256, &sha256_oid), 64), sha256_oid);

    assert!(
        !resolve(&repository.workspace, "missing-ref")
            .status
            .success()
    );
    let outside = support::temporary_target("git-scope-outside-oracle", &NEXT_REPOSITORY);
    if outside.exists() {
        fs::remove_dir_all(&outside).unwrap();
    }
    fs::create_dir_all(&outside).unwrap();
    write(outside.join(".git"), "gitdir: missing\n");
    assert!(!resolve(&outside, "main").status.success());

    let tree =
        String::from_utf8(successful_git(&repository.root, &["rev-parse", "HEAD^{tree}"]).stdout)
            .unwrap();
    let tree = tree.trim();
    let unrelated = commit_tree(&repository.root, tree, &[], "unrelated", 1);
    let unavailable = merge_base(&repository.workspace, &unrelated, "HEAD");
    assert!(!unavailable.status.success());
    assert!(unavailable.stdout.is_empty());

    let left = commit_tree(
        &repository.root,
        tree,
        &[&repository.base_commit],
        "left",
        2,
    );
    let right = commit_tree(
        &repository.root,
        tree,
        &[&repository.base_commit],
        "right",
        3,
    );
    let left_merge = commit_tree(&repository.root, tree, &[&left, &right], "left merge", 4);
    let right_merge = commit_tree(&repository.root, tree, &[&right, &left], "right merge", 5);
    let ambiguous = merge_base(&repository.workspace, &left_merge, &right_merge);
    assert!(ambiguous.status.success());
    let bases: BTreeSet<_> = String::from_utf8(ambiguous.stdout)
        .unwrap()
        .split_ascii_whitespace()
        .map(str::to_owned)
        .collect();
    assert_eq!(bases, BTreeSet::from([left, right]));
    assert_eq!(oracle["verdict"], "pass");
}
