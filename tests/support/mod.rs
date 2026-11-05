#![allow(dead_code, reason = "each test crate uses its own subset of these helpers")]

pub(crate) mod corpus;
pub(crate) mod fixture_inputs;
pub(crate) mod rule_scaling;

pub(crate) use fixture_inputs::fixture_dir;

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FileState {
    hash: blake3::Hash,
    length: u64,
    modified: Option<SystemTime>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitRepositoryState {
    pub(crate) config_hash: String,
    pub(crate) head: String,
    pub(crate) objects_hash: String,
    pub(crate) refs_hash: String,
    pub(crate) status_hash: String,
    pub(crate) tracked_files_hash: String,
    pub(crate) tree: String,
    pub(crate) working_state_hash: String,
}

#[cfg(unix)]
pub(crate) struct ProcessHarness {
    bin: PathBuf,
    log: PathBuf,
}

pub(crate) const CATEGORIES: [&str; 6] = [
    "correctness",
    "dependencies",
    "maintainability",
    "performance",
    "reliability",
    "security",
];

pub(crate) fn expected_clippy_command(policy: &serde_json::Value) -> Vec<String> {
    let base = [
        "cargo",
        "clippy",
        "--workspace",
        "--no-deps",
        "--keep-going",
        "--message-format=json",
        "--",
        "-A",
        "clippy::all",
    ];
    base.into_iter()
        .map(str::to_owned)
        .chain(
            policy["rules"]
                .as_array()
                .expect("a policy should publish its rules")
                .iter()
                .filter(|rule| {
                    rule["id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("clippy::"))
                        && rule["level"] != "off"
                })
                .flat_map(|rule| {
                    [
                        "-W".to_owned(),
                        rule["id"].as_str().unwrap_or_default().to_owned(),
                    ]
                }),
        )
        .chain(["-W".to_owned(), "warnings".to_owned()])
        .collect()
}

pub(crate) fn temporary_target(scope: &str, counter: &AtomicUsize) -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    base.canonicalize()
        .unwrap_or(base)
        .join(scope)
        .join(format!(
            "{}-{}",
            std::process::id(),
            counter.fetch_add(1, Ordering::Relaxed)
        ))
}

pub(crate) fn fresh_workspace(scope: &str, counter: &AtomicUsize) -> PathBuf {
    let root = temporary_target(scope, counter);
    fs::create_dir_all(&root).expect("a scratch workspace should be creatable");
    root
}

pub(crate) const NO_ADVISORY_DB: [&str; 2] = ["--advisory-db", "/dev/null"];

pub(crate) fn rust_doctor() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rust-doctor"));
    command
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("RUSTFLAGS")
        .env_remove("CLAUDECODE")
        .env_remove("CODEX_SANDBOX")
        .env_remove("CURSOR_AGENT");
    command
}

pub(crate) fn scan_target(workspace: &Path) -> PathBuf {
    let key = blake3::hash(workspace.as_os_str().as_encoded_bytes()).to_hex();
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/scan-caches");
    base.canonicalize().unwrap_or(base).join(key.get(..16).unwrap_or_default())
}

pub(crate) fn self_scan(keep: impl Fn(&str) -> bool) -> rust_doctor::InspectReport {
    let mut request = rust_doctor::InspectRequest::new(Path::new(env!("CARGO_MANIFEST_DIR")));
    for entry in rust_doctor::catalog() {
        if !keep(entry.id) {
            request = request.with_rule_override(rust_doctor::RuleOverride::new(
                entry.id,
                rust_doctor::RuleLevel::Off,
            ));
        }
    }
    rust_doctor::inspect(request)
}

pub(crate) fn project_current_wire_to_v7(output: &[u8]) -> Vec<u8> {
    let prefix = format!(
        "{{\"schema_version\":{},\"audit\":",
        rust_doctor::SCHEMA_VERSION
    );
    let payload = output
        .strip_prefix(prefix.as_bytes())
        .expect("the current wire should open on its schema version and its audit member");
    let suffix = &payload[value_end(payload, 0)..];
    let mut projected = Vec::with_capacity(output.len());
    projected.extend_from_slice(b"{\"schema_version\":7");
    projected.extend_from_slice(suffix);

    let summary = find(&projected, 0, b"\"summary\":{").expect("a report should carry a summary");
    let projected = remove_member(&projected, summary, "occurrences");
    let summary = find(&projected, 0, b"\"summary\":{").expect("a report should carry a summary");
    let mut projected = remove_member(&projected, summary, "distinct");
    while let Some(rule) = find(&projected, 0, b"\"tier\":") {
        projected = remove_member(&projected, rule, "tier");
    }
    while let Some(diagnostic) = find(&projected, 0, b"\"context\":") {
        projected = remove_member(&projected, diagnostic, "context");
    }
    while let Some(diagnostic) = find(&projected, 0, b"\"related\":") {
        projected = remove_member(&projected, diagnostic, "related");
    }
    while let Some(diagnostic) = find(&projected, 0, b"\"similarity_basis_points\":") {
        projected = remove_member(&projected, diagnostic, "similarity_basis_points");
    }
    while let Some(diagnostic) = find(&projected, 0, b"\"complexity\":") {
        projected = remove_member(&projected, diagnostic, "complexity");
    }
    while let Some(rule) = find(&projected, 0, b"\"corpus_noise_basis_points\":") {
        projected = remove_member(&projected, rule, "corpus_noise_basis_points");
    }
    while let Some(rule) = find(&projected, 0, b"\"corpus_reviewed_sites\":") {
        projected = remove_member(&projected, rule, "corpus_reviewed_sites");
    }
    while let Some(diagnostic) = find(&projected, 0, b"\"suggestion\":") {
        projected = remove_member(&projected, diagnostic, "suggestion");
    }
    while let Some(diagnostic) = find(&projected, 0, b"\"unscored\":") {
        projected = remove_member(&projected, diagnostic, "unscored");
    }
    if let Some(toolchain) = find(&projected, 0, b"\"rust_doctor\":") {
        projected = remove_member(&projected, toolchain, "rust_doctor");
    }
    if let Some(toolchain) = find(&projected, 0, b"\"removed_lint_flags\":") {
        projected = remove_member(&projected, toolchain, "removed_lint_flags");
    }
    while let Some(rule) = find(&projected, 0, b"\"not_evaluated\":") {
        projected = remove_member(&projected, rule, "not_evaluated");
    }
    for added in ["base_ref", "staged", "untracked_unreported"] {
        if let Some(scope) = find(&projected, 0, format!("\"{added}\":").as_bytes()) {
            projected = remove_member(&projected, scope, added);
        }
    }
    if let Some(summary) = find(&projected, 0, b"\"rule_count_matches\":") {
        projected = remove_member(&projected, summary, "rule_count_matches");
    }
    for added in ["ignore", "overrides", "suppressions", "ignored", "excluded_generated", "advisory_db"] {
        if let Some(member) = find(&projected, 0, format!("\"{added}\":").as_bytes()) {
            projected = remove_member(&projected, member, added);
        }
    }
    drop_scan_command(&projected)
}

pub(crate) fn drop_scan_command(wire: &[u8]) -> Vec<u8> {
    match find(wire, 0, b"\"scan\":{") {
        Some(scan) => remove_member(wire, scan, "command"),
        None => wire.to_vec(),
    }
}

fn find(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|position| from + position)
}

fn value_end(wire: &[u8], from: usize) -> usize {
    let mut values =
        serde_json::Deserializer::from_slice(&wire[from..]).into_iter::<serde_json::Value>();
    values
        .next()
        .expect("a JSON value should follow")
        .expect("the projected wire should stay valid JSON");
    from + values.byte_offset()
}

fn remove_member(wire: &[u8], from: usize, member: &str) -> Vec<u8> {
    let key = format!("\"{member}\":").into_bytes();
    let start = find(wire, from, &key).expect("the projected member should exist");
    let end = value_end(wire, start + key.len());
    let (start, end) = if wire[start - 1] == b',' {
        (start - 1, end)
    } else if wire[end] == b',' {
        (start, end + 1)
    } else {
        (start, end)
    };
    let mut projected = Vec::with_capacity(wire.len());
    projected.extend_from_slice(&wire[..start]);
    projected.extend_from_slice(&wire[end..]);
    projected
}

pub(crate) fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    let mut entries: Vec<_> = fs::read_dir(source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let target = destination.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(path, target).unwrap();
        }
    }
}

pub(crate) fn resolve_program(name: &str) -> PathBuf {
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
        .unwrap()
}

pub(crate) fn file_states(root: &Path) -> BTreeMap<String, FileState> {
    fn visit(root: &Path, directory: &Path, states: &mut BTreeMap<String, FileState>) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, states);
            } else {
                let Ok(metadata) = fs::symlink_metadata(&path) else {
                    continue;
                };
                let contents = if metadata.file_type().is_symlink() {
                    fs::read_link(&path)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                        .into_bytes()
                } else {
                    match fs::read(&path) {
                        Ok(contents) => contents,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => {
                            unreachable!("reading {} should not fail: {error}", path.display())
                        }
                    }
                };
                states.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    FileState {
                        hash: blake3::hash(&contents),
                        length: metadata.len(),
                        modified: metadata.modified().ok(),
                    },
                );
            }
        }
    }

    let mut states = BTreeMap::new();
    visit(root, root, &mut states);
    states
}

pub(crate) fn content_states(root: &Path) -> BTreeMap<String, (blake3::Hash, u64)> {
    file_states(root)
        .into_iter()
        .map(|(path, state)| (path, (state.hash, state.length)))
        .collect()
}

pub(crate) fn git_output(root: &Path, arguments: &[&str]) -> Output {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("Git process should start");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn hash_git_output(root: &Path, arguments: &[&str]) -> String {
    blake3::hash(&git_output(root, arguments).stdout)
        .to_hex()
        .to_string()
}

fn is_git_bookkeeping(path: &str) -> bool {
    let relative = Path::new(path);
    relative == Path::new(".git").join("index")
        || (relative.starts_with(".git") && path.ends_with(".lock"))
}

pub(crate) fn git_repository_state(root: &Path) -> GitRepositoryState {
    let content = content_states(root)
        .into_iter()
        .filter(|(path, _)| !is_git_bookkeeping(path))
        .map(|(path, (hash, length))| format!("{path}\0{hash}\0{length}"))
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = |arguments: &[&str]| {
        String::from_utf8(git_output(root, arguments).stdout)
            .expect("Git output should be UTF-8")
            .trim_end()
            .to_owned()
    };
    GitRepositoryState {
        head: trimmed(&["rev-parse", "HEAD"]),
        tree: trimmed(&["rev-parse", "HEAD^{tree}"]),
        refs_hash: hash_git_output(root, &["show-ref"]),
        tracked_files_hash: hash_git_output(root, &["ls-files", "-s"]),
        status_hash: hash_git_output(root, &["status", "--porcelain=v2", "--untracked-files=all"]),
        config_hash: hash_git_output(root, &["config", "--local", "--list"]),
        objects_hash: hash_git_output(root, &["count-objects", "-v"]),
        working_state_hash: blake3::hash(content.as_bytes()).to_hex().to_string(),
    }
}

#[cfg(unix)]
impl ProcessHarness {
    pub(crate) fn install(root: &Path) -> Self {
        Self::install_with(root, &[])
    }

    pub(crate) fn install_with_git(root: &Path) -> Self {
        Self::install_with(
            root,
            &[(
                "git",
                concat!(
                    "#!/bin/sh\n",
                    "operation=\n",
                    "for argument in \"$@\"; do\n",
                    "  case \"$argument\" in\n",
                    "    rev-parse|merge-base|diff|diff-index|ls-tree|read-tree|checkout-index) operation=$argument; printf 'git-%s\\n' \"$argument\" >> \"$RUST_DOCTOR_PROCESS_LOG\"; break ;;\n",
                    "  esac\n",
                    "done\n",
                    "case \"$RUST_DOCTOR_GIT_FAULT:$operation\" in\n",
                    "  merge-base-unavailable:merge-base) exit 1 ;;\n",
                    "  merge-base-ambiguous:merge-base) printf '%040d\\n%040d\\n' 1 2; exit 0 ;;\n",
                    "  baseline-limit:ls-tree) printf '100644 blob 1111111111111111111111111111111111111111 67108865\\tlarge.rs\\0'; exit 0 ;;\n",
                    "  baseline-gitlink:ls-tree) printf '160000 commit 1111111111111111111111111111111111111111 -\\tprivate-gitlink\\0'; exit 0 ;;\n",
                    "  snapshot-index-unreadable:*)\n",
                    "    case \"$*\" in *--work-tree=*) printf 'not an index' > \"$GIT_INDEX_FILE\" ;; esac ;;\n",
                    "  cleanup-failure:checkout-index)\n",
                    "    \"$RUST_DOCTOR_REAL_GIT\" \"$@\" || exit $?\n",
                    "    chmod 0500 \"$RUST_DOCTOR_CLEANUP_PARENT\" || exit $?\n",
                    "    exit 0 ;;\n",
                    "esac\n",
                    "exec \"$RUST_DOCTOR_REAL_GIT\" \"$@\"\n",
                ),
            )],
        )
    }

    fn install_with(root: &Path, extra_wrappers: &[(&str, &str)]) -> Self {
        let bin = root.join("wrapper-bin");
        let log = root.join("process.log");
        fs::create_dir_all(&bin).unwrap();
        for (name, source) in [
            (
                "cargo",
                concat!(
                    "#!/bin/sh\n",
                    "case \"$1\" in\n",
                    "  metadata) stage=metadata ;;\n",
                    "  --version) stage=cargo-version ;;\n",
                    "  clippy)\n",
                    "    if [ \"$2\" = \"--version\" ]; then stage=clippy-version; else stage=clippy; fi ;;\n",
                    "  *) stage=other ;;\n",
                    "esac\n",
                    "printf '%s\\n' \"$stage\" >> \"$RUST_DOCTOR_PROCESS_LOG\"\n",
                    "if [ -n \"$RUST_DOCTOR_PROCESS_DETAIL_LOG\" ]; then printf '%s\\t%s\\t%s\\t%s\\t%s\\n' \"$stage\" \"$PWD\" \"$CARGO_TARGET_DIR\" \"$*\" \"$RUSTUP_TOOLCHAIN\" >> \"$RUST_DOCTOR_PROCESS_DETAIL_LOG\"; fi\n",
                    "exec \"$RUST_DOCTOR_REAL_CARGO\" \"$@\"\n",
                ),
            ),
            (
                "rustc",
                concat!(
                    "#!/bin/sh\n",
                    "if [ \"$1\" = \"--version\" ]; then\n",
                    "  printf 'rustc-version\\n' >> \"$RUST_DOCTOR_PROCESS_LOG\"\n",
                    "fi\n",
                    "exec \"$RUST_DOCTOR_REAL_RUSTC\" \"$@\"\n",
                ),
            ),
            (
                "rustup",
                concat!(
                    "#!/bin/sh\n",
                    "if [ \"$1 $2\" = \"show active-toolchain\" ]; then\n",
                    "  printf 'rustup-toolchain\\n' >> \"$RUST_DOCTOR_PROCESS_LOG\"\n",
                    "  if [ -n \"$RUST_DOCTOR_FAIL_ACTIVE_TOOLCHAIN\" ]; then exit 1; fi\n",
                    "fi\n",
                    "exec \"$RUST_DOCTOR_REAL_RUSTUP\" \"$@\"\n",
                ),
            ),
        ]
        .into_iter()
        .chain(extra_wrappers.iter().copied())
        {
            install_executable(&bin.join(name), source);
        }
        fs::write(&log, []).unwrap();
        Self { bin, log }
    }

    pub(crate) fn command_path(&self) -> OsString {
        env::join_paths(
            std::iter::once(self.bin.clone())
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )
        .unwrap()
    }

    pub(crate) fn reset(&self) {
        fs::write(&self.log, []).unwrap();
    }

    pub(crate) fn counts(&self) -> BTreeMap<String, usize> {
        let mut counters = BTreeMap::new();
        for stage in self.events() {
            *counters.entry(stage).or_insert(0) += 1;
        }
        counters
    }

    pub(crate) fn events(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    pub(crate) fn log_path(&self) -> &Path {
        &self.log
    }
}

#[cfg(unix)]
fn install_executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}
