use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::validate::{CatalogError, published_tier, validate_catalog};
use super::*;
use crate::configuration::WorkspaceConfiguration;
use crate::policy::{
    PolicyInput, PolicyPlan, RuleLevelSource, ValidatedPolicy, active_rules_in, compile_rules,
};

static SYNTHETIC_CLIPPY_RULE: RuleDefinition = RuleDefinition {
    id: "clippy::synthetic_rule",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Synthetic authoring proof.",
};

/// The shipped catalog with one more rule in it, built rather than retyped.
///
/// A second hand-written list is a second list to keep sorted, and one the
/// catalog can outgrow in silence: a rule admitted here and forgotten there
/// left every assertion below passing over a catalog that no longer shipped.
fn synthetic_catalog() -> [&'static RuleDefinition; CATALOG.len() + 1] {
    let mut rules = CATALOG.to_vec();
    rules.push(&SYNTHETIC_CLIPPY_RULE);
    rules.sort_by_key(|definition| definition.id);
    rules
        .try_into()
        .expect("the synthetic catalog is the shipped one plus a single rule")
}

fn historical_oracle() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/policy-gate/oracle.json"))
        .expect("policy oracle should be valid JSON")
}

/// Identifiers published by the historical oracle, in catalog order. The
/// catalog grows, the oracle does not move: the comparison therefore bears
/// on this frozen subset, not on a filter by exclusion.
const HISTORICAL_IDS: [&str; 7] = [
    "clippy::dbg_macro",
    "clippy::todo",
    "clippy::unimplemented",
    "rust_doctor::cargo::unbounded_registry_dependency",
    "rust_doctor::cargo::unpinned_git_dependency",
    "rust_doctor::source::disabled_tls_verification",
    "rust_doctor::source::dynamic_shell_command",
];

#[test]
fn catalog_is_the_exact_normative_inventory() {
    validate_catalog(&CATALOG).expect("canonical catalog should be valid");
    assert_eq!(CATALOG.len(), 62);
    assert_eq!(
        CATEGORIES,
        [
            "correctness",
            "dependencies",
            "maintainability",
            "performance",
            "reliability",
            "security",
        ]
    );

    // The oracle is compared against what the tool publishes, `CatalogEntry`,
    // rather than against the definition it projects from. A field added to
    // the published shape is what would change `rules list --json` and the
    // website that generates from it, so that is the shape the frozen record
    // has to hold.
    let historical: Vec<_> = catalog()
        .into_iter()
        .filter(|entry| HISTORICAL_IDS.contains(&entry.id))
        .collect();
    assert_eq!(historical.len(), HISTORICAL_IDS.len());
    assert_eq!(
        serde_json::to_value(historical).expect("historical catalog should serialize"),
        historical_oracle()["catalog"]
    );

    let clippy_ids: Vec<_> = CATALOG
        .iter()
        .filter(|definition| definition.producer == Producer::Clippy)
        .map(|definition| definition.id)
        .collect();
    assert_eq!(clippy_ids.len(), 37);
    assert!(
        clippy_ids
            .iter()
            .all(|id| id
                .strip_prefix("clippy::")
                .is_some_and(|lint| !lint.is_empty()
                    && lint
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte == b'_')))
    );

    // Every score dimension owns at least three rules, without which it
    // would stay frozen at 100 and its weight would be inert.
    let mut per_dimension = BTreeMap::new();
    for definition in CATALOG {
        let (_, dimension) = crate::audit::category_mapping(definition.category)
            .expect("every catalogued category should map to a dimension");
        *per_dimension.entry(dimension).or_insert(0_usize) += 1;
    }
    assert_eq!(per_dimension.len(), 5);
    assert!(
        per_dimension.values().all(|count| *count >= 3),
        "{per_dimension:?}"
    );
}

#[test]
fn exact_lookup_and_producer_subsets_use_canonical_definitions() {
    assert!(find("clippy::to").is_none());
    assert!(find("clippy::todo_suffix").is_none());
    assert!(find("todo").is_none());
    for definition in CATALOG {
        assert!(std::ptr::eq(find(definition.id).unwrap(), definition));
    }

    let plan = PolicyPlan::default();
    assert_eq!(plan.active_rules(Producer::Clippy).count(), 37);
    assert_eq!(plan.active_rules(Producer::CargoHealth).count(), 11);
    assert_eq!(plan.active_rules(Producer::SourceKernel).count(), 2);
    assert_eq!(plan.active_rules(Producer::Structure).count(), 9);
    assert_eq!(plan.active_rules(Producer::Repo).count(), 3);
}

#[test]
fn malformed_synthetic_catalogs_fail_deterministically() {
    let mut duplicate = CATALOG;
    duplicate[1] = duplicate[0];
    assert_eq!(validate_catalog(&duplicate), Err(CatalogError::DuplicateId));

    let mut unsorted = CATALOG;
    unsorted.swap(0, 1);
    assert_eq!(validate_catalog(&unsorted), Err(CatalogError::Unsorted));

    static EMPTY_HELP: RuleDefinition = RuleDefinition {
        help: "",
        // Derived from the first catalog entry: the substitution happens at
        // position 0, so the identifier must stay that one for a single
        // defect at a time to be under test.
        ..CLIPPY_ARC_WITH_NON_SEND_SYNC
    };
    let mut empty = CATALOG;
    empty[0] = &EMPTY_HELP;
    assert_eq!(validate_catalog(&empty), Err(CatalogError::EmptyMetadata));

    static UNKNOWN_CATEGORY: RuleDefinition = RuleDefinition {
        category: "style",
        // Derived from the first catalog entry: the substitution happens at
        // position 0, so the identifier must stay that one for a single
        // defect at a time to be under test.
        ..CLIPPY_ARC_WITH_NON_SEND_SYNC
    };
    let mut category = CATALOG;
    category[0] = &UNKNOWN_CATEGORY;
    assert_eq!(
        validate_catalog(&category),
        Err(CatalogError::UnknownCategory)
    );

    static TIER_TOO_MILD: RuleDefinition = RuleDefinition {
        tier: RuleTier::P3,
        // Derived from the first catalog entry: the substitution happens at
        // position 0, so the identifier must stay that one for a single
        // defect at a time to be under test.
        ..CLIPPY_ARC_WITH_NON_SEND_SYNC
    };
    let mut mild = CATALOG;
    mild[0] = &TIER_TOO_MILD;
    assert_eq!(
        validate_catalog(&mild),
        Err(CatalogError::TierOutsideCategoryWindow)
    );

    static TIER_TOO_SEVERE: RuleDefinition = RuleDefinition {
        tier: RuleTier::P0,
        ..CLIPPY_ARC_WITH_NON_SEND_SYNC
    };
    let mut severe = CATALOG;
    severe[0] = &TIER_TOO_SEVERE;
    assert_eq!(
        validate_catalog(&severe),
        Err(CatalogError::TierOutsideCategoryWindow)
    );

    static INVALID_PRODUCER: RuleDefinition = RuleDefinition {
        producer: Producer::SourceKernel,
        // Derived from the first catalog entry: the substitution happens at
        // position 0, so the identifier must stay that one for a single
        // defect at a time to be under test.
        ..CLIPPY_ARC_WITH_NON_SEND_SYNC
    };
    let mut producer = CATALOG;
    producer[0] = &INVALID_PRODUCER;
    assert_eq!(
        validate_catalog(&producer),
        Err(CatalogError::InvalidProducer)
    );

    static HOSTILE_DEFINITION: RuleDefinition = RuleDefinition {
        id: "/private/secret\u{1b}[31m",
        // Derived from the first catalog entry: the substitution happens at
        // position 0, so the identifier must stay that one for a single
        // defect at a time to be under test.
        ..CLIPPY_ARC_WITH_NON_SEND_SYNC
    };
    let error = validate_catalog(&[&HOSTILE_DEFINITION]).unwrap_err();
    let rendered = format!("{error:?}");
    assert_eq!(error, CatalogError::InvalidProducer);
    assert!(!rendered.contains("/private"));
    assert!(!rendered.contains('\u{1b}'));
}

/// The tier is a closed value on the published surface.
///
/// In the code, the enumeration makes a missing tier or one outside the
/// four values impossible to build. In the report, the tier is a string:
/// that is the only surface where the invalid state exists, so that is the
/// one the validation closes.
#[test]
fn an_absent_or_unknown_published_tier_fails_with_a_closed_error() {
    for definition in CATALOG {
        let published = serde_json::to_value(definition).unwrap();
        assert_eq!(published_tier(&published), Ok(definition.tier));
    }

    for hostile in [
        json!({ "id": "clippy::dbg_macro" }),
        json!({ "id": "clippy::dbg_macro", "tier": null }),
        json!({ "id": "clippy::dbg_macro", "tier": "P4" }),
        json!({ "id": "clippy::dbg_macro", "tier": "p0" }),
        json!({ "id": "clippy::dbg_macro", "tier": 0 }),
        json!({ "id": "clippy::dbg_macro", "tier": "/private/secret\u{1b}[31m" }),
    ] {
        let error = published_tier(&hostile).unwrap_err();
        let rendered = format!("{error:?}");
        assert_eq!(error, CatalogError::InvalidTier);
        assert!(!rendered.contains("/private"));
        assert!(!rendered.contains('\u{1b}'));
    }

    assert_eq!(
        RuleTier::ALL.map(RuleTier::as_str),
        ["P0", "P1", "P2", "P3"]
    );
    assert!(RuleTier::ALL.windows(2).all(|pair| pair[0] < pair[1]));
}

/// The tier neither copies the default level nor replaces it.
///
/// The twelve rules share `RuleLevel::Warn` but cover four tiers, so no
/// score function can deduce one from the other.
#[test]
fn the_tier_is_independent_from_the_default_level() {
    assert!(
        CATALOG
            .iter()
            .all(|definition| definition.default_level == RuleLevel::Warn)
    );
    let tiers: BTreeSet<_> = CATALOG.iter().map(|definition| definition.tier).collect();
    assert_eq!(tiers.len(), 4);

    let blocking: Vec<_> = CATALOG
        .iter()
        .filter(|definition| definition.tier == RuleTier::P0)
        .map(|definition| definition.id)
        .collect();
    assert_eq!(
        blocking,
        [
            "rust_doctor::source::disabled_tls_verification",
            "rust_doctor::source::dynamic_shell_command",
        ],
        "the P0 set stays restricted to exploitable security detectors",
    );
}

#[test]
fn a_synthetic_clippy_rule_crosses_catalog_lookup_policy_and_arguments() {
    let catalog = synthetic_catalog();
    validate_catalog(&catalog).expect("synthetic catalog should be valid");
    assert!(std::ptr::eq(
        find_in(&catalog, SYNTHETIC_CLIPPY_RULE.id).unwrap(),
        &SYNTHETIC_CLIPPY_RULE
    ));

    let input = PolicyInput::default()
        .with_rule(SYNTHETIC_CLIPPY_RULE.id, RuleLevel::Error)
        .with_category("correctness", RuleLevel::Off);
    let policy = ValidatedPolicy::of(&input, &catalog).expect("synthetic policy should validate");
    let rules = compile_rules(&catalog, &policy, &WorkspaceConfiguration::default());
    let synthetic = rules
        .iter()
        .find(|rule| rule.definition.id == SYNTHETIC_CLIPPY_RULE.id)
        .unwrap();
    assert_eq!(synthetic.level, RuleLevel::Error);
    assert_eq!(synthetic.source, RuleLevelSource::RequestRule);
    assert!(synthetic.restamped());

    // The command is a fixed head, then one `-W` per active Clippy rule in
    // catalog order. Freezing the whole list would be one more counter to edit
    // on every admitted lint, and it would prove less than the three things
    // that are actually the contract: the head, the pairing and the order.
    const HEAD: [&str; 7] = [
        "clippy",
        "--workspace",
        "--no-deps",
        "--message-format=json",
        "--",
        "-A",
        "clippy::all",
    ];
    let arguments =
        crate::execution::clippy_arguments_for_rules(active_rules_in(&rules, Producer::Clippy));
    let (head, warned) = arguments.split_at(HEAD.len());
    assert_eq!(head, HEAD);

    let active: Vec<_> = rules
        .iter()
        .filter(|rule| rule.definition.producer == Producer::Clippy && rule.level.is_active())
        .map(|rule| rule.definition.id)
        .collect();
    let expected: Vec<_> = active.iter().flat_map(|id| ["-W", id]).collect();
    assert_eq!(warned, expected);
    assert!(active.contains(&SYNTHETIC_CLIPPY_RULE.id));
    assert!(
        !active.contains(&CLIPPY_ARC_WITH_NON_SEND_SYNC.id),
        "the opened category stays off, the rule overridden inside it does not"
    );
}

/// US-072: a category override bears on every rule of the opened category,
/// `performance` like the others.
#[test]
fn a_category_override_reaches_every_rule_of_an_opened_category() {
    for category in ["performance", "dependencies"] {
        let input = PolicyInput::default().with_category(category, RuleLevel::Off);
        let policy = input.validate().expect("an opened category should validate");
        let rules = compile_rules(&CATALOG, &policy, &WorkspaceConfiguration::default());
        let targeted: Vec<_> = rules
            .iter()
            .filter(|rule| rule.definition.category == category)
            .collect();

        assert!(targeted.len() >= 3, "{category}");
        assert!(
            targeted.iter().all(|rule| rule.level == RuleLevel::Off
                && rule.source == RuleLevelSource::RequestCategory),
            "{category}"
        );
        assert!(
            rules
                .iter()
                .filter(|rule| rule.definition.category != category)
                .all(|rule| rule.level == RuleLevel::Warn),
            "{category}"
        );
    }
}

/// The policy passes the rule the catalog publishes. `oversized_unit` reports a
/// file at a thousand lines and a module block at five hundred, and every file
/// of this module holds both bounds: the tests and the admissibility check have
/// files of their own for that reason, and a file that grows back past the
/// bound fails here rather than on a self-scan nobody reads.
#[test]
fn the_policy_holds_the_size_bound_the_catalog_publishes_for() {
    for own in [
        include_str!("../../policy.rs"),
        include_str!("../catalog.rs"),
        include_str!("../coverage.rs"),
        include_str!("../noise.rs"),
        include_str!("tests.rs"),
        include_str!("validate.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the policy is {lines} lines long, over the {} it publishes",
            crate::structure::FILE_LINES
        );
    }
}
