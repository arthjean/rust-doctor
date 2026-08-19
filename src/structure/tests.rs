use std::path::Path;

use cargo_metadata::{Metadata, MetadataCommand};

use std::collections::BTreeSet;

use super::*;
use crate::policy::{
    Producer, STRUCTURE_COMPLEX_FUNCTION, STRUCTURE_OVERSIZED_UNIT, STRUCTURE_UNREASONED_ALLOW,
};
use crate::source_kernel::enumerate;

fn metadata(relative: &str) -> Metadata {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
        .join("Cargo.toml");
    MetadataCommand::new()
        .manifest_path(manifest)
        .no_deps()
        .other_options(["--offline".to_owned(), "--locked".to_owned()])
        .exec()
        .expect("fixture metadata should load")
}

/// This repository, as the pass reads it. Two tests scan it: what the
/// nomination recalls, and which of its own hotspots it names.
pub(super) fn repository() -> Metadata {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    MetadataCommand::new()
        .manifest_path(manifest)
        .no_deps()
        .other_options(["--offline".to_owned(), "--locked".to_owned()])
        .exec()
        .expect("this repository should describe itself")
}

/// Every rule the catalogue publishes under this producer is a rule the
/// pass runs, and the reverse. A rule declared in the catalogue and left
/// out of a family's table would be published by `rules list`, scored
/// against, and never actually observed.
#[test]
fn the_pass_produces_every_catalogued_structural_rule() {
    let mut produced: Vec<&str> = rules().map(|rule| rule.id).collect();
    produced.sort_unstable();
    let mut catalogued: Vec<&str> = crate::policy::CATALOG
        .iter()
        .filter(|rule| matches!(rule.producer, Producer::Structure))
        .map(|rule| rule.id)
        .collect();
    catalogued.sort_unstable();
    assert_eq!(produced, catalogued);
    assert_eq!(
        produced.len(),
        produced.iter().collect::<BTreeSet<_>>().len(),
        "a rule is declared by two families: {produced:?}"
    );
}

/// A workspace with no unit at all returns an empty result and no error:
/// there is nothing to be partial about.
#[test]
fn an_empty_enumeration_produces_neither_finding_nor_error() {
    let scan = analyze(
        &metadata("structure/empty"),
        &Enumeration::default(),
        &PolicyPlan::default(),
        &StructureSettings::default(),
    );
    assert_eq!(scan, StructureScan::default());
}

/// A unit the parser could not read is skipped and named, and the pass
/// completes over every other unit of the same workspace.
#[test]
fn an_unparseable_unit_is_skipped_named_and_never_aborts_the_pass() {
    let errors = metadata("source-kernel/errors");
    let enumeration = enumerate(&errors);
    let scan = analyze(
        &errors,
        &enumeration,
        &PolicyPlan::default(),
        &StructureSettings::default(),
    );

    let skipped: Vec<&str> = scan
        .errors
        .iter()
        .map(|error| {
            assert_eq!(error.code, "parse-error");
            error.message.as_str()
        })
        .collect();
    assert!(!skipped.is_empty(), "the fixture parses cleanly after all");
    assert!(
        skipped
            .iter()
            .all(|message| !message.contains(env!("CARGO_MANIFEST_DIR"))),
        "{skipped:?}"
    );
    let analysed = enumeration
        .units()
        .filter(|unit| unit.parses_cleanly())
        .count();
    assert!(
        analysed > 0,
        "the pass stopped at the first unreadable unit"
    );
}

/// US-009, FR-10: a budget the pass cannot meet stops it rather than
/// letting it run, and the stop is published as an error under the
/// `structure` stage, which is what takes the authoritative flag off the
/// score.
#[test]
fn an_exhausted_budget_stops_the_pass_and_says_so() {
    let duplicates = metadata("structure/duplicate-function");
    let enumeration = enumerate(&duplicates);
    let stopped = analyze_within(
        &duplicates,
        &enumeration,
        &PolicyPlan::default(),
        &StructureSettings::default(),
        Duration::ZERO,
    );
    assert_eq!(
        stopped
            .errors
            .iter()
            .map(|error| error.code)
            .collect::<Vec<_>>(),
        ["time-budget"],
        "{:?}",
        stopped.errors
    );
    assert!(
        stopped.errors.iter().all(|error| !error
            .message
            .contains(env!("CARGO_MANIFEST_DIR"))),
        "{:?}",
        stopped.errors
    );

    // The same workspace under the shipped budget finishes, so the stop
    // above is the budget and not the workspace.
    let complete = analyze(
        &duplicates,
        &enumeration,
        &PolicyPlan::default(),
        &StructureSettings::default(),
    );
    assert!(complete.errors.is_empty(), "{:?}", complete.errors);
    assert!(!complete.findings.is_empty());
}

/// FR-10: partiality is what a phase reports, never what a clock reads
/// afterwards. A pass that analysed every unit publishes a complete report
/// even when it finished after its own budget, because calling it partial
/// costs the score its authoritative flag for nothing.
#[test]
fn a_pass_that_finished_late_is_not_a_partial_pass() {
    let duplicates = metadata("structure/duplicate-function");
    let enumeration = enumerate(&duplicates);
    // A budget of one nanosecond is spent by the time the first unit is
    // read, so every phase of this scan ran with an exceeded clock. The
    // walk itself is what decides, and it decided to stop.
    let stopped = analyze_within(
        &duplicates,
        &enumeration,
        &PolicyPlan::default(),
        &StructureSettings::default(),
        Duration::from_nanos(1),
    );
    assert!(stopped.errors.iter().any(|error| error.code == "time-budget"));

    // The same scan under a budget it cannot exceed reports nothing, and
    // that is the only difference between the two.
    let complete = analyze_within(
        &duplicates,
        &enumeration,
        &PolicyPlan::default(),
        &StructureSettings::default(),
        Duration::from_secs(3_600),
    );
    assert!(complete.errors.is_empty(), "{:?}", complete.errors);
}

/// The pass is switched off by the policy like any other producer, and
/// costs nothing when it is.
#[test]
fn an_inactive_rule_leaves_the_pass_with_nothing_to_do() {
    let allows = metadata("structure/unreasoned-allow");
    let enumeration = enumerate(&allows);
    let input = crate::policy::PolicyInput::default()
        .with_rule(STRUCTURE_UNREASONED_ALLOW.id, crate::policy::RuleLevel::Off);
    let plan = PolicyPlan::compile(&input).expect("policy should compile");
    assert!(
        analyze(&allows, &enumeration, &plan, &StructureSettings::default())
            .findings
            .iter()
            .all(|finding| finding.definition.id != STRUCTURE_UNREASONED_ALLOW.id),
        "an inactive structural rule still produced a finding"
    );
    assert!(
        !analyze(
            &allows,
            &enumeration,
            &PolicyPlan::default(),
            &StructureSettings::default()
        )
        .findings
        .is_empty()
    );
}

/// Every rule off, and the pass does not even look at the units.
#[test]
fn a_policy_with_no_structural_rule_returns_the_empty_scan() {
    let mut input = crate::policy::PolicyInput::default();
    for rule in rules() {
        input = input.with_rule(rule.id, crate::policy::RuleLevel::Off);
    }
    let plan = PolicyPlan::compile(&input).expect("policy should compile");
    let allows = metadata("structure/unreasoned-allow");
    let scan = analyze(
        &allows,
        &enumerate(&allows),
        &plan,
        &StructureSettings::default(),
    );
    assert_eq!(scan, StructureScan::default());
}

/// The identity of a family is its rule and its key, and nothing else.
#[test]
fn the_structural_hash_depends_only_on_the_rule_and_the_key() {
    let hash = structural_hash("rust_doctor::structure::unreasoned_allow_attribute", "outer|a");
    assert_eq!(
        hash,
        structural_hash("rust_doctor::structure::unreasoned_allow_attribute", "outer|a")
    );
    assert_ne!(
        hash,
        structural_hash("rust_doctor::structure::unreasoned_allow_attribute", "outer|b")
    );
    assert_ne!(hash, structural_hash("rust_doctor::structure::other", "outer|a"));
    assert_eq!(hash.len(), 64);
}

fn member(path: &str) -> Member {
    Member {
        path: path.to_owned(),
        span: SourceSpan {
            line_start: 1,
            column_start: 1,
            line_end: 1,
            column_end: 2,
        },
        context: None,
    }
}

#[test]
fn a_family_is_marked_only_when_every_member_agrees() {
    let production = member("src/lib.rs");
    let tested = Member {
        context: Some(DiagnosticContext::Tests),
        ..production.clone()
    };
    assert_eq!(
        unanimous_context(std::slice::from_ref(&tested)),
        Some(DiagnosticContext::Tests)
    );
    assert_eq!(
        unanimous_context(&[tested.clone(), tested.clone()]),
        Some(DiagnosticContext::Tests)
    );
    assert_eq!(unanimous_context(&[tested, production.clone()]), None);
    assert_eq!(unanimous_context(&[production]), None);
    assert_eq!(unanimous_context(&[]), None);
}

/// One key, one family: the first arrival says what the family is, and
/// every later one adds a member to it. Recording is a merge, never a
/// replacement, whichever producer the members come from.
#[test]
fn recording_the_same_key_twice_merges_instead_of_replacing() {
    let mut families = BTreeMap::new();
    let rule = STRUCTURE_OVERSIZED_UNIT.id;
    record_family(
        &mut families,
        rule,
        "file|a".to_owned(),
        Summary::of("first".to_owned()),
        [member("src/a.rs")],
    );
    record_family(
        &mut families,
        rule,
        "file|a".to_owned(),
        Summary::of("second".to_owned()),
        [member("src/b.rs")],
    );
    let family = families
        .get(&(rule, "file|a".to_owned()))
        .expect("the family is under its key");
    assert_eq!(family.summary.subject, "first");
    assert_eq!(family.members.len(), 2);

    // An empty subject is a subject, not a signal: nothing later overwrites
    // it.
    let mut empty = BTreeMap::new();
    record_family(
        &mut empty,
        rule,
        "file|b".to_owned(),
        Summary::of(String::new()),
        [member("src/a.rs")],
    );
    record_family(
        &mut empty,
        rule,
        "file|b".to_owned(),
        Summary::of("later".to_owned()),
        [member("src/b.rs")],
    );
    assert_eq!(
        empty
            .get(&(rule, "file|b".to_owned()))
            .map(|family| family.summary.subject.as_str()),
        Some("")
    );
}

/// US-011: a recognized generator header excludes the file from the whole
/// structural pass, silently.
#[test]
fn a_generated_file_is_excluded() {
    for header in [
        "// @generated by prost-build",
        "// DO NOT EDIT: regenerated on every build",
        "// Automatically generated by bindgen.",
    ] {
        assert!(is_generated(&format!("{header}\nfn free() {{}}\n")), "{header}");
    }
    assert!(!is_generated("fn free() {}\n"));
    // The markers this file documents are not a header of it.
    assert!(!is_generated(include_str!("../structure.rs")));
}

/// The inventory is one walk, and it is the one every family reads.
#[test]
fn the_inventory_collects_every_kind_the_detectors_read() {
    let unit = Unit::probe(
        "#![allow(dead_code)]\nmod inner { impl Probe { fn method() { println!(\"a\"); } } }\n",
        "src/lib.rs",
    );
    assert_eq!(unit.inventory.attributes.len(), 1);
    assert_eq!(unit.inventory.functions.len(), 1);
    assert_eq!(unit.inventory.implementations.len(), 1);
    assert_eq!(unit.inventory.modules.len(), 1);
    assert_eq!(unit.inventory.macro_calls.len(), 1);
}

/// EP-003, definition of done, inverted: a scan of this repository names no
/// oversized unit and no complexity hotspot anywhere under `src/`, through the
/// same pass `inspect` runs.
///
/// It used to assert the opposite, that `src/report.rs` was named oversized,
/// which froze the crate's largest self-violation in place: repairing the file
/// failed the suite. The rule's own evidence that it fires belongs on a
/// fixture, and `tests/rule_evidence.json` names the two tests that carry it.
/// What belongs here is the gate: the tool has to pass what it reports.
///
/// The nine `the_X_holds_the_size_bound` tests scattered across the crate stay,
/// because each of them fails on its own module and says which one. This one
/// covers every file none of them names, and the three units below the file
/// level that none of them can see.
#[test]
fn no_unit_of_this_crate_s_own_source_is_a_hotspot() {
    let metadata = repository();
    let scan = analyze(
        &metadata,
        &enumerate(&metadata),
        &PolicyPlan::default(),
        &StructureSettings::default(),
    );

    let named = |rule: &str| -> Vec<String> {
        scan.findings
            .iter()
            .filter(|finding| finding.definition.id == rule && finding.path.starts_with("src/"))
            .map(|finding| format!("{}: {}", finding.path, finding.message))
            .collect()
    };

    let oversized = named(STRUCTURE_OVERSIZED_UNIT.id);
    assert!(
        oversized.is_empty(),
        "the crate reports itself oversized:\n{}",
        oversized.join("\n")
    );

    let tangled = named(STRUCTURE_COMPLEX_FUNCTION.id);
    assert!(
        tangled.is_empty(),
        "the crate reports its own complexity hotspots:\n{}",
        tangled.join("\n")
    );
}

/// The pass no longer names its own root as oversized: extracting the
/// suppression rules and the benchmark is what took it back under the bound
/// it publishes.
#[test]
fn the_pass_holds_its_own_size_bound() {
    for own in [
        include_str!("../structure.rs"),
        include_str!("benchmark.rs"),
        include_str!("duplication.rs"),
        include_str!("duplication/tests.rs"),
        include_str!("hotspots.rs"),
        include_str!("manifest.rs"),
        include_str!("normalize.rs"),
        include_str!("suppression.rs"),
        include_str!("tests.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < hotspots::FILE_LINES,
            "a file of the structural pass is {lines} lines long, over the {} it reports",
            hotspots::FILE_LINES
        );
    }
}
