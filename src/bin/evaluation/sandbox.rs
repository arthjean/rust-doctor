use super::{EvalError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const EVALUATION_CARGO_HOME: &str = ".rust-doctor-cargo-home";
const EVALUATION_CARGO_MARKER: &str = ".rust-doctor-evaluation-cache";
const TMP_BYTES: usize = 128 * 1024 * 1024;
const AUXILIARY_TMPFS_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn validate_checkout_tree(root: &Path) -> Result<()> {
    validate_tree(root, true).map(|_| ())
}

pub(crate) fn initialize_evaluation_cargo_home(checkout_root: &Path) -> Result<PathBuf> {
    let checkout_root = checkout_root.canonicalize().map_err(|error| {
        EvalError::io(
            "cannot canonicalize corpus checkout root",
            checkout_root,
            error,
        )
    })?;
    let cargo_home = checkout_root.join(EVALUATION_CARGO_HOME);
    std::fs::create_dir_all(&cargo_home).map_err(|error| {
        EvalError::io("cannot create evaluation Cargo home", &cargo_home, error)
    })?;
    let marker = cargo_home.join(EVALUATION_CARGO_MARKER);
    if !marker.exists() {
        let has_content = std::fs::read_dir(&cargo_home)
            .map_err(|error| {
                EvalError::io("cannot inspect evaluation Cargo home", &cargo_home, error)
            })?
            .next()
            .is_some();
        if has_content {
            return Err(EvalError::Unsupported(format!(
                "evaluation Cargo home '{}' is not empty and has no ownership marker",
                cargo_home.display()
            )));
        }
        std::fs::write(&marker, b"rust-doctor evaluation cache v1\n")
            .map_err(|error| EvalError::io("cannot mark evaluation Cargo home", &marker, error))?;
    }
    validate_evaluation_cargo_home(&checkout_root)
}

pub(crate) fn validate_evaluation_cargo_home(checkout_root: &Path) -> Result<PathBuf> {
    let checkout_root = checkout_root.canonicalize().map_err(|error| {
        EvalError::io(
            "cannot canonicalize corpus checkout root",
            checkout_root,
            error,
        )
    })?;
    let expected = checkout_root.join(EVALUATION_CARGO_HOME);
    let cargo_home = validate_tree(&expected, false)?;
    if !cargo_home.starts_with(&checkout_root) {
        return Err(EvalError::Unsupported(
            "evaluation Cargo home escapes the corpus checkout root".to_string(),
        ));
    }
    let marker = cargo_home.join(EVALUATION_CARGO_MARKER);
    let marker_metadata = std::fs::symlink_metadata(&marker)
        .map_err(|error| EvalError::io("cannot inspect evaluation cache marker", &marker, error))?;
    if !marker_metadata.is_file() || marker_metadata.file_type().is_symlink() {
        return Err(EvalError::Unsupported(
            "evaluation Cargo home has no regular ownership marker".to_string(),
        ));
    }
    for credential_file in ["credentials", "credentials.toml"] {
        if cargo_home.join(credential_file).exists() {
            return Err(EvalError::Unsupported(format!(
                "evaluation Cargo home contains forbidden {credential_file}"
            )));
        }
    }
    if inherited_cargo_home().is_some_and(|host| host == cargo_home) {
        return Err(EvalError::Unsupported(
            "evaluation refuses to mount the inherited host Cargo home".to_string(),
        ));
    }
    Ok(cargo_home)
}

fn validate_tree(root: &Path, skip_git: bool) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .map_err(|error| EvalError::io("cannot canonicalize read-only tree", root, error))?;
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| EvalError::io("cannot inspect checkout", &directory, error))?
        {
            let entry = entry.map_err(|error| {
                EvalError::io("cannot inspect checkout entry", &directory, error)
            })?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| EvalError::io("cannot inspect checkout path", &path, error))?;
            if metadata.file_type().is_symlink() {
                let target = path.canonicalize().map_err(|error| {
                    EvalError::io("cannot resolve checkout symlink", &path, error)
                })?;
                if !target.starts_with(&root) {
                    return Err(EvalError::Command(format!(
                        "sandbox rejected symlink escape '{}'",
                        relative_display(&root, &path)
                    )));
                }
            } else if metadata.is_dir() && (!skip_git || entry.file_name() != ".git") {
                pending.push(path);
            }
        }
    }
    Ok(root)
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::too_many_lines,
    reason = "mount ordering is a security invariant kept visible in one command constructor"
)]
pub(crate) fn command(
    checkout: &Path,
    binary: &Path,
    cargo_home: &Path,
    scratch_bytes: usize,
    scan_args: &[String],
) -> Result<Command> {
    let checkout = canonical(checkout, "checkout")?;
    let binary = canonical(binary, "rust-doctor binary")?;
    let bwrap = find_executable("bwrap")?;
    let sysroot = active_sysroot()?;
    let scratch_bytes = scratch_bytes.to_string();
    let tmp_bytes = TMP_BYTES.to_string();
    let auxiliary_bytes = AUXILIARY_TMPFS_BYTES.to_string();

    let mut command = Command::new(bwrap);
    command.args([
        "--die-with-parent",
        "--new-session",
        "--unshare-all",
        "--unshare-user",
        "--disable-userns",
        "--clearenv",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--size",
    ]);
    command
        .arg(tmp_bytes)
        .args(["--tmpfs", "/tmp", "--dir", "/workspace", "--size"]);
    command.arg(scratch_bytes).args([
        "--tmpfs",
        "/scratch",
        "--dir",
        "/tool",
        "--dir",
        "/toolchain",
        "--dir",
        "/toolchain/bin",
        "--dir",
        "/home",
        "--size",
    ]);
    command
        .arg(&auxiliary_bytes)
        .args(["--tmpfs", "/home/evaluator", "--size"]);
    command.arg(auxiliary_bytes).args([
        "--tmpfs",
        "/cargo",
        "--dir",
        "/rustup",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind-try",
        "/bin",
        "/bin",
        "--ro-bind-try",
        "/lib",
        "/lib",
        "--ro-bind-try",
        "/lib64",
        "/lib64",
    ]);
    bind_if_present(
        &mut command,
        &cargo_home.join("registry"),
        "/cargo/registry",
    );
    bind_if_present(&mut command, &cargo_home.join("git"), "/cargo/git");
    command
        .arg("--ro-bind")
        .arg(&sysroot)
        .arg("/toolchain")
        .arg("--ro-bind")
        .arg(checkout)
        .arg("/workspace")
        .arg("--ro-bind")
        .arg(binary)
        .arg("/tool/rust-doctor")
        .args(["--remount-ro", "/"])
        .args([
            "--setenv",
            "HOME",
            "/home/evaluator",
            "--setenv",
            "CARGO_HOME",
            "/cargo",
            "--setenv",
            "CARGO_TARGET_DIR",
            "/scratch/target",
            "--setenv",
            "CARGO_NET_OFFLINE",
            "true",
            "--setenv",
            "CARGO_BUILD_JOBS",
            "2",
            "--setenv",
            "GIT_CONFIG_NOSYSTEM",
            "1",
            "--setenv",
            "GIT_CONFIG_GLOBAL",
            "/dev/null",
            "--setenv",
            "GIT_TERMINAL_PROMPT",
            "0",
            "--setenv",
            "PATH",
            "/toolchain/bin:/usr/bin:/bin",
            "--chdir",
            "/workspace",
            "--",
            "/tool/rust-doctor",
        ])
        .args(scan_args);
    Ok(command)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn command(
    _checkout: &Path,
    _binary: &Path,
    _cargo_home: &Path,
    _scratch_bytes: usize,
    _scan_args: &[String],
) -> Result<Command> {
    Err(EvalError::Unsupported(
        "corpus execution requires Linux bubblewrap; preparation and artifact smoke remain cross-platform"
            .to_string(),
    ))
}

fn canonical(path: &Path, label: &str) -> Result<PathBuf> {
    path.canonicalize()
        .map_err(|error| EvalError::io("cannot canonicalize sandbox input", path, error))
        .and_then(|canonical| {
            if canonical.exists() {
                Ok(canonical)
            } else {
                Err(EvalError::Command(format!(
                    "{label} '{}' does not exist",
                    path.display()
                )))
            }
        })
}

fn inherited_cargo_home() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let cargo_home =
        std::env::var_os("CARGO_HOME").map_or_else(|| home.join(".cargo"), PathBuf::from);
    cargo_home.canonicalize().ok()
}

#[cfg(target_os = "linux")]
fn bind_if_present(command: &mut Command, source: &Path, destination: &str) {
    if source.exists() {
        command.arg("--ro-bind").arg(source).arg(destination);
    }
}

#[cfg(target_os = "linux")]
fn active_sysroot() -> Result<PathBuf> {
    let rustc = find_executable("rustc")?;
    let output = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .map_err(|error| EvalError::Command(format!("cannot identify active sysroot: {error}")))?;
    if !output.status.success() {
        return Err(EvalError::Unsupported(
            "rustc could not identify the active toolchain sysroot".to_string(),
        ));
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let sysroot = canonical(&sysroot, "active toolchain sysroot")?;
    for tool in ["cargo", "rustc", "rustdoc", "cargo-clippy", "clippy-driver"] {
        let executable = sysroot
            .join("bin")
            .join(tool)
            .canonicalize()
            .map_err(|error| {
                EvalError::io(
                    "cannot canonicalize active toolchain executable",
                    sysroot.join("bin").join(tool),
                    error,
                )
            })?;
        if !executable.starts_with(&sysroot) || !executable.is_file() {
            return Err(EvalError::Unsupported(format!(
                "active toolchain executable {tool} escapes its sysroot"
            )));
        }
    }
    Ok(sysroot)
}

#[cfg(target_os = "linux")]
fn find_executable(name: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| EvalError::Unsupported("PATH is unavailable".to_string()))?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            EvalError::Unsupported(format!(
                "{name} is required; corpus scans fail closed without a network sandbox"
            ))
        })
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::process::run_capped;
    use std::time::Duration;

    #[cfg(unix)]
    #[test]
    fn checkout_validation_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        symlink("/tmp", root.path().join("escape")).unwrap();
        let error = validate_checkout_tree(root.path()).unwrap_err();
        assert!(error.to_string().contains("symlink escape"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn malicious_build_script_cannot_write_outside_sandbox() {
        use std::os::unix::fs::PermissionsExt;

        if find_executable("bwrap").is_err() {
            assert!(
                std::env::var_os("RUST_DOCTOR_REQUIRE_SANDBOX_TESTS").is_none(),
                "bubblewrap is mandatory for protected corpus fixture tests"
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let checkout = root.path().join("checkout");
        let cargo_home = root.path().join("cargo");
        std::fs::create_dir_all(checkout.join("src")).unwrap();
        std::fs::create_dir_all(&cargo_home).unwrap();
        let marker = PathBuf::from(format!(
            "/tmp/rust-doctor-sandbox-escape-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        std::fs::write(
            checkout.join("Cargo.toml"),
            "[package]\nname=\"malicious\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
        )
        .unwrap();
        std::fs::write(checkout.join("src/lib.rs"), "pub fn value() {}\n").unwrap();
        std::fs::write(
            checkout.join("build.rs"),
            format!(
                "fn main() {{ let _ = std::fs::write({marker:?}, b\"escape\"); let _ = std::fs::write(\"/workspace/escape\", b\"escape\"); }}"
            ),
        )
        .unwrap();
        let mut lock = Command::new("cargo");
        lock.args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(checkout.join("Cargo.toml"));
        let lock_output = run_capped(lock, Duration::from_secs(20), 1024 * 1024).unwrap();
        assert!(
            lock_output.status.success(),
            "{}",
            String::from_utf8_lossy(&lock_output.stderr)
        );
        let launcher = root.path().join("candidate");
        std::fs::write(
            &launcher,
            "#!/bin/sh\nexec cargo check --locked --offline --manifest-path /workspace/Cargo.toml\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&launcher).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&launcher, permissions).unwrap();

        let command = command(&checkout, &launcher, &cargo_home, 128 * 1024 * 1024, &[]).unwrap();
        let output = run_capped(command, Duration::from_mins(1), 1024 * 1024).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!marker.exists());
        assert!(!checkout.join("escape").exists());
    }
}
