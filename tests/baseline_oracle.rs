#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

static NEXT_ORACLE: AtomicUsize = AtomicUsize::new(0);
const GIT_ENVIRONMENT_OVERRIDES: [&str; 10] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_EXTERNAL_DIFF",
    "GIT_PAGER",
];

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

fn normative_git(
    workspace: &Path,
    operation: impl IntoIterator<Item = OsString>,
    index: Option<&Path>,
) -> Output {
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
    for variable in GIT_ENVIRONMENT_OVERRIDES {
        command.env_remove(variable);
    }
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    run(&mut command)
}

fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

#[derive(Debug, PartialEq, Eq)]
struct RepositoryState {
    commands: Vec<Vec<u8>>,
    config: Vec<u8>,
    objects: std::collections::BTreeMap<String, (blake3::Hash, u64)>,
}

fn repository_state(root: &Path) -> RepositoryState {
    let commands = [
        vec!["status", "--porcelain=v1", "-z"],
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

#[derive(Debug, PartialEq, Eq)]
struct InventoryMeasurement {
    entries: usize,
    max_blob_bytes: u64,
    total_blob_bytes: u64,
    max_path_bytes: usize,
    inventory_stdout_bytes: usize,
}

fn measure_inventory(root: &Path, commit: &str) -> InventoryMeasurement {
    let inventory = git(root, &["ls-tree", "-r", "-z", "-l", "--full-tree", commit]).stdout;
    assert!(inventory.ends_with(&[0]));

    let mut measurement = InventoryMeasurement {
        entries: 0,
        max_blob_bytes: 0,
        total_blob_bytes: 0,
        max_path_bytes: 0,
        inventory_stdout_bytes: inventory.len(),
    };
    for record in inventory[..inventory.len() - 1].split(|byte| *byte == 0) {
        let separator = record.iter().position(|byte| *byte == b'\t').unwrap();
        let fields: Vec<_> = record[..separator]
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .collect();
        let size = std::str::from_utf8(fields[3])
            .unwrap()
            .parse::<u64>()
            .unwrap();
        measurement.entries += 1;
        measurement.max_blob_bytes = measurement.max_blob_bytes.max(size);
        measurement.total_blob_bytes += size;
        measurement.max_path_bytes = measurement.max_path_bytes.max(record.len() - separator - 1);
    }
    measurement
}

#[test]
fn git_255_oracle_materializes_through_an_external_index_without_mutation() {
    let root = support::temporary_target("baseline-oracle", &NEXT_ORACLE);
    let _ = fs::remove_dir_all(&root);
    let repository = root.join("repository");
    let workspace = repository.join("workspace");
    write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"baseline-oracle\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    );
    write(workspace.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n");
    write(repository.join("shared.txt"), "shared\n");
    symlink("lib.rs", workspace.join("src/internal.rs")).unwrap();
    git(&repository, &["init", "--initial-branch=main", "--quiet"]);
    git(&repository, &["config", "user.name", "Rust Doctor"]);
    git(
        &repository,
        &["config", "user.email", "rust-doctor@example.invalid"],
    );
    git(&repository, &["add", "."]);
    run(Command::new("git")
        .args(["commit", "--quiet", "-m", "baseline"])
        .current_dir(&repository)
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z"));
    git(&repository, &["branch", "private-base"]);
    write(workspace.join("src/lib.rs"), "pub fn value() -> u8 { 2 }\n");

    let before = repository_state(&repository);
    let revision = OsString::from("private-base^{commit}");
    let base = normative_git(
        &workspace,
        [
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from("--end-of-options"),
            revision,
        ],
        None,
    );
    let base = String::from_utf8(base.stdout).unwrap().trim().to_owned();
    let merge_base = normative_git(
        &workspace,
        [
            OsString::from("merge-base"),
            OsString::from("--all"),
            OsString::from(&base),
            OsString::from("HEAD"),
        ],
        None,
    );
    let merge_base = String::from_utf8(merge_base.stdout)
        .unwrap()
        .trim()
        .to_owned();
    assert_eq!(merge_base, base);
    assert!(matches!(merge_base.len(), 40 | 64));
    assert!(merge_base.bytes().all(|byte| byte.is_ascii_hexdigit()));

    let inventory = normative_git(
        &workspace,
        [
            OsString::from("ls-tree"),
            OsString::from("-r"),
            OsString::from("-z"),
            OsString::from("-l"),
            OsString::from("--full-tree"),
            OsString::from(&merge_base),
        ],
        None,
    );
    assert!(inventory.stdout.ends_with(&[0]));
    let inventory_paths: BTreeSet<_> = inventory.stdout[..inventory.stdout.len() - 1]
        .split(|byte| *byte == 0)
        .map(|record| {
            let separator = record.iter().position(|byte| *byte == b'\t').unwrap();
            std::str::from_utf8(&record[separator + 1..])
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        inventory_paths,
        BTreeSet::from([
            "shared.txt".to_owned(),
            "workspace/Cargo.toml".to_owned(),
            "workspace/src/internal.rs".to_owned(),
            "workspace/src/lib.rs".to_owned(),
        ])
    );

    let snapshot = root.join("snapshot");
    let tree = snapshot.join("tree");
    fs::create_dir_all(&tree).unwrap();
    fs::set_permissions(&snapshot, fs::Permissions::from_mode(0o700)).unwrap();
    let index = snapshot.join("index");
    normative_git(
        &workspace,
        [OsString::from("read-tree"), OsString::from(&merge_base)],
        Some(&index),
    );
    let mut prefix = OsString::from("--prefix=");
    prefix.push(tree.as_os_str());
    prefix.push(OsStr::new("/"));
    normative_git(
        &workspace,
        [
            OsString::from("checkout-index"),
            OsString::from("--all"),
            OsString::from("--force"),
            prefix,
        ],
        Some(&index),
    );
    assert_eq!(
        fs::read_to_string(tree.join("workspace/src/lib.rs")).unwrap(),
        "pub fn value() -> u8 { 1 }\n"
    );
    assert_eq!(
        fs::read_link(tree.join("workspace/src/internal.rs")).unwrap(),
        Path::new("lib.rs")
    );
    assert_eq!(repository_state(&repository), before);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn measured_product_and_pinned_repositories_retain_the_candidate_limits() {
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/baseline/oracle.json")).unwrap();
    let git_version = run(Command::new("git").arg("--version"));
    assert_eq!(
        std::str::from_utf8(&git_version.stdout).unwrap().trim(),
        oracle["git_version"].as_str().unwrap()
    );
    assert_eq!(oracle["decision"], "retain-candidate-limits");
    let limits = &oracle["limits"];
    let measurements = oracle["measurements"].as_array().unwrap();
    assert_eq!(measurements.len(), 4);
    for measurement in measurements {
        assert!(measurement["entries"].as_u64().unwrap() <= limits["entries"].as_u64().unwrap());
        assert!(
            measurement["max_blob_bytes"].as_u64().unwrap()
                <= limits["blob_bytes"].as_u64().unwrap()
        );
        assert!(
            measurement["total_blob_bytes"].as_u64().unwrap()
                <= limits["total_blob_bytes"].as_u64().unwrap()
        );
        assert!(
            measurement["inventory_stdout_bytes"].as_u64().unwrap()
                <= limits["inventory_stdout_bytes"].as_u64().unwrap()
        );
        assert!(
            measurement["max_path_bytes"].as_u64().unwrap()
                <= limits["path_bytes"].as_u64().unwrap()
        );
    }

    let product = &measurements[0];
    let measured = measure_inventory(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        product["commit"].as_str().unwrap(),
    );
    assert_eq!(
        measured.entries as u64,
        product["entries"].as_u64().unwrap()
    );
    assert_eq!(
        measured.max_blob_bytes,
        product["max_blob_bytes"].as_u64().unwrap()
    );
    assert_eq!(
        measured.total_blob_bytes,
        product["total_blob_bytes"].as_u64().unwrap()
    );
    assert_eq!(
        measured.max_path_bytes as u64,
        product["max_path_bytes"].as_u64().unwrap()
    );
    assert_eq!(
        measured.inventory_stdout_bytes as u64,
        product["inventory_stdout_bytes"].as_u64().unwrap()
    );
}
