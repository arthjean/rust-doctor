#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![cfg(unix)]

//! EP-005 US-018: a baseline or staged snapshot is created only under a parent
//! no other user can write. `/tmp` at mode 1777 is refused, since another user
//! could plant a `.cargo/config.toml` or a `rust-toolchain.toml` there that
//! Cargo would apply to the snapshot, and with no other candidate passing the
//! scan reports why instead of creating anything.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

mod support;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A committed one-crate repository with one more change staged, and a home
/// its group can write, both under a scratch directory.
fn repository() -> (PathBuf, PathBuf) {
    let scratch = support::temporary_target("snapshot-parent", &NEXT);
    let _ = fs::remove_dir_all(&scratch);
    let root = scratch.join("repository");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"snapshot-parent\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn base() {}\n").unwrap();
    for arguments in [
        &["init", "-q"][..],
        &["add", "-A"],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "base",
        ],
    ] {
        support::git_output(&root, arguments);
    }
    fs::write(root.join("src/lib.rs"), "pub fn base() {}\npub fn staged() {}\n").unwrap();
    support::git_output(&root, &["add", "-A"]);
    let home = scratch.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o775)).unwrap();
    (root, home)
}

fn snapshots_in(directory: &Path) -> BTreeSet<PathBuf> {
    fs::read_dir(directory)
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

#[test]
fn a_world_writable_tmp_and_no_private_candidate_create_no_snapshot() {
    let tmp = Path::new("/tmp");
    let mode = fs::metadata(tmp).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o1777, "this proof needs /tmp at mode 1777");
    let (root, home) = repository();

    for arguments in [
        &["--scope", "baseline", "--base", "HEAD"][..],
        &["--staged", "--scope", "files"],
    ] {
        let before = snapshots_in(tmp);
        let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
            .arg(&root)
            .args(["--yes", "--json"])
            .args(arguments)
            .env("TMPDIR", tmp)
            .env("HOME", &home)
            .env_remove("XDG_RUNTIME_DIR")
            .env_remove("XDG_CACHE_HOME")
            .env("CARGO_TARGET_DIR", support::scan_target(&root))
            .env("CARGO_NET_OFFLINE", "true")
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();

        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {report:#}");
        assert_ne!(report["status"], "complete", "{arguments:?}: {report:#}");
        let errors = report["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1, "{arguments:?}: {report:#}");
        assert_eq!(errors[0]["code"], "baseline-temp-unavailable");
        assert_eq!(
            errors[0]["message"],
            "no private directory for the snapshot: /tmp is writable by other users",
            "{arguments:?}"
        );
        let after = snapshots_in(tmp);
        assert!(
            after.is_subset(&before),
            "{arguments:?} created {:?} under /tmp",
            after.difference(&before).collect::<Vec<_>>()
        );
        assert!(!home.join(".cache").exists(), "{arguments:?} created a cache home");
    }
}

/// The same run with a private runtime directory takes it, completes, and
/// leaves no snapshot behind in it.
#[test]
fn a_private_runtime_directory_takes_the_snapshot_a_world_writable_tmp_refuses() {
    let (root, home) = repository();
    let runtime = home.parent().unwrap().join("runtime");
    fs::create_dir_all(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(home.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(&root)
        .args(["--yes", "--json", "--staged", "--scope", "files"])
        .env("TMPDIR", "/tmp")
        .env("HOME", &home)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("XDG_CACHE_HOME")
        .env("CARGO_TARGET_DIR", support::scan_target(&root))
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(report["errors"], Value::Array(Vec::new()), "{report:#}");
    assert!(snapshots_in(&runtime).is_empty(), "the snapshot was not removed");
}
