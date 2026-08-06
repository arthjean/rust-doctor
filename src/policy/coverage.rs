//! Coverage of the catalog against the lint universe of the normative
//! toolchain.
//!
//! `clippy-driver -W help` enumerates every lint the toolchain can emit, so the
//! upstream side of this catalog is finite and countable. Three sets partition
//! it: the rules the catalog admits, the lints `rejected.json` turns down with
//! a reason, and the rest, which is the candidate queue. Publishing the size of
//! that third set turns the recall this repository never claimed into a number
//! that only moves one way.
//!
//! The queue is ordered by what a user already sees. A lint the toolchain warns
//! about by default reaches the report whether or not the catalog knows it:
//! `report::diagnostics` only drops a diagnostic whose rule is catalogued and
//! inactive, so an uncatalogued warning arrives with no category, no tier and
//! no help, and costs the score its authoritative flag. Those come first.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde::Deserialize;

use super::{CATALOG, Producer};

/// Lowest number of universe lints this repository has decided, admitted or
/// rejected. Every triage batch raises it; nothing may lower it.
const DECIDED_FLOOR: usize = 52;

/// Level a lint carries in the toolchain, read from the `-W help` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolchainLevel {
    Allow,
    Warn,
    Deny,
}

impl ToolchainLevel {
    /// Closed read of the level column. Any other second field means the line
    /// is not a lint declaration.
    fn parse(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(Self::Allow),
            "warn" => Some(Self::Warn),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// Why a lint of the universe is not a candidate. Closed, so a rejection is
/// audited by its class instead of being read one reason at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RejectionClass {
    /// Denied by default upstream, so a scan cannot carry it at all.
    DenyByDefault,
    /// An admitted rule already reports the same defect.
    Covered,
    /// A matter of taste, not a defect the score should move for.
    StyleOnly,
    /// Outside what a workspace scan claims to inspect.
    OutOfScope,
    /// Measured noise leaves it unusable at any default level.
    Noisy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rejection {
    class: RejectionClass,
    id: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rejections {
    schema_version: u32,
    criterion: String,
    rejected: Vec<Rejection>,
}

/// The toolchain's lint table: every lint with its default level, and the
/// groups each lint belongs to.
struct Universe {
    levels: BTreeMap<String, ToolchainLevel>,
    groups: BTreeMap<String, BTreeSet<String>>,
}

/// Canonical form of a lint or group name: the table prints kebab-case, the
/// catalog stores snake_case, and group members carry a trailing comma.
fn normalize(name: &str) -> Option<String> {
    let suffix = name.trim().trim_end_matches(',').strip_prefix("clippy::")?;
    (!suffix.is_empty()).then(|| format!("clippy::{}", suffix.replace('-', "_")))
}

fn help_table() -> String {
    let help = Command::new("clippy-driver")
        .args(["-W", "help"])
        .output()
        .expect("clippy-driver should start");
    assert!(help.status.success(), "clippy-driver refused to list lints");
    String::from_utf8(help.stdout).expect("the lint table should be UTF-8")
}

/// Reads both tables in one pass. A lint line carries a level in its second
/// field, a group line carries its first member there, so the two never mix.
fn universe() -> Universe {
    let help = help_table();
    let mut levels = BTreeMap::new();
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for line in help.lines() {
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next().and_then(normalize) else {
            continue;
        };
        let Some(second) = fields.next() else {
            continue;
        };
        if let Some(level) = ToolchainLevel::parse(second) {
            levels.insert(name, level);
        } else if second.starts_with("clippy::") {
            for member in std::iter::once(second)
                .chain(fields)
                .filter_map(normalize)
            {
                groups.entry(member).or_default().insert(name.clone());
            }
        }
    }

    Universe { levels, groups }
}

fn rejections() -> Rejections {
    serde_json::from_str(include_str!("rejected.json")).expect("rejected.json should parse")
}

fn admitted() -> BTreeSet<&'static str> {
    CATALOG
        .iter()
        .filter(|definition| definition.producer == Producer::Clippy)
        .map(|definition| definition.id)
        .collect()
}

/// Lints the catalog has not admitted and `rejected.json` has not turned down,
/// minus the ones the toolchain denies by default, which no scan can carry.
/// Warned lints come first: they already reach the report uncatalogued.
fn queue(universe: &Universe, decided: &BTreeSet<String>) -> Vec<(String, ToolchainLevel)> {
    let mut queue: Vec<_> = universe
        .levels
        .iter()
        .filter(|(id, level)| **level != ToolchainLevel::Deny && !decided.contains(*id))
        .map(|(id, level)| (id.clone(), *level))
        .collect();
    queue.sort_by(|left, right| {
        (left.1 != ToolchainLevel::Warn, &left.0).cmp(&(right.1 != ToolchainLevel::Warn, &right.0))
    });
    queue
}

#[test]
fn the_toolchain_publishes_a_finite_lint_universe_with_its_groups() {
    let universe = universe();

    assert!(
        universe.levels.len() > 500,
        "{} lints listed",
        universe.levels.len()
    );
    // A format change upstream must fail loudly here rather than silently
    // empty the queue and report full coverage.
    for (lint, groups) in &universe.groups {
        assert!(
            universe.levels.contains_key(lint),
            "{lint} is grouped but absent from the lint table"
        );
        assert!(!groups.is_empty(), "{lint} carries an empty group set");
    }
    for group in ["clippy::all", "clippy::pedantic", "clippy::restriction"] {
        assert!(
            universe
                .groups
                .values()
                .any(|groups| groups.contains(group)),
            "{group} names no lint"
        );
    }
}

#[test]
fn every_rejection_names_a_lint_of_the_toolchain_and_gives_its_reason() {
    let rejections = rejections();
    let universe = universe();

    assert_eq!(rejections.schema_version, 1);
    assert!(rejections.criterion.len() > 80, "the criterion is a stub");

    let mut previous = "";
    for rejection in &rejections.rejected {
        assert!(
            universe.levels.contains_key(&rejection.id),
            "{} is not a lint of the normative toolchain",
            rejection.id
        );
        assert!(
            rejection.reason.len() > 40 && rejection.reason.ends_with('.'),
            "{} is turned down without a stated reason",
            rejection.id
        );
        assert!(
            previous < rejection.id.as_str(),
            "{} is out of order or listed twice",
            rejection.id
        );
        previous = rejection.id.as_str();
    }
}

#[test]
fn a_deny_by_default_rejection_stays_tied_to_what_the_toolchain_denies() {
    let universe = universe();

    for rejection in &rejections().rejected {
        let denied = universe.levels.get(&rejection.id) == Some(&ToolchainLevel::Deny);
        assert_eq!(
            rejection.class == RejectionClass::DenyByDefault,
            denied,
            "{} is classified against the level the toolchain gives it",
            rejection.id
        );
    }
}

#[test]
fn no_lint_is_both_admitted_and_rejected() {
    let admitted = admitted();

    for rejection in &rejections().rejected {
        assert!(
            !admitted.contains(rejection.id.as_str()),
            "{} is shipped and turned down at the same time",
            rejection.id
        );
    }
}

#[test]
fn the_candidate_queue_is_published_and_coverage_never_regresses() {
    let universe = universe();
    let admitted = admitted();
    let rejections = rejections();

    let decided: BTreeSet<String> = admitted
        .iter()
        .map(|id| (*id).to_owned())
        .chain(rejections.rejected.iter().map(|entry| entry.id.clone()))
        .collect();
    let queue = queue(&universe, &decided);
    let warned = queue
        .iter()
        .filter(|(_, level)| *level == ToolchainLevel::Warn)
        .count();

    println!(
        "universe {}, decided {}, queue {} ({warned} already reaching the report uncatalogued)",
        universe.levels.len(),
        decided.len(),
        queue.len()
    );
    for (id, level) in &queue {
        let groups: Vec<_> = universe
            .groups
            .get(id)
            .map(|groups| groups.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let level = match level {
            ToolchainLevel::Warn => "warn",
            ToolchainLevel::Allow => "allow",
            ToolchainLevel::Deny => "deny",
        };
        println!("{level}\t{id}\t{}", groups.join(","));
    }

    assert!(
        decided.len() >= DECIDED_FLOOR,
        "coverage fell from {DECIDED_FLOOR} to {} decided lints",
        decided.len()
    );
}
