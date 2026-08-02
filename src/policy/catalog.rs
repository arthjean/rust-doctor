#[cfg(test)]
use std::collections::BTreeSet;

use serde::Serialize;

use super::RuleLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Producer {
    Clippy,
    CargoHealth,
    SourceKernel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct RuleDefinition {
    pub(crate) id: &'static str,
    pub(crate) category: &'static str,
    pub(crate) producer: Producer,
    pub(crate) default_level: RuleLevel,
    pub(crate) help: &'static str,
}

pub(crate) const CATEGORIES: [&str; 4] =
    ["correctness", "maintainability", "reliability", "security"];

pub(crate) static CLIPPY_DBG_MACRO: RuleDefinition = RuleDefinition {
    id: "clippy::dbg_macro",
    category: "maintainability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    help: "Remove dbg! or replace it with intentional logging.",
};
pub(crate) static CLIPPY_TODO: RuleDefinition = RuleDefinition {
    id: "clippy::todo",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    help: "Replace todo! with the intended implementation or remove the reachable placeholder.",
};
pub(crate) static CLIPPY_UNIMPLEMENTED: RuleDefinition = RuleDefinition {
    id: "clippy::unimplemented",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    help: "Implement this code path or remove the reachable placeholder.",
};
pub(crate) static CARGO_UNBOUNDED_REGISTRY: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::unbounded_registry_dependency",
    category: "reliability",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    help: "Replace the unbounded version requirement with the minimum compatible version intended by the project.",
};
pub(crate) static CARGO_UNPINNED_GIT: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::unpinned_git_dependency",
    category: "security",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    help: "Set rev to the full 40-character commit SHA intended by the project.",
};
pub(crate) static SOURCE_DISABLED_TLS: RuleDefinition = RuleDefinition {
    id: "rust_doctor::source::disabled_tls_verification",
    category: "security",
    producer: Producer::SourceKernel,
    default_level: RuleLevel::Warn,
    help: "Keep TLS verification enabled and configure the required trust roots or server name instead.",
};
pub(crate) static SOURCE_DYNAMIC_SHELL: RuleDefinition = RuleDefinition {
    id: "rust_doctor::source::dynamic_shell_command",
    category: "security",
    producer: Producer::SourceKernel,
    default_level: RuleLevel::Warn,
    help: "Avoid the shell and pass values as separate Command arguments; otherwise apply shell-specific escaping at the trust boundary.",
};

pub(crate) const CATALOG: [&RuleDefinition; 7] = [
    &CLIPPY_DBG_MACRO,
    &CLIPPY_TODO,
    &CLIPPY_UNIMPLEMENTED,
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::configuration::WorkspaceConfiguration;
    use crate::policy::{PolicyInput, PolicyPlan, RuleLevelSource, active_rules_in, compile_rules};

    static SYNTHETIC_CLIPPY_RULE: RuleDefinition = RuleDefinition {
        id: "clippy::synthetic_rule",
        category: "correctness",
        producer: Producer::Clippy,
        default_level: RuleLevel::Warn,
        help: "Synthetic authoring proof.",
    };

    const SYNTHETIC_CATALOG: [&RuleDefinition; 8] = [
        &CLIPPY_DBG_MACRO,
        &SYNTHETIC_CLIPPY_RULE,
        &CLIPPY_TODO,
        &CLIPPY_UNIMPLEMENTED,
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
        assert_eq!(CATALOG.len(), 7);
        assert_eq!(
            CATEGORIES,
            ["correctness", "maintainability", "reliability", "security"]
        );

        assert_eq!(
            serde_json::to_value(CATALOG).expect("catalog should serialize"),
            historical_oracle()["catalog"]
        );
        assert_eq!(
            CATALOG
                .iter()
                .filter(|definition| definition.producer == Producer::Clippy)
                .map(|definition| definition.id)
                .collect::<Vec<_>>(),
            ["clippy::dbg_macro", "clippy::todo", "clippy::unimplemented",]
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
        assert_eq!(plan.active_rules(Producer::Clippy).count(), 3);
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

    #[test]
    fn synthetic_eighth_clippy_rule_crosses_catalog_lookup_policy_and_arguments() {
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
                "clippy::synthetic_rule",
            ]
        );
    }
}
