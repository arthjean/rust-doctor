#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![cfg(target_os = "linux")]

//! Proofs of EP-001 of the security audit remediation: a scan with every
//! Clippy rule off, `rules explain` and `ci install` start no program the
//! scanned tree selects.
//!
//! One fixture carries every way a tree selected a program before the fix:
//! three git filter drivers (one in `.git/config`, one through
//! `include.path`, one whose name holds `=`), a `.cargo/config.toml`
//! `[alias] clippy` that builds and runs the tree, a `rust-toolchain.toml`
//! whose `path` (`/proc/self/cwd/tc`) follows the working directory into the
//! tree, and a tracked git directory layout. Every program the fixture names
//! leaves a marker under a directory outside the tree, so "nothing ran" is an
//! empty directory, and a control run of plain git or Cargo proves each one
//! fires when it is reached.
//!
//! Linux only: `/proc/self/cwd` is what makes the path toolchain land in the
//! tree wherever it sits, and the markers are shell scripts.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

mod support;

static NEXT: AtomicUsize = AtomicUsize::new(0);

const BY_PATH: &str = "the workspace selects its toolchain by path; set RUSTUP_TOOLCHAIN to a toolchain name to scan it without Clippy";

/// The eight scope runs of the epic: four scopes, each also judging the index.
const SCOPES: [(&str, bool); 8] = [
    ("files", false),
    ("files", true),
    ("lines", false),
    ("lines", true),
    ("contextual", false),
    ("contextual", true),
    ("baseline", false),
    ("baseline", true),
];

#[derive(Clone, Copy, Default)]
struct Features {
    drivers: bool,
    alias: bool,
    path_toolchain: bool,
    embedded: bool,
}

const EVERYTHING: Features = Features {
    drivers: true,
    alias: true,
    path_toolchain: true,
    embedded: true,
};

/// A clone under `root`, and beside it the markers its programs leave and the
/// programs its git configuration names.
struct Tree {
    root: PathBuf,
    clone: PathBuf,
}

impl Tree {
    fn markers(&self) -> Vec<String> {
        let mut found: Vec<_> = fs::read_dir(self.root.join("markers"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        found
    }

    fn assert_no_marker(&self, what: &str) {
        assert_eq!(self.markers(), Vec::<String>::new(), "{what} started a program the tree selects");
    }
}

fn executable(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Git for building fixtures: no configuration of the machine running the
/// suite, and fixed identities and dates, so two fixtures with one content
/// hold the same commits.
fn git(directory: &Path, arguments: &[&str]) -> Output {
    Command::new("git")
        .args(["-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"])
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap()
}

fn git_ok(directory: &Path, arguments: &[&str]) {
    let output = git(directory, arguments);
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A git directory layout, `HEAD`, `objects/`, `refs/` and an index naming
/// `planted.env`, written into `directory` as ordinary files.
fn embedded_layout(root: &Path, directory: &Path, worktree: bool) {
    let donor = root.join("donor");
    fs::create_dir_all(&donor).unwrap();
    git_ok(&donor, &["init", "--quiet"]);
    write(&donor.join("planted.env"), "SECRET=planted\n");
    git_ok(&donor, &["add", "planted.env"]);
    for entry in ["objects", "refs"] {
        support::copy_tree(&donor.join(".git").join(entry), &directory.join(entry));
    }
    fs::copy(donor.join(".git/index"), directory.join("index")).unwrap();
    write(&directory.join("HEAD"), "ref: refs/heads/main\n");
    write(&directory.join("refs/heads/.keep"), "");
    write(&directory.join("objects/.keep"), "");
    let config = if worktree {
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n\tworktree = planted-tree\n"
    } else {
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n"
    };
    write(&directory.join("config"), config);
    fs::create_dir_all(directory.join("planted-tree")).unwrap();
    write(&directory.join("planted-tree/.keep"), "");
}

/// The fixture: a crate on `main`, a `feature` branch one commit ahead, a
/// staged change and an unstaged one, and every program `features` asks for.
/// The drivers are configured once the history is written, so building the
/// fixture runs none of them.
fn tree(features: Features) -> Tree {
    let root = support::fresh_workspace("untrusted-tree", &NEXT);
    let clone = root.join("clone");
    let markers = root.join("markers");
    fs::create_dir_all(&markers).unwrap();
    fs::create_dir_all(&clone).unwrap();
    let marker = |name: &str| markers.join(name).display().to_string();

    write(
        &clone.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&clone.join("src/lib.rs"), "mod b;\nmod c;\n\npub fn a() -> u32 {\n    b::b() + c::c()\n}\n");
    write(&clone.join("src/b.rs"), "pub fn b() -> u32 {\n    1\n}\n");
    write(&clone.join("src/c.rs"), "pub fn c() -> u32 {\n    2\n}\n");
    write(
        &clone.join(".gitattributes"),
        "*.rs filter=direct\nsrc/b.rs filter=included\nsrc/c.rs filter=p=q\n",
    );
    write(&clone.join(".gitignore"), "target/\n");
    if features.alias {
        write(&clone.join(".cargo/config.toml"), "[alias]\nclippy = [\"run\", \"--quiet\", \"--\"]\n");
        write(&clone.join("src/main.rs"), "fn main() {}\n");
        write(
            &clone.join("build.rs"),
            &format!(
                "fn main() {{\n    let _ = std::fs::write({:?}, \"\");\n}}\n",
                marker("alias-build")
            ),
        );
    }
    if features.path_toolchain {
        write(&clone.join("rust-toolchain.toml"), "[toolchain]\npath = \"/proc/self/cwd/tc\"\n");
        for name in ["cargo", "rustc"] {
            executable(
                &clone.join("tc/bin").join(name),
                &format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker(&format!("path-{name}"))),
            );
        }
    }
    if features.embedded {
        embedded_layout(&root, &clone.join("vendor/tool"), false);
    }

    git_ok(&clone, &["init", "--quiet"]);
    git_ok(&clone, &["add", "--all", "--force"]);
    git_ok(&clone, &["commit", "--quiet", "-m", "base"]);
    git_ok(&clone, &["checkout", "--quiet", "-b", "feature"]);
    write(&clone.join("src/b.rs"), "pub fn b() -> u32 {\n    10\n}\n");
    git_ok(&clone, &["commit", "--quiet", "-am", "feature"]);
    write(&clone.join("src/c.rs"), "pub fn c() -> u32 {\n    20\n}\n");
    git_ok(&clone, &["add", "src/c.rs"]);
    write(&clone.join("src/lib.rs"), "mod b;\nmod c;\n\npub fn a() -> u32 {\n    b::b() * c::c()\n}\n");

    if features.drivers {
        let programs = root.join("programs");
        executable(&programs.join("pass"), &format!("#!/bin/sh\ntouch '{}'/\"$1\"\ncat\n", markers.display()));
        executable(&programs.join("fail"), &format!("#!/bin/sh\ntouch '{}'/\"$1\"\nexit 1\n", markers.display()));
        let driver = |name: &str| {
            let tag = name.replace('=', "-");
            format!(
                "[filter \"{name}\"]\n\tclean = {pass} {tag}-clean\n\tsmudge = {pass} {tag}-smudge\n\tprocess = {fail} {tag}-process\n",
                pass = programs.join("pass").display(),
                fail = programs.join("fail").display(),
            )
        };
        let mut config = fs::read_to_string(clone.join(".git/config")).unwrap();
        config.push_str(&driver("direct"));
        config.push_str("[include]\n\tpath = included.config\n");
        config.push_str(&driver("p=q"));
        fs::write(clone.join(".git/config"), config).unwrap();
        fs::write(clone.join(".git/included.config"), driver("included")).unwrap();
    }
    Tree { root, clone }
}

/// The installed toolchain this repository resolves to, the name a pinned
/// scan runs under, or `None` without rustup.
fn installed_toolchain() -> Option<String> {
    static FOUND: OnceLock<Option<String>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let output = Command::new("rustup")
                .args(["show", "active-toolchain"])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .env_remove("RUSTUP_TOOLCHAIN")
                .output()
                .ok()?;
            let answer = String::from_utf8(output.stdout).ok()?;
            output
                .status
                .success()
                .then(|| answer.split_whitespace().next().map(str::to_owned))
                .flatten()
        })
        .clone()
}

/// `--rule <id>=off` for every catalogued Clippy rule.
fn clippy_off() -> &'static [String] {
    static ARGUMENTS: OnceLock<Vec<String>> = OnceLock::new();
    ARGUMENTS.get_or_init(|| {
        let output = support::rust_doctor().args(["rules", "list", "--json"]).output().unwrap();
        assert!(output.status.success());
        let rules: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
        rules
            .iter()
            .filter_map(|rule| rule["id"].as_str())
            .filter(|id| id.starts_with("clippy::"))
            .flat_map(|id| ["--rule".to_owned(), format!("{id}=off")])
            .collect()
    })
}

/// How the scanned binary's `RUSTUP_TOOLCHAIN` is set.
#[derive(Clone, Copy)]
enum Pin {
    /// The installed toolchain's name, as a rustup-proxied caller exports it.
    Named,
    /// Removed, as a caller outside any rustup proxy runs.
    Removed,
}

fn command(path: &Path, pin: Pin) -> Command {
    let mut command = support::rust_doctor();
    command
        .env("CARGO_TARGET_DIR", support::scan_target(path))
        .env("CARGO_NET_OFFLINE", "true")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("RUSTUP_TOOLCHAIN");
    if let (Pin::Named, Some(toolchain)) = (pin, installed_toolchain()) {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    command
}

/// A `--json` scan of `path` with every Clippy rule off.
fn scan(path: &Path, arguments: &[&str], pin: Pin) -> (Output, Value) {
    let output = command(path, pin)
        .arg(path)
        .args(["--yes", "--json"])
        .args(support::NO_ADVISORY_DB)
        .args(arguments)
        .args(clippy_off())
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (output, report)
}

fn scope_arguments(scope: &str, staged: bool) -> Vec<&str> {
    let mut arguments = vec!["--scope", scope, "--base", "main"];
    if staged {
        arguments.push("--staged");
    }
    arguments
}

fn assert_scanned(output: &Output, report: &Value, what: &str) {
    assert!(
        report["status"] == "complete",
        "{what}: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        report["errors"]
    );
}

fn diagnostic_paths(report: &Value, code: &str) -> Vec<String> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == code)
        .filter_map(|diagnostic| diagnostic["path"].as_str().map(str::to_owned))
        .collect()
}

/// US-002: the three drivers fire under plain git, and none of them under
/// any of the eight scope runs, whose reports equal those of the same tree
/// without the drivers.
#[test]
fn no_configured_filter_driver_runs_under_any_scope() {
    let driven = tree(Features {
        drivers: true,
        ..Features::default()
    });
    let plain = tree(Features::default());
    for (scope, staged) in SCOPES {
        let arguments = scope_arguments(scope, staged);
        let (output, report) = scan(&driven.clone, &arguments, Pin::Named);
        assert_scanned(&output, &report, &format!("{arguments:?}"));
        driven.assert_no_marker(&format!("{arguments:?}"));
        let (_, reference) = scan(&plain.clone, &arguments, Pin::Named);
        assert_eq!(report, reference, "{arguments:?}: the drivers changed the report");
    }

    // The control: plain git runs each driver, so the fixture is live.
    git(&driven.clone, &["diff", "main"]);
    git(
        &driven.clone,
        &["checkout-index", "--all", &format!("--prefix={}/control/", driven.root.display())],
    );
    let markers = driven.markers();
    for driver in ["direct", "included", "p-q"] {
        assert!(
            markers.iter().any(|marker| marker.starts_with(&format!("{driver}-"))),
            "plain git ran no program of {driver}: {markers:?}"
        );
    }
}

/// US-003: a crate directory that is also a git directory layout is judged
/// on the files it holds, never on the repository its tracked files fake.
#[test]
fn an_embedded_git_directory_never_answers_for_the_clone() {
    let embedded = |worktree: bool| {
        let root = support::fresh_workspace("untrusted-tree", &NEXT);
        let tool = root.join("clone/vendor/tool");
        write(&tool.join("Cargo.toml"), "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
        write(&tool.join("src/lib.rs"), "pub fn tool() {}\n");
        write(&tool.join(".env"), "TOKEN=clone\n");
        embedded_layout(&root, &tool, worktree);
        let clone = root.join("clone");
        git_ok(&clone, &["init", "--quiet"]);
        git_ok(&clone, &["add", "--all", "--force"]);
        git_ok(&clone, &["commit", "--quiet", "-m", "clone"]);
        (clone, tool)
    };
    let (clone, tool) = embedded(false);

    // Discovery from the clone refuses the layout as its one candidate.
    let (_, report) = scan(&clone, &[], Pin::Named);
    assert_eq!(report["errors"][0]["code"], "no-manifest", "{report}");

    // Named directly, the crate is judged on its own files: git refuses the
    // fabricated repository and the pass walks what is there.
    let (output, report) = scan(&tool, &[], Pin::Named);
    assert_scanned(&output, &report, "the embedded crate");
    assert_eq!(diagnostic_paths(&report, "rust_doctor::repo::tracked_secret_file"), [".env"]);
    assert!(!report.to_string().contains("planted"), "{report}");

    // `core.worktree` in the embedded config changes nothing.
    let (_, worktree) = embedded(true);
    let (_, moved) = scan(&worktree, &[], Pin::Named);
    assert_eq!(moved, report);

    // With no clone above it, the layout alone is walked the same way.
    let alone = support::fresh_workspace("untrusted-tree", &NEXT).join("tool");
    support::copy_tree(&tool, &alone);
    let (output, alone) = scan(&alone, &[], Pin::Named);
    assert_scanned(&output, &alone, "the layout alone");
    assert_eq!(diagnostic_paths(&alone, "rust_doctor::repo::tracked_secret_file"), [".env"]);
    assert!(!alone.to_string().contains("planted"), "{alone}");
}

/// US-004: a Clippy-off scan probes no version, so a workspace alias of
/// `clippy` never builds the tree, and the report says "not probed".
#[test]
fn a_clippy_off_scan_probes_no_toolchain_version() {
    let tree = tree(Features {
        alias: true,
        ..Features::default()
    });
    let (output, report) = scan(&tree.clone, &[], Pin::Named);
    assert_scanned(&output, &report, "a full scan");
    tree.assert_no_marker("a full Clippy-off scan");
    for tool in ["rustc", "cargo", "clippy"] {
        assert_eq!(report["toolchain"][tool], Value::Null, "{tool}");
    }

    // The linear report prints no version line and no placeholder for it.
    let linear = command(&tree.clone, Pin::Named)
        .arg(&tree.clone)
        .arg("--yes")
        .args(support::NO_ADVISORY_DB)
        .args(clippy_off())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&linear.stdout);
    for absent in ["null", "rustc", "clippy 0."] {
        assert!(!text.contains(absent), "{absent:?} in {text}");
    }

    // A baseline scan run twice reuses the base side the first stored, with
    // the versions it never probed hashed as empty fields.
    let baseline = || {
        let output = command(&tree.clone, Pin::Named)
            .arg(&tree.clone)
            .args(["--yes", "--verbose", "--scope", "baseline", "--base", "main"])
            .args(support::NO_ADVISORY_DB)
            .args(clippy_off())
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    if installed_toolchain().is_some() {
        assert!(baseline().contains("Base side cache: miss"));
        assert!(baseline().contains("Base side cache: hit"));
    }
    tree.assert_no_marker("a Clippy-off baseline scan");

    // The control: Cargo itself runs the alias, which builds the tree.
    Command::new("cargo")
        .args(["clippy", "--version"])
        .current_dir(&tree.clone)
        .env("CARGO_TARGET_DIR", support::scan_target(&tree.root))
        .output()
        .unwrap();
    assert_eq!(tree.markers(), ["alias-build"]);
}

/// US-005: a path toolchain never runs under a Clippy-off scan. Without an
/// inherited name the scan is refused before metadata starts; with one, it
/// runs under that name.
#[test]
fn a_path_toolchain_never_runs_under_a_clippy_off_scan() {
    let Some(_) = installed_toolchain() else {
        eprintln!("skipped: rustup is not on PATH, so no proxy reads a toolchain file");
        return;
    };
    let tree = tree(Features {
        path_toolchain: true,
        ..Features::default()
    });
    let (output, report) = scan(&tree.clone, &[], Pin::Removed);
    tree.assert_no_marker("an unpinned Clippy-off scan");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(report["errors"][0]["message"], BY_PATH, "{report}");

    let (output, report) = scan(&tree.clone, &[], Pin::Named);
    assert_scanned(&output, &report, "a pinned scan");
    tree.assert_no_marker("a pinned Clippy-off scan");

    // The control: the proxy runs the tree's cargo when nothing pins it.
    Command::new("cargo")
        .arg("--version")
        .current_dir(&tree.clone)
        .env_remove("RUSTUP_TOOLCHAIN")
        .output()
        .unwrap();
    assert_eq!(tree.markers(), ["path-cargo"]);
}

/// US-005: without rustup on `PATH` no proxy reads a toolchain file, and
/// metadata runs unpinned as it always did.
#[test]
fn without_rustup_metadata_runs_unpinned() {
    let tree = tree(Features::default());
    let bin = tree.root.join("no-rustup-bin");
    fs::create_dir_all(&bin).unwrap();
    for program in ["cargo", "git"] {
        std::os::unix::fs::symlink(support::resolve_program(program), bin.join(program)).unwrap();
    }
    let output = command(&tree.clone, Pin::Removed)
        .arg(&tree.clone)
        .args(["--yes", "--json"])
        .args(support::NO_ADVISORY_DB)
        .args(clippy_off())
        .env("PATH", &bin)
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_scanned(&output, &report, "a scan without rustup");
}

/// `rules explain` and `ci install --dry-run` on `path`, through `harness`
/// when one is given, so every Cargo they start is logged.
fn explain_and_install(path: &Path, harness: Option<&support::ProcessHarness>) -> [Output; 2] {
    let run = |before: &[&str], after: &[&str]| {
        let mut command = command(path, Pin::Removed);
        command.args(before).arg(path).args(after);
        if let Some(harness) = harness {
            harness_environment(&mut command, harness);
        }
        command.output().unwrap()
    };
    [
        run(&["rules", "explain", "clippy::todo"], &[]),
        run(&["ci", "install"], &["--branch", "main", "--dry-run"]),
    ]
}

fn harness_environment(command: &mut Command, harness: &support::ProcessHarness) {
    command
        .env("PATH", harness.command_path())
        .env("RUST_DOCTOR_PROCESS_LOG", harness.log_path())
        .env("RUST_DOCTOR_REAL_CARGO", support::resolve_program("cargo"))
        .env("RUST_DOCTOR_REAL_RUSTC", support::resolve_program("rustc"))
        .env("RUST_DOCTOR_REAL_RUSTUP", support::resolve_program("rustup"));
}

fn stdout(output: &Output) -> String {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// US-006: both commands find the workspace root from the manifests alone,
/// so the path toolchain never runs, no Cargo starts, and a member path
/// answers for its workspace.
#[test]
fn explain_and_ci_install_start_no_toolchain_process() {
    let toolchain_file = tree(Features {
        path_toolchain: true,
        ..Features::default()
    });
    let plain = tree(Features::default());
    let fixed = explain_and_install(&toolchain_file.clone, None).map(|output| stdout(&output));
    toolchain_file.assert_no_marker("rules explain and ci install --dry-run");
    let reference = explain_and_install(&plain.clone, None).map(|output| stdout(&output));
    assert_eq!(fixed, reference);

    // Under the fake Cargo harness, neither command starts Cargo at all,
    // while a scan through the same harness is seen asking for metadata.
    let harness = support::ProcessHarness::install(&plain.root);
    for output in explain_and_install(&plain.clone, Some(&harness)) {
        stdout(&output);
    }
    assert_eq!(harness.events(), Vec::<String>::new());
    let mut scanned = command(&plain.clone, Pin::Named);
    scanned.arg(&plain.clone).args(["--yes", "--json"]).args(support::NO_ADVISORY_DB);
    harness_environment(&mut scanned, &harness);
    scanned.args(clippy_off()).output().unwrap();
    assert!(harness.events().contains(&"metadata".to_owned()), "{:?}", harness.events());

    // A member path answers for the directory that declares `[workspace]`.
    let workspace = tree(Features::default());
    let manifest = fs::read_to_string(workspace.clone.join("Cargo.toml")).unwrap();
    fs::write(
        workspace.clone.join("Cargo.toml"),
        format!("{manifest}\n[workspace]\nmembers = [\"member\"]\n"),
    )
    .unwrap();
    write(
        &workspace.clone.join("member/Cargo.toml"),
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&workspace.clone.join("member/src/lib.rs"), "");
    write(&workspace.clone.join("rust-doctor.toml"), "[rules]\n\"clippy::todo\" = \"error\"\n");
    let member = explain_and_install(&workspace.clone.join("member"), None).map(|output| stdout(&output));
    let root = explain_and_install(&workspace.clone, None).map(|output| stdout(&output));
    assert!(member[0].contains("Effective level: error (config-rule)"), "{}", member[0]);
    assert_eq!(member, root);
}

/// EP-001's definition of done: the one fixture carrying every way a tree
/// selected a program leaves no marker across the eight scope runs, unpinned
/// or pinned, nor under `rules explain` and `ci install --dry-run`.
#[test]
fn the_epic_fixture_leaves_no_marker() {
    let tree = tree(EVERYTHING);
    for (scope, staged) in SCOPES {
        let arguments = scope_arguments(scope, staged);
        // Unpinned, the path toolchain is refused before metadata starts.
        let (output, _) = scan(&tree.clone, &arguments, Pin::Removed);
        tree.assert_no_marker(&format!("{arguments:?} unpinned"));
        if installed_toolchain().is_some() {
            assert_eq!(output.status.code(), Some(2), "{arguments:?} unpinned");
        }
        // Pinned, the scan runs every git call its scope makes.
        let (output, report) = scan(&tree.clone, &arguments, Pin::Named);
        tree.assert_no_marker(&format!("{arguments:?} pinned"));
        if installed_toolchain().is_some() {
            assert_scanned(&output, &report, &format!("{arguments:?}"));
        }
    }
    for output in explain_and_install(&tree.clone, None) {
        stdout(&output);
    }
    tree.assert_no_marker("rules explain and ci install --dry-run");
}
