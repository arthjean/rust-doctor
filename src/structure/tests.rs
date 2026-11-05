use std::path::Path;

use cargo_metadata::{Metadata, MetadataCommand};

use std::collections::BTreeSet;

use super::*;
use crate::policy::{
    Producer, STRUCTURE_COMPLEX_FUNCTION, STRUCTURE_OVERSIZED_UNIT, STRUCTURE_UNREASONED_ALLOW,
};
use crate::generated::has_generator_header;
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

pub(super) fn repository() -> Metadata {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    MetadataCommand::new()
        .manifest_path(manifest)
        .no_deps()
        .other_options(["--offline".to_owned(), "--locked".to_owned()])
        .exec()
        .expect("this repository should describe itself")
}

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

#[test]
fn an_empty_enumeration_produces_neither_finding_nor_error() {
    let scan = analyze(
        &metadata("structure/empty"),
        &Enumeration::default(),
        &PolicyPlan::default(),
        &StructureSettings::default(),
        None,
    );
    assert_eq!(scan, StructureScan::default());
}

#[test]
fn an_incomplete_module_tree_withholds_its_package_and_counts_it() {
    let orphans = |relative: &str| {
        let metadata = metadata(relative);
        let scan = analyze(
            &metadata,
            &enumerate(&metadata),
            &PolicyPlan::default(),
            &StructureSettings::default(),
            None,
        );
        let paths: Vec<String> = scan
            .findings
            .iter()
            .filter(|finding| finding.definition.id == crate::policy::STRUCTURE_ORPHAN_MODULE_FILE.id)
            .map(|finding| finding.path.clone())
            .collect();
        (paths, scan.counters.orphan_abstentions, scan.errors)
    };

    let (paths, abstentions, errors) = orphans("structure/orphan-lost-module");
    assert!(paths.is_empty(), "{paths:?}");
    assert_eq!(abstentions, 1);
    assert!(errors.is_empty(), "{errors:?}");

    let (paths, abstentions, errors) = orphans("structure/orphan-conditional");
    assert_eq!(paths, ["fallback/src/stray.rs"]);
    assert_eq!(abstentions, 1, "only the package declaring a module in a macro");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn an_unparseable_unit_is_skipped_named_and_never_aborts_the_pass() {
    let errors = metadata("source-kernel/errors");
    let enumeration = enumerate(&errors);
    let scan = analyze(
        &errors,
        &enumeration,
        &PolicyPlan::default(),
        &StructureSettings::default(),
        None,
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

#[test]
fn an_exhausted_budget_stops_the_pass_and_says_so() {
    let duplicates = metadata("structure/duplicate-function");
    let enumeration = enumerate(&duplicates);
    let stopped = analyze(
        &duplicates,
        &enumeration,
        &PolicyPlan::default(),
        &StructureSettings::default(),
        Some(Duration::ZERO),
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

    let complete = analyze(
        &duplicates,
        &enumeration,
        &PolicyPlan::default(),
        &StructureSettings::default(),
        None,
    );
    assert!(complete.errors.is_empty(), "{:?}", complete.errors);
    assert!(!complete.findings.is_empty());
}

#[test]
fn a_pass_that_finished_late_is_not_a_partial_pass() {
    let duplicates = metadata("structure/duplicate-function");
    let enumeration = enumerate(&duplicates);
    let stopped = analyze_within(
        &duplicates,
        &enumeration,
        &PolicyPlan::default(),
        &StructureSettings::default(),
        Duration::from_nanos(1),
    );
    assert!(stopped.errors.iter().any(|error| error.code == "time-budget"));

    let complete = analyze_within(
        &duplicates,
        &enumeration,
        &PolicyPlan::default(),
        &StructureSettings::default(),
        Duration::from_secs(3_600),
    );
    assert!(complete.errors.is_empty(), "{:?}", complete.errors);
}

#[test]
fn an_inactive_rule_leaves_the_pass_with_nothing_to_do() {
    let allows = metadata("structure/unreasoned-allow");
    let enumeration = enumerate(&allows);
    let input = crate::policy::PolicyInput::default()
        .with_rule(STRUCTURE_UNREASONED_ALLOW.id, crate::policy::RuleLevel::Off);
    let plan = PolicyPlan::compile(&input).expect("policy should compile");
    assert!(
        analyze(&allows, &enumeration, &plan, &StructureSettings::default(), None)
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
            &StructureSettings::default(),
            None
        )
        .findings
        .is_empty()
    );
}

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
        None,
    );
    assert_eq!(scan, StructureScan::default());
}

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
        test_case: false,
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

#[test]
fn a_family_no_member_of_which_ships_takes_the_mark_of_its_anchor() {
    let benched = Member {
        context: Some(DiagnosticContext::Benchmark),
        ..member("benches/throughput.rs")
    };
    let tested = Member {
        context: Some(DiagnosticContext::Tests),
        ..member("tests/integration.rs")
    };

    assert_eq!(
        unanimous_context(&[benched.clone(), tested.clone()]),
        Some(DiagnosticContext::Benchmark)
    );
    assert_eq!(
        unanimous_context(&[tested.clone(), benched.clone()]),
        Some(DiagnosticContext::Tests)
    );
    assert_eq!(unanimous_context(&[benched, tested, member("src/lib.rs")]), None);
}

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

#[test]
fn a_generated_file_is_excluded() {
    for header in [
        "// @generated by prost-build",
        "// DO NOT EDIT: regenerated on every build",
        "// Automatically generated by bindgen.",
    ] {
        assert!(has_generator_header(&format!("{header}\nfn free() {{}}\n")), "{header}");
    }
    assert!(!has_generator_header("fn free() {}\n"));
    assert!(!has_generator_header(include_str!("../generated.rs")));
}

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

fn whole_repository(metadata: &Metadata) -> StructureScan {
    analyze_within(
        metadata,
        &enumerate(metadata),
        &PolicyPlan::default(),
        &StructureSettings::default(),
        Duration::from_secs(3_600),
    )
}

#[test]
fn no_unit_of_this_crate_s_own_source_is_a_hotspot() {
    let metadata = repository();
    let scan = whole_repository(&metadata);

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

#[test]
fn no_finding_of_this_crate_s_own_test_code_is_published_as_production() {
    let metadata = repository();
    let scan = whole_repository(&metadata);

    fn is_test_path(path: &str) -> bool {
        let mut components = path.split('/').collect::<Vec<_>>();
        let file = components.pop().unwrap_or_default();
        components
            .iter()
            .any(|component| matches!(*component, "tests" | "benches" | "examples"))
            || file == "tests.rs"
    }

    let misfiled: Vec<String> = scan
        .findings
        .iter()
        .filter(|finding| finding.context.is_none())
        .filter(|finding| {
            is_test_path(&finding.path)
                && finding
                    .related
                    .iter()
                    .all(|member| is_test_path(&member.path))
        })
        .map(|finding| {
            let members: Vec<&str> = std::iter::once(finding.path.as_str())
                .chain(finding.related.iter().map(|member| member.path.as_str()))
                .collect();
            format!("{}: {}", finding.definition.id, members.join(", "))
        })
        .collect();

    assert!(
        misfiled.is_empty(),
        "the crate charges itself for findings that are entirely test code:\n{}",
        misfiled.join("\n")
    );
}

#[test]
fn the_structure_holds_the_size_bound_it_reports_for() {
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
