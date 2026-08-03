#[cfg(test)]
use std::collections::BTreeSet;

use serde::Serialize;
#[cfg(test)]
use serde_json::Value;

use super::RuleLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Producer {
    Clippy,
    CargoHealth,
    SourceKernel,
}

/// Criticité d'une règle, indépendante de `default_level` et de la sévérité
/// effective d'un diagnostic.
///
/// Le tier ne pilote que le score `core-v2`: il impose un plafond à la
/// dimension concernée et à la note globale. Il n'entre ni dans `base_severity`
/// ni dans `fingerprint()`, donc il ne déplace aucune baseline.
///
/// L'ordre déclaré va du plus grave au moins grave: `P0 < P1 < P2 < P3`, donc
/// le pire tier d'un ensemble est son minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum RuleTier {
    P0,
    P1,
    P2,
    P3,
}

impl RuleTier {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 4] = [Self::P0, Self::P1, Self::P2, Self::P3];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
        }
    }

    /// Lecture fermée d'un tier publié. Toute autre valeur est refusée sans
    /// écho de l'entrée.
    #[cfg(test)]
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "P0" => Some(Self::P0),
            "P1" => Some(Self::P1),
            "P2" => Some(Self::P2),
            "P3" => Some(Self::P3),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct RuleDefinition {
    pub(crate) id: &'static str,
    pub(crate) category: &'static str,
    pub(crate) producer: Producer,
    pub(crate) default_level: RuleLevel,
    pub(crate) tier: RuleTier,
    pub(crate) help: &'static str,
}

pub(crate) const CATEGORIES: [&str; 4] =
    ["correctness", "maintainability", "reliability", "security"];

pub(crate) static CLIPPY_DBG_MACRO: RuleDefinition = RuleDefinition {
    id: "clippy::dbg_macro",
    category: "maintainability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Remove dbg! or replace it with intentional logging.",
};
pub(crate) static CLIPPY_MEM_FORGET: RuleDefinition = RuleDefinition {
    id: "clippy::mem_forget",
    category: "reliability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Avoid leaking a value with drop semantics; use an explicit ownership or lifetime strategy.",
};
pub(crate) static CLIPPY_NON_SEND_FIELDS_IN_SEND_TY: RuleDefinition = RuleDefinition {
    id: "clippy::non_send_fields_in_send_ty",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the unsafe Send implementation or ensure every field is safe to send between threads.",
};
pub(crate) static CLIPPY_PERMISSIONS_SET_READONLY_FALSE: RuleDefinition = RuleDefinition {
    id: "clippy::permissions_set_readonly_false",
    category: "security",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Set explicit Unix permission bits instead of clearing readonly on Unix.",
};
pub(crate) static CLIPPY_SUSPICIOUS_COMMAND_ARG_SPACE: RuleDefinition = RuleDefinition {
    id: "clippy::suspicious_command_arg_space",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Pass each process argument separately instead of embedding spaces in one argument.",
};
pub(crate) static CLIPPY_TODO: RuleDefinition = RuleDefinition {
    id: "clippy::todo",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Replace todo! with the intended implementation or remove the reachable placeholder.",
};
pub(crate) static CLIPPY_UNIMPLEMENTED: RuleDefinition = RuleDefinition {
    id: "clippy::unimplemented",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Implement this code path or remove the reachable placeholder.",
};
pub(crate) static CLIPPY_ZOMBIE_PROCESSES: RuleDefinition = RuleDefinition {
    id: "clippy::zombie_processes",
    category: "reliability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Wait on the child process or otherwise reap it before the handle is dropped.",
};
pub(crate) static CARGO_UNBOUNDED_REGISTRY: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::unbounded_registry_dependency",
    category: "reliability",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Replace the unbounded version requirement with the minimum compatible version intended by the project.",
};
pub(crate) static CARGO_UNPINNED_GIT: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::unpinned_git_dependency",
    category: "security",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Set rev to the full 40-character commit SHA intended by the project.",
};
pub(crate) static SOURCE_DISABLED_TLS: RuleDefinition = RuleDefinition {
    id: "rust_doctor::source::disabled_tls_verification",
    category: "security",
    producer: Producer::SourceKernel,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P0,
    help: "Keep TLS verification enabled and configure the required trust roots or server name instead.",
};
pub(crate) static SOURCE_DYNAMIC_SHELL: RuleDefinition = RuleDefinition {
    id: "rust_doctor::source::dynamic_shell_command",
    category: "security",
    producer: Producer::SourceKernel,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P0,
    help: "Avoid the shell and pass values as separate Command arguments; otherwise apply shell-specific escaping at the trust boundary.",
};

pub(crate) const CATALOG: [&RuleDefinition; 12] = [
    &CLIPPY_DBG_MACRO,
    &CLIPPY_MEM_FORGET,
    &CLIPPY_NON_SEND_FIELDS_IN_SEND_TY,
    &CLIPPY_PERMISSIONS_SET_READONLY_FALSE,
    &CLIPPY_SUSPICIOUS_COMMAND_ARG_SPACE,
    &CLIPPY_TODO,
    &CLIPPY_UNIMPLEMENTED,
    &CLIPPY_ZOMBIE_PROCESSES,
    &CARGO_UNBOUNDED_REGISTRY,
    &CARGO_UNPINNED_GIT,
    &SOURCE_DISABLED_TLS,
    &SOURCE_DYNAMIC_SHELL,
];

pub(crate) fn find(id: &str) -> Option<&'static RuleDefinition> {
    find_in(&CATALOG, id)
}

pub(super) fn find_in<'a>(catalog: &'a [&RuleDefinition], id: &str) -> Option<&'a RuleDefinition> {
    catalog
        .binary_search_by_key(&id, |definition| definition.id)
        .ok()
        .map(|index| catalog[index])
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatalogError {
    DuplicateId,
    Unsorted,
    EmptyMetadata,
    UnknownCategory,
    InvalidProducer,
    InvalidTier,
}

#[cfg(test)]
fn validate_catalog(catalog: &[&RuleDefinition]) -> Result<(), CatalogError> {
    let mut ids = BTreeSet::new();
    if catalog.iter().any(|definition| !ids.insert(definition.id)) {
        return Err(CatalogError::DuplicateId);
    }
    if catalog.windows(2).any(|pair| pair[0].id > pair[1].id) {
        return Err(CatalogError::Unsorted);
    }
    if catalog.iter().any(|definition| {
        definition.id.is_empty() || definition.category.is_empty() || definition.help.is_empty()
    }) {
        return Err(CatalogError::EmptyMetadata);
    }
    if catalog
        .iter()
        .any(|definition| CATEGORIES.binary_search(&definition.category).is_err())
    {
        return Err(CatalogError::UnknownCategory);
    }
    if catalog.iter().any(|definition| !match definition.producer {
        Producer::Clippy => definition.id.starts_with("clippy::"),
        Producer::CargoHealth => definition.id.starts_with("rust_doctor::cargo::"),
        Producer::SourceKernel => definition.id.starts_with("rust_doctor::source::"),
    }) {
        return Err(CatalogError::InvalidProducer);
    }
    for definition in catalog {
        let published = serde_json::to_value(definition).map_err(|_| CatalogError::InvalidTier)?;
        if published_tier(&published)? != definition.tier {
            return Err(CatalogError::InvalidTier);
        }
    }
    Ok(())
}

/// Le type interdit déjà un tier absent ou hors des quatre valeurs. La forme
/// publiée, elle, est une chaîne: c'est là que l'état invalide redevient
/// représentable, donc c'est là que la validation porte. L'erreur est fermée et
/// n'échoit ni la valeur lue, ni un chemin, ni une séquence d'échappement.
#[cfg(test)]
fn published_tier(definition: &Value) -> Result<RuleTier, CatalogError> {
    definition
        .get("tier")
        .and_then(Value::as_str)
        .and_then(RuleTier::parse)
        .ok_or(CatalogError::InvalidTier)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::configuration::WorkspaceConfiguration;
    use crate::policy::{PolicyInput, PolicyPlan, RuleLevelSource, active_rules_in, compile_rules};

    static SYNTHETIC_CLIPPY_RULE: RuleDefinition = RuleDefinition {
        id: "clippy::synthetic_rule",
        category: "correctness",
        producer: Producer::Clippy,
        default_level: RuleLevel::Warn,
        tier: RuleTier::P2,
        help: "Synthetic authoring proof.",
    };

    const SYNTHETIC_CATALOG: [&RuleDefinition; 13] = [
        &CLIPPY_DBG_MACRO,
        &CLIPPY_MEM_FORGET,
        &CLIPPY_NON_SEND_FIELDS_IN_SEND_TY,
        &CLIPPY_PERMISSIONS_SET_READONLY_FALSE,
        &CLIPPY_SUSPICIOUS_COMMAND_ARG_SPACE,
        &SYNTHETIC_CLIPPY_RULE,
        &CLIPPY_TODO,
        &CLIPPY_UNIMPLEMENTED,
        &CLIPPY_ZOMBIE_PROCESSES,
        &CARGO_UNBOUNDED_REGISTRY,
        &CARGO_UNPINNED_GIT,
        &SOURCE_DISABLED_TLS,
        &SOURCE_DYNAMIC_SHELL,
    ];

    fn historical_oracle() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/policy-gate/oracle.json"))
            .expect("policy oracle should be valid JSON")
    }

    #[test]
    fn catalog_is_the_exact_normative_inventory() {
        validate_catalog(&CATALOG).expect("canonical catalog should be valid");
        assert_eq!(CATALOG.len(), 12);
        assert_eq!(
            CATEGORIES,
            ["correctness", "maintainability", "reliability", "security"]
        );

        let historical: Vec<_> = CATALOG
            .iter()
            .filter(|definition| {
                !matches!(
                    definition.id,
                    "clippy::mem_forget"
                        | "clippy::non_send_fields_in_send_ty"
                        | "clippy::permissions_set_readonly_false"
                        | "clippy::suspicious_command_arg_space"
                        | "clippy::zombie_processes"
                )
            })
            .copied()
            .collect();
        assert_eq!(
            serde_json::to_value(historical).expect("historical catalog should serialize"),
            historical_oracle()["catalog"]
        );
        assert_eq!(
            CATALOG
                .iter()
                .filter(|definition| definition.producer == Producer::Clippy)
                .map(|definition| definition.id)
                .collect::<Vec<_>>(),
            [
                "clippy::dbg_macro",
                "clippy::mem_forget",
                "clippy::non_send_fields_in_send_ty",
                "clippy::permissions_set_readonly_false",
                "clippy::suspicious_command_arg_space",
                "clippy::todo",
                "clippy::unimplemented",
                "clippy::zombie_processes",
            ]
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
        assert_eq!(plan.active_rules(Producer::Clippy).count(), 8);
        assert_eq!(plan.active_rules(Producer::CargoHealth).count(), 2);
        assert_eq!(plan.active_rules(Producer::SourceKernel).count(), 2);
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
            ..CLIPPY_DBG_MACRO
        };
        let mut empty = CATALOG;
        empty[0] = &EMPTY_HELP;
        assert_eq!(validate_catalog(&empty), Err(CatalogError::EmptyMetadata));

        static UNKNOWN_CATEGORY: RuleDefinition = RuleDefinition {
            category: "style",
            ..CLIPPY_DBG_MACRO
        };
        let mut category = CATALOG;
        category[0] = &UNKNOWN_CATEGORY;
        assert_eq!(
            validate_catalog(&category),
            Err(CatalogError::UnknownCategory)
        );

        static INVALID_PRODUCER: RuleDefinition = RuleDefinition {
            producer: Producer::SourceKernel,
            ..CLIPPY_DBG_MACRO
        };
        let mut producer = CATALOG;
        producer[0] = &INVALID_PRODUCER;
        assert_eq!(
            validate_catalog(&producer),
            Err(CatalogError::InvalidProducer)
        );

        static HOSTILE_DEFINITION: RuleDefinition = RuleDefinition {
            id: "/private/secret\u{1b}[31m",
            ..CLIPPY_DBG_MACRO
        };
        let error = validate_catalog(&[&HOSTILE_DEFINITION]).unwrap_err();
        let rendered = format!("{error:?}");
        assert_eq!(error, CatalogError::InvalidProducer);
        assert!(!rendered.contains("/private"));
        assert!(!rendered.contains('\u{1b}'));
    }

    /// Le tier est une valeur fermée sur la surface publiée.
    ///
    /// Dans le code, l'énumération rend un tier absent ou hors des quatre
    /// valeurs impossible à construire. Dans le rapport, le tier est une
    /// chaîne: c'est la seule surface où l'état invalide existe, donc c'est
    /// celle que la validation ferme.
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

    /// Le tier ne recopie pas le niveau par défaut et ne le remplace pas.
    ///
    /// Les douze règles partagent `RuleLevel::Warn` mais couvrent quatre tiers,
    /// donc aucune fonction du score ne peut déduire l'un de l'autre.
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
            "l'ensemble P0 reste restreint aux détecteurs de sécurité exploitables",
        );
    }

    #[test]
    fn synthetic_thirteenth_clippy_rule_crosses_catalog_lookup_policy_and_arguments() {
        validate_catalog(&SYNTHETIC_CATALOG).expect("synthetic catalog should be valid");
        assert!(std::ptr::eq(
            find_in(&SYNTHETIC_CATALOG, SYNTHETIC_CLIPPY_RULE.id).unwrap(),
            &SYNTHETIC_CLIPPY_RULE
        ));

        let input = PolicyInput::default()
            .with_rule(SYNTHETIC_CLIPPY_RULE.id, RuleLevel::Error)
            .with_category("correctness", RuleLevel::Off);
        let rules = compile_rules(
            &SYNTHETIC_CATALOG,
            &input,
            &WorkspaceConfiguration::default(),
        )
        .expect("synthetic policy should compile");
        let synthetic = rules
            .iter()
            .find(|rule| rule.definition.id == SYNTHETIC_CLIPPY_RULE.id)
            .unwrap();
        assert_eq!(synthetic.level, RuleLevel::Error);
        assert_eq!(synthetic.source, RuleLevelSource::RequestRule);
        assert!(synthetic.restamp);
        assert_eq!(
            crate::execution::clippy_arguments_for_rules(active_rules_in(&rules, Producer::Clippy)),
            [
                "clippy",
                "--workspace",
                "--all-targets",
                "--no-deps",
                "--message-format=json",
                "--",
                "-W",
                "clippy::dbg_macro",
                "-W",
                "clippy::mem_forget",
                "-W",
                "clippy::permissions_set_readonly_false",
                "-W",
                "clippy::synthetic_rule",
                "-W",
                "clippy::zombie_processes",
            ]
        );
    }
}
