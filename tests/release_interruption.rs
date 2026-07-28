//! Repeated release-level interruption contract (EP-004 / US-014).

#![allow(clippy::expect_used, clippy::unwrap_used)]

#[cfg(unix)]
mod unix {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    const TRIALS: usize = 20;
    const DEADLINE: Duration = Duration::from_secs(2);

    fn project() -> tempfile::TempDir {
        let fixture = tempfile::tempdir().unwrap();
        fs::create_dir_all(fixture.path().join("src")).unwrap();
        fs::write(
            fixture.path().join("Cargo.toml"),
            "[package]\nname='interrupt-fixture'\nversion='0.1.0'\nedition='2024'\nrust-version='1.97'\n",
        )
        .unwrap();
        fs::write(
            fixture.path().join("rust-doctor.toml"),
            "lint = true\ndependencies = false\n",
        )
        .unwrap();
        fs::write(
            fixture.path().join("src/lib.rs"),
            "pub fn value(input: Option<u8>) -> u8 { input.unwrap() }\n",
        )
        .unwrap();
        fixture
    }

    fn fake_clippy(root: &Path) -> std::path::PathBuf {
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        let executable = bin.join("cargo-clippy");
        fs::write(
            &executable,
            "#!/usr/bin/env bash\nset -eu\nprintf '%s\\n' \"$$\" > \"$RUST_DOCTOR_SIGNAL_CHILD\"\nexec sleep 30\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).unwrap();
        bin
    }

    fn wait_for_child_pid(path: &Path, parent: &mut Child) -> u32 {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Ok(value) = fs::read_to_string(path)
                && let Ok(pid) = value.trim().parse::<u32>()
            {
                return pid;
            }
            let status = parent.try_wait().unwrap();
            assert!(
                status.is_none(),
                "rust-doctor exited before analyzer launch: {status:?}"
            );
            assert!(
                Instant::now() < deadline,
                "analyzer child did not start within two seconds"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_exit(child: &mut Child) -> std::process::ExitStatus {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "rust-doctor did not exit within two seconds"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn process_exists(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn run_trial(signal: &str, fixture: &Path, fake_bin: &Path, child_pid_file: &Path) {
        fs::write(child_pid_file, "").unwrap();
        let inherited_path = std::env::var_os("PATH").unwrap_or_default();
        let joined_path = std::env::join_paths(
            std::iter::once(fake_bin.to_path_buf()).chain(std::env::split_paths(&inherited_path)),
        )
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
            .arg(fixture)
            .args(["--offline", "--no-color"])
            .env("PATH", joined_path)
            .env("RUST_DOCTOR_SIGNAL_CHILD", child_pid_file)
            .env("RUST_DOCTOR_DISABLE_ANIMATION", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let analyzer_pid = wait_for_child_pid(child_pid_file, &mut child);
        let sent = Command::new("kill")
            .args([signal, &child.id().to_string()])
            .status()
            .unwrap();
        assert!(sent.success(), "failed to send {signal}");
        let status = wait_for_exit(&mut child);
        assert_eq!(status.code(), Some(130), "{signal} returned {status}");
        thread::sleep(Duration::from_millis(20));
        assert!(
            !process_exists(analyzer_pid),
            "{signal} left analyzer process {analyzer_pid} alive"
        );
    }

    #[test]
    #[ignore = "release certification repeats 20 SIGINT and 20 SIGTERM trials"]
    fn signals_terminate_analyzer_groups_within_two_seconds() {
        let fixture = project();
        let fake_bin = fake_clippy(fixture.path());
        let child_pid_file = fixture.path().join("analyzer.pid");
        for signal in ["-INT", "-TERM"] {
            for _ in 0..TRIALS {
                run_trial(signal, fixture.path(), &fake_bin, &child_pid_file);
            }
        }
    }
}
