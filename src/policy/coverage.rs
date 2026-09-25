//! Coverage of the catalog against the lint universe of the normative
//! toolchain.
//!
//! `clippy-driver -W help` enumerates every lint the toolchain can emit, so the
//! upstream side of this catalog is finite and countable. Three sets cover it:
//! the rules the catalog admits, the lints `rejected.json` turns down with a
//! reason, and the rest, which is the candidate queue. Publishing the size of
//! that third set turns the recall this repository never claimed into a number
//! that only moves one way.
//!
//! A lint the toolchain denies by default is a candidate like any other. The
//! scan passes `-A clippy::all` before its `-W` flags, so a catalogued
//! deny-by-default lint is carried as a warning, and one switched off is
//! allowed rather than denied: `docs/correctness-group-2026-09.md` measured it
//! on the pinned and minimum toolchains.
//!
//! The queue is ordered by what plain `cargo clippy` shows a user and the scan
//! silences: `-A clippy::all` allows every uncatalogued Clippy lint, so the
//! lints the toolchain denies by default come first, then the ones it warns
//! about, then the ones it only allows.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::sync::OnceLock;

use serde::Deserialize;

use super::{CATALOG, Producer};

/// Lowest number of universe lints this repository has decided, admitted or
/// rejected. Every triage batch raises it; nothing may lower it.
const DECIDED_FLOOR: usize = 118;

/// Level a lint carries in the toolchain, read from the `-W help` table.
///
/// The declared order runs from what the toolchain enforces hardest to what it
/// leaves off, which is the order of the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ToolchainLevel {
    Deny,
    Warn,
    Allow,
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

    /// The column this level was read from, written back as it was read.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Warn => "warn",
            Self::Deny => "deny",
        }
    }
}

/// Why a lint of the universe is not a candidate. Closed, so a rejection is
/// audited by its class instead of being read one reason at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RejectionClass {
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
    #[allow(
        dead_code,
        reason = "parsing it is the check: a class outside the closed vocabulary fails to load"
    )]
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

/// The lint table, read once. Every test below asks the same question of the
/// same toolchain, and each answer costs a `clippy-driver` run.
fn universe() -> &'static Universe {
    static UNIVERSE: OnceLock<Universe> = OnceLock::new();
    UNIVERSE.get_or_init(read_universe)
}

/// Reads both tables in one pass. A lint line carries a level in its second
/// field, a group line carries its first member there, so the two never mix.
fn read_universe() -> Universe {
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

fn rejections() -> &'static Rejections {
    static REJECTIONS: OnceLock<Rejections> = OnceLock::new();
    REJECTIONS.get_or_init(|| {
        serde_json::from_str(include_str!("rejected.json")).expect("rejected.json should parse")
    })
}

fn admitted() -> BTreeSet<&'static str> {
    CATALOG
        .iter()
        .filter(|definition| definition.producer == Producer::Clippy)
        .map(|definition| definition.id)
        .collect()
}

/// Lints the catalog has not admitted and `rejected.json` has not turned down,
/// denied ones first, then warned, then allowed.
fn queue(universe: &Universe, decided: &BTreeSet<String>) -> Vec<(String, ToolchainLevel)> {
    let mut queue: Vec<_> = universe
        .levels
        .iter()
        .filter(|(id, _)| !decided.contains(*id))
        .map(|(id, level)| (id.clone(), *level))
        .collect();
    // The table is read from a `BTreeMap`, so the identifiers already come
    // sorted and a stable sort on the level alone is the whole ordering.
    queue.sort_by_key(|(_, level)| *level);
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
    let queue = queue(universe, &decided);
    let denied: Vec<&str> = queue
        .iter()
        .filter(|(_, level)| *level == ToolchainLevel::Deny)
        .map(|(id, _)| id.as_str())
        .collect();

    println!(
        "universe {}, decided {}, queue {} ({} denied by default)",
        universe.levels.len(),
        decided.len(),
        queue.len(),
        denied.len()
    );
    // The members of `clippy::correctness` a fixture cannot trigger without an
    // external crate stay undecided, and a denied lint is a candidate like any
    // other: `docs/correctness-group-2026-09.md` holds their classification.
    for waiting in [
        "clippy::invalid_regex",
        "clippy::let_underscore_lock",
        "clippy::serde_api_misuse",
    ] {
        assert!(denied.contains(&waiting), "{waiting} left the queue");
    }
    for (id, level) in &queue {
        let groups: Vec<_> = universe
            .groups
            .get(id)
            .map(|groups| groups.iter().map(String::as_str).collect())
            .unwrap_or_default();
        println!("{}\t{id}\t{}", level.as_str(), groups.join(","));
    }

    assert!(
        decided.len() >= DECIDED_FLOOR,
        "coverage fell from {DECIDED_FLOOR} to {} decided lints",
        decided.len()
    );
}
