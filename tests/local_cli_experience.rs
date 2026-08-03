#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
}

fn project(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

fn kernel(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/kernel-contract")
        .join(name)
}

fn local(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/local-cli-experience")
        .join(name)
}

fn run(arguments: &[&str], path: &Path) -> Output {
    binary()
        .args(arguments)
        .arg(path)
        .stdin(Stdio::null())
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .expect("rust-doctor should start")
}

fn terminal(arguments: &[&str], path: &Path) -> String {
    let output = run(arguments, path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Scanning Rust files..."));
    String::from_utf8(output.stdout).expect("terminal output should be UTF-8")
}

#[test]
fn root_path_default_and_historical_alias_execute_the_same_inspection() {
    let fixture = project("clean");
    let root = run(&["--json"], &fixture);
    let alias = run(&["inspect", "--json"], &fixture);
    assert!(root.status.success());
    assert!(alias.status.success());
    assert_eq!(root.stdout, alias.stdout);
    assert!(root.stderr.is_empty());
    assert!(alias.stderr.is_empty());

    let default = binary()
        .current_dir(&fixture)
        .args(["--json", "--yes"])
        .stdin(Stdio::null())
        .output()
        .expect("rust-doctor should start");
    assert!(default.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&default.stdout).unwrap()["project"]["manifest_path"],
        "Cargo.toml"
    );
}

#[test]
fn json_is_one_clean_v8_document_and_invalid_scope_stops_before_scan() {
    let output = run(&["--json"], &project("clippy-warning"));
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 8);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Scanning Rust files"));

    let rejected = binary()
        .args(["--scope", "files"])
        .arg("/path/that/must/not/be-inspected")
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
    assert!(rejected.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("--scope files requires --base <REF>"));
    assert!(!stderr.contains("Scanning Rust files"));
}

#[test]
fn terminal_default_verbose_clean_partial_and_no_rust_modes_are_truthful() {
    let standard = terminal(&["--yes"], &kernel("todo"));
    let ordered = [
        "Scope: full codebase",
        "Scanned 1 files in ",
        "Top warning:",
        "All 6 issues",
        "Run with --verbose",
        "Rust Doctor score:",
        "Share:",
        "Docs:",
        "GitHub:",
    ];
    let mut previous = 0usize;
    for marker in ordered {
        let position = standard
            .find(marker)
            .expect("terminal section should exist");
        assert!(
            position >= previous,
            "{marker} was out of order\n{standard}"
        );
        previous = position;
    }

    let verbose = terminal(&["--verbose", "--yes"], &kernel("dbg-macro"));
    assert_eq!(verbose.matches("src/lib.rs:").count(), 3);
    assert_eq!(verbose.matches("Help: Remove dbg!").count(), 3);
    assert!(!verbose.contains("Run with --verbose"));

    let clean = terminal(&["--yes"], &project("clean"));
    assert!(clean.contains("No issues found."));
    assert!(clean.contains("All 0 issues"));
    assert!(clean.contains("Rust Doctor score: 100/100 Great"));
    assert!(!clean.contains("What would you like to do next?"));

    let partial = run(&["--yes"], &project("compile-error"));
    assert_eq!(partial.status.code(), Some(1));
    let partial = String::from_utf8(partial.stdout).unwrap();
    assert!(partial.contains("Core partial"));
    assert!(!partial.contains("Share:"));
    assert!(!partial.contains("projected"));

    let no_rust = run(&["--yes"], &local("no-rust"));
    let no_rust = String::from_utf8(no_rust.stdout).unwrap();
    assert!(no_rust.contains("Score unavailable: no Rust files were analyzed."));
    assert!(!no_rust.contains("Share:"));
    assert!(!no_rust.contains("What would you like to do next?"));
}

#[test]
fn redirected_ci_and_yes_runs_never_offer_or_launch_handoff() {
    for environment in [None, Some("1")] {
        let mut command = binary();
        command
            .arg(project("clippy-warning"))
            .stdin(Stdio::null())
            .env("CARGO_NET_OFFLINE", "true");
        if let Some(value) = environment {
            command.env("CI", value);
        }
        let output = command.output().unwrap();
        let terminal = String::from_utf8(output.stdout).unwrap();
        assert!(output.status.success());
        assert!(!terminal.contains("What would you like to do next?"));
        assert!(!terminal.contains("Choose what to scan"));
    }

    let yes = terminal(&["--yes"], &project("clippy-warning"));
    assert!(!yes.contains("What would you like to do next?"));

    let redirected = binary()
        .arg("--yes")
        .arg(kernel("todo"))
        .env("COLUMNS", "140")
        .env("TERM", "xterm-256color")
        .output()
        .unwrap();
    let redirected = String::from_utf8(redirected.stdout).unwrap();
    assert!(
        redirected.lines().all(|line| line.chars().count() <= 80),
        "{redirected}"
    );
}

#[cfg(unix)]
mod tty {
    use std::env;
    use std::fs::{self, File};
    use std::io::{self, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{binary, kernel};

    static TEMPORARY: AtomicUsize = AtomicUsize::new(0);

    struct PtyProcess {
        child: Child,
        terminal: File,
        output: Vec<u8>,
    }

    impl PtyProcess {
        fn spawn(mut command: Command) -> Self {
            Self::spawn_with_stderr(&mut command, true)
        }

        fn spawn_with_redirected_stderr(mut command: Command) -> Self {
            Self::spawn_with_stderr(&mut command, false)
        }

        fn spawn_with_stderr(command: &mut Command, stderr_is_terminal: bool) -> Self {
            let mut master = 0;
            let mut slave = 0;
            // SAFETY: openpty initializes both descriptors, which are immediately owned by File.
            let result = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            assert_eq!(result, 0, "{}", io::Error::last_os_error());
            // SAFETY: openpty returned unique owned descriptors on success.
            let terminal = unsafe { File::from_raw_fd(master) };
            // SAFETY: openpty returned unique owned descriptors on success.
            let slave = unsafe { File::from_raw_fd(slave) };
            let flags = unsafe { libc::fcntl(terminal.as_raw_fd(), libc::F_GETFL) };
            assert!(flags >= 0, "{}", io::Error::last_os_error());
            // SAFETY: terminal is a valid open descriptor and F_SETFL only updates its flags.
            let result = unsafe {
                libc::fcntl(
                    terminal.as_raw_fd(),
                    libc::F_SETFL,
                    flags | libc::O_NONBLOCK,
                )
            };
            assert_eq!(result, 0, "{}", io::Error::last_os_error());

            let slave_fd = slave.as_raw_fd();
            let stderr = if stderr_is_terminal {
                Stdio::from(slave.try_clone().unwrap())
            } else {
                Stdio::null()
            };
            command
                .stdin(Stdio::from(slave.try_clone().unwrap()))
                .stdout(Stdio::from(slave.try_clone().unwrap()))
                .stderr(stderr)
                .env("TERM", "xterm-256color")
                .env("COLUMNS", "100")
                .env_remove("CI")
                .env_remove("NO_COLOR");
            // SAFETY: the closure only invokes async-signal-safe libc calls before exec.
            unsafe {
                command.pre_exec(move || {
                    if libc::setsid() < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if libc::ioctl(slave_fd, libc::TIOCSCTTY, 0) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let child = command.spawn().unwrap();
            Self {
                child,
                terminal,
                output: Vec::new(),
            }
        }

        fn wait_for(&mut self, needle: &str) {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                self.read_available();
                if String::from_utf8_lossy(&self.output).contains(needle) {
                    return;
                }
                if let Some(status) = self.child.try_wait().unwrap() {
                    panic!(
                        "process exited with {status} before {needle:?}: {}",
                        String::from_utf8_lossy(&self.output)
                    );
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {needle:?}"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn send(&mut self, input: &[u8]) {
            self.terminal.write_all(input).unwrap();
            self.terminal.flush().unwrap();
        }

        fn wait(mut self) -> (ExitStatus, String) {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                self.read_available();
                if let Some(status) = self.child.try_wait().unwrap() {
                    self.read_available();
                    return (status, String::from_utf8_lossy(&self.output).into_owned());
                }
                if Instant::now() >= deadline {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    panic!(
                        "PTY child timed out: {}",
                        String::from_utf8_lossy(&self.output)
                    );
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn read_available(&mut self) {
            let mut buffer = [0_u8; 8_192];
            loop {
                match self.terminal.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => self.output.extend_from_slice(&buffer[..read]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                    Err(error) => panic!("PTY read failed: {error}"),
                }
            }
        }
    }

    impl Drop for PtyProcess {
        fn drop(&mut self) {
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    fn temporary_root(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/local-cli-tty")
            .join(format!(
                "{}-{name}-{}",
                std::process::id(),
                TEMPORARY.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn run_git(root: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .args(arguments)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?}");
    }

    fn changed_project() -> PathBuf {
        let root = temporary_root("cancel");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"tty-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
        run_git(&root, &["init", "--quiet"]);
        run_git(&root, &["add", "."]);
        run_git(
            &root,
            &[
                "-c",
                "user.name=Rust Doctor",
                "-c",
                "user.email=rust-doctor@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        );
        fs::write(root.join("src/lib.rs"), "pub fn value() -> u8 { 2 }\n").unwrap();
        root
    }

    fn write_agent(path: &Path) {
        fs::write(
            path,
            "#!/bin/sh\nprintf '%s' \"$#\" > \"$RD_HANDOFF_CAPTURE/argc\"\nprintf '%s' \"$1\" > \"$RD_HANDOFF_CAPTURE/payload\"\npwd > \"$RD_HANDOFF_CAPTURE/cwd\"\nexec 3> \"$RD_HANDOFF_CAPTURE/tty\"\nfor fd in 0 1 2; do if [ -t \"$fd\" ]; then printf y >&3; else printf n >&3; fi; done\n",
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn escape_q_and_ctrl_c_cancel_scope_selection_before_scan_with_exit_130() {
        for (name, input) in [
            ("q", b"q".as_slice()),
            ("escape", b"\x1b".as_slice()),
            ("ctrl-c", b"\x03".as_slice()),
        ] {
            let root = changed_project();
            let before = fs::read(root.join("src/lib.rs")).unwrap();
            let mut command = binary();
            command.arg(&root);
            let mut process = PtyProcess::spawn(command);
            process.wait_for("Choose what to scan");
            process.send(input);
            let (status, output) = process.wait();
            assert_eq!(status.code(), Some(130), "{name}: {output}");
            assert!(output.contains("Scan cancelled."), "{name}: {output}");
            assert!(!output.contains("Scanned "), "{name}: {output}");
            assert_eq!(fs::read(root.join("src/lib.rs")).unwrap(), before);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn scope_selection_uses_stdin_and_stdout_when_stderr_is_redirected() {
        let root = changed_project();
        let mut command = binary();
        command.arg(&root);
        let mut process = PtyProcess::spawn_with_redirected_stderr(command);
        process.wait_for("Choose what to scan");
        process.send(b"q");
        let (status, output) = process.wait();
        assert_eq!(status.code(), Some(130), "{output}");
        assert!(!output.contains("Scanned "), "{output}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_scope_skips_scope_prompt_and_codex_inherits_one_payload_cwd_and_tty() {
        let root = temporary_root("handoff");
        let agents = root.join("agents");
        let capture = root.join("capture");
        fs::create_dir_all(&agents).unwrap();
        fs::create_dir_all(&capture).unwrap();
        for executable in ["claude", "codex", "cursor-agent"] {
            write_agent(&agents.join(executable));
        }
        let path = env::join_paths(
            std::iter::once(agents.clone())
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )
        .unwrap();

        let mut command = binary();
        command
            .arg(kernel("todo"))
            .args(["--scope", "full"])
            .env("PATH", path)
            .env("RD_HANDOFF_CAPTURE", &capture);
        let mut process = PtyProcess::spawn(command);
        process.wait_for("What would you like to do next?");
        assert!(!String::from_utf8_lossy(&process.output).contains("Choose what to scan"));
        process.send(b"\x1b[B\r");
        let (status, output) = process.wait();
        assert_eq!(status.code(), Some(0), "{output}");
        assert_eq!(fs::read_to_string(capture.join("argc")).unwrap(), "1");
        let payload = fs::read(capture.join("payload")).unwrap();
        assert!(payload.len() <= 12 * 1024);
        assert!(String::from_utf8_lossy(&payload).contains("Validate with:"));
        assert_eq!(fs::read_to_string(capture.join("tty")).unwrap(), "yyy");
        assert_eq!(
            PathBuf::from(fs::read_to_string(capture.join("cwd")).unwrap().trim()),
            kernel("todo").canonicalize().unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
