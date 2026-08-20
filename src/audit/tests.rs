//! Tests for the audit block.
//!
//! They live beside the module rather than inside it so that every file of the module stays
//! under the thousand lines `oversized_unit` reports, which is what
//! `the_audit_holds_the_size_bound_it_scores_for` keeps true.

use serde::Deserialize;
use serde_json::Value;

use super::*;
use crate::policy::{CATALOG, Producer};
use crate::report::{DiagnosticSource, DiagnosticSpan};

mod lambda_freeze;
mod scale;

/// The block passes the rule the score it computes ranks. `oversized_unit` reports a file at a
/// thousand lines, and this module holds that bound: these tests and the source inventory have
/// files of their own for that reason, and a file that grows back past it fails here rather than
/// on a self-scan nobody reads.
#[test]
fn the_audit_holds_the_size_bound_it_scores_for() {
    for own in [
        include_str!("../audit.rs"),
        include_str!("density.rs"),
        include_str!("source_inventory.rs"),
        include_str!("tests.rs"),
        include_str!("tests/lambda_freeze.rs"),
        include_str!("tests/scale.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the audit block is {lines} lines long, over the {} it reports",
            crate::structure::FILE_LINES
        );
    }
}

/// The categories are published in their declaration order, and nothing restates that order.
///
/// `Ord` derives from the declaration, the tally map is keyed by it and `Audit::is_valid` checks
/// it, so this is the one place the sequence itself is written down and frozen.
#[test]
fn the_categories_are_published_in_their_declaration_order() {
    let declared = [
        AuditCategoryName::Security,
        AuditCategoryName::Bugs,
        AuditCategoryName::Performance,
        AuditCategoryName::Dependencies,
        AuditCategoryName::Maintainability,
        AuditCategoryName::Other,
    ];
    let mut sorted = declared;
    sorted.sort();
    assert_eq!(sorted, declared);

    let tallies = category_tallies(&[
        diagnostic_with_category(Some("maintainability")),
        diagnostic_with_category(None),
        diagnostic_with_category(Some("security")),
    ]);
    assert_eq!(
        tallies
            .iter()
            .map(|tally| tally.name)
            .collect::<Vec<_>>(),
        [
            AuditCategoryName::Security,
            AuditCategoryName::Maintainability,
            AuditCategoryName::Other,
        ]
    );
}

/// `ScoreDimension::ALL` names every dimension exactly once.
///
/// The match is exhaustive on purpose: a dimension declared and forgotten in `ALL` stops this
/// file compiling, which no assertion over a length could catch.
#[test]
fn every_dimension_is_listed_once() {
    for dimension in ScoreDimension::ALL {
        match dimension {
            ScoreDimension::Security
            | ScoreDimension::Reliability
            | ScoreDimension::Maintainability
            | ScoreDimension::Performance
            | ScoreDimension::Dependencies => {}
        }
    }
    let distinct: BTreeSet<_> = ScoreDimension::ALL.into_iter().collect();
    assert_eq!(distinct.len(), ScoreDimension::ALL.len());
}

/// A rule that only ever fired outside production code is counted and shown, and costs nothing.
///
/// The two used to be separated by which population each caller aggregated over, so the cost the
/// report ranked by was not the cost the score charged.
#[test]
fn a_finding_outside_production_code_is_counted_and_charged_nothing() {
    let mut in_tests = diagnostic_with_category(Some("security"));
    in_tests.code = Some("clippy::todo".to_owned());
    in_tests.severity = Severity::Error;
    in_tests.occurrences = 40;
    in_tests.context = Some(crate::report::DiagnosticContext::Tests);

    let aggregation = aggregate_rules(100, [&in_tests]);
    let rule = aggregation
        .rules
        .first()
        .expect("the rule is aggregated like any other");
    assert_eq!(rule.occurrences, 40, "it stays counted");
    assert_eq!(rule.contribution(), 0, "and costs the score nothing");
    assert!(aggregation.diagnostics_are_authoritative);

    let audit = Audit::build(1, 100, Status::Complete, &[in_tests]);
    let score = audit.score.as_ref().expect("a scored workspace");
    assert_eq!(score.value, 100);
    assert!(score.projected_rule_ids.is_empty());
}

fn diagnostic_with_category(category: Option<&str>) -> Diagnostic {
    Diagnostic {
        context: None,
        id: format!("id-{}", category.unwrap_or("none")),
        source: DiagnosticSource::Clippy,
        code: Some(format!("clippy::{}", category.unwrap_or("bare"))),
        base_severity: Severity::Warning,
        severity: Severity::Warning,
        category: category.map(str::to_owned),
        message: "message".to_owned(),
        help: None,
        package: None,
        target: None,
        path: None,
        span: None,
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        occurrences: 1,
    }
}

/// The producer names the oracle spells, paired with the enum they name.
const PRODUCERS: [(&str, Producer); 5] = [
    ("cargo-health", Producer::CargoHealth),
    ("clippy", Producer::Clippy),
    ("repo", Producer::Repo),
    ("source-kernel", Producer::SourceKernel),
    ("structure", Producer::Structure),
];

/// The dimension names the oracle spells, paired with the enum they name.
const DIMENSIONS: [(&str, ScoreDimension); 5] = [
    ("dependencies", ScoreDimension::Dependencies),
    ("maintainability", ScoreDimension::Maintainability),
    ("performance", ScoreDimension::Performance),
    ("reliability", ScoreDimension::Reliability),
    ("security", ScoreDimension::Security),
];

#[derive(Debug, Deserialize)]
struct Oracle {
    schema_version: u8,
    model: String,
    uncategorized_bucket: String,
    category_mappings: BTreeMap<String, CategoryExpectation>,
    rule_tiers: BTreeMap<String, String>,
    tier_ceilings: Vec<TierCeilingExpectation>,
    denominators: Vec<DenominatorExpectation>,
    score_boundaries: Vec<ScoreBoundaryExpectation>,
    density_cases: Vec<DensityExpectation>,
    score_cases: Vec<ScoreCase>,
    share_cases: Vec<ShareCase>,
}

#[derive(Debug, Deserialize)]
struct TierCeilingExpectation {
    tier: String,
    dimension: Option<u8>,
    overall: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct DenominatorExpectation {
    producer: String,
    per_kiloline: bool,
}

#[derive(Debug, Deserialize)]
struct CategoryExpectation {
    display: String,
    dimension: String,
}

#[derive(Debug, Deserialize)]
struct ScoreBoundaryExpectation {
    name: String,
    dimensions: ScoreDimensions,
    expected_value: u8,
    expected_label: String,
    expected_counted_rule_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DensityExpectation {
    dimension: String,
    density: f64,
    score: u8,
}

#[derive(Debug, Deserialize)]
struct ScoreCase {
    name: String,
    source_files: usize,
    complete: bool,
    diagnostics: Vec<OracleDiagnostic>,
    expected_audit: Value,
    expected_counted_rule_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OracleDiagnostic {
    rule_id: Option<String>,
    category: Option<String>,
    severity: String,
    occurrences: usize,
}

#[derive(Debug, Deserialize)]
struct ShareCase {
    score: u8,
    errors: usize,
    warnings: usize,
    info: usize,
    files: usize,
    /// Production lines, absent from the two frozen oracles: they were taken
    /// before the count existed, and a count of zero is omitted from the
    /// query, so their expected URLs still hold. What the new key does is
    /// asserted by `the_share_url_carries_the_production_line_count`.
    #[serde(default)]
    lines: usize,
    expected: Option<String>,
}

/// Projects a current audit back onto the shape the version-2 oracle froze.
///
/// That record was taken before the score had a denominator to publish, so the
/// projection consists solely of removing the members added since. This is the
/// condition that makes a frozen archive durable, and the same one
/// `project_v11_wire_to_v7` holds for the report: a schema that adds projects,
/// a schema that moves the value of an existing field does not.
fn projected_onto_oracle(audit: &Audit) -> serde_json::Value {
    let mut value = serde_json::to_value(audit).expect("a valid audit should serialize");
    if let Some(members) = value.as_object_mut() {
        members.remove("production_lines");
    }
    value
}

fn oracle() -> Oracle {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/local-cli-experience/audit-core-v3.json"
    ))
    .expect("audit oracle should be valid")
}

fn diagnostic(input: &OracleDiagnostic, index: usize) -> Diagnostic {
    Diagnostic {
        context: None,
        id: format!("finding-{index}"),
        source: DiagnosticSource::Clippy,
        code: input.rule_id.clone(),
        base_severity: severity(&input.severity).expect("oracle severity should be known"),
        severity: severity(&input.severity).expect("oracle severity should be known"),
        category: input.category.clone(),
        message: format!("oracle finding {index}"),
        help: None,
        package: None,
        target: None,
        path: Some(format!("src/{index}.rs")),
        span: Some(DiagnosticSpan {
            line_start: 1,
            column_start: 1,
            line_end: 1,
            column_end: 2,
        }),
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        occurrences: input.occurrences,
    }
}

fn severity(value: &str) -> Option<Severity> {
    match value {
        "error" => Some(Severity::Error),
        "warning" => Some(Severity::Warning),
        "info" => Some(Severity::Info),
        "unknown" => Some(Severity::Unknown),
        _ => None,
    }
}

#[test]
fn versioned_oracle_covers_categories_labels_scores_and_rule_identity() {
    let oracle = oracle();
    assert_eq!(oracle.schema_version, 3);
    assert_eq!(oracle.model, SCORE_MODEL);
    for (category, expected) in oracle.category_mappings {
        let (display, dimension) = category_mapping(&category).expect("mapped category");
        assert_eq!(display.to_string(), expected.display, "{category}");
        assert_eq!(format!("{dimension:?}"), expected.dimension, "{category}");
        assert_eq!(category_bucket(Some(&category)), display, "{category}");
    }
    for unmapped in [None, Some("future"), Some(""), Some("style")] {
        assert_eq!(
            category_bucket(unmapped).to_string(),
            oracle.uncategorized_bucket
        );
    }

    let tiers: BTreeMap<_, _> = CATALOG
        .iter()
        .map(|definition| {
            (
                definition.id.to_owned(),
                definition.tier.as_str().to_owned(),
            )
        })
        .collect();
    assert_eq!(tiers, oracle.rule_tiers);

    let mut previous: Option<(u8, u8)> = None;
    for expected in oracle.tier_ceilings {
        let tier = RuleTier::parse(&expected.tier).expect("published tier should be known");
        assert_eq!(tier_dimension_ceiling(tier), expected.dimension, "{tier:?}");
        assert_eq!(tier_overall_ceiling(tier), expected.overall, "{tier:?}");
        let current = (
            expected.dimension.unwrap_or(100),
            expected.overall.unwrap_or(100),
        );
        if let Some(previous) = previous {
            assert!(
                previous.0 < current.0 && previous.1 <= current.1,
                "caps must decrease strictly with gravity: {previous:?} then {current:?}",
            );
        }
        previous = Some(current);
    }

    for expected in oracle.denominators {
        let producer = PRODUCERS
            .iter()
            .find(|(name, _)| *name == expected.producer)
            .map(|(_, producer)| *producer)
            .unwrap_or_else(|| {
                unreachable!(
                    "the oracle names a producer the catalog does not: {}",
                    expected.producer
                )
            });
        assert_eq!(
            DensityScope::of(producer) == DensityScope::PerKiloline,
            expected.per_kiloline,
            "{}",
            expected.producer
        );
    }
    for expected in oracle.score_boundaries {
        assert_eq!(
            weighted_score(expected.dimensions),
            expected.expected_value,
            "{}",
            expected.name
        );
        assert_eq!(
            score_label(expected.expected_value).to_string(),
            expected.expected_label,
            "{}",
            expected.name
        );
        assert!(
            expected.expected_counted_rule_ids.is_empty(),
            "dimension-only boundary cases must not invent Rule IDs: {}",
            expected.name
        );
    }
    for expected in oracle.density_cases {
        let dimension = DIMENSIONS
            .iter()
            .find(|(name, _)| *name == expected.dimension)
            .map(|(_, dimension)| *dimension)
            .unwrap_or_else(|| {
                unreachable!(
                    "the oracle names a dimension the score does not: {}",
                    expected.dimension
                )
            });
        assert_eq!(
            density::dimension_score(expected.density, dimension),
            expected.score,
            "{} at density {}",
            expected.dimension,
            expected.density
        );
        let dimensions = ScoreDimensions {
            security: expected.score,
            reliability: expected.score,
            maintainability: expected.score,
            performance: expected.score,
            dependencies: expected.score,
        };
        assert_eq!(weighted_score(dimensions), expected.score);
    }
    for case in oracle.score_cases {
        let diagnostics: Vec<_> = case
            .diagnostics
            .iter()
            .enumerate()
            .map(|(index, input)| diagnostic(input, index))
            .collect();
        let status = if case.complete {
            Status::Complete
        } else {
            Status::Incomplete
        };
        let audit = Audit::build(
            case.source_files,
            case.source_files * 100,
            status,
            &diagnostics,
        );
        assert_eq!(
            projected_onto_oracle(&audit),
            case.expected_audit,
            "{}",
            case.name
        );
        let input = aggregate_rules(case.source_files * 100, &diagnostics);
        let counted: Vec<_> = input
            .rules
            .into_iter()
            .filter(RuleAggregate::is_scorable)
            .map(|rule| rule.id)
            .collect();
        assert_eq!(counted, case.expected_counted_rule_ids, "{}", case.name);
        assert!(audit.is_valid(), "{}", case.name);
    }
}

#[test]
fn versioned_share_oracle_matches_public_query_bounds() {
    for case in oracle().share_cases {
        let actual = build_share_url(
            case.score,
            case.errors,
            case.warnings,
            case.info,
            case.files,
            case.lines,
        )
        .ok();
        assert_eq!(actual, case.expected);
    }
    assert_eq!(score_label(52), ScoreLabel::NeedsWork);
}

#[test]
fn invalid_score_state_is_rejected_before_sharing() {
    let audit = Audit {
        source_files: 1,
        production_lines: 100,
        categories: Vec::new(),
        inventory_is_complete: true,
        score: Some(AuditScore {
            model: SCORE_MODEL.to_owned(),
            value: 101,
            label: ScoreLabel::Great,
            authoritative: true,
            dimensions: ScoreDimensions {
                security: 100,
                reliability: 100,
                maintainability: 100,
                performance: 100,
                dependencies: 100,
            },
            worst_tier: None,
            applied_ceiling: None,
            projected_after_top_three: None,
            projected_rule_ids: Vec::new(),
            withheld_rule_ids: Vec::new(),
        }),
    };

    assert!(!audit.is_valid());
    assert_eq!(audit.share_url(), Err(ShareError::InvalidPayload));
    assert!(serde_json::to_vec(&audit).is_err());
    assert_eq!(
        build_share_url(100, 1_000_001, 0, 0, 1, 100),
        Err(ShareError::InvalidPayload)
    );
    // The line count is bounded like every other published count.
    assert_eq!(
        build_share_url(100, 0, 0, 0, 1, 1_000_001),
        Err(ShareError::InvalidPayload)
    );
}

/// The denominator the score is computed against travels with the score.
#[test]
fn the_share_url_carries_the_production_line_count() {
    let audit = Audit::build(3, 420, Status::Complete, &[]);

    assert_eq!(
        audit.share_url(),
        Ok("https://rust-doctor.com/share?s=100&f=3&l=420".to_owned())
    );
}

fn catalog_diagnostics(occurrences: usize) -> Vec<Diagnostic> {
    CATALOG
        .iter()
        .enumerate()
        .map(|(index, definition)| Diagnostic {
            context: None,
            id: format!("finding-{index}"),
            source: DiagnosticSource::Clippy,
            code: Some(definition.id.to_owned()),
            base_severity: Severity::Error,
            severity: Severity::Error,
            category: Some(definition.category.to_owned()),
            message: "worst case".to_owned(),
            help: None,
            package: None,
            target: None,
            path: None,
            span: None,
            related: Vec::new(),
            similarity_basis_points: None,
            complexity: None,
            occurrences,
        })
        .collect()
}

/// What the real catalog produces, as opposed to the injected dimensions
/// of the oracle.
///
/// Under `core-v1`, saturating the twelve rules left the score at 96,
/// label `Great`: the additive scale was structurally unable to go down.
/// Under `core-v2` the worst observed tier caps the score, so the same
/// catalog reaches the `Critical` band. Under `core-v3` the workspace has to be named for the
/// question to mean anything: sixty-two rules firing once is a catastrophe in a hundred lines
/// and an ordinary Tuesday in a hundred thousand, so this fires them across fifty kilolines,
/// which is a workspace the corpus would recognize.
#[test]
fn the_catalog_drives_the_score_out_of_its_top_label() {
    let diagnostics = catalog_diagnostics(1);
    let audit = Audit::build(500, 50_000, Status::Complete, &diagnostics);
    let score = audit.score.expect("a scored audit should exist");

    assert_eq!(score.value, 26);
    assert_eq!(score.label, ScoreLabel::Critical);
    assert_eq!(score.worst_tier, Some(RuleTier::P0));
    assert_eq!(score.applied_ceiling, Some(40));
    assert!(score.authoritative);

    // Two of the five are the density and three are not, which is the whole model in one score.
    // `security` and `dependencies` fall on their own: security reads a lambda of one, and the
    // eleven manifest rules are charged per workspace, so neither is diluted by fifty kilolines.
    // `performance` is its `P2` ceiling and `security` would be capped at twenty if the curve had
    // not already taken it below.
    assert_eq!(score.dimensions.security, 0);
    assert_eq!(score.dimensions.reliability, 34);
    assert_eq!(score.dimensions.maintainability, 42);
    // EP-024 opens `performance` and `dependencies`: no dimension stays
    // frozen at 100, so no weight of the scale is inert any more.
    assert_eq!(score.dimensions.performance, 75);
    assert_eq!(score.dimensions.dependencies, 4);
    assert!(
        score
            .dimensions
            .values()
            .into_iter()
            .all(|value| value < 100),
        "{:?}",
        score.dimensions
    );
}

/// One diagnostic per distinct site, which is what a core-v3 numerator counts.
///
/// The last member of a tuple is a number of sites and not a number of occurrences: the score
/// charges a rule for the places a reader has to go and look at, so a rule firing sixty times
/// is sixty diagnostics here, each in a file of its own.
fn diagnostics_for(rules: &[(&str, &str, Severity, usize)]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (rule, (code, category, severity, sites)) in rules.iter().enumerate() {
        for site in 0..*sites {
            let index = diagnostics.len();
            diagnostics.push(Diagnostic {
                context: None,
                id: format!("finding-{rule}-{site}"),
                source: DiagnosticSource::Clippy,
                code: Some((*code).to_owned()),
                base_severity: *severity,
                severity: *severity,
                category: Some((*category).to_owned()),
                message: format!("finding {index}"),
                help: None,
                package: None,
                target: None,
                path: Some(format!("src/{index}.rs")),
                span: None,
                related: Vec::new(),
                similarity_basis_points: None,
                complexity: None,
                occurrences: 1,
            });
        }
    }
    diagnostics
}

/// The rule that fires most is not the rule worth fixing first.
///
/// `clippy::indexing_slicing` is adjudicated at 10000 basis points on the
/// pinned corpus, forty reviewed sites and no true positive, while
/// `rust_doctor::cargo::duplicate_major_versions` sits at zero. Ranking by
/// contribution alone put the noisy rule first because volume is exactly
/// what it has the most of, which is advice to go and change correct code.
#[test]
fn a_rule_the_corpus_measured_wrong_yields_the_lead_to_a_quieter_one() {
    let score = scored(&[
        ("clippy::indexing_slicing", "reliability", Severity::Warning, 60),
        (
            "rust_doctor::cargo::duplicate_major_versions",
            "dependencies",
            Severity::Warning,
            2,
        ),
    ]);

    assert_eq!(
        score.projected_rule_ids,
        vec!["rust_doctor::cargo::duplicate_major_versions".to_owned()],
        "a rule measured at no true positive is left out rather than ranked last"
    );
}

/// Absence of a measurement is not evidence of noise.
#[test]
fn a_rule_the_corpus_never_adjudicated_keeps_its_full_rank() {
    let score = scored(&[
        ("clippy::indexing_slicing", "reliability", Severity::Warning, 60),
        ("rust_doctor::repo::tracked_secret_file", "security", Severity::Warning, 1),
    ]);

    assert_eq!(
        score.projected_rule_ids,
        vec!["rust_doctor::repo::tracked_secret_file".to_owned()],
        "an unmeasured rule is ranked on its contribution, undiscounted"
    );
}

/// A workspace whose every scoring rule is measured wrong has nothing worth
/// repairing, and the report says so by naming nothing.
#[test]
fn nothing_is_projected_when_every_scoring_rule_is_measured_wrong() {
    let score = scored(&[
        ("clippy::indexing_slicing", "reliability", Severity::Warning, 60),
        ("clippy::string_slice", "reliability", Severity::Warning, 12),
    ]);

    assert!(score.projected_rule_ids.is_empty());
    assert_eq!(score.projected_after_top_three, None);
    assert!(score.value < 100, "the findings still cost the score");
}

/// What the ranking dropped is named, in the order that makes the omission
/// legible: the loudest first, since that is the one a reader misses.
#[test]
fn the_rules_the_ranking_dropped_are_published_loudest_first() {
    let score = scored(&[
        ("clippy::string_slice", "reliability", Severity::Warning, 2),
        ("clippy::indexing_slicing", "reliability", Severity::Warning, 60),
        (
            "rust_doctor::cargo::duplicate_major_versions",
            "dependencies",
            Severity::Warning,
            2,
        ),
    ]);

    assert_eq!(
        score.withheld_rule_ids,
        vec![
            "clippy::indexing_slicing".to_owned(),
            "clippy::string_slice".to_owned()
        ]
    );
    assert!(
        !score
            .withheld_rule_ids
            .contains(&"rust_doctor::cargo::duplicate_major_versions".to_owned()),
        "a rule the corpus found right is never withheld"
    );
}

/// An incomplete scan still scores and still caps, and it names neither what to fix next nor
/// what the ranking withheld, because it ranked nothing.
///
/// The two inputs are the two ways of asking: a rule the corpus adjudicated wrong is the one
/// that would be withheld, and a `P0` rule is the one that would cap.
#[test]
fn an_incomplete_scan_caps_and_names_neither_a_projection_nor_a_withholding() {
    let noisy = scored_at(
        Status::Incomplete,
        &[(
            "clippy::indexing_slicing",
            "reliability",
            Severity::Warning,
            60,
        )],
    );
    let capped = scored_at(
        Status::Incomplete,
        &[(
            "rust_doctor::source::dynamic_shell_command",
            "security",
            Severity::Warning,
            1,
        )],
    );
    assert_eq!(capped.value, 40);
    assert_eq!(capped.applied_ceiling, Some(40));

    for score in [noisy, capped] {
        assert!(!score.authoritative);
        assert_eq!(score.projected_after_top_three, None);
        assert!(score.projected_rule_ids.is_empty());
        assert!(score.withheld_rule_ids.is_empty());
    }
}

/// The promise the report makes is the score with those three rules gone, not an estimate of it.
///
/// The projection is rescored through the same path the published value takes, ceilings
/// included, which is what this replays: score once, drop every diagnostic the three named rules
/// raised, score again, and the two numbers have to be the same. It is the one claim a reader
/// acts on, and the contribution the ranking orders by is deliberately computed without the
/// ceilings, so the two are separate facts and only this one is published.
#[test]
fn the_projection_is_the_score_the_named_repairs_actually_reach() {
    let profile = [
        ("clippy::dbg_macro", "maintainability", Severity::Warning, 9),
        ("clippy::todo", "correctness", Severity::Warning, 6),
        (
            "rust_doctor::cargo::duplicate_major_versions",
            "dependencies",
            Severity::Warning,
            3,
        ),
        ("clippy::string_slice", "reliability", Severity::Warning, 2),
    ];
    let diagnostics = diagnostics_for(&profile);
    let score = Audit::build(20, 20_000, Status::Complete, &diagnostics)
        .score
        .expect("twenty kilolines is a scorable workspace");

    assert!(score.authoritative);
    assert_eq!(score.projected_rule_ids.len(), 3);
    let named: BTreeSet<&str> = score
        .projected_rule_ids
        .iter()
        .map(String::as_str)
        .collect();

    let repaired: Vec<_> = diagnostics
        .into_iter()
        .filter(|diagnostic| {
            diagnostic
                .code
                .as_deref()
                .is_none_or(|code| !named.contains(code))
        })
        .collect();
    let after = Audit::build(20, 20_000, Status::Complete, &repaired)
        .score
        .expect("repairing findings never makes a workspace unscorable");

    assert_eq!(
        score.projected_after_top_three,
        Some(after.value),
        "the projection promised {:?} and the repair reaches {}",
        score.projected_after_top_three,
        after.value
    );
    assert!(after.value > score.value, "the three repairs are worth points");
}

fn scored(rules: &[(&str, &str, Severity, usize)]) -> AuditScore {
    scored_at(Status::Complete, rules)
}

fn scored_at(status: Status, rules: &[(&str, &str, Severity, usize)]) -> AuditScore {
    Audit::build(1, 100, status, &diagnostics_for(rules))
        .score
        .expect("a scored audit should exist")
}

/// A clean codebase takes no cap, and a cap is not invented out of a rule
/// outside the catalog.
#[test]
fn a_clean_codebase_scores_one_hundred_without_any_ceiling() {
    let clean = Audit::build(1, 100, Status::Complete, &[])
        .score
        .expect("a scored audit should exist");
    assert_eq!(clean.value, 100);
    assert_eq!(clean.worst_tier, None);
    assert_eq!(clean.applied_ceiling, None);

    let uncatalogued = scored(&[("clippy::unknown_rule", "security", Severity::Error, 1)]);
    assert_eq!(uncatalogued.worst_tier, None);
    assert_eq!(uncatalogued.applied_ceiling, None);
}

/// A tier only caps when it acts: a rule switched off by the policy carries
/// an unknown severity, hence neither penalty nor cap.
#[test]
fn a_disabled_rule_neither_penalizes_nor_caps() {
    let disabled = scored(&[(
        "rust_doctor::source::dynamic_shell_command",
        "security",
        Severity::Unknown,
        1,
    )]);
    assert_eq!(disabled.value, 100);
    assert_eq!(disabled.worst_tier, None);
    assert_eq!(disabled.applied_ceiling, None);
    assert_eq!(disabled.dimensions.security, 100);
}

/// The worst tier of a dimension overrides the others, and a graver tier in
/// another dimension still brings the overall score down.
#[test]
fn only_the_worst_tier_applies_per_dimension_and_overall() {
    let mixed = scored(&[
        ("clippy::todo", "correctness", Severity::Warning, 1),
        ("clippy::unimplemented", "correctness", Severity::Warning, 1),
        ("clippy::dbg_macro", "maintainability", Severity::Warning, 1),
        (
            "rust_doctor::source::disabled_tls_verification",
            "security",
            Severity::Warning,
            1,
        ),
    ]);

    assert_eq!(mixed.dimensions.reliability, 50, "P1 overrides P2");
    assert_eq!(mixed.dimensions.security, 20, "P0 caps its dimension");
    assert!(mixed.dimensions.maintainability > 75, "P3 does not cap");
    assert_eq!(mixed.worst_tier, Some(RuleTier::P0));
    assert_eq!(mixed.applied_ceiling, Some(40));
    assert_eq!(mixed.value, 40);
}

/// A diagnostic with no catalog category stays counted: both quantities of
/// the categories reconstitute the report population exactly.
#[test]
fn every_diagnostic_lands_in_exactly_one_bucket() {
    let mut diagnostics = diagnostics_for(&[
        ("clippy::todo", "correctness", Severity::Warning, 3),
        ("clippy::dbg_macro", "maintainability", Severity::Info, 1),
    ]);
    diagnostics.push(Diagnostic {
        context: None,
        id: "compiler".to_owned(),
        source: DiagnosticSource::Rustc,
        code: Some("E0433".to_owned()),
        base_severity: Severity::Error,
        severity: Severity::Error,
        category: None,
        message: "unresolved import".to_owned(),
        help: None,
        package: None,
        target: None,
        path: None,
        span: None,
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        occurrences: 2,
    });

    let audit = Audit::build(1, 100, Status::Incomplete, &diagnostics);
    let (distinct, occurrences) = audit.totals();

    assert_eq!(distinct.total, 5);
    assert_eq!(occurrences.total, 6);
    assert_eq!(distinct.errors, 1);
    assert_eq!(occurrences.errors, 2);
    assert_eq!(
        audit
            .categories
            .iter()
            .map(|category| category.name)
            .collect::<Vec<_>>(),
        [
            AuditCategoryName::Bugs,
            AuditCategoryName::Maintainability,
            AuditCategoryName::Other,
        ]
    );
    assert!(audit.is_valid());
}

#[test]
fn incomplete_source_inventory_never_emits_an_authoritative_score() {
    let audit = Audit::build_from_inventory(
        SourceFileInventory {
            files: 1,
            production_lines: 40,
            complete: false,
        },
        Status::Complete,
        &[],
    );

    assert_eq!(audit.score.as_ref().map(|score| score.value), Some(100));
    assert_eq!(
        audit.score.as_ref().map(|score| score.authoritative),
        Some(false)
    );
}


