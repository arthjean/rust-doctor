//! Whether a catalog is admissible, asked one list-level question at a time
//! and then one definition at a time.
//!
//! It is compiled under `cfg(test)` alone: nothing a scan does depends on it,
//! and a catalog that fails it is a catalog that never ships. Keeping it out of
//! the shipped file also keeps the file that declares 62 rules readable as what
//! it is, a list.

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
}

/// Severity window each category admits, from its worst admissible tier to its
/// mildest. The category bounds how bad the defect can be: a maintainability
/// finding is never a `P0`, a security finding is never a `P3`.
///
/// These are the windows the shipped rules occupy. Widening one is a decision
/// about what the score means, so it is made here, once, instead of drifting
/// one rule at a time as the catalog grows.
const TIER_WINDOWS: [(&str, RuleTier, RuleTier); CATEGORIES.len()] = [
    ("correctness", RuleTier::P1, RuleTier::P2),
    ("dependencies", RuleTier::P1, RuleTier::P2),
    ("maintainability", RuleTier::P3, RuleTier::P3),
    ("performance", RuleTier::P2, RuleTier::P3),
    ("reliability", RuleTier::P2, RuleTier::P3),
    ("security", RuleTier::P0, RuleTier::P1),
];

/// Two questions about the list, then one question per rule. The list-level
/// checks come first because a duplicate or an unsorted pair is a defect of the
/// list, not of either rule it names.
pub(super) fn validate_catalog(catalog: &[&RuleDefinition]) -> Result<(), CatalogError> {
    inventory(catalog)?;
    catalog
        .iter()
        .try_for_each(|definition| admissible(definition))
}

/// The list is a set, and it is sorted: `find_in` binary-searches it.
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

/// One rule against everything a rule has to be: named, categorized in a known
/// category, produced by the pass its identifier claims, and graded inside the
/// window its category admits.
fn admissible(definition: &RuleDefinition) -> Result<(), CatalogError> {
    if definition.id.is_empty() || definition.category.is_empty() || definition.help.is_empty() {
        return Err(CatalogError::EmptyMetadata);
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

/// The identifier prefix a producer owns. `validate_catalog` refuses any other,
/// which is what keeps the producer field and the identifier one fact.
const fn prefix(producer: Producer) -> &'static str {
    match producer {
        Producer::Clippy => "clippy::",
        Producer::CargoHealth => "rust_doctor::cargo::",
        Producer::SourceKernel => "rust_doctor::source::",
        Producer::Structure => "rust_doctor::structure::",
        Producer::Repo => "rust_doctor::repo::",
    }
}

/// The type already forbids a missing tier or one outside the four values. The
/// published form, however, is a string: that is where the invalid state
/// becomes representable again, so that is where the validation applies. The
/// error is closed and echoes neither the read value, nor a path, nor an escape
/// sequence.
pub(super) fn published_tier(definition: &Value) -> Result<RuleTier, CatalogError> {
    definition
        .get("tier")
        .and_then(Value::as_str)
        .and_then(RuleTier::parse)
        .ok_or(CatalogError::InvalidTier)
}
