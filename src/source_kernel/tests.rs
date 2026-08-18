use std::path::PathBuf;

use cargo_metadata::{Metadata, MetadataCommand};
use ra_ap_syntax::{Edition, SourceFile, SyntaxKind, SyntaxNode, TextRange};

use super::walk::enumerate_with_limits;
use super::*;

use crate::policy::{
    PolicyInput, Producer, RuleLevel, RuleTier, SOURCE_DISABLED_TLS, SOURCE_DYNAMIC_SHELL,
    STRUCTURE_COMPLEX_FUNCTION, STRUCTURE_CRATE_LEVEL_ALLOW, STRUCTURE_DUPLICATE_FUNCTION_BODY,
    STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY, STRUCTURE_ORPHAN_MODULE_FILE, STRUCTURE_OVERSIZED_UNIT,
    STRUCTURE_STACKED_ALLOW, STRUCTURE_UNREASONED_ALLOW, STRUCTURE_UNREFERENCED_FEATURE,
};

fn scan(metadata: &Metadata) -> SourceScan {
    scan_for_plan(metadata, &PolicyPlan::default())
}

/// Reproduces the gate `execution` applies: the workspace is walked only
/// when a producer reading source text still has an active rule, so a
/// pruned policy proves an absence of IO rather than an empty result.
fn scan_for_plan(metadata: &Metadata, plan: &PolicyPlan) -> SourceScan {
    if !enumeration_required(plan) {
        return SourceScan::default();
    }
    inspect(&enumerate(metadata), plan)
}

fn scan_with_limits(metadata: &Metadata, limits: Limits) -> SourceScan {
    inspect_with_limits(
        &enumerate_with_limits(metadata, limits),
        limits,
        &PolicyPlan::default(),
    )
}

fn solicitations(counters: &SourceCounters, id: &str) -> usize {
    counters.analysis.predicates.get(id).copied().unwrap_or_default()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/source-kernel")
        .join(name)
}

fn metadata(name: &str) -> Metadata {
    let manifest = fixture(name).join("Cargo.toml");
    let mut command = MetadataCommand::new();
    command
        .manifest_path(manifest)
        .no_deps()
        .other_options(["--offline".to_owned(), "--locked".to_owned()]);
    command.exec().unwrap()
}

static SILENT_RULE: RuleDefinition = RuleDefinition {
    id: "rust_doctor::source::silent_probe",
    category: "security",
    producer: Producer::SourceKernel,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Synthetic registry proof.",
};

/// A registered detector that never emits: it proves that a silent
/// detector costs one solicitation and nothing else.
static SILENT: Detector = Detector {
    definition: &SILENT_RULE,
    node: SyntaxKind::SOURCE_FILE,
    inspect: silent,
};

/// Whether a plan asks for anything the walk feeds.
///
/// A helper of these tests alone: the orchestrator asks the plan directly, and
/// answering the same question in two modules is how the two drifted apart.
fn enumeration_required(plan: &PolicyPlan) -> bool {
    [Producer::SourceKernel, Producer::Structure]
        .into_iter()
        .any(|producer| plan.active_rules(producer).next().is_some())
}

const REGISTERED: [&Detector; 2] = DETECTORS;

fn silent(_: &detectors::Context<'_>, _: &SyntaxNode) -> Option<Detection> {
    None
}

/// Synthetic unit: the registry is exercised without touching the disk.
fn synthetic_unit(source: &str) -> SourceUnit {
    let parse = SourceFile::parse(source, Edition::Edition2024);
    SourceUnit {
        source: source.to_owned(),
        error_ranges: parse.errors().iter().map(|error| error.range()).collect(),
        parse,
        edition: Edition::Edition2024,
        relative_path: "src/lib.rs".to_owned(),
        reachability: BTreeSet::from([Reachability {
            package_id: "synthetic".to_owned(),
            package_name: "synthetic".to_owned(),
            target_key: "synthetic:0".to_owned(),
            target_name: "synthetic".to_owned(),
        }]),
        traversals: BTreeSet::new(),
    }
}

fn synthetic_enumeration(source: &str) -> Enumeration {
    Enumeration {
        units: BTreeMap::from([(
            Identity {
                path: PathBuf::from("/synthetic/lib.rs"),
                edition: Edition::Edition2024,
            },
            synthetic_unit(source),
        )]),
        tables: BTreeMap::from([(
            "synthetic".to_owned(),
            CrateAliases::from([("http_client".to_owned(), "reqwest".to_owned())]),
        )]),
        ..Enumeration::default()
    }
}

fn analyze(source: &str, detectors: &[&'static Detector]) -> (Vec<&'static str>, SourceCounters) {
    let (candidates, counters, _) = analyze_with_limits(source, detectors, LIMITS);
    (candidates, counters)
}

fn analyze_with_limits(
    source: &str,
    detectors: &[&'static Detector],
    limits: Limits,
) -> (Vec<&'static str>, SourceCounters, Vec<SourceError>) {
    let enumeration = synthetic_enumeration(source);
    let registry = Registry::new(detectors.iter().copied());
    let mut findings = Findings::default();
    for unit in enumeration.units.values() {
        analyze_unit(unit, &enumeration, &registry, limits, &mut findings);
    }
    let codes = findings
        .candidates
        .into_values()
        .map(|candidate| candidate.definition.id)
        .collect();
    (codes, findings.counters, findings.errors)
}

#[test]
fn pinned_parser_supports_all_target_editions_and_recoverable_errors() {
    let valid = "fn main() { let value = 1; }";
    for edition in [
        Edition::Edition2015,
        Edition::Edition2018,
        Edition::Edition2021,
        Edition::Edition2024,
    ] {
        let parse = SourceFile::parse(valid, edition);
        assert!(parse.errors().is_empty());
        assert_eq!(
            usize::from(parse.tree().syntax().text_range().len()),
            valid.len()
        );
    }

    let recoverable = "fn valid() {}\nfn broken( {\nfn after() {}";
    let parse = SourceFile::parse(recoverable, Edition::Edition2024);
    let errors = parse.errors();
    assert!(!errors.is_empty());
    assert_eq!(
        usize::from(parse.tree().syntax().text_range().len()),
        recoverable.len()
    );
    assert!(errors.iter().all(|error| {
        usize::from(error.range().start()) <= recoverable.len()
            && usize::from(error.range().end()) <= recoverable.len()
    }));
}

#[test]
fn producer_uses_the_two_canonical_catalog_entries() {
    let definitions: Vec<_> = PolicyPlan::default()
        .active_rules(Producer::SourceKernel)
        .map(|(definition, _)| definition.id)
        .collect();
    assert_eq!(definitions, [SOURCE_DISABLED_TLS.id, SOURCE_DYNAMIC_SHELL.id]);
}

#[test]
fn policy_prunes_source_io_and_each_inactive_predicate() {
    let metadata = metadata("precision");
    let all_off = PolicyInput::default()
        .with_rule(SOURCE_DISABLED_TLS.id, RuleLevel::Off)
        .with_rule(SOURCE_DYNAMIC_SHELL.id, RuleLevel::Off)
        .with_rule(STRUCTURE_UNREASONED_ALLOW.id, RuleLevel::Off)
        .with_rule(STRUCTURE_CRATE_LEVEL_ALLOW.id, RuleLevel::Off)
        .with_rule(STRUCTURE_STACKED_ALLOW.id, RuleLevel::Off)
        .with_rule(STRUCTURE_COMPLEX_FUNCTION.id, RuleLevel::Off)
        .with_rule(STRUCTURE_DUPLICATE_FUNCTION_BODY.id, RuleLevel::Off)
        .with_rule(STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY.id, RuleLevel::Off)
        .with_rule(STRUCTURE_ORPHAN_MODULE_FILE.id, RuleLevel::Off)
        .with_rule(STRUCTURE_OVERSIZED_UNIT.id, RuleLevel::Off)
        .with_rule(STRUCTURE_UNREFERENCED_FEATURE.id, RuleLevel::Off);
    let all_off = PolicyPlan::compile(&all_off).expect("policy should compile");
    assert!(!enumeration_required(&all_off));
    let scanned = scan_for_plan(&metadata, &all_off);
    assert!(scanned.candidates.is_empty());
    assert!(scanned.errors.is_empty());
    assert_eq!(scanned.counters.walk, WalkCounters::default());
    assert_eq!(scanned.counters.analysis.nodes_visited, 0);
    assert!(scanned.counters.analysis.predicates.is_empty());

    let shell_off = PolicyInput::default().with_rule(SOURCE_DYNAMIC_SHELL.id, RuleLevel::Off);
    let shell_off = PolicyPlan::compile(&shell_off).expect("policy should compile");
    let scanned = scan_for_plan(&metadata, &shell_off);
    assert!(scanned.counters.walk.files_read > 0);
    assert!(solicitations(&scanned.counters, SOURCE_DISABLED_TLS.id) > 0);
    assert_eq!(solicitations(&scanned.counters, SOURCE_DYNAMIC_SHELL.id), 0);
    assert!(
        scanned
            .candidates
            .iter()
            .all(|candidate| candidate.definition.id != SOURCE_DYNAMIC_SHELL.id)
    );

    let tls_off = PolicyInput::default().with_rule(SOURCE_DISABLED_TLS.id, RuleLevel::Off);
    let tls_off = PolicyPlan::compile(&tls_off).expect("policy should compile");
    let scanned = scan_for_plan(&metadata, &tls_off);
    assert_eq!(solicitations(&scanned.counters, SOURCE_DISABLED_TLS.id), 0);
    assert!(solicitations(&scanned.counters, SOURCE_DYNAMIC_SHELL.id) > 0);
    assert!(
        scanned
            .candidates
            .iter()
            .all(|candidate| candidate.definition.id != SOURCE_DISABLED_TLS.id)
    );
}

/// The imported form and the fully qualified form go through the same
/// mechanism, and an undecidable provenance silences the detector.
#[test]
fn detectors_resolve_written_paths_through_the_alias_map() {
    let both = "use std::process::Command;
fn run(user: &str) {
    let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\"));
    let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\"));
}";
    assert_eq!(
        analyze(both, &REGISTERED).0,
        [SOURCE_DYNAMIC_SHELL.id, SOURCE_DYNAMIC_SHELL.id]
    );

    let renamed = "use http_client::Client;
fn build() {
    let _ = Client::builder().danger_accept_invalid_certs(true);
}";
    assert_eq!(analyze(renamed, &REGISTERED).0, [SOURCE_DISABLED_TLS.id]);

    for abstained in [
        "use std::process::*;
fn run(user: &str) { let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\")); }",
        "fn run(user: &str) {
    struct Command;
    let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\"));
}",
        "use unknown_crate::Client;
fn build() { let _ = Client::builder().danger_accept_invalid_certs(true); }",
        "fn build() { let _ = other_alias::Client::builder().danger_accept_invalid_certs(true); }",
    ] {
        assert!(analyze(abstained, &REGISTERED).0.is_empty(), "{abstained}");
    }
}

/// The walk is single and independent of the registry: the number of
/// visited nodes depends neither on the number of detectors nor on their
/// order, and a silent detector does not change the result.
#[test]
fn the_registry_shares_one_traversal_and_stays_order_independent() {
    let source = "use std::process::Command;
fn run(user: &str) {
    let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\"));
    let _ = http_client::Client::builder().danger_accept_invalid_certs(true);
}";
    let nodes = SourceFile::parse(source, Edition::Edition2024)
        .tree()
        .syntax()
        .descendants()
        .count();
    let method_calls = SourceFile::parse(source, Edition::Edition2024)
        .tree()
        .syntax()
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::METHOD_CALL_EXPR)
        .count();

    let (expected, counters) = analyze(source, &REGISTERED);
    assert_eq!(expected, [SOURCE_DISABLED_TLS.id, SOURCE_DYNAMIC_SHELL.id]);
    assert_eq!(counters.analysis.nodes_visited, nodes);
    for detector in REGISTERED {
        assert_eq!(
            solicitations(&counters, detector.definition.id),
            method_calls,
            "{}",
            detector.definition.id
        );
    }

    let permuted = analyze(source, &[REGISTERED[1], REGISTERED[0]]);
    assert_eq!(permuted.0, expected);
    assert_eq!(permuted.1.analysis.nodes_visited, nodes);

    let single = analyze(source, &[REGISTERED[0]]);
    assert_eq!(single.1.analysis.nodes_visited, nodes);
    assert_eq!(solicitations(&single.1, SOURCE_DYNAMIC_SHELL.id), 0);

    // A registered detector that emits nothing leaves the result unchanged.
    let with_silent = analyze(source, &[REGISTERED[0], &SILENT, REGISTERED[1]]);
    assert_eq!(with_silent.0, expected);
    assert_eq!(with_silent.1.analysis.nodes_visited, nodes);
    assert_eq!(solicitations(&with_silent.1, SILENT_RULE.id), 1);
}

/// Grouping the registry by node kind may not change what a detector is
/// asked: a detector is solicited on every node of its kind and on no other,
/// whichever kinds its neighbours declared.
#[test]
fn a_detector_is_solicited_on_its_own_kind_and_never_on_another() {
    let source = "fn run(user: &str) { let _ = Command::new(\"sh\").arg(user); }";
    let tree = SourceFile::parse(source, Edition::Edition2024);
    let source_files = tree
        .tree()
        .syntax()
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::SOURCE_FILE)
        .count();

    let (_, counters) = analyze(source, &[&SILENT, REGISTERED[0], REGISTERED[1]]);
    assert_eq!(solicitations(&counters, SILENT_RULE.id), source_files);
    assert_eq!(source_files, 1);
    for detector in REGISTERED {
        assert!(solicitations(&counters, detector.definition.id) > 0);
    }
}

/// `format!` arguments are classified after parsing, never after splitting the
/// token tree on commas and gluing the remaining tokens back into text.
///
/// The generic arguments below carry a comma that separates nothing, and the
/// explicit `{2}` has to land on the third argument rather than on whatever
/// fragment a text split left in third position. The named forms are read as
/// assignment expressions, which is what a hand-written guard against `==` used
/// to approximate.
#[test]
fn format_arguments_are_classified_after_parsing_not_after_splitting_text() {
    let shell = |payload: &str| {
        format!(
            "use std::process::Command;
fn run(user: &str) {{ let _ = Command::new(\"sh\").arg(\"-c\").arg({payload}); }}"
        )
    };

    for constant in [
        // `{2}` names `\"c\"`, a literal, and the comma inside the generic
        // arguments shifts no position.
        "format!(\"echo {2}\", \"a\", probe::<u8, u16>(), \"c\")",
        "format!(\"echo {name}\", name = \"literal\")",
        "format!(\"echo {}\", \"a, b\")",
    ] {
        assert!(
            analyze(&shell(constant), &REGISTERED).0.is_empty(),
            "{constant}"
        );
    }

    for dynamic in [
        "format!(\"echo {user}\")",
        "format!(\"echo {}\", user)",
        "format!(\"echo {name}\", name = user)",
        // A comparison is not a named argument, whatever splitting its `=`
        // would have suggested.
        "format!(\"echo {}\", user == \"root\")",
        "format!(\"echo {}\", probe::<u8, u16>(user))",
        // `format!` means the same by each delimiter pair.
        "format!{\"echo {user}\"}",
    ] {
        assert_eq!(
            analyze(&shell(dynamic), &REGISTERED).0,
            [SOURCE_DYNAMIC_SHELL.id],
            "{dynamic}"
        );
    }
}

/// A unit whose alias map saturates is still analysed, but no provenance
/// is decidable in it any more.
#[test]
fn a_saturated_alias_map_reports_a_bounded_error_and_abstains() {
    let source = "use std::process::Command;
use std::io::Write;
fn run(user: &str) { let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\")); }";
    let limits = Limits {
        alias_bindings: 1,
        ..LIMITS
    };
    let (candidates, _, errors) = analyze_with_limits(source, &REGISTERED, limits);

    assert!(candidates.is_empty());
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "limit-exceeded");
    assert!(errors[0].message.contains(Limit::AliasBindings.name()));
}

#[test]
fn corpus_follows_modules_once_and_emits_only_closed_predicates() {
    let scanned = scan(&metadata("precision"));
    assert!(scanned.errors.is_empty(), "{:?}", scanned.errors);
    assert_eq!(scanned.counters.walk.files_read, 13);
    assert_eq!(scanned.candidates.len(), 13, "{:?}", scanned.candidates);
    assert!(
        scanned
            .candidates
            .iter()
            .all(|candidate| candidate.package.as_deref() == Some("source-kernel-app"))
    );
    assert!(scanned.candidates.iter().any(|candidate| {
        candidate.definition.id == SOURCE_DYNAMIC_SHELL.id
            && candidate.path == "app/src/shared.rs"
            && candidate.target.is_none()
    }));
    assert_eq!(
        scanned
            .candidates
            .iter()
            .filter(|candidate| candidate.definition.id == SOURCE_DISABLED_TLS.id)
            .count(),
        7
    );
    assert!(
        scanned
            .candidates
            .iter()
            .all(|candidate| candidate.path != "app/src/ignored.rs")
    );
    assert!(
        scanned
            .candidates
            .iter()
            .all(|candidate| !candidate.path.contains("/tests/"))
    );
}

#[test]
fn partial_failures_are_private_deduplicated_and_preserve_valid_findings() {
    let scanned = scan(&metadata("errors"));
    let codes: Vec<_> = scanned.errors.iter().map(|error| error.code).collect();
    assert_eq!(
        codes,
        [
            "module-ambiguous",
            "module-not-found",
            "parse-error",
            "parse-error",
            "path-outside-workspace",
            "path-outside-workspace",
        ]
    );
    assert_eq!(scanned.candidates.len(), 2, "{:?}", scanned.candidates);
    assert_eq!(scanned.counters.walk.files_read, 5);
    assert!(
        scanned
            .candidates
            .iter()
            .all(|candidate| candidate.definition.id == SOURCE_DYNAMIC_SHELL.id)
    );
    assert!(
        scanned
            .candidates
            .iter()
            .all(|candidate| candidate.path != "src/intersected.rs")
    );
    let rendered = format!("{:?}", scanned.errors);
    assert!(!rendered.contains(env!("CARGO_MANIFEST_DIR")));
    assert!(!rendered.contains("secret"));
    assert!(!rendered.contains("fn invalid"));
}

#[test]
fn existing_and_missing_roots_outside_the_workspace_are_never_read() {
    let existing = scan(&metadata("external-root"));
    assert_eq!(existing.counters.walk.files_read, 0);
    assert!(existing.candidates.is_empty());
    assert_eq!(existing.errors.len(), 1);
    assert_eq!(existing.errors[0].code, "path-outside-workspace");
    assert!(
        !existing.errors[0]
            .message
            .contains(env!("CARGO_MANIFEST_DIR"))
    );

    let mut missing_metadata = metadata("external-root");
    let missing = missing_metadata
        .workspace_root
        .join("../missing-external-main.rs");
    missing_metadata.packages[0].targets[0].src_path = missing;
    let missing = scan(&missing_metadata);
    assert_eq!(missing.counters.walk.files_read, 0);
    assert!(missing.candidates.is_empty());
    assert_eq!(missing.errors.len(), 1);
    assert_eq!(missing.errors[0].code, "path-outside-workspace");
}

#[cfg(unix)]
#[test]
fn symlinks_outside_the_workspace_are_rejected_before_reading() {
    use std::fs;
    use std::os::unix::fs::symlink;

    // Built outside this repository on purpose. Under `target/` the tree
    // inherits whatever that path is on the machine, and a `target` symlink
    // pointing at a build directory elsewhere makes every unit resolve
    // outside the workspace root cargo reports, so the walk reads nothing
    // and the test fails for a reason that has nothing to do with symlink
    // containment.
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!(
            "rust-doctor-source-kernel-symlink-{}",
            std::process::id()
        ));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"symlink-proof\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "mod escape;\n").unwrap();
    symlink(fixture("outside.rs"), root.join("src/escape.rs")).unwrap();

    let mut command = MetadataCommand::new();
    command
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .other_options(["--offline".to_owned()]);
    let scanned = scan(&command.exec().unwrap());

    assert_eq!(scanned.counters.walk.files_read, 1);
    assert_eq!(scanned.errors.len(), 1);
    assert_eq!(scanned.errors[0].code, "path-outside-workspace");
    assert!(
        !scanned.errors[0]
            .message
            .contains(&root.display().to_string())
    );
    assert!(!scanned.errors[0].message.contains(env!("CARGO_MANIFEST_DIR")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_utf8_is_a_private_read_failure_that_still_charges_the_byte_budget() {
    use std::fs;

    let clean = scan(&metadata("precision"));
    let mut metadata = metadata("precision");
    let path = metadata.workspace_root.join(format!(
        "target/source-kernel-invalid-{}.rs",
        std::process::id()
    ));
    let package = metadata
        .packages
        .iter_mut()
        .find(|package| package.name.as_str() == "source-kernel-app")
        .unwrap();
    let mut target = package.targets[0].clone();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, [0xff, 0xfe]).unwrap();
    target.src_path = path.clone();
    package.targets = vec![target];

    let scanned = scan(&metadata);
    fs::remove_file(path).unwrap();

    assert_eq!(scanned.counters.walk.files_read, 1);
    assert_eq!(scanned.errors.len(), 1);
    assert_eq!(scanned.errors[0].code, "read-failed");
    assert!(!scanned.errors[0].message.contains(env!("CARGO_MANIFEST_DIR")));
    // The two bytes left the disk, so the budget was charged for them even
    // though no unit came out. A walk that charged only what decoded could
    // read the per-file limit over and over with the total never moving.
    assert!(scanned.counters.walk.bytes_read > 0);
    assert!(scanned.counters.walk.bytes_read < clean.counters.walk.bytes_read);
}

#[test]
fn target_editions_do_not_override_the_package_edition() {
    let mut metadata = metadata("precision");
    let package = metadata
        .packages
        .iter_mut()
        .find(|package| package.name.as_str() == "source-kernel-app")
        .unwrap();
    let mut edition_2018 = package.targets[0].clone();
    edition_2018.name = "edition-2018".to_owned();
    edition_2018.edition = cargo_metadata::Edition::E2018;
    let mut edition_2024 = edition_2018.clone();
    edition_2024.name = "edition-2024".to_owned();
    edition_2024.edition = cargo_metadata::Edition::E2024;
    package.targets = vec![edition_2018, edition_2024];

    let scanned = scan(&metadata);

    assert!(scanned.errors.is_empty(), "{:?}", scanned.errors);
    assert_eq!(scanned.counters.walk.files_read, 11);
    assert_eq!(scanned.candidates.len(), 13);
    assert!(
        scanned
            .candidates
            .iter()
            .all(|candidate| candidate.target.is_none())
    );
}

#[test]
fn unsupported_package_editions_fail_closed() {
    let mut metadata = metadata("precision");
    let package = metadata
        .packages
        .iter_mut()
        .find(|package| package.name.as_str() == "source-kernel-app")
        .unwrap();
    package.edition = cargo_metadata::Edition::_E2027;

    let scanned = scan(&metadata);

    assert!(scanned.candidates.is_empty());
    assert_eq!(scanned.errors.len(), 1);
    assert_eq!(scanned.errors[0].code, "parse-error");
    assert!(scanned.errors[0].message.contains("unsupported Rust edition"));
}

#[test]
fn file_total_unit_and_depth_limits_stop_the_required_work() {
    let metadata = metadata("precision");
    for (limits, limit) in [
        (
            Limits {
                file_bytes: 1,
                ..LIMITS
            },
            Limit::FileBytes,
        ),
        (
            Limits {
                total_bytes: 1,
                ..LIMITS
            },
            Limit::TotalBytes,
        ),
        (Limits { units: 0, ..LIMITS }, Limit::SourceUnits),
        (
            Limits {
                module_depth: 0,
                ..LIMITS
            },
            Limit::ModuleDepth,
        ),
        (
            Limits {
                alias_bindings: 1,
                ..LIMITS
            },
            Limit::AliasBindings,
        ),
    ] {
        let scanned = scan_with_limits(&metadata, limits);
        assert_eq!(
            scanned
                .errors
                .iter()
                .filter(|error| {
                    error.code == "limit-exceeded" && error.message.contains(limit.name())
                })
                .count(),
            1,
            "{limit:?}"
        );
    }
}

/// The global byte budget ends the walk rather than skipping one file, and
/// that decision is carried by `Loaded`, not recovered from the sentence the
/// limit printed. Exhausting it must leave the walk with fewer units than a
/// complete run, not merely with an error beside a complete result.
#[test]
fn an_exhausted_byte_budget_stops_the_walk_instead_of_skipping_a_file() {
    let metadata = metadata("precision");
    let complete = enumerate(&metadata);
    let complete_units = complete.units().count();
    assert!(complete_units > 1);

    let exhausted = enumerate_with_limits(
        &metadata,
        Limits {
            total_bytes: 1,
            ..LIMITS
        },
    );
    assert!(exhausted.units().count() < complete_units);
    assert_eq!(
        exhausted
            .errors
            .iter()
            .filter(|error| error.message.contains(Limit::TotalBytes.name()))
            .count(),
        1
    );
}

#[test]
fn twenty_metadata_reachability_permutations_are_identical() {
    let mut metadata = metadata("precision");
    let package_index = metadata
        .packages
        .iter()
        .position(|package| package.name.as_str() == "source-kernel-app")
        .unwrap();
    let root = metadata.packages[package_index].targets[0].clone();
    let targets: Vec<_> = (0..5)
        .map(|index| {
            let mut target = root.clone();
            target.name = format!("target-{index}");
            target
        })
        .collect();
    let mut order = [0, 1, 2, 3, 4];
    let mut expected = None;
    for _ in 0..20 {
        metadata.packages[package_index].targets =
            order.iter().map(|index| targets[*index].clone()).collect();
        let scanned = scan(&metadata);
        let observation = format!(
            "{:?}|{:?}|{:?}",
            scanned.candidates, scanned.errors, scanned.counters
        );
        match expected.as_ref() {
            Some(expected) => assert_eq!(&observation, expected),
            None => expected = Some(observation),
        }
        assert!(next_permutation(&mut order));
    }
}

/// Unanimity is the module's one recurring answer, so it is proved once: an
/// empty reach and a disagreeing reach both abstain, and only a reach that
/// agrees publishes a value.
#[test]
fn unanimity_abstains_on_disagreement_and_on_an_empty_reach() {
    assert_eq!(unanimous(Vec::<u8>::new()), None);
    assert_eq!(unanimous([7]), Some(7));
    assert_eq!(unanimous([7, 7, 7]), Some(7));
    assert_eq!(unanimous([7, 7, 8]), None);
    // Options collapse the same way, which is what lets a missing mark and a
    // disagreeing mark be one case at the call sites.
    assert_eq!(unanimous([Some(1), Some(1)]).flatten(), Some(1));
    assert_eq!(unanimous([Some(1), None]).flatten(), None);
    assert_eq!(unanimous([None::<u8>, None]).flatten(), None);
}

/// A file two packages reach names neither, and a file one package reaches
/// through two targets names the package but no target.
#[test]
fn a_unit_publishes_only_what_every_reacher_agrees_on() {
    let reach = |package: &str, target: usize| Reachability {
        package_id: package.to_owned(),
        package_name: package.to_owned(),
        target_key: format!("{package}:{target}"),
        target_name: format!("target-{target}"),
    };

    let mut unit = synthetic_unit("pub fn probe() {}");
    unit.reachability = BTreeSet::from([reach("alpha", 0)]);
    assert_eq!(unit.package().as_deref(), Some("alpha"));
    assert_eq!(unit.target().as_deref(), Some("target-0"));

    unit.reachability.insert(reach("alpha", 1));
    assert_eq!(unit.package().as_deref(), Some("alpha"));
    assert_eq!(unit.target(), None);

    unit.reachability.insert(reach("beta", 0));
    assert_eq!(unit.package(), None);
    assert_eq!(unit.target(), None);

    // A context every reacher marks the same way survives; one they disagree
    // on does not, because silencing shipped code is the expensive mistake.
    let agreeing = TargetContexts::from([
        ("alpha:0".to_owned(), Some(DiagnosticContext::Tests)),
        ("alpha:1".to_owned(), Some(DiagnosticContext::Tests)),
        ("beta:0".to_owned(), Some(DiagnosticContext::Tests)),
    ]);
    assert_eq!(unit.context(&agreeing), Some(DiagnosticContext::Tests));
    assert!(unit.is_test_target(&agreeing));

    let mut disagreeing = agreeing.clone();
    disagreeing.insert("beta:0".to_owned(), None);
    assert_eq!(unit.context(&disagreeing), None);
    assert!(!unit.is_test_target(&disagreeing));
}

/// Cargo's target kind is the authority on test material, and the path
/// convention only adds what no target names on its own.
#[test]
fn test_code_is_named_by_cargo_first_and_by_the_path_only_as_a_fallback() {
    let mut unit = synthetic_unit("pub fn probe() {}");
    let empty = TargetContexts::new();
    assert!(!unit.is_test_code(&empty));

    let benched = TargetContexts::from([(
        "synthetic:0".to_owned(),
        Some(DiagnosticContext::Benchmark),
    )]);
    // A bench target carries no `tests` path segment: only Cargo can say so.
    assert!(!path_contains_tests_segment(unit.relative_path()));
    assert!(unit.is_test_code(&benched));

    unit.relative_path = "src/probe/tests/helpers.rs".to_owned();
    assert!(unit.is_test_code(&empty));
}

#[test]
#[ignore = "manual probe for the five explicitly approved pinned repositories"]
fn pinned_real_world_evaluation_probe() {
    let manifest = std::env::var_os("RUST_DOCTOR_EVALUATION_MANIFEST")
        .map(PathBuf::from)
        .expect("RUST_DOCTOR_EVALUATION_MANIFEST must name an approved manifest");
    let mut command = MetadataCommand::new();
    command
        .manifest_path(manifest)
        .no_deps()
        .other_options(["--offline".to_owned()]);
    let scanned = scan(&command.exec().expect("approved metadata should load"));
    let mut counts = BTreeMap::from([
        (SOURCE_DISABLED_TLS.id, 0_usize),
        (SOURCE_DYNAMIC_SHELL.id, 0_usize),
    ]);
    for candidate in &scanned.candidates {
        *counts.entry(candidate.definition.id).or_default() += 1;
    }
    let mut errors = BTreeMap::<&str, usize>::new();
    for error in &scanned.errors {
        *errors.entry(error.code).or_default() += 1;
    }
    let findings: Vec<_> = scanned
        .candidates
        .iter()
        .map(|candidate| {
            serde_json::json!({
                "code": candidate.definition.id,
                "package": candidate.package,
                "target": candidate.target,
                "path": candidate.path,
                "span": {
                    "line_start": candidate.span.line_start,
                    "column_start": candidate.span.column_start,
                    "line_end": candidate.span.line_end,
                    "column_end": candidate.span.column_end,
                }
            })
        })
        .collect();
    println!(
        "RUST_DOCTOR_SOURCE_EVALUATION={}",
        serde_json::json!({
            "files_read": scanned.counters.walk.files_read,
            "bytes_parsed": scanned.counters.walk.bytes_read,
            "counts": counts,
            "source_errors": errors,
            "findings": findings,
        })
    );
}

/// The kernel passes the rule it feeds. `oversized_unit` reports a file at a
/// thousand lines, the structural pass it enumerates for holds that bound, and
/// so does this one: the walk and these tests have files of their own for that
/// reason, and a file that grows back past it fails here rather than on a
/// self-scan nobody reads.
#[test]
fn the_kernel_holds_the_size_bound_it_enumerates_for() {
    for own in [
        include_str!("../source_kernel.rs"),
        include_str!("../source_text.rs"),
        include_str!("aliases.rs"),
        include_str!("detectors.rs"),
        include_str!("references.rs"),
        include_str!("tests.rs"),
        include_str!("walk.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the source kernel is {lines} lines long, over the {} it reports",
            crate::structure::FILE_LINES
        );
    }
}

fn next_permutation(values: &mut [usize]) -> bool {
    let Some(pivot) = (0..values.len() - 1)
        .rev()
        .find(|index| values[*index] < values[*index + 1])
    else {
        return false;
    };
    let successor = (pivot + 1..values.len())
        .rev()
        .find(|index| values[*index] > values[pivot])
        .unwrap();
    values.swap(pivot, successor);
    values[pivot + 1..].reverse();
    true
}

/// A detection with nothing to point at is not a finding. The guard sits in
/// the emitter rather than in each detector, so a new detector cannot forget
/// it and publish a zero-width span the report would render on no character.
#[test]
fn an_empty_detection_range_emits_nothing() {
    let emitter = Emitter {
        package: None,
        target: None,
        path: "src/lib.rs",
        line_starts: line_starts("fn main() {}"),
        source: "fn main() {}",
    };
    let mut candidates = BTreeMap::new();
    emitter.insert(
        &mut candidates,
        &SILENT_RULE,
        &Detection {
            message: "unreachable",
            range: TextRange::empty(0.into()),
        },
    );
    assert!(candidates.is_empty());
}
