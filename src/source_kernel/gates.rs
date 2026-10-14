//! Whether a gated rule's detectors read a unit: the version each gated crate
//! resolves to for the packages that reach it.
//!
//! A rule written against one crate's API is only as good as the versions it
//! was checked on. The version a unit sees is the lowest, over the packages
//! reaching it, of the `Cargo.lock` entry their direct dependency resolves to;
//! without a lockfile entry it is the lower bound of the manifest's
//! requirement, and unknown for `*`. An unknown version opens the gate: the
//! detectors still resolve every path through the manifest's own dependency
//! names, so a crate the unit cannot name is never matched.

use std::collections::BTreeMap;

use cargo_metadata::Metadata;
use cargo_metadata::semver::{Op, Version, VersionReq};

use crate::policy::{CrateGate, RuleDefinition};

/// Per package id, the version of each direct dependency, by crate name with
/// `-` read as `_`.
pub(super) type DependencyVersions = BTreeMap<String, BTreeMap<String, Option<Version>>>;

pub(super) fn dependency_versions(metadata: &Metadata) -> DependencyVersions {
    let locked = crate::cargo_health::locked_direct_versions(metadata.workspace_root.as_std_path());
    metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| {
            let resolved = locked.get(&(package.name.to_string(), package.version.to_string()));
            let versions = package
                .dependencies
                .iter()
                .map(|dependency| {
                    let version = resolved
                        .and_then(|resolved| resolved.get(dependency.name.as_str()))
                        .and_then(|version| Version::parse(version).ok())
                        .or_else(|| lower_bound(&dependency.req));
                    (crate_name(&dependency.name), version)
                })
                .collect();
            (package.id.repr.clone(), versions)
        })
        .collect()
}

/// The smallest version a requirement admits, read from its first
/// comparator; `None` for `*`.
fn lower_bound(requirement: &VersionReq) -> Option<Version> {
    let comparator = requirement.comparators.first()?;
    matches!(
        comparator.op,
        Op::Exact | Op::Greater | Op::GreaterEq | Op::Tilde | Op::Caret | Op::Wildcard
    )
    .then(|| {
        Version::new(
            comparator.major,
            comparator.minor.unwrap_or(0),
            comparator.patch.unwrap_or(0),
        )
    })
}

fn crate_name(name: &str) -> String {
    name.replace('-', "_")
}

/// What a unit's gate answered for one rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Gate {
    Open,
    /// No gated crate reaches the unit.
    Absent,
    /// A gated crate reaches it at a version outside the rule's range.
    Unsupported,
}

/// One rule's gates, their ranges parsed once per scan.
pub(super) struct ParsedGates {
    pub(super) rule: &'static str,
    gates: Vec<(String, VersionReq)>,
}

impl ParsedGates {
    /// `None` for an ungated rule. A range that does not parse is left out,
    /// which `validate_catalog` makes unreachable for a shipped rule.
    pub(super) fn of(definition: &'static RuleDefinition) -> Option<Self> {
        (!definition.requires.is_empty()).then(|| Self {
            rule: definition.id,
            gates: definition
                .requires
                .iter()
                .filter_map(|CrateGate { krate, versions }| {
                    Some((crate_name(krate), VersionReq::parse(versions).ok()?))
                })
                .collect(),
        })
    }

    /// The gate of a unit the packages `reaching` reach.
    pub(super) fn evaluate(&self, reaching: &[Option<&BTreeMap<String, Option<Version>>>]) -> Gate {
        let mut present = false;
        for (krate, range) in &self.gates {
            let versions = reaching
                .iter()
                .flatten()
                .filter_map(|dependencies| dependencies.get(krate));
            let mut seen = false;
            let mut lowest: Option<&Version> = None;
            for version in versions {
                seen = true;
                if let Some(version) = version {
                    lowest = Some(lowest.map_or(version, |lowest| lowest.min(version)));
                }
            }
            present |= seen;
            if lowest.is_some_and(|version| !range.matches(version)) {
                return Gate::Unsupported;
            }
        }
        if present { Gate::Open } else { Gate::Absent }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<Version>> {
        pairs
            .iter()
            .map(|(name, version)| ((*name).to_owned(), version.map(|version| Version::parse(version).unwrap())))
            .collect()
    }

    fn gates(pairs: &[(&str, &str)]) -> ParsedGates {
        ParsedGates {
            rule: "rust_doctor::source::probe",
            gates: pairs
                .iter()
                .map(|(krate, range)| ((*krate).to_owned(), VersionReq::parse(range).unwrap()))
                .collect(),
        }
    }

    #[test]
    fn the_lowest_version_reaching_a_unit_decides_and_an_absent_crate_closes() {
        let tokio = gates(&[("tokio", ">=1, <2")]);
        let one = versions(&[("tokio", Some("1.38.0"))]);
        let zero = versions(&[("tokio", Some("0.2.25"))]);
        let none = versions(&[("serde", Some("1.0.0"))]);
        let unknown = versions(&[("tokio", None)]);
        assert_eq!(tokio.evaluate(&[Some(&one)]), Gate::Open);
        assert_eq!(tokio.evaluate(&[Some(&one), Some(&zero)]), Gate::Unsupported);
        assert_eq!(tokio.evaluate(&[Some(&none)]), Gate::Absent);
        assert_eq!(tokio.evaluate(&[None]), Gate::Absent);
        assert_eq!(tokio.evaluate(&[Some(&unknown)]), Gate::Open);
    }

    #[test]
    fn any_gated_crate_opens_and_every_present_one_must_be_in_range() {
        let cors = gates(&[("tower_http", ">=0.5, <0.7"), ("actix_cors", ">=0.6, <0.8")]);
        let tower = versions(&[("tower_http", Some("0.6.2"))]);
        let old_actix = versions(&[("actix_cors", Some("0.5.0"))]);
        assert_eq!(cors.evaluate(&[Some(&tower)]), Gate::Open);
        assert_eq!(cors.evaluate(&[Some(&old_actix)]), Gate::Unsupported);
    }

    #[test]
    fn a_requirement_lower_bound_stands_in_for_a_missing_lock_entry() {
        let bound = |text: &str| lower_bound(&VersionReq::parse(text).unwrap());
        assert_eq!(bound("1.2"), Some(Version::new(1, 2, 0)));
        assert_eq!(bound("^0.12.4"), Some(Version::new(0, 12, 4)));
        assert_eq!(bound("=9"), Some(Version::new(9, 0, 0)));
        assert_eq!(bound("*"), None);
        assert_eq!(bound("<2"), None);
    }
}
