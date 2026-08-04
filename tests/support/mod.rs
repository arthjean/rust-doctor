#![allow(dead_code)]

pub(crate) mod corpus;
pub(crate) mod rule_scaling;

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
    pub(crate) index_hash: String,
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

/// Catégories admissibles du catalogue, dans l'ordre publié. Les éteindre
/// toutes éteint chaque producteur, quel que soit le volume du catalogue.
pub(crate) const CATEGORIES: [&str; 6] = [
    "correctness",
    "dependencies",
    "maintainability",
    "performance",
    "reliability",
    "security",
];

/// Commande Clippy attendue pour une policy publiée.
///
/// Le catalogue grandit d'une tranche à l'autre, donc figer la liste dans
/// chaque test la rendrait fausse à chaque élargissement. La commande reste
/// néanmoins contrainte exactement: base, séparateur unique, puis un `-W` par
/// règle Clippy active de `policy.rules`, dans l'ordre publié.
pub(crate) fn expected_clippy_command(policy: &serde_json::Value) -> Vec<String> {
    let base = [
        "cargo",
        "clippy",
        "--workspace",
        "--no-deps",
        "--message-format=json",
        "--",
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
        .collect()
}

pub(crate) fn temporary_target(scope: &str, counter: &AtomicUsize) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(scope)
        .join(format!(
            "{}-{}",
            std::process::id(),
            counter.fetch_add(1, Ordering::Relaxed)
        ))
}

/// Ramène un rapport v10 sur les octets figés de v7.
///
/// v8 avait ajouté le bloc `audit`, v9 le `tier` de chaque règle de policy et
/// les deux grandeurs nommées de `summary`, v10 le `context` de chaque
/// diagnostic. Aucun champ historique n'est retiré ni retypé, donc la
/// projection consiste uniquement à enlever les membres ajoutés depuis.
///
/// C'est la condition qui rend une archive gelée durable: un schéma qui ajoute
/// se projette, un schéma qui déplace la valeur d'un champ existant ne se
/// projette pas.
pub(crate) fn project_v10_wire_to_v7(output: &[u8]) -> Vec<u8> {
    const PREFIX: &[u8] = b"{\"schema_version\":10,\"audit\":";
    let payload = output
        .strip_prefix(PREFIX)
        .expect("schema v10 should start with its audit member");
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
    drop_scan_command(&projected)
}

/// Retire `scan.command` d'un rapport sérialisé.
///
/// Une archive gelée prouve qu'aucun champ lu par un consommateur d'une version
/// passée n'a disparu ni changé de type. La commande, elle, est le relevé de ce
/// qui a réellement tourné: elle change dès que le catalogue s'élargit ou que
/// le périmètre bouge, deux choses que ce PRD fait exprès. La figer octet pour
/// octet reviendrait à jurer que la policy ne bougera plus, ce qui contredit
/// l'objectif de passer de 12 à 40 règles. Les tests qui tiennent à la commande
/// l'affirment séparément, sur sa forme et sur ce qu'elle a délibérément perdu.
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

/// Fin du premier document JSON présent à `from`, offsets d'octets compris.
fn value_end(wire: &[u8], from: usize) -> usize {
    let mut values =
        serde_json::Deserializer::from_slice(&wire[from..]).into_iter::<serde_json::Value>();
    values
        .next()
        .expect("a JSON value should follow")
        .expect("the projected wire should stay valid JSON");
    from + values.byte_offset()
}

/// Retire le premier membre `member` rencontré à partir de `from`, avec la
/// virgule qui le rattache à son objet.
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
                let metadata = fs::symlink_metadata(&path).unwrap();
                let contents = if metadata.file_type().is_symlink() {
                    fs::read_link(&path)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                        .into_bytes()
                } else {
                    fs::read(&path).unwrap()
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

pub(crate) fn git_repository_state(root: &Path) -> GitRepositoryState {
    let content = content_states(root)
        .into_iter()
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
        index_hash: hash_git_output(root, &["hash-object", ".git/index"]),
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
                    "    rev-parse|merge-base|diff|ls-tree|read-tree|checkout-index) operation=$argument; printf 'git-%s\\n' \"$argument\" >> \"$RUST_DOCTOR_PROCESS_LOG\"; break ;;\n",
                    "  esac\n",
                    "done\n",
                    "case \"$RUST_DOCTOR_GIT_FAULT:$operation\" in\n",
                    "  merge-base-unavailable:merge-base) exit 1 ;;\n",
                    "  merge-base-ambiguous:merge-base) printf '%040d\\n%040d\\n' 1 2; exit 0 ;;\n",
                    "  baseline-limit:ls-tree) printf '100644 blob 1111111111111111111111111111111111111111 67108865\\tlarge.rs\\0'; exit 0 ;;\n",
                    "  baseline-gitlink:ls-tree) printf '160000 commit 1111111111111111111111111111111111111111 -\\tprivate-gitlink\\0'; exit 0 ;;\n",
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
