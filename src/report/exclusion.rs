//! What leaves the report before it is scored, and in which order.
//!
//! Five narrowings, each one answering a question the reader asked for: a path
//! `[ignore]` covers, a file declared generated or vendored, a member
//! `--package` did not select, a site a native directive suppresses, and a
//! level an `[[overrides]]` table sets under a path. Each is counted or listed,
//! so nothing leaves without the report saying so. They run before the scope
//! projection, so a directive whose finding sits outside the changed lines is
//! still applied rather than called unused.

use std::collections::BTreeMap;

use super::normalize::canonical_severity;
use super::{Diagnostic, DiagnosticSource, SuppressionReport, SuppressionStatus};
use crate::directive::Directive;
use crate::execution::ScanExclusions;
use crate::policy::{CATALOG, PolicyPlan, RuleLevel, find};

/// A directive may name the rule it meant within this many edits and still be
/// pointed at it.
const HINT_DISTANCE: usize = 3;

#[derive(Debug, Default)]
pub(super) struct Exclusion {
    pub(super) ignored: usize,
    pub(super) excluded_generated: usize,
    pub(super) suppressions: Vec<SuppressionReport>,
}

pub(super) fn apply(
    diagnostics: &mut Vec<Diagnostic>,
    plan: &PolicyPlan,
    exclusions: &ScanExclusions,
    dependency_keys: &BTreeMap<String, String>,
) -> Exclusion {
    let ignored = remove(diagnostics, |path| plan.is_ignored(path));
    let excluded_generated = remove(diagnostics, |path| exclusions.generated.contains(path));
    if let Some(selected) = &exclusions.packages {
        diagnostics.retain(|diagnostic| {
            diagnostic
                .package
                .as_ref()
                .is_none_or(|package| selected.contains(package))
        });
    }
    let suppressions = exclusions
        .directives
        .iter()
        .filter(|directive| {
            !plan.is_ignored(&directive.path) && !exclusions.generated.contains(&directive.path)
        })
        .map(|directive| suppress(diagnostics, directive, dependency_keys))
        .collect();
    apply_overrides(diagnostics, plan);
    Exclusion {
        ignored,
        excluded_generated,
        suppressions,
    }
}

/// Removes every diagnostic whose path `excluded` names, and counts them.
fn remove(diagnostics: &mut Vec<Diagnostic>, excluded: impl Fn(&str) -> bool) -> usize {
    let before = diagnostics.len();
    diagnostics.retain(|diagnostic| !diagnostic.path.as_deref().is_some_and(&excluded));
    before - diagnostics.len()
}

/// Applies one directive, or says why it may not suppress anything. A reason
/// is checked first, since a directive without one is refused whatever it
/// names; a Clippy lint next, since the compiler already has the attribute
/// that suppresses it with a reason.
fn suppress(
    diagnostics: &mut Vec<Diagnostic>,
    directive: &Directive,
    dependency_keys: &BTreeMap<String, String>,
) -> SuppressionReport {
    let clippy = directive
        .rules
        .iter()
        .find(|rule| rule.starts_with("clippy::"));
    let unknown = directive
        .rules
        .iter()
        .find(|rule| !rule.starts_with("rust_doctor::") || find(rule).is_none());
    let (status, help, hint) = if !directive.has_reason {
        (SuppressionStatus::MissingReason, None, None)
    } else if let Some(lint) = clippy {
        (
            SuppressionStatus::UseExpect,
            Some(format!(
                "A Clippy lint is suppressed by the compiler: write #[expect({lint}, reason = \"...\")] on the item."
            )),
            None,
        )
    } else if let Some(rule) = unknown {
        (SuppressionStatus::UnknownRule, None, closest(rule))
    } else {
        let before = diagnostics.len();
        diagnostics.retain(|diagnostic| !covers(directive, diagnostic, dependency_keys));
        let status = if diagnostics.len() < before {
            SuppressionStatus::Applied
        } else {
            SuppressionStatus::Unused
        };
        (status, None, None)
    };
    SuppressionReport {
        path: directive.path.clone(),
        line: directive.line,
        rules: directive.rules.clone(),
        status,
        help,
        hint,
    }
}

/// A native finding of a named rule, in the directive's file, whose primary
/// span starts on the directive's line or on the next one. A dependency
/// finding has no span, and is covered when the next line declares its key.
fn covers(
    directive: &Directive,
    diagnostic: &Diagnostic,
    dependency_keys: &BTreeMap<String, String>,
) -> bool {
    diagnostic.source == DiagnosticSource::RustDoctor
        && diagnostic.path.as_deref() == Some(directive.path.as_str())
        && diagnostic
            .code
            .as_ref()
            .is_some_and(|code| directive.rules.contains(code))
        && diagnostic.span.as_ref().map_or_else(
            || {
                directive
                    .key
                    .as_ref()
                    .is_some_and(|key| dependency_keys.get(&diagnostic.id) == Some(key))
            },
            |span| span.line_start == directive.line || span.line_start == directive.line + 1,
        )
}

/// The catalogued id nearest to `rule`, when one is within `HINT_DISTANCE`.
fn closest(rule: &str) -> Option<String> {
    CATALOG
        .iter()
        .map(|definition| (edit_distance(rule, definition.id), definition.id))
        .filter(|(distance, _)| *distance <= HINT_DISTANCE)
        .min()
        .map(|(_, id)| id.to_owned())
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_char) in left.chars().enumerate() {
        let mut current = Vec::with_capacity(previous.len());
        current.push(row + 1);
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous.get(column).copied().unwrap_or(usize::MAX)
                + usize::from(left_char != *right_char);
            let deletion = previous.get(column + 1).copied().unwrap_or(usize::MAX) + 1;
            let insertion = current.last().copied().unwrap_or(usize::MAX) + 1;
            current.push(substitution.min(deletion).min(insertion));
        }
        previous = current;
    }
    previous.last().copied().unwrap_or_default()
}

/// The level an `[[overrides]]` table sets under the finding's path: `off`
/// removes it, any other level restamps it.
fn apply_overrides(diagnostics: &mut Vec<Diagnostic>, plan: &PolicyPlan) {
    diagnostics.retain_mut(|diagnostic| {
        let (Some(code), Some(path)) = (diagnostic.code.as_deref(), diagnostic.path.as_deref())
        else {
            return true;
        };
        match plan.path_level(code, path) {
            Some(RuleLevel::Off) => false,
            Some(level) => {
                diagnostic.severity = canonical_severity(level);
                true
            }
            None => true,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_misspelled_rule_is_pointed_at_the_one_it_meant() {
        assert_eq!(
            closest("rust_doctor::source::disabled_tls_verificaton").as_deref(),
            Some("rust_doctor::source::disabled_tls_verification")
        );
        assert_eq!(closest("rust_doctor::nothing::close"), None);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
    }
}
