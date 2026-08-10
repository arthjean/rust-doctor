#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Admission gate: no rule ships without a recorded trigger.
//!
//! `tests/corpus.json` measures how often a rule is wrong on healthy public
//! code, and its `gate.unproven` names the rules that corpus never triggered,
//! because a healthy repository does not commit the defect they aim at. That
//! silence proves nothing either way, so it cannot be what admits a rule.
//! `tests/rule_evidence.json` carries the other half of the admission: for
//! every catalogued rule, the place where a test has seen it fire.
//!
//! Only observed positions count. A rule named by a negative fixture is a rule
//! that must stay quiet there, which is why the arrays read here are the ones
//! listing what the oracle saw, never the ones listing what it refuses.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rust_doctor::{InspectRequest, Status, inspect};
use serde_json::Value;

/// Arrays of a frozen oracle that list observed diagnostics.
const OBSERVED_ARRAYS: [&str; 4] = ["positive", "rules", "historical_rules", "findings"];

/// Fields such an array uses to carry the rule identifier.
const IDENTIFIER_FIELDS: [&str; 2] = ["code", "id"];

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> Value {
    assert!(path.exists(), "{} is missing", path.display());
    let text = fs::read_to_string(path).expect("the file should be readable");
    serde_json::from_str(&text).expect("the file should be JSON")
}

fn catalogued() -> BTreeSet<String> {
    read(&repository().join("tests/corpus.json"))["catalog"]
        .as_array()
        .expect("the published catalog is an array")
        .iter()
        .filter_map(|rule| rule["id"].as_str().map(str::to_owned))
        .collect()
}

fn evidence() -> Value {
    read(&repository().join("tests/rule_evidence.json"))
}

fn entries(index: &Value) -> &Vec<Value> {
    index["evidence"]
        .as_array()
        .expect("the index publishes an array of entries")
}

/// Rules a frozen oracle records as observed, read only from the arrays that
/// list what it saw.
fn observed_in(oracle: &Value) -> BTreeSet<String> {
    let mut observed = BTreeSet::new();
    for array in OBSERVED_ARRAYS {
        let Some(items) = oracle.get(array).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            for field in IDENTIFIER_FIELDS {
                if let Some(rule) = item.get(field).and_then(Value::as_str) {
                    observed.insert(rule.to_owned());
                }
            }
        }
    }
    observed
}

#[test]
fn every_catalogued_rule_records_where_it_was_seen_firing() {
    let index = evidence();
    let entries = entries(&index);

    assert_eq!(index["schema_version"], 1);
    assert!(
        index["criterion"].as_str().is_some_and(|text| text.len() > 80),
        "the criterion is a stub"
    );

    let mut previous = String::new();
    let mut declared = BTreeSet::new();
    for entry in entries {
        let id = entry["id"]
            .as_str()
            .expect("an entry names its rule")
            .to_owned();
        assert!(previous < id, "{id} is out of order or listed twice");
        previous.clone_from(&id);
        assert_eq!(
            entry.get("oracle").is_some(),
            entry.get("test").is_none(),
            "{id} must carry exactly one pointer, an oracle or a test"
        );
        declared.insert(id);
    }

    // Both directions: a rule shipping without evidence is the failure this
    // gate exists for, and an entry outliving its rule rots in silence.
    let catalogued = catalogued();
    assert_eq!(
        catalogued.difference(&declared).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "these rules ship with no recorded trigger"
    );
    assert_eq!(
        declared.difference(&catalogued).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "these entries name a rule the catalog no longer ships"
    );
}

/// What each Clippy lint of the toolchain says it catches, read from the same
/// table the candidate queue reads.
fn published_meanings() -> BTreeMap<String, String> {
    let help = Command::new("clippy-driver")
        .args(["-W", "help"])
        .output()
        .expect("clippy-driver should start");
    assert!(help.status.success(), "clippy-driver refused to list lints");
    let help = String::from_utf8(help.stdout).expect("the lint table should be UTF-8");

    // The table is column-aligned, so each field is split off the front of the
    // line rather than counted from a fixed offset.
    help.lines()
        .filter_map(|line| {
            let (name, rest) = line.trim_start().split_once(char::is_whitespace)?;
            let name = name.strip_prefix("clippy::")?;
            let (level, meaning) = rest.trim_start().split_once(char::is_whitespace)?;
            matches!(level, "allow" | "warn" | "deny").then(|| {
                (
                    format!("clippy::{}", name.replace('-', "_")),
                    meaning.trim().to_owned(),
                )
            })
        })
        .collect()
}

#[test]
fn what_a_clippy_rule_catches_is_what_the_toolchain_says_it_catches() {
    let index = evidence();
    let meanings = published_meanings();
    assert!(meanings.len() > 500, "{} lints listed", meanings.len());

    for entry in entries(&index) {
        let id = entry["id"].as_str().expect("an entry names its rule");
        let catches = entry["catches"].as_str().unwrap_or_default().trim().to_owned();
        assert!(
            !catches.is_empty(),
            "{id} states nothing about what it catches"
        );

        // A native detector has no upstream description to agree with; a
        // Clippy lint does, and a meaning that shifts upstream is a contract
        // that no longer describes what ships.
        if let Some(published) = meanings.get(id) {
            assert_eq!(
                &catches, published,
                "{id} no longer matches the description the toolchain publishes"
            );
        } else {
            assert!(
                !id.starts_with("clippy::"),
                "{id} is not a lint of the normative toolchain"
            );
        }
    }
}

#[test]
fn an_oracle_reference_names_its_rule_in_an_observed_position() {
    let index = evidence();
    let mut oracles: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();

    for entry in entries(&index) {
        let Some(oracle) = entry["oracle"].as_str() else {
            continue;
        };
        let id = entry["id"].as_str().expect("an entry names its rule");
        let path = repository().join("tests/fixtures").join(oracle);
        let observed = oracles
            .entry(path.clone())
            .or_insert_with(|| observed_in(&read(&path)));
        assert!(
            observed.contains(id),
            "{oracle} records no observation of {id}"
        );
    }

    assert!(!oracles.is_empty(), "the index cites no oracle at all");
}

/// Trigger of every rule admitted outside a pack and outside a frozen oracle.
///
/// The packs cover the dimensions they were built for, and the EP-018 archive
/// is frozen on the five candidates it measured. A rule that belongs to
/// neither gets its own single-purpose crate under
/// `tests/fixtures/rule-admission/<kebab-name>/`, holding the form the lint
/// reports and the neighbouring form it must leave alone. This test is the
/// pointer those entries name, so the fixture is scanned through the same
/// `inspect` a user runs, and the published category, tier and help are the
/// catalog's own.
#[test]
fn every_rule_admitted_outside_a_pack_fires_on_its_own_fixture() {
    const POINTER: &str =
        "tests/rule_admission.rs::every_rule_admitted_outside_a_pack_fires_on_its_own_fixture";
    let index = evidence();
    let admitted: Vec<&str> = entries(&index)
        .iter()
        .filter(|entry| entry["test"].as_str() == Some(POINTER))
        .filter_map(|entry| entry["id"].as_str())
        .collect();
    assert!(!admitted.is_empty(), "no rule points at this fixture set");

    for id in admitted {
        let lint = id
            .strip_prefix("clippy::")
            .expect("this fixture set holds Clippy lints");
        let root = repository()
            .join("tests/fixtures/rule-admission")
            .join(lint.replace('_', "-"));
        assert!(root.is_dir(), "{id} names the missing {}", root.display());

        let report = inspect(InspectRequest::new(&root));
        assert_eq!(report.status, Status::Complete, "{id}: {:?}", report.errors);
        let published = serde_json::to_value(&report).expect("a valid report should serialize");
        let rule = published["policy"]["rules"]
            .as_array()
            .expect("a scan publishes its rules")
            .iter()
            .find(|rule| rule["id"] == id);
        assert!(rule.is_some(), "{id} is not in the shipped policy");
        let rule = rule.expect("the rule was just asserted");

        let observed: Vec<&Value> = published["diagnostics"]
            .as_array()
            .expect("diagnostics should be an array")
            .iter()
            .filter(|diagnostic| !diagnostic["category"].is_null())
            .collect();
        assert_eq!(
            observed
                .iter()
                .map(|diagnostic| diagnostic["code"].as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
            [id],
            "{id}: the fixture reports something else, or nothing"
        );

        // The positive is one site, and the neighbouring form next to it stays
        // quiet: a rule with no recorded silence is a rule nobody checked for
        // over-reach.
        let finding = observed[0];
        assert_eq!(finding["occurrences"], 1, "{id}");
        assert_eq!(finding["category"], rule["category"], "{id}");
        assert_eq!(finding["severity"], "warning", "{id}");
        assert!(
            finding["help"]
                .as_str()
                .is_some_and(|help| !help.is_empty()),
            "{id} publishes no help"
        );
        assert!(
            rule["tier"].as_str().is_some_and(|tier| !tier.is_empty()),
            "{id} publishes no tier"
        );
    }
}

#[test]
fn a_test_reference_names_a_function_that_still_exists() {
    let index = evidence();

    for entry in entries(&index) {
        let Some(reference) = entry["test"].as_str() else {
            continue;
        };
        let id = entry["id"].as_str().expect("an entry names its rule");
        let (file, function) = reference
            .rsplit_once("::")
            .expect("a test reference is <file>::<function>");
        let path = repository().join(file);
        assert!(path.exists(), "{id} points at the missing {file}");

        let source = fs::read_to_string(&path).expect("the test file should be readable");
        assert!(
            source.contains(&format!("fn {function}(")),
            "{file} no longer defines {function}, which {id} relies on"
        );
    }
}

/// The README is not a third source of truth: its rule count and breakdown
/// follow the published catalog, which the corpus test already proves equal to
/// the shipped policy. Editing the catalog without touching the README fails
/// here.
#[test]
fn the_readme_rule_count_matches_the_published_catalog() {
    let catalog = catalogued();
    let clippy = catalog.iter().filter(|id| id.starts_with("clippy::")).count();
    let structural = catalog
        .iter()
        .filter(|id| id.starts_with("rust_doctor::structure::"))
        .count();
    let native = catalog.len() - clippy - structural;

    let readme = fs::read_to_string(repository().join("README.md")).expect("the README should be readable");
    let expected = format!(
        "{} rules today: {clippy} selected Clippy lints, {native} native detectors and {structural} structural rules.",
        catalog.len()
    );
    assert!(
        readme.contains(&expected),
        "README no longer states \"{expected}\""
    );

    // Every structural rule appears in the README detector table.
    for id in catalog.iter().filter(|id| id.starts_with("rust_doctor::structure::")) {
        assert!(readme.contains(id.as_str()), "README table misses {id}");
    }
}
