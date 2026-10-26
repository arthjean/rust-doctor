#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![cfg(target_os = "linux")]

//! US-001: the two assumptions the Clippy-off metadata pin rests on, checked
//! on the toolchain that runs this suite.
//!
//! A1: `cargo metadata --no-deps` starts no program the workspace's
//! `.cargo/config.toml` names. A2: a `RUSTUP_TOOLCHAIN` name outranks a
//! `rust-toolchain.toml` `path` toolchain, and `rustup show active-toolchain`
//! answers that path without running anything from it. Each check runs the
//! child with `RUSTUP_TOOLCHAIN` removed unless the check sets it, and its
//! working directory set to the fixture, as `metadata_command` runs it.
//!
//! Linux only, because `/proc/self/cwd` is how the path toolchain follows the
//! working directory of whichever process reads the file.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicUsize;

mod support;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A program that leaves `markers/<name>` behind and fails, so whatever ran it
/// stops there and the marker is the only trace it leaves.
fn marker_program(root: &Path, path: &Path, name: &str) {
    let marker = root.join("markers").join(name);
    executable(
        path,
        &format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
    );
}

fn markers(root: &Path) -> Vec<String> {
    let mut found: Vec<_> = fs::read_dir(root.join("markers"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

/// A package whose Cargo configuration names a marker for every program it
/// can, and, when `path_toolchain` holds, a `rust-toolchain.toml` that selects
/// the marker toolchain under `tc/` by path.
fn fixture(path_toolchain: bool) -> PathBuf {
    let root = support::fresh_workspace("toolchain-pin-assumptions", &NEXT);
    for directory in ["src", ".cargo", "tc/bin", "markers"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    for name in ["rustc", "wrapper", "alias", "env"] {
        marker_program(&root, &root.join(format!("{name}.sh")), name);
    }
    let program = |name: &str| root.join(format!("{name}.sh")).display().to_string();
    fs::write(
        root.join(".cargo/config.toml"),
        format!(
            "[build]\nrustc = \"{}\"\nrustc-wrapper = \"{}\"\n\n[alias]\nmetadata = \"!{}\"\n\n[env]\nRUSTC_WORKSPACE_WRAPPER = \"{}\"\n",
            program("rustc"),
            program("wrapper"),
            program("alias"),
            program("env"),
        ),
    )
    .unwrap();
    for name in ["cargo", "rustc"] {
        marker_program(&root, &root.join("tc/bin").join(name), &format!("path-{name}"));
    }
    if path_toolchain {
        fs::write(
            root.join("rust-toolchain.toml"),
            "[toolchain]\npath = \"/proc/self/cwd/tc\"\n",
        )
        .unwrap();
    }
    root
}

/// The first field of `rustup show active-toolchain` in `directory`, or
/// `None` with the reason printed when rustup cannot answer.
fn active_toolchain(directory: &Path) -> Option<String> {
    let output = Command::new("rustup")
        .args(["show", "active-toolchain"])
        .current_dir(directory)
        .env_remove("RUSTUP_TOOLCHAIN")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let answer = String::from_utf8(output.stdout).ok()?;
    output
        .status
        .success()
        .then(|| answer.split_whitespace().next().map(str::to_owned))
        .flatten()
}

/// The installed toolchain this repository resolves to, which is the name a
/// pinned check runs under. `None`, with the reason printed, skips the test.
fn installed_toolchain() -> Option<String> {
    let found = active_toolchain(Path::new(env!("CARGO_MANIFEST_DIR")));
    if found.is_none() {
        eprintln!("skipped: rustup is not on PATH, so no toolchain file is read by a proxy");
    }
    found
}

fn metadata(root: &Path, toolchain: Option<&str>) -> std::process::ExitStatus {
    let mut command = Command::new("cargo");
    command
        .args(["metadata", "--no-deps", "--format-version", "1", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .current_dir(root)
        .env_remove("RUSTUP_TOOLCHAIN")
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(toolchain) = toolchain {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    command.status().unwrap()
}

/// A1: metadata starts none of the programs the workspace's Cargo
/// configuration names.
#[test]
fn metadata_runs_no_program_the_cargo_configuration_names() {
    let Some(toolchain) = installed_toolchain() else {
        return;
    };
    let root = fixture(false);
    assert!(metadata(&root, Some(&toolchain)).success());
    assert_eq!(markers(&root), Vec::<String>::new());
}

/// A2: a toolchain name outranks the path toolchain, and the control without
/// the name proves the fixture would run it.
#[test]
fn a_toolchain_name_outranks_a_path_toolchain() {
    let Some(toolchain) = installed_toolchain() else {
        return;
    };
    let root = fixture(true);
    assert!(metadata(&root, Some(&toolchain)).success());
    assert_eq!(markers(&root), Vec::<String>::new());

    assert!(!metadata(&root, None).success());
    assert_eq!(markers(&root), ["path-cargo"]);
}

/// A2: rustup answers the path without running anything from it, and the
/// answer is no toolchain name.
#[test]
fn rustup_names_the_path_toolchain_without_running_it() {
    if installed_toolchain().is_none() {
        return;
    }
    let root = fixture(true);
    let answer = active_toolchain(&root);
    assert_eq!(markers(&root), Vec::<String>::new());
    let answer = answer.unwrap();
    assert_eq!(answer, "/proc/self/cwd/tc");
    // `accepted` admits only ASCII alphanumerics, `-`, `_` and `.`; its own
    // unit test refuses this exact answer.
    assert!(answer.contains('/'));
}
