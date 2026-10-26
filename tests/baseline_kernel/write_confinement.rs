//! A committed target directory cannot steer the scan's own writes: a link at
//! any component the baseline and staged scans write under is refused, the
//! scan completes on the snapshot's private target, and nothing lands where
//! the link points.

use super::*;

#[test]
fn a_symlinked_target_component_takes_no_write_and_the_scan_completes() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let baseline: &[&str] = &["--scope", "baseline", "--base", SELECTOR];
    let staged: &[&str] = &["--staged", "--scope", "files"];
    for (component, arguments) in [
        ("rust-doctor", baseline),
        ("rust-doctor/baseline", baseline),
        ("rust-doctor/cache", baseline),
        ("rust-doctor/staged", staged),
    ] {
        let fixture = fixture(true);
        if arguments == staged {
            // The index holds a configuration the staged scan refuses; stage
            // the working one, which every other test leaves unstaged.
            let manifests = ["rust-doctor.toml", "rust-toolchain.toml", "Cargo.toml", "Cargo.lock"];
            git(&fixture.project, &[&["add"][..], &manifests].concat());
        }
        let elsewhere = fixture.root.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        let link = fixture.target.join(component);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(&elsewhere, &link).unwrap();
        // Twice: a cache the first run stored would be read back by the second.
        for _ in 0..2 {
            // The plain environment: the hostile git variables `inspect` sets
            // leave a staged scan no index to read.
            let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
                .args(["inspect", "--json", "--yes"])
                .args(arguments)
                .arg(&fixture.project)
                .env("CARGO_TARGET_DIR", &fixture.target)
                .env("CARGO_NET_OFFLINE", "true")
                .env_remove("RUSTUP_TOOLCHAIN")
                .output()
                .unwrap();
            assert!(
                !output.stdout.is_empty(),
                "{component}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let report = report(&output);
            assert_eq!(
                report["status"], "complete",
                "{component}: {}",
                report["errors"]
            );
        }
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0, "{component}");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        fs::remove_dir_all(&fixture.root).unwrap();
    }
}
