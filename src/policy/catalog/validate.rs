use std::collections::BTreeSet;

use serde_json::Value;

use super::{CATEGORIES, Producer, RuleDefinition, RuleTier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CatalogError {
    DuplicateId,
    Unsorted,
    EmptyMetadata,
    UnknownCategory,
    InvalidProducer,
    InvalidTier,
    TierOutsideCategoryWindow,
    TitleTooLong,
    InvalidGate,
}

const TITLE_WORDS: usize = 8;

const TIER_WINDOWS: [(&str, RuleTier, RuleTier); CATEGORIES.len()] = [
    ("correctness", RuleTier::P1, RuleTier::P2),
    ("dependencies", RuleTier::P1, RuleTier::P2),
    ("maintainability", RuleTier::P3, RuleTier::P3),
    ("performance", RuleTier::P2, RuleTier::P3),
    ("reliability", RuleTier::P2, RuleTier::P3),
    ("security", RuleTier::P0, RuleTier::P1),
];

pub(super) fn validate_catalog(catalog: &[&RuleDefinition]) -> Result<(), CatalogError> {
    inventory(catalog)?;
    catalog
        .iter()
        .try_for_each(|definition| admissible(definition))
}

fn inventory(catalog: &[&RuleDefinition]) -> Result<(), CatalogError> {
    let mut ids = BTreeSet::new();
    if catalog.iter().any(|definition| !ids.insert(definition.id)) {
        return Err(CatalogError::DuplicateId);
    }
    if catalog.windows(2).any(|pair| pair[0].id > pair[1].id) {
        return Err(CatalogError::Unsorted);
    }
    Ok(())
}

fn admissible(definition: &RuleDefinition) -> Result<(), CatalogError> {
    if definition.id.is_empty()
        || definition.category.is_empty()
        || definition.help.is_empty()
        || definition.title.trim().is_empty()
        || definition.not_a_defect_when.trim().is_empty()
    {
        return Err(CatalogError::EmptyMetadata);
    }
    if definition.title.split_whitespace().count() > TITLE_WORDS {
        return Err(CatalogError::TitleTooLong);
    }
    if definition.requires.iter().any(|gate| {
        gate.krate.is_empty()
            || cargo_metadata::semver::VersionReq::parse(gate.versions).is_err()
    }) {
        return Err(CatalogError::InvalidGate);
    }
    if CATEGORIES.binary_search(&definition.category).is_err() {
        return Err(CatalogError::UnknownCategory);
    }
    if !definition.id.starts_with(prefix(definition.producer)) {
        return Err(CatalogError::InvalidProducer);
    }
    let published = serde_json::to_value(definition).map_err(|_| CatalogError::InvalidTier)?;
    if published_tier(&published)? != definition.tier {
        return Err(CatalogError::InvalidTier);
    }
    let (_, worst, mildest) = TIER_WINDOWS
        .iter()
        .find(|(category, _, _)| *category == definition.category)
        .ok_or(CatalogError::UnknownCategory)?;
    if definition.tier < *worst || definition.tier > *mildest {
        return Err(CatalogError::TierOutsideCategoryWindow);
    }
    Ok(())
}

const fn prefix(producer: Producer) -> &'static str {
    match producer {
        Producer::Clippy => "clippy::",
        Producer::CargoHealth => "rust_doctor::cargo::",
        Producer::SourceKernel => "rust_doctor::source::",
        Producer::Structure => "rust_doctor::structure::",
        Producer::Repo => "rust_doctor::repo::",
    }
}

pub(super) fn published_tier(definition: &Value) -> Result<RuleTier, CatalogError> {
    definition
        .get("tier")
        .and_then(Value::as_str)
        .and_then(RuleTier::parse)
        .ok_or(CatalogError::InvalidTier)
}
