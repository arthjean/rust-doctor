use super::*;

static NEXT_REPOSITORY: AtomicUsize = AtomicUsize::new(0);

fn lfs_repository() -> (PathBuf, PathBuf) {
    let root = support::temporary_target("baseline-network-lfs", &NEXT_REPOSITORY);
    let _ = fs::remove_dir_all(&root);
    let repository = root.join("repository");
    write(repository.join("Cargo.toml"), base_manifest());
    write(repository.join(".gitignore"), "/target\n");
    write(repository.join(".gitattributes"), "*.bin filter=lfs diff=lfs merge=lfs -text\n");
    write(repository.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n");
    let pointer = |size: u32| {
        format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {size}\n",
            "4d7a2146".repeat(8)
        )
    };
    write(repository.join("assets/model.bin"), &pointer(1024));
    run(Command::new(env!("CARGO"))
        .args(["generate-lockfile", "--offline"])
        .current_dir(&repository));
    git(&repository, &["init", "--initial-branch=main", "--quiet"]);
    git(&repository, &["config", "user.name", "Rust Doctor"]);
    git(&repository, &["config", "user.email", "rust-doctor@example.invalid"]);
    git(&repository, &["add", "-A"]);
    commit(&repository, "base");
    write(repository.join("assets/model.bin"), &pointer(2048));
    write(repository.join("src/lib.rs"), "pub fn value() -> u8 { 2 }\n");
    git(&repository, &["add", "-A"]);
    for (key, value) in [
        ("filter.lfs.process", "git-lfs filter-process"),
        ("filter.lfs.smudge", "git-lfs smudge -- %f"),
        ("filter.lfs.clean", "git-lfs clean -- %f"),
        ("filter.lfs.required", "true"),
    ] {
        git(&repository, &["config", key, value]);
    }
    (repository, root)
}

#[test]
fn a_snapshot_never_runs_the_lfs_filter() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let (repository, root) = lfs_repository();
    let tools = root.join("tools");
    let calls = root.join("git-lfs.calls");
    write(
        tools.join("git-lfs"),
        &format!("#!/bin/sh\necho \"$@\" >> '{}'\nexit 1\n", calls.display()),
    );
    fs::set_permissions(tools.join("git-lfs"), fs::Permissions::from_mode(0o755)).unwrap();
    let path = env::join_paths(
        std::iter::once(tools.clone()).chain(env::split_paths(&env::var_os("PATH").unwrap())),
    )
    .unwrap();

    let probe = root.join("probe");
    let checkout = Command::new("git")
        .arg("checkout-index")
        .arg("--all")
        .arg(format!("--prefix={}/", probe.display()))
        .current_dir(&repository)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(!checkout.status.success(), "the stand-in was never reached");
    assert!(calls.is_file(), "the stand-in recorded nothing");
    fs::remove_file(&calls).unwrap();

    for arguments in [&["--scope", "baseline", "--base", "HEAD"][..], &["--staged", "--scope", "lines"][..]] {
        let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
            .arg(&repository)
            .args(["--yes", "--json", "--advisory-db", "/dev/null"])
            .args(arguments)
            .env("PATH", &path)
            .env("CARGO_TARGET_DIR", root.join("scan-target"))
            .env("CARGO_NET_OFFLINE", "true")
            .env_remove("RUSTFLAGS")
            .output()
            .unwrap();
        let report = report(&output);
        assert_eq!(report["status"], "complete", "{arguments:?}: {}", report["errors"]);
        assert!(
            !calls.exists(),
            "{arguments:?} ran git-lfs: {}",
            fs::read_to_string(&calls).unwrap_or_default()
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_offline_scan_of_an_uncached_dependency_ends_incomplete_on_cargo_s_error() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = support::temporary_target("baseline-network-offline", &NEXT_REPOSITORY);
    let _ = fs::remove_dir_all(&root);
    let workspace = root.join("workspace");
    write(
        workspace.join("Cargo.toml"),
        &format!("{}\n[dependencies]\nrust-doctor-uncached-probe = \"=0.0.1\"\n", base_manifest()),
    );
    write(workspace.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n");
    let started = std::time::Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(&workspace)
        .args(["--yes", "--json", "--advisory-db", "/dev/null", "--max-duration", "300"])
        .env("CARGO_TARGET_DIR", root.join("scan-target"))
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("RUSTFLAGS")
        .output()
        .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(120));
    assert_eq!(output.status.code(), Some(1));
    let report = report(&output);
    assert_eq!(report["status"], "incomplete");
    assert!(report["audit"]["score"].is_null(), "{}", report["audit"]["score"]);
    let quoted = report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|error| error["message"].as_str())
        .find(|message| message.contains("Cargo reported:"));
    assert!(quoted.is_some(), "no error quotes Cargo: {}", report["errors"]);
    let quoted = quoted.unwrap();
    assert!(quoted.contains("rust-doctor-uncached-probe"), "{quoted}");
    assert!(quoted.contains("offline"), "{quoted}");
    assert!(quoted.len() < 1_200, "{} bytes", quoted.len());
    let _ = fs::remove_dir_all(root);
}
