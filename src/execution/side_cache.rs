//! The base side of a baseline comparison, judged once per base.
//!
//! An agent rescanning with `--scope baseline` after every edit used to
//! materialize and lint the unchanged base on every run. A complete base side
//! is a function of its git tree, the toolchain, the policy and what Cargo
//! reads from the environment and from outside the repository, so it is
//! stored under a digest of exactly those, as the normalized diagnostics and
//! the evidence the delta pairs them on. A hit then needs no snapshot, no
//! Cargo process and no read of the base's sources.
//!
//! The cache can only ever cost a rescan. A file that does not decode, an
//! entry whose workspace members differ from the current ones, or an
//! unreadable key ingredient is a miss, never an error, and
//! `RUST_DOCTOR_NO_CACHE=1` turns it off. It writes only under Cargo's target
//! directory, through a temporary file and a rename, merged with what another
//! run stored meanwhile and pruned to [`ENTRY_LIMIT`] entries.

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use cargo_metadata::Metadata;
use serde::{Deserialize, Serialize};

use super::{CommandEnvironment, KnownLints, RunOptions, Toolchain};
use crate::delta::Evidence;
use crate::git::{GitCall, GitFailure, OUTPUT_TOO_LARGE, git_arguments, run_git};
use crate::policy::PolicyPlan;
use crate::report::{Diagnostic, SCHEMA_VERSION};
use crate::structure::StructureSettings;

/// Base sides kept per target directory: a few branches, each a few edits
/// apart from where it started.
const ENTRY_LIMIT: usize = 20;
const FILE_VERSION: u32 = 1;
/// The largest cache file read back; past it the cache is a miss.
const FILE_BYTES_LIMIT: u64 = 256 * 1024 * 1024;
const ENVIRONMENT_PREFIXES: [&str; 5] = ["CARGO", "RUST", "CC", "CFLAGS", "CXX"];

/// A base side read back from the cache.
#[derive(Debug)]
pub(crate) struct CachedSide {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) evidence: Evidence,
}

/// Where a miss stores the base side it is about to compute.
#[derive(Debug)]
pub(crate) struct SideCache {
    path: PathBuf,
    key: String,
}

/// What the lookup answered, published under `--verbose` and nowhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOutcome {
    Hit,
    Miss,
    Disabled,
}

impl CacheOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug)]
pub(crate) enum Lookup {
    Disabled,
    Hit(CachedSide),
    Miss(SideCache),
}

impl Lookup {
    pub(crate) const fn outcome(&self) -> CacheOutcome {
        match self {
            Self::Disabled => CacheOutcome::Disabled,
            Self::Hit(_) => CacheOutcome::Hit,
            Self::Miss(_) => CacheOutcome::Miss,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    key: String,
    /// Every workspace member of the base, with its manifest relative to the
    /// workspace root.
    members: Vec<(String, String)>,
    diagnostics: Vec<Diagnostic>,
    evidence: Vec<Option<String>>,
}

/// Everything the base side's findings depend on besides the base's tree.
pub(crate) struct KeyInputs<'a> {
    pub(crate) workspace_root: &'a Path,
    pub(crate) target_directory: &'a Path,
    pub(crate) comparison_base: &'a str,
    pub(crate) toolchain: &'a Toolchain,
    pub(crate) environment: &'a CommandEnvironment,
    pub(crate) lints: Option<&'a KnownLints>,
    pub(crate) plan: &'a PolicyPlan,
    pub(crate) settings: &'a StructureSettings,
    pub(crate) declared: &'a BTreeSet<String>,
    pub(crate) options: &'a RunOptions,
}

/// Looks the base side up, and says where to store it on a miss.
pub(crate) fn lookup(inputs: &KeyInputs<'_>, current: &Metadata) -> Lookup {
    if env::var_os("RUST_DOCTOR_NO_CACHE").is_some_and(|value| value == "1") {
        return Lookup::Disabled;
    }
    let Some(key) = key(inputs) else {
        return Lookup::Disabled;
    };
    let path = inputs
        .target_directory
        .join("rust-doctor/cache/base-sides.json");
    let members = members(current);
    let hit = read(&path).entries.into_iter().find(|entry| entry.key == key);
    match hit.filter(|entry| entry.members == members) {
        Some(entry) => match decode_evidence(&entry.evidence) {
            Some(evidence) if evidence.len() == entry.diagnostics.len() => {
                Lookup::Hit(CachedSide {
                    diagnostics: entry.diagnostics,
                    evidence,
                })
            }
            _ => Lookup::Miss(SideCache { path, key }),
        },
        None => Lookup::Miss(SideCache { path, key }),
    }
}

impl SideCache {
    /// Stores a complete base side, merged with what is on disk. A failure
    /// to write costs the next run a rescan and nothing else.
    pub(crate) fn store(
        self,
        members: Vec<(String, String)>,
        diagnostics: &[Diagnostic],
        evidence: &Evidence,
    ) {
        let mut file = read(&self.path);
        file.version = FILE_VERSION;
        file.entries.retain(|entry| entry.key != self.key);
        file.entries.push(Entry {
            key: self.key,
            members,
            diagnostics: diagnostics.to_vec(),
            evidence: evidence
                .iter()
                .map(|fingerprint| fingerprint.map(|bytes| blake3::Hash::from(bytes).to_hex().to_string()))
                .collect(),
        });
        let excess = file.entries.len().saturating_sub(ENTRY_LIMIT);
        file.entries.drain(..excess);
        let _ = write(&self.path, &file);
    }
}

fn read(path: &Path) -> CacheFile {
    let readable = fs::metadata(path).is_ok_and(|metadata| metadata.len() <= FILE_BYTES_LIMIT);
    readable
        .then(|| fs::read(path).ok())
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok())
        .filter(|file| file.version == FILE_VERSION)
        .unwrap_or_default()
}

fn write(path: &Path, file: &CacheFile) -> Option<()> {
    let directory = path.parent()?;
    fs::create_dir_all(directory).ok()?;
    let bytes = serde_json::to_vec(file).ok()?;
    let staging = directory.join(format!("base-sides.{}.partial", std::process::id()));
    fs::write(&staging, bytes).ok()?;
    fs::rename(&staging, path).ok()
}

fn decode_evidence(evidence: &[Option<String>]) -> Option<Evidence> {
    evidence
        .iter()
        .map(|fingerprint| match fingerprint {
            None => Some(None),
            Some(hex) => blake3::Hash::from_hex(hex).ok().map(|hash| Some(*hash.as_bytes())),
        })
        .collect()
}

/// Every workspace member, with its manifest relative to the workspace root:
/// what an entry must agree with the current metadata on.
pub(crate) fn members(metadata: &Metadata) -> Vec<(String, String)> {
    let root = metadata.workspace_root.as_std_path();
    let mut members: Vec<(String, String)> = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| {
            let manifest = package.manifest_path.as_std_path();
            let relative = manifest.strip_prefix(root).unwrap_or(manifest);
            (package.name.to_string(), relative.to_string_lossy().into_owned())
        })
        .collect();
    members.sort();
    members
}

/// The digest of every key ingredient, length-prefixed so no two lists of
/// fields hash alike. `None` when the base's tree cannot be named.
fn key(inputs: &KeyInputs<'_>) -> Option<String> {
    let tree = tree(inputs)?;
    let repository = inputs
        .workspace_root
        .ancestors()
        .find(|ancestor| ancestor.join(".git").symlink_metadata().is_ok())?;
    let relative = inputs.workspace_root.strip_prefix(repository).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut field = |value: &[u8]| {
        hasher.update(&(value.len() as u64).to_le_bytes());
        hasher.update(value);
    };
    field(env!("CARGO_PKG_VERSION").as_bytes());
    field(&[SCHEMA_VERSION]);
    field(tree.as_bytes());
    field(relative.as_os_str().as_encoded_bytes());
    field(inputs.toolchain.cargo.as_bytes());
    field(inputs.toolchain.rustc.as_bytes());
    field(inputs.toolchain.clippy.as_bytes());
    field(
        inputs
            .environment
            .rustup_toolchain
            .as_deref()
            .map_or(&[][..], |toolchain| toolchain.as_encoded_bytes()),
    );
    let lints = match inputs.lints {
        Some(Ok(names)) => names.iter().map(String::as_str).collect::<Vec<_>>().join("\n"),
        Some(Err(_)) => "unavailable".to_owned(),
        None => String::new(),
    };
    field(lints.as_bytes());
    field(format!("{:?}", inputs.plan).as_bytes());
    field(format!("{:?}", inputs.settings).as_bytes());
    field(format!("{:?}", inputs.options.packages).as_bytes());
    field(format!("{:?}", inputs.declared).as_bytes());
    field(advisory_identity(inputs.options).as_bytes());
    for (name, value) in environment() {
        field(name.as_encoded_bytes());
        field(value.as_encoded_bytes());
    }
    for config in outside_configs(repository) {
        field(config.as_os_str().as_encoded_bytes());
        field(&fs::read(&config).unwrap_or_default());
    }
    Some(hasher.finalize().to_hex().to_string())
}

/// `git rev-parse <base>^{tree}`: what the base side is a scan of.
fn tree(inputs: &KeyInputs<'_>) -> Option<String> {
    let failure = GitFailure::new("tree-unavailable", "The base tree could not be named.");
    let call = GitCall {
        arguments: git_arguments(
            inputs.workspace_root,
            [OsString::from("rev-parse"), OsString::from(format!("{}^{{tree}}", inputs.comparison_base))],
        ),
        stdout_limit: 4_096,
        stage: "baseline",
        failure,
        overflow: OUTPUT_TOO_LARGE,
    };
    let answer = run_git(
        Path::new("git"),
        inputs.workspace_root,
        &call,
        inputs.options.deadline,
    )
    .ok()?;
    let tree = String::from_utf8(answer).ok()?.trim().to_owned();
    (!tree.is_empty() && tree.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(tree)
}

/// The variables Cargo, rustc and the C toolchains a build script drives
/// read, sorted. rust-cache keys its own caches on the same prefixes. This
/// tool's own `RUST_DOCTOR_*` switches change no compiler input and stay out.
fn environment() -> Vec<(OsString, OsString)> {
    let mut variables: Vec<(OsString, OsString)> = env::vars_os()
        .filter(|(name, _)| {
            name.to_str().is_some_and(|name| {
                !name.starts_with("RUST_DOCTOR_")
                    && ENVIRONMENT_PREFIXES
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
            })
        })
        .collect();
    variables.sort();
    variables
}

/// The Cargo configuration files outside the repository, the only ones its
/// tree does not already pin: every ancestor's, then `$CARGO_HOME`'s.
fn outside_configs(repository: &Path) -> Vec<PathBuf> {
    let home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")));
    repository
        .ancestors()
        .skip(1)
        .map(|ancestor| ancestor.join(".cargo"))
        .chain(home)
        .flat_map(|directory| [directory.join("config"), directory.join("config.toml")])
        .filter(|path| path.is_file())
        .collect()
}

/// Which RustSec database the advisory rules read, and how recently it moved.
fn advisory_identity(options: &RunOptions) -> String {
    let Some(database) = &options.advisory_db else {
        return String::new();
    };
    let root = &database.path;
    let newest = [
        root.to_path_buf(),
        root.join(".git/HEAD"),
        root.join(".git/index"),
        root.join(".git/FETCH_HEAD"),
    ]
    .iter()
    .filter_map(|path| fs::metadata(path).and_then(|metadata| metadata.modified()).ok())
    .filter_map(|modified| modified.duration_since(UNIX_EPOCH).ok())
    .max()
    .map_or(0, |since| since.as_nanos());
    format!("{database:?}@{newest}")
}
