//! The resolved graph, read from `Cargo.lock` rather than from `cargo
//! metadata`.
//!
//! `cargo metadata` runs with `--no-deps`, so `metadata.resolve` is always
//! absent: the only graph readable without a registry index and without the
//! network is the lockfile. It sits in a file of its own so that every file of
//! the pack stays under the thousand lines `oversized_unit` reports at, and
//! because the two rules here are the only ones that read anything Clippy would
//! rewrite: they run before it for that reason.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use cargo_metadata::Metadata;
use serde::Deserialize;

use super::{
    CARGO_DUPLICATE_MAJOR_VERSIONS, CARGO_MISSING_LOCKFILE, Candidate, CargoHealthError,
    CargoHealthScan, produces_a_binary, workspace_packages,
};
use crate::policy::ActiveRules;

/// Name of the offline resolved graph. `cargo metadata` runs with `--no-deps`,
/// so `metadata.resolve` is always absent: the only graph readable without a
/// registry index and without the network is the lockfile.
const LOCKFILE: &str = "Cargo.lock";

/// Bounds the pack's work on a hostile or gigantic lockfile. Beyond it, the
/// pack abstains instead of loading the file.
const MAX_LOCKFILE_BYTES: u64 = 4 * 1024 * 1024;

/// Minimal shape of the lockfile: only the resolved list matters, and all the
/// rest of the document is ignored without making it invalid.
#[derive(Debug, Deserialize)]
struct LockDocument {
    package: Option<Vec<LockPackage>>,
}

#[derive(Debug, Deserialize)]
struct LockPackage {
    name: Option<String>,
    version: Option<String>,
}

/// State of the resolved graph as the pack can observe it offline.
#[derive(Debug, PartialEq, Eq)]
enum Resolution {
    /// No lockfile: this is an observed fact, not an error.
    Absent,
    /// The file exists but its resolution section is unusable.
    Unusable(CargoHealthError),
    /// (name, version) pairs of the resolved graph, in file order.
    Packages(Vec<(String, String)>),
}

/// The two rules that read the resolved graph rather than the manifests.
pub(super) fn inspect_resolution(metadata: &Metadata, active: &ActiveRules, scan: &mut CargoHealthScan) {
    let missing_lockfile = active.on(&CARGO_MISSING_LOCKFILE);
    let duplicate_majors = active.on(&CARGO_DUPLICATE_MAJOR_VERSIONS);
    if !missing_lockfile && !duplicate_majors {
        return;
    }
    let workspace_root = metadata.workspace_root.as_std_path();

    match read_resolution(workspace_root) {
        Resolution::Absent if missing_lockfile => {
            for package in workspace_packages(metadata).filter(|package| produces_a_binary(package))
            {
                scan.candidates.push(Candidate {
                    definition: &CARGO_MISSING_LOCKFILE,
                    // The pack observes the disk, not the version control
                    // index: the message therefore states what is measured,
                    // the missing file, and the rule help carries the fix.
                    message: format!(
                        "Package \"{}\" produces a binary but no {LOCKFILE} sits next to its workspace manifest.",
                        package.name
                    ),
                    package: package.name.to_string(),
                    manifest_path: package
                        .manifest_path
                        .strip_prefix(&metadata.workspace_root)
                        .ok()
                        .map(|path| path.as_str().to_owned()),
                    span: None,
                });
            }
        }
        Resolution::Absent => {}
        Resolution::Unusable(error) => scan.errors.push(error),
        Resolution::Packages(packages) => {
            scan.counters.resolved_packages = packages.len();
            if duplicate_majors {
                let owner = resolution_owner(metadata);
                for (name, versions) in duplicate_major_versions(&packages) {
                    scan.candidates.push(Candidate {
                        definition: &CARGO_DUPLICATE_MAJOR_VERSIONS,
                        message: format!(
                            "Crate \"{name}\" is resolved with incompatible major versions {versions}."
                        ),
                        package: owner.clone(),
                        manifest_path: Some(LOCKFILE.to_owned()),
                        span: None,
                    });
                }
            }
        }
    }
}

/// The resolved graph belongs to the workspace, not to a member. The diagnostic
/// is therefore attached to the root package when it exists, otherwise to the
/// first member by name order, which stays deterministic on a virtual
/// workspace.
pub(super) fn resolution_owner(metadata: &Metadata) -> String {
    let root_manifest = metadata.workspace_root.join("Cargo.toml");
    if let Some(root) = workspace_packages(metadata)
        .find(|package| package.manifest_path == root_manifest)
        .or_else(|| workspace_packages(metadata).min_by_key(|package| package.name.to_string()))
    {
        return root.name.to_string();
    }
    metadata
        .workspace_root
        .file_name()
        .unwrap_or_default()
        .to_owned()
}

fn read_resolution(workspace_root: &Path) -> Resolution {
    let path = workspace_root.join(LOCKFILE);
    let Ok(metadata) = fs::metadata(&path) else {
        return Resolution::Absent;
    };
    if !metadata.is_file() || metadata.len() > MAX_LOCKFILE_BYTES {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-unreadable",
            message: "the lockfile is not a readable regular file within the published size limit",
        });
    }
    let Ok(contents) = fs::read_to_string(&path) else {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-unreadable",
            message: "the lockfile could not be read as UTF-8 text",
        });
    };
    let Ok(document) = toml::from_str::<LockDocument>(&contents) else {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-invalid",
            message: "the lockfile is not valid TOML",
        });
    };
    let Some(entries) = document.package else {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-resolution-absent",
            message: "the lockfile carries no resolved package section",
        });
    };

    let packages: Vec<_> = entries
        .iter()
        .filter_map(|entry| Some((entry.name.clone()?, entry.version.clone()?)))
        .collect();
    if packages.len() != entries.len() {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-resolution-absent",
            message: "the lockfile carries a resolved package without a name or a version",
        });
    }
    Resolution::Packages(packages)
}

/// Two versions of the same crate whose major number differs are not
/// interchangeable: their types do not unify and the binary embeds both copies.
/// Pre-releases and build metadata are ignored, only the major part of the
/// triplet is compared.
pub(super) fn duplicate_major_versions(packages: &[(String, String)]) -> Vec<(String, String)> {
    let mut majors = BTreeMap::<&str, Vec<&str>>::new();
    for (name, version) in packages {
        let entry = majors.entry(name.as_str()).or_default();
        if !entry.contains(&version.as_str()) {
            entry.push(version.as_str());
        }
    }

    majors
        .into_iter()
        .filter_map(|(name, mut versions)| {
            versions.sort_unstable();
            let distinct: Vec<_> = versions
                .iter()
                .filter_map(|version| major_of(version))
                .collect();
            let mut unique = distinct.clone();
            unique.sort_unstable();
            unique.dedup();
            (unique.len() > 1).then(|| (name.to_owned(), versions.join(", ")))
        })
        .collect()
}

pub(super) fn major_of(version: &str) -> Option<&str> {
    let major = version
        .split(['+', '-'])
        .next()?
        .split('.')
        .next()
        .filter(|major| !major.is_empty() && major.bytes().all(|byte| byte.is_ascii_digit()))?;
    Some(major)
}
