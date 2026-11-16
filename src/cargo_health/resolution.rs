use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use cargo_metadata::Metadata;
use serde::Deserialize;

use super::advisories::{self, AdvisoryDatabase};
use super::{
    ADVISORY_RULES, CARGO_DUPLICATE_MAJOR_VERSIONS, CARGO_MISSING_LOCKFILE, Candidate,
    CargoHealthError, CargoHealthScan, produces_a_binary, workspace_manifest, workspace_packages,
};
use crate::policy::ActiveRules;

#[cfg(test)]
mod tests;

const LOCKFILE: &str = "Cargo.lock";

const MAX_LOCKFILE_BYTES: u64 = 4 * 1024 * 1024;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LockedPackage {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) source: Option<String>,
    pub(super) dependencies: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Resolution {
    Absent,
    Unusable(CargoHealthError),
    Packages(Vec<LockedPackage>, String),
}

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
                    message: format!(
                        "Package \"{}\" produces a binary but no {LOCKFILE} sits next to its workspace manifest.",
                        package.name
                    ),
                    package: Some(package.name.to_string()),
                    manifest_path: workspace_manifest(package, metadata),
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
                    let manifest = duplicate
                        .removable_by()
                        .and_then(|member| {
                            workspace_packages(metadata).find(|package| package.name.as_str() == member)
                        })
                        .and_then(|package| workspace_manifest(package, metadata));
                    scan.candidates.push(Candidate {
                        definition: &CARGO_DUPLICATE_MAJOR_VERSIONS,
                        message: duplicate.message(),
                        package: owner.clone(),
                        manifest_path: Some(manifest.unwrap_or_else(|| LOCKFILE.to_owned())),
                        span: None,
                        key: None,
                        informational: duplicate.removable_by().is_none(),
                    });
                }
            }
        }
    }
}

pub(super) fn resolution_owner(metadata: &Metadata) -> Option<String> {
    let mut members = workspace_packages(metadata);
    let only = members.next()?;
    members.next().is_none().then(|| only.name.to_string())
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Requirement {
    Direct(Vec<String>),
    Through(String),
    Unreached,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedVersion {
    pub(super) version: String,
    pub(super) requirement: Requirement,
    pub(super) members_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DuplicateMajor {
    pub(super) name: String,
    pub(super) versions: Vec<ResolvedVersion>,
}

const NAMED_MEMBERS: usize = 3;

impl DuplicateMajor {
    pub(super) fn message(&self) -> String {
        let versions: Vec<String> = self
            .versions
            .iter()
            .map(|resolved| {
                let version = &resolved.version;
                match &resolved.requirement {
                    Requirement::Direct(members) => {
                        format!("{version} (required by {})", named_members(members))
                    }
                    Requirement::Through(dependency) => format!("{version} (through {dependency})"),
                    Requirement::Unreached => version.clone(),
                }
            })
            .collect();
        format!(
            "Crate \"{}\" is resolved with incompatible versions {}.",
            self.name,
            versions.join(", ")
        )
    }

    pub(super) fn removable_by(&self) -> Option<&str> {
        self.versions
            .iter()
            .filter(|resolved| resolved.members_only)
            .find_map(|resolved| match &resolved.requirement {
                Requirement::Direct(members) => members.first().map(String::as_str),
                Requirement::Through(_) | Requirement::Unreached => None,
            })
    }
}

fn named_members(members: &[String]) -> String {
    let named = members
        .iter()
        .take(NAMED_MEMBERS)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    match members.len().saturating_sub(NAMED_MEMBERS) {
        0 => named,
        more => format!("{named} +{more} more"),
    }
}

fn semver_key(version: &str) -> (Vec<u64>, bool, &str) {
    let release = version.split('+').next().unwrap_or(version);
    let (core, pre) = release.split_once('-').unwrap_or((release, ""));
    let numbers = core
        .split('.')
        .map(|component| component.parse().unwrap_or(u64::MAX))
        .collect();
    (numbers, pre.is_empty(), pre)
}

fn semver_order(left: &str, right: &str) -> std::cmp::Ordering {
    semver_key(left).cmp(&semver_key(right)).then_with(|| left.cmp(right))
}

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
            versions.sort_unstable_by(|left, right| semver_order(left, right));
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
                    .map(|version| ResolvedVersion {
                        version: version.to_owned(),
                        requirement: graph.requirement(members, name, version),
                        members_only: graph.required_only_by(members, (name, version)),
                    })
                    .collect(),
            })
        })
        .collect()
}

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

    fn member_nodes(&self, members: &BTreeSet<String>) -> Vec<(&'a str, &'a str)> {
        self.edges
            .keys()
            .filter(|(name, version)| members.contains(*name) && *version == self.first_version(name))
            .copied()
            .collect()
    }

    fn requirement(&self, members: &BTreeSet<String>, name: &str, version: &str) -> Requirement {
        let target = (name, version);
        let direct: Vec<String> = self
            .member_nodes(members)
            .into_iter()
            .filter(|node| self.edges.get(node).is_some_and(|next| next.contains(&target)))
            .map(|(member, _)| member.to_owned())
            .collect();
        if !direct.is_empty() {
            return Requirement::Direct(direct);
        }
        self.carrier(members, target)
            .map_or(Requirement::Unreached, |carrier| Requirement::Through(carrier.to_owned()))
    }

    fn required_only_by(&self, members: &BTreeSet<String>, target: (&str, &str)) -> bool {
        let mut requirers = self
            .edges
            .iter()
            .filter(|(_, next)| next.contains(&target))
            .map(|((name, _), _)| *name)
            .peekable();
        requirers.peek().is_some() && requirers.all(|name| members.contains(name))
    }

    fn carrier(&self, members: &BTreeSet<String>, target: (&str, &str)) -> Option<&'a str> {
        let mut carriers: BTreeMap<(&'a str, &'a str), Option<&'a str>> =
            self.member_nodes(members).into_iter().map(|node| (node, None)).collect();
        let mut layer: Vec<(&'a str, &'a str)> = carriers.keys().copied().collect();
        while !layer.is_empty() {
            let mut next = BTreeMap::<(&'a str, &'a str), Option<&'a str>>::new();
            for node in &layer {
                let carried = carriers.get(node).copied().flatten();
                for successor in self.edges.get(node).into_iter().flatten() {
                    if carriers.contains_key(successor) {
                        continue;
                    }
                    let candidate = carried.or_else(|| {
                        (!members.contains(successor.0)).then_some(successor.0)
                    });
                    let entry = next.entry(*successor).or_insert(candidate);
                    if let (Some(current), Some(offered)) = (*entry, candidate)
                        && offered < current
                    {
                        *entry = Some(offered);
                    }
                }
            }
            if let Some(found) = next.get(&target) {
                return *found;
            }
            layer = next.keys().copied().collect();
            carriers.extend(next);
        }
        None
    }

    fn first_version(&self, name: &str) -> &'a str {
        self.by_name
            .get(name)
            .and_then(|versions| versions.first().copied())
            .unwrap_or("")
    }

}

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
