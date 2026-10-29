//! The resolved graph, read from `Cargo.lock` rather than from `cargo
//! metadata`.
//!
//! `cargo metadata` runs with `--no-deps`, so `metadata.resolve` is always
//! absent: the only graph readable without a registry index and without the
//! network is the lockfile. It sits in a file of its own so that every file of
//! the pack stays under the thousand lines `oversized_unit` reports at, and
//! because the two rules here are the only ones that read anything Clippy would
//! rewrite: they run before it for that reason.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;

use cargo_metadata::Metadata;
use serde::Deserialize;

use super::advisories::{self, AdvisoryDatabase};
use super::{
    ADVISORY_RULES, CARGO_DUPLICATE_MAJOR_VERSIONS, CARGO_MISSING_LOCKFILE, Candidate,
    CargoHealthError, CargoHealthScan, produces_a_binary, workspace_packages,
};
use crate::policy::ActiveRules;

#[cfg(test)]
mod tests;

/// Name of the offline resolved graph. `cargo metadata` runs with `--no-deps`,
/// so `metadata.resolve` is always absent: the only graph readable without a
/// registry index and without the network is the lockfile.
const LOCKFILE: &str = "Cargo.lock";

/// Bounds the pack's work on a hostile or gigantic lockfile. Beyond it, the
/// pack abstains instead of loading the file.
const MAX_LOCKFILE_BYTES: u64 = 4 * 1024 * 1024;

/// Minimal shape of the lockfile: the resolved list and the edges between its
/// entries, and all the rest of the document is ignored without making it
/// invalid.
#[derive(Debug, Deserialize)]
struct LockDocument {
    package: Option<Vec<LockPackage>>,
}

#[derive(Debug, Deserialize)]
struct LockPackage {
    name: Option<String>,
    version: Option<String>,
    source: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
}

/// One resolved package and the packages it depends on, as the lockfile
/// spells them: `name`, `name version` or `name version (source)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LockedPackage {
    pub(super) name: String,
    pub(super) version: String,
    /// Where the package comes from, absent for a path or workspace member.
    pub(super) source: Option<String>,
    pub(super) dependencies: Vec<String>,
}

/// State of the resolved graph as the pack can observe it offline.
#[derive(Debug, PartialEq, Eq)]
enum Resolution {
    /// No lockfile: this is an observed fact, not an error.
    Absent,
    /// The file exists but its resolution section is unusable.
    Unusable(CargoHealthError),
    /// The resolved graph, in file order, and the text it was read from.
    Packages(Vec<LockedPackage>, String),
}

/// The four rules that read the resolved graph rather than the manifests.
pub(super) fn inspect_resolution(
    metadata: &Metadata,
    active: &ActiveRules,
    database: Option<&AdvisoryDatabase>,
    scan: &mut CargoHealthScan,
) {
    let missing_lockfile = active.on(&CARGO_MISSING_LOCKFILE);
    let duplicate_majors = active.on(&CARGO_DUPLICATE_MAJOR_VERSIONS);
    let advisories = active.any_of(&ADVISORY_RULES);
    if !missing_lockfile && !duplicate_majors && !advisories {
        return;
    }
    let workspace_root = metadata.workspace_root.as_std_path();

    let resolution = read_resolution(workspace_root);
    if advisories {
        let (packages, text) = match &resolution {
            Resolution::Packages(packages, text) => (packages.as_slice(), text.as_str()),
            Resolution::Absent | Resolution::Unusable(_) => (&[][..], ""),
        };
        advisories::inspect(database, packages, text, resolution_owner(metadata).as_deref(), active, scan);
    }
    match resolution {
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
                    package: Some(package.name.to_string()),
                    manifest_path: package
                        .manifest_path
                        .strip_prefix(&metadata.workspace_root)
                        .ok()
                        .map(|path| path.as_str().to_owned()),
                    span: None,
                    key: None,
                    informational: false,
                });
            }
        }
        Resolution::Absent => {}
        Resolution::Unusable(error) => scan.errors.push(error),
        Resolution::Packages(packages, _) => {
            scan.counters.resolved_packages = packages.len();
            if duplicate_majors {
                let owner = resolution_owner(metadata);
                let members: BTreeSet<String> = workspace_packages(metadata)
                    .map(|package| package.name.to_string())
                    .collect();
                for duplicate in duplicate_major_versions(&packages, &members) {
                    scan.candidates.push(Candidate {
                        definition: &CARGO_DUPLICATE_MAJOR_VERSIONS,
                        message: duplicate.message(),
                        package: owner.clone(),
                        manifest_path: Some(LOCKFILE.to_owned()),
                        span: None,
                        key: None,
                        informational: duplicate.is_transitive_only(),
                    });
                }
            }
        }
    }
}

/// The resolved graph, the root manifest and the workspace's Cargo
/// configuration belong to the workspace, not to a member. A finding about
/// them is attached to the one package of a single-package workspace, which is
/// that workspace, and to no member otherwise, so it weighs on the workspace
/// score alone and whole. It used to go to the root package or the first member
/// by name: paneflow's `paneflow-agent-config` carried thirteen duplicate
/// majors arriving through `cocoa`, `gpui` and `clap`, and scored 77 on
/// dependencies it never declared.
pub(super) fn resolution_owner(metadata: &Metadata) -> Option<String> {
    let mut members = workspace_packages(metadata);
    let only = members.next()?;
    members.next().is_none().then(|| only.name.to_string())
}

/// The version each workspace member's direct dependencies resolve to in
/// `Cargo.lock`, keyed by member name and version, then by dependency name.
/// Empty without a usable lockfile: the version gates then read the
/// manifests' requirements instead.
pub(crate) fn locked_direct_versions(
    workspace_root: &Path,
) -> BTreeMap<(String, String), BTreeMap<String, String>> {
    let Resolution::Packages(packages, _) = read_resolution(workspace_root) else {
        return BTreeMap::new();
    };
    let graph = Graph::of(&packages);
    packages
        .iter()
        .filter(|package| package.source.is_none())
        .map(|package| {
            let dependencies = package
                .dependencies
                .iter()
                .filter_map(|dependency| graph.resolve(dependency))
                .map(|(name, version)| (name.to_owned(), version.to_owned()))
                .collect();
            ((package.name.clone(), package.version.clone()), dependencies)
        })
        .collect()
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
        .filter_map(|entry| {
            Some(LockedPackage {
                name: entry.name.clone()?,
                version: entry.version.clone()?,
                source: entry.source.clone(),
                dependencies: entry.dependencies.clone(),
            })
        })
        .collect();
    if packages.len() != entries.len() {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-resolution-absent",
            message: "the lockfile carries a resolved package without a name or a version",
        });
    }
    Resolution::Packages(packages, contents)
}

/// How one version of a duplicated crate reaches the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Requirement {
    /// A workspace member lists it in its own manifest: the requirement the
    /// workspace can move.
    Direct(String),
    /// It arrives under a dependency a member declares, named so the reader
    /// knows which crate to bump or drop.
    Through(String),
    /// No member reaches it, which a lockfile that carries no edges answers.
    Unreached,
}

/// One crate resolved at two incompatible versions, each with the way
/// it reaches the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DuplicateMajor {
    pub(super) name: String,
    pub(super) versions: Vec<(String, Requirement)>,
}

impl DuplicateMajor {
    /// The finding, versions and provenance included.
    ///
    /// Under the old message a reader was told two majors were resolved and
    /// nothing about who asked for either, which on this crate's own scan
    /// named `winnow` twice through two crates no manifest here mentions. The
    /// provenance says which requirement to align or which dependency to bump.
    pub(super) fn message(&self) -> String {
        let versions: Vec<String> = self
            .versions
            .iter()
            .map(|(version, requirement)| match requirement {
                Requirement::Direct(member) => format!("{version} (required by {member})"),
                Requirement::Through(dependency) => format!("{version} (through {dependency})"),
                Requirement::Unreached => version.clone(),
            })
            .collect();
        format!(
            "Crate \"{}\" is resolved with incompatible versions {}.",
            self.name,
            versions.join(", ")
        )
    }

    /// Whether every version arrives through a dependency, so no manifest of
    /// the workspace holds a requirement it could align.
    ///
    /// Such a duplicate is published at `Info` and weighs nothing. Charged
    /// alike under core-v5, `winnow` and `rustc-hash`, which reach this crate
    /// only through `ra_ap_syntax` and `toml`, held its dependencies at 64 and
    /// topped the advice to repair. A version a member requires directly, or
    /// one no member reaches, keeps the warning: the first is a requirement
    /// the workspace moves, and the second is a lockfile the scan cannot
    /// attribute.
    pub(super) fn is_transitive_only(&self) -> bool {
        self.versions
            .iter()
            .all(|(_, requirement)| matches!(requirement, Requirement::Through(_)))
    }
}

/// Two versions of the same crate in different Cargo compatibility classes
/// are not interchangeable: their types do not unify and the binary embeds
/// both copies. Pre-releases and build metadata are ignored. Each version is
/// attributed to the member requiring it or to the member's dependency it
/// arrives under.
pub(super) fn duplicate_major_versions(
    packages: &[LockedPackage],
    members: &BTreeSet<String>,
) -> Vec<DuplicateMajor> {
    let graph = Graph::of(packages);
    let mut majors = BTreeMap::<&str, Vec<&str>>::new();
    for package in packages {
        let entry = majors.entry(package.name.as_str()).or_default();
        if !entry.contains(&package.version.as_str()) {
            entry.push(package.version.as_str());
        }
    }

    majors
        .into_iter()
        .filter_map(|(name, mut versions)| {
            versions.sort_unstable();
            let mut unique: Vec<_> = versions
                .iter()
                .filter_map(|version| compatibility_class(version))
                .collect();
            unique.sort_unstable();
            unique.dedup();
            (unique.len() > 1).then(|| DuplicateMajor {
                name: name.to_owned(),
                versions: versions
                    .into_iter()
                    .map(|version| {
                        (
                            version.to_owned(),
                            graph.requirement(members, name, version),
                        )
                    })
                    .collect(),
            })
        })
        .collect()
}

/// The lockfile's edges, keyed by `name version`.
struct Graph<'a> {
    edges: BTreeMap<(&'a str, &'a str), Vec<(&'a str, &'a str)>>,
    by_name: BTreeMap<&'a str, Vec<&'a str>>,
}

impl<'a> Graph<'a> {
    fn of(packages: &'a [LockedPackage]) -> Self {
        let mut by_name = BTreeMap::<&str, Vec<&str>>::new();
        for package in packages {
            by_name
                .entry(package.name.as_str())
                .or_default()
                .push(package.version.as_str());
        }
        for versions in by_name.values_mut() {
            versions.sort_unstable();
        }
        let mut graph = Self {
            edges: BTreeMap::new(),
            by_name,
        };
        for package in packages {
            let targets: Vec<_> = package
                .dependencies
                .iter()
                .filter_map(|dependency| graph.resolve(dependency))
                .collect();
            graph
                .edges
                .insert((package.name.as_str(), package.version.as_str()), targets);
        }
        graph
    }

    /// One dependency line of the lockfile, resolved to the entry it names.
    /// A bare name is unambiguous by Cargo's own rule, so the single version
    /// it resolves to is the one the graph holds.
    fn resolve(&self, dependency: &'a str) -> Option<(&'a str, &'a str)> {
        let mut words = dependency.split_whitespace();
        let name = words.next()?;
        let versions = self.by_name.get(name)?;
        match words.next() {
            Some(version) if version.starts_with(|byte: char| byte.is_ascii_digit()) => versions
                .iter()
                .find(|candidate| **candidate == version)
                .map(|version| (name, *version)),
            _ => versions.first().map(|version| (name, *version)),
        }
    }

    /// Who asks for this version: a member directly, or the member's
    /// dependency it arrives under, the first by name.
    fn requirement(&self, members: &BTreeSet<String>, name: &str, version: &str) -> Requirement {
        let target = (name, version);
        let mut through: Option<&str> = None;
        for member in members {
            let Some(direct) = self.edges.get(&(member.as_str(), self.first_version(member))) else {
                continue;
            };
            if direct.contains(&target) {
                return Requirement::Direct(member.clone());
            }
            for dependency in direct {
                if through.is_some_and(|known| known <= dependency.0) {
                    continue;
                }
                if self.reaches(*dependency, target) {
                    through = Some(dependency.0);
                }
            }
        }
        through.map_or(Requirement::Unreached, |dependency| {
            Requirement::Through(dependency.to_owned())
        })
    }

    fn first_version(&self, name: &str) -> &'a str {
        self.by_name
            .get(name)
            .and_then(|versions| versions.first().copied())
            .unwrap_or("")
    }

    /// Whether `target` sits under `from`, `from` included. Every node is
    /// visited at most once, so the walk is bounded by the lockfile.
    fn reaches(&self, from: (&'a str, &'a str), target: (&str, &str)) -> bool {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([from]);
        while let Some(node) = queue.pop_front() {
            if node == target {
                return true;
            }
            if !seen.insert(node) {
                continue;
            }
            if let Some(next) = self.edges.get(&node) {
                queue.extend(next.iter().copied());
            }
        }
        false
    }
}

/// The Cargo compatibility class of a version: its left-most non-zero
/// component and every zero before it, `M` for `M.x.y`, `0.m` for `0.m.y` and
/// `0.0.p` for `0.0.p`. Cargo considers only a change there incompatible, so
/// `0.59` and `0.61` are two classes, which reading the first number alone
/// called one. Pre-release and build metadata are ignored, and a version that
/// is not three numbers has no class.
pub(super) fn compatibility_class(version: &str) -> Option<&str> {
    let core = version.split(['+', '-']).next()?;
    let mut end = 0;
    let mut settled = false;
    let mut components = 0;
    for component in core.split('.') {
        if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        if !settled {
            end += component.len() + usize::from(components > 0);
            settled = component.bytes().any(|byte| byte != b'0');
        }
        components += 1;
    }
    if components == 3 { core.get(..end) } else { None }
}
