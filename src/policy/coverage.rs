use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::sync::OnceLock;

use serde::Deserialize;

use super::lint_table::{self, TableLine};
use super::{CATALOG, Producer};

const DECIDED_FLOOR: usize = 122;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ToolchainLevel {
    Deny,
    Warn,
    Allow,
}

impl ToolchainLevel {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(Self::Allow),
            "warn" => Some(Self::Warn),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Warn => "warn",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RejectionClass {
    Covered,
    StyleOnly,
    OutOfScope,
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

struct Universe {
    levels: BTreeMap<String, ToolchainLevel>,
    groups: BTreeMap<String, BTreeSet<String>>,
}

fn help_table() -> String {
    let help = Command::new("clippy-driver")
        .args(["-W", "help"])
        .output()
        .expect("clippy-driver should start");
    assert!(help.status.success(), "clippy-driver refused to list lints");
    String::from_utf8(help.stdout).expect("the lint table should be UTF-8")
}

fn universe() -> &'static Universe {
    static UNIVERSE: OnceLock<Universe> = OnceLock::new();
    UNIVERSE.get_or_init(read_universe)
}

fn read_universe() -> Universe {
    let help = help_table();
    let mut levels = BTreeMap::new();
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for line in help.lines().filter_map(lint_table::parse_line) {
        match line {
            TableLine::Lint { name, level } => {
                if let Some(level) = ToolchainLevel::parse(level) {
                    levels.insert(name, level);
                }
            }
            TableLine::Group { name, members } => {
                for member in members {
                    groups.entry(member).or_default().insert(name.clone());
                }
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

fn queue(universe: &Universe, decided: &BTreeSet<String>) -> Vec<(String, ToolchainLevel)> {
    let mut queue: Vec<_> = universe
        .levels
        .iter()
        .filter(|(id, _)| !decided.contains(*id))
        .map(|(id, level)| (id.clone(), *level))
        .collect();
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
