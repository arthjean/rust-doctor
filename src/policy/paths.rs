//! The part of a policy that depends on where a finding is: the paths
//! `rust-doctor.toml` ignores, and the levels it sets under one path.
//!
//! Both are read from the configuration file only. A level the request set
//! with `--rule` or `--category` is the reader's answer for this run, so no
//! file overrides it, under any path.

use std::collections::BTreeMap;

use serde::Serialize;

use super::{RuleDefinition, RuleLevel};
use crate::path_glob::PathGlob;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PathPolicy {
    pub(crate) ignore: Vec<PathGlob>,
    pub(crate) overrides: Vec<PathOverride>,
}

/// One `[[overrides]]` table: the levels it sets, and where.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PathOverride {
    pub(crate) paths: Vec<PathGlob>,
    pub(crate) rules: BTreeMap<String, RuleLevel>,
    pub(crate) categories: BTreeMap<String, RuleLevel>,
}

/// One `[[overrides]]` table as the report publishes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathOverrideReport {
    pub paths: Vec<String>,
    pub rules: BTreeMap<String, RuleLevel>,
    pub categories: BTreeMap<String, RuleLevel>,
}

impl PathPolicy {
    pub(crate) fn is_ignored(&self, path: &str) -> bool {
        self.ignore.iter().any(|glob| glob.matches(path))
    }

    /// The level the last override covering `path` sets for this rule, its
    /// rule selector before its category selector.
    pub(crate) fn level(&self, definition: &RuleDefinition, path: &str) -> Option<RuleLevel> {
        self.overrides.iter().rev().find_map(|table| {
            table
                .paths
                .iter()
                .any(|glob| glob.matches(path))
                .then(|| {
                    table
                        .rules
                        .get(definition.id)
                        .or_else(|| table.categories.get(definition.category))
                        .copied()
                })
                .flatten()
        })
    }

    pub(crate) fn ignore_report(&self) -> Vec<String> {
        self.ignore.iter().map(|glob| glob.as_str().to_owned()).collect()
    }

    pub(crate) fn overrides_report(&self) -> Vec<PathOverrideReport> {
        self.overrides
            .iter()
            .map(|table| PathOverrideReport {
                paths: table
                    .paths
                    .iter()
                    .map(|glob| glob.as_str().to_owned())
                    .collect(),
                rules: table.rules.clone(),
                categories: table.categories.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::find;

    fn table(paths: &[&str], rules: &[(&str, RuleLevel)], categories: &[(&str, RuleLevel)]) -> PathOverride {
        PathOverride {
            paths: paths.iter().map(|path| PathGlob::parse(path).unwrap()).collect(),
            rules: rules.iter().map(|(id, level)| ((*id).to_owned(), *level)).collect(),
            categories: categories
                .iter()
                .map(|(name, level)| ((*name).to_owned(), *level))
                .collect(),
        }
    }

    #[test]
    fn the_last_override_covering_a_path_wins_and_a_rule_beats_its_category() {
        let todo = find("clippy::todo").unwrap();
        let policy = PathPolicy {
            ignore: Vec::new(),
            overrides: vec![
                table(&["src/**"], &[("clippy::todo", RuleLevel::Error)], &[]),
                table(&["src/gen"], &[], &[(todo.category, RuleLevel::Off)]),
                table(&["src/gen/keep.rs"], &[("clippy::todo", RuleLevel::Warn)], &[(todo.category, RuleLevel::Off)]),
            ],
        };
        assert_eq!(policy.level(todo, "src/lib.rs"), Some(RuleLevel::Error));
        assert_eq!(policy.level(todo, "src/gen/a.rs"), Some(RuleLevel::Off));
        assert_eq!(policy.level(todo, "src/gen/keep.rs"), Some(RuleLevel::Warn));
        assert_eq!(policy.level(todo, "tests/a.rs"), None);
    }
}
