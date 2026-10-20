//! What `rules explain` and `why` answer: what a rule does and how this
//! workspace sets it, and why a line is reported.
//!
//! Explaining a rule reads the catalog and, given a path, the
//! `rust-doctor.toml` of the workspace root a scan of that path would judge,
//! which `cargo metadata` names as it does for the scan. The site half lives
//! in `why.rs`.

use std::fmt::{self, Write as _};
use std::path::Path;

use serde::Serialize;

use crate::configuration::{CONFIG_FILE_NAME, load_path};
use crate::policy::{
    CatalogEntry, PathOverrideReport, PolicyInput, PolicyPlan, RuleLevel, RuleLevelSource,
    catalog, corpus_measurement,
};
use crate::presentation::rule_url;

mod why;

pub use why::{Absence, SiteExplanation, WhyHit, WhyLocation, WhySuppression, explain_site};

/// The furthest a misspelled id may be from a catalogued one, or from its last
/// segment, to be named as the closest.
const CLOSEST_DISTANCE: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct RuleExplanation {
    #[serde(flatten)]
    pub rule: CatalogEntry,
    pub url: String,
    /// Smoothed false-positive rate on the pinned corpus, in basis points,
    /// absent when the corpus never adjudicated the rule.
    pub corpus_noise_basis_points: Option<u16>,
    pub corpus_reviewed_sites: Option<u64>,
    /// How the workspace named by `PATH` sets the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective: Option<EffectiveLevel>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EffectiveLevel {
    pub level: RuleLevel,
    pub source: RuleLevelSource,
    /// Every `[[overrides]]` table that sets this rule or its category, in
    /// the order the later one wins in.
    pub overrides: Vec<PathOverrideReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplainError {
    /// No catalogued id matches, with the nearest one when any is close.
    Unknown { query: String, closest: Option<&'static str> },
    /// The suffix names several rules.
    Ambiguous { query: String, candidates: Vec<&'static str> },
    /// The workspace's `rust-doctor.toml` could not be read.
    Configuration(String),
}

impl fmt::Display for ExplainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { query, closest } => {
                write!(formatter, "no catalogued rule is named `{query}`")?;
                match closest {
                    Some(closest) => write!(formatter, "; did you mean `{closest}`?"),
                    None => formatter.write_str("; `rust-doctor rules list` prints them all."),
                }
            }
            Self::Ambiguous { query, candidates } => write!(
                formatter,
                "`{query}` names several rules: {}.",
                candidates.join(", ")
            ),
            Self::Configuration(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ExplainError {}

/// The paths the repository at `workspace_root` declares generated or
/// vendored in `.gitattributes`, read through `git check-attr`: what a scan
/// excludes before any producer grades a file.
pub fn declared_generated(workspace_root: &Path) -> std::collections::BTreeSet<String> {
    crate::generated::declared(workspace_root, None)
}

/// The catalogued rule an exact id or an unambiguous last segment names:
/// `todo` is `clippy::todo`.
pub fn resolve_rule(query: &str) -> Result<CatalogEntry, ExplainError> {
    let rules = catalog();
    if let Some(rule) = rules.iter().find(|rule| rule.id == query) {
        return Ok(*rule);
    }
    let suffixed: Vec<&CatalogEntry> = rules
        .iter()
        .filter(|rule| rule.id.rsplit("::").next() == Some(query))
        .collect();
    match suffixed.as_slice() {
        [rule] => Ok(**rule),
        [] => Err(ExplainError::Unknown {
            query: query.to_owned(),
            closest: closest(&rules, query),
        }),
        several => Err(ExplainError::Ambiguous {
            query: query.to_owned(),
            candidates: several.iter().map(|rule| rule.id).collect(),
        }),
    }
}

fn closest(rules: &[CatalogEntry], query: &str) -> Option<&'static str> {
    rules
        .iter()
        .map(|rule| {
            let segment = rule.id.rsplit("::").next().unwrap_or(rule.id);
            let distance = crate::report::edit_distance(query, rule.id)
                .min(crate::report::edit_distance(query, segment));
            (distance, rule.id)
        })
        .filter(|(distance, _)| *distance <= CLOSEST_DISTANCE)
        .min()
        .map(|(_, id)| id)
}

/// The rule `query` names, and, given `workspace`, the level its
/// `rust-doctor.toml` sets and where that level comes from.
pub fn explain_rule(query: &str, workspace: Option<&Path>) -> Result<RuleExplanation, ExplainError> {
    let rule = resolve_rule(query)?;
    let measurement = corpus_measurement(rule.id);
    let effective = workspace.map(|root| effective_level(&rule, root)).transpose()?;
    Ok(RuleExplanation {
        url: rule_url(rule.id),
        corpus_noise_basis_points: measurement.map(|measured| measured.noise_basis_points()),
        corpus_reviewed_sites: measurement.map(|measured| measured.reviewed()),
        effective,
        rule,
    })
}

fn effective_level(rule: &CatalogEntry, path: &Path) -> Result<EffectiveLevel, ExplainError> {
    // The configuration a scan of `path` reads: its workspace root's, found
    // the way a scan finds it. A member's own file, which scans ignore, used
    // to answer here. Without Cargo, or outside a workspace, `path` is taken
    // as the root it was given as.
    let root = crate::cargo_workspace_root(path).unwrap_or_else(|| path.to_path_buf());
    let configuration = load_path(&root.join(CONFIG_FILE_NAME))
        .map_err(|error| ExplainError::Configuration(error.message))?;
    let input = PolicyInput::default();
    let policy = input
        .validate()
        .map_err(|error| ExplainError::Configuration(error.message.to_owned()))?;
    let plan = PolicyPlan::compile_with_configuration(&policy, &configuration);
    let (level, source) = plan
        .effective_rules()
        .find(|(definition, _, _)| definition.id == rule.id)
        .map_or((rule.default_level, RuleLevelSource::Default), |(_, level, source)| {
            (level, source)
        });
    let overrides = configuration
        .paths
        .overrides_report()
        .into_iter()
        .filter(|table| {
            table.rules.contains_key(rule.id) || table.categories.contains_key(rule.category)
        })
        .collect();
    Ok(EffectiveLevel {
        level,
        source,
        overrides,
    })
}

impl RuleExplanation {
    /// The explanation as `rules explain` prints it, one fact per line.
    pub fn text(&self) -> String {
        let rule = &self.rule;
        let mut text = String::new();
        let _ = writeln!(text, "{}", rule.id);
        let _ = writeln!(text, "Producer: {}", rule.producer.as_str());
        let _ = writeln!(text, "Category: {}", rule.category);
        let _ = writeln!(text, "Tier: {}", rule.tier.as_str());
        let _ = writeln!(text, "Default level: {}", rule.default_level);
        let _ = writeln!(text, "Help: {}", rule.help);
        let _ = writeln!(text, "Rule: {}", self.url);
        let _ = match (self.corpus_noise_basis_points, self.corpus_reviewed_sites) {
            (Some(noise), Some(sites)) => writeln!(
                text,
                "Corpus noise: {}% on {sites} adjudicated {}",
                (u32::from(noise) + 50) / 100,
                if sites == 1 { "site" } else { "sites" }
            ),
            _ => writeln!(text, "Corpus noise: unmeasured"),
        };
        if let Some(effective) = &self.effective {
            let _ = writeln!(
                text,
                "Effective level: {} ({})",
                effective.level,
                effective.source.as_str()
            );
            for table in &effective.overrides {
                let level = table
                    .rules
                    .get(rule.id)
                    .or_else(|| table.categories.get(rule.category));
                if let Some(level) = level {
                    let _ = writeln!(text, "Override: {level} under {}", table.paths.join(", "));
                }
            }
        }
        text
    }
}

#[cfg(test)]
mod tests;
