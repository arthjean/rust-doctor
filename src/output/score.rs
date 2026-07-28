//! Score Core V2.
//!
//! Impact is decided by explicit rule priority and an explicit aggregation
//! policy taken from the canonical catalog, never by occurrence count alone and
//! never by presentation severity. Only rules that are score-eligible *and*
//! owned by a core analyzer contribute: whether an optional external executable
//! happens to be installed can change diagnostics and completeness, but not the
//! Core Score.

#![expect(
    clippy::redundant_pub_crate,
    reason = "score vocabulary is consumed by sibling modules through this private module"
)]

use super::score_model::{ScoreModel, score_model};
use crate::catalog::built_in_catalog;
use crate::diagnostics::{
    CanonicalDiagnostic, Category, Diagnostic, DimensionScores, ScoreLabel, Severity,
};
use crate::trust::{AggregationPolicy, Priority};
use std::collections::BTreeMap;

/// Determine which scoring dimension a category belongs to.
const fn category_dimension(category: &Category) -> Dimension {
    match category {
        Category::Security => Dimension::Security,
        // Async and Framework map to Reliability as they typically involve correctness.
        Category::Correctness | Category::ErrorHandling | Category::Async | Category::Framework => {
            Dimension::Reliability
        }
        Category::Architecture | Category::Style => Dimension::Maintainability,
        Category::Performance => Dimension::Performance,
        Category::Cargo | Category::Dependencies => Dimension::Dependencies,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Dimension {
    Security,
    Reliability,
    Maintainability,
    Performance,
    Dependencies,
}

impl Dimension {
    pub(crate) const ALL: [Self; 5] = [
        Self::Security,
        Self::Reliability,
        Self::Maintainability,
        Self::Performance,
        Self::Dependencies,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::Reliability => "reliability",
            Self::Maintainability => "maintainability",
            Self::Performance => "performance",
            Self::Dependencies => "dependencies",
        }
    }

    const fn weight(self, model: &ScoreModel) -> f64 {
        match self {
            Self::Security => model.dimension_weights.security,
            Self::Reliability => model.dimension_weights.reliability,
            Self::Maintainability => model.dimension_weights.maintainability,
            Self::Performance => model.dimension_weights.performance,
            Self::Dependencies => model.dimension_weights.dependencies,
        }
    }
}

/// Calculate health score from diagnostics using Score Core V2.
///
/// Returns `(score, label, dimension_scores)`.
pub fn calculate_score(diagnostics: &[Diagnostic]) -> Option<(u32, ScoreLabel, DimensionScores)> {
    calculate_score_for_categories(diagnostics, &[])
}

/// Calculate a score restricted to the selected diagnostic categories.
///
/// Diagnostics outside the selection are ignored. The overall score only
/// includes dimensions represented by the selection, while retaining the
/// approved dimension weights. An empty selection preserves full-scan scoring.
pub fn calculate_score_for_categories(
    diagnostics: &[Diagnostic],
    selected_categories: &[Category],
) -> Option<(u32, ScoreLabel, DimensionScores)> {
    calculate_score_entries(
        diagnostics.iter().map(|diagnostic| {
            (
                &diagnostic.category,
                diagnostic.severity,
                diagnostic.rule.as_str(),
                diagnostic.rule.as_str(),
            )
        }),
        selected_categories,
    )
}

pub(super) fn calculate_score_for_canonical<'a>(
    diagnostics: impl IntoIterator<Item = &'a CanonicalDiagnostic>,
    selected_categories: &[Category],
) -> Option<(u32, ScoreLabel, DimensionScores)> {
    calculate_score_entries(
        diagnostics.into_iter().map(|diagnostic| {
            (
                &diagnostic.category,
                diagnostic.severity,
                diagnostic.rule.as_str(),
                diagnostic
                    .root_cause_key
                    .as_deref()
                    .unwrap_or(&diagnostic.rule),
            )
        }),
        selected_categories,
    )
}

/// One aggregation group: every occurrence of one rule inside one dimension.
#[derive(Debug, Clone, Copy)]
struct ScoreGroup {
    priority: Priority,
    aggregation: AggregationPolicy,
    occurrences: usize,
}

fn calculate_score_entries<'a>(
    diagnostics: impl IntoIterator<Item = (&'a Category, Severity, &'a str, &'a str)>,
    selected_categories: &[Category],
) -> Option<(u32, ScoreLabel, DimensionScores)> {
    let model = score_model()?;
    let (penalties, has_confirmed_p0) =
        accumulate_penalties(diagnostics, selected_categories, model);

    let score_for = |dimension: Dimension| -> u32 {
        let penalty = penalties.get(&dimension).copied().unwrap_or_default();
        (100.0 - penalty).round().clamp(0.0, 100.0) as u32
    };
    let dimensions = DimensionScores {
        security: score_for(Dimension::Security),
        reliability: score_for(Dimension::Reliability),
        maintainability: score_for(Dimension::Maintainability),
        performance: score_for(Dimension::Performance),
        dependencies: score_for(Dimension::Dependencies),
    };

    let includes_dimension = |dimension| {
        selected_categories.is_empty()
            || selected_categories
                .iter()
                .any(|category| category_dimension(category) == dimension)
    };
    let (weighted_sum, total_weight) = Dimension::ALL
        .into_iter()
        .filter(|dimension| includes_dimension(*dimension))
        .fold((0.0, 0.0), |(sum, weight_sum), dimension| {
            let weight = dimension.weight(model);
            (
                f64::from(dimension_value(&dimensions, dimension)).mul_add(weight, sum),
                weight_sum + weight,
            )
        });
    let mut score = if total_weight > 0.0 {
        (weighted_sum / total_weight).round().clamp(0.0, 100.0) as u32
    } else {
        100
    };
    if has_confirmed_p0 {
        score = score.min(model.p0_score_ceiling);
    }
    let label = model.label(score);

    Some((score, label, dimensions))
}

fn accumulate_penalties<'a>(
    diagnostics: impl IntoIterator<Item = (&'a Category, Severity, &'a str, &'a str)>,
    selected_categories: &[Category],
    model: &ScoreModel,
) -> (BTreeMap<Dimension, f64>, bool) {
    let catalog = built_in_catalog().ok();
    // BTreeMap keeps accumulation order independent from scan parallelism.
    let mut groups: BTreeMap<(Dimension, String), ScoreGroup> = BTreeMap::new();
    for (category, _severity, rule, root_cause_key) in diagnostics {
        if !selected_categories.is_empty() && !selected_categories.contains(category) {
            continue;
        }
        let Some(descriptor) = catalog.and_then(|catalog| catalog.exact(rule)) else {
            // Unknown and dynamically discovered rules stay observable but
            // score-ineligible and unranked.
            continue;
        };
        if !descriptor.trust.contributes_to_core_score() {
            continue;
        }
        let Some(priority) = descriptor.trust.priority else {
            continue;
        };
        let dimension = category_dimension(category);
        groups
            .entry((dimension, root_cause_key.to_string()))
            .and_modify(|group| group.occurrences += 1)
            .or_insert(ScoreGroup {
                priority,
                aggregation: descriptor.trust.aggregation,
                occurrences: 1,
            });
    }

    let mut penalties: BTreeMap<Dimension, f64> = BTreeMap::new();
    let mut has_confirmed_p0 = false;
    for ((dimension, _), group) in &groups {
        let base = model.priority_penalties.for_priority(group.priority);
        let penalty = match group.aggregation {
            AggregationPolicy::AuditOnly => 0.0,
            AggregationPolicy::UniqueRule | AggregationPolicy::RootCause => base,
            AggregationPolicy::BoundedOccurrence => model.bounded_penalty(base, group.occurrences),
        };
        if penalty > 0.0 && group.priority == Priority::P0 {
            has_confirmed_p0 = true;
        }
        *penalties.entry(*dimension).or_default() += penalty;
    }
    (penalties, has_confirmed_p0)
}

#[cfg(test)]
pub(crate) fn canonical_penalties<'a>(
    diagnostics: impl IntoIterator<Item = &'a CanonicalDiagnostic>,
) -> BTreeMap<Dimension, f64> {
    let model = score_model().expect("the checked score model must parse");
    accumulate_penalties(
        diagnostics.into_iter().map(|diagnostic| {
            (
                &diagnostic.category,
                diagnostic.severity,
                diagnostic.rule.as_str(),
                diagnostic
                    .root_cause_key
                    .as_deref()
                    .unwrap_or(&diagnostic.rule),
            )
        }),
        &[],
        model,
    )
    .0
}

#[derive(Debug, Clone)]
pub(crate) struct GroupScoreContribution {
    pub(crate) dimension: &'static str,
    pub(crate) current_penalty: f64,
    pub(crate) maximum_penalty: f64,
    pub(crate) remediation_title: String,
}

pub(crate) fn group_score_contribution(
    rule: &str,
    category: &Category,
    occurrences: usize,
) -> Option<GroupScoreContribution> {
    let model = score_model()?;
    let catalog = built_in_catalog().ok()?;
    let descriptor = catalog.exact(rule)?;
    if !descriptor.trust.contributes_to_core_score() {
        return None;
    }
    let priority = descriptor.trust.priority?;
    let base = model.priority_penalties.for_priority(priority);
    let (current_penalty, maximum_penalty) = match descriptor.trust.aggregation {
        AggregationPolicy::AuditOnly => return None,
        AggregationPolicy::UniqueRule | AggregationPolicy::RootCause => (base, base),
        AggregationPolicy::BoundedOccurrence => (
            model.bounded_penalty(base, occurrences),
            base * model.occurrence_multiplier_cap,
        ),
    };
    Some(GroupScoreContribution {
        dimension: category_dimension(category).as_str(),
        current_penalty,
        maximum_penalty,
        remediation_title: descriptor.fix_guidance.clone(),
    })
}

const fn dimension_value(scores: &DimensionScores, dimension: Dimension) -> u32 {
    match dimension {
        Dimension::Security => scores.security,
        Dimension::Reliability => scores.reliability,
        Dimension::Maintainability => scores.maintainability,
        Dimension::Performance => scores.performance,
        Dimension::Dependencies => scores.dependencies,
    }
}

pub(super) fn score_label(score: u32) -> ScoreLabel {
    score_model().map_or(ScoreLabel::Critical, |model| model.label(score))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn diagnostic(rule: &str, category: Category, severity: Severity) -> Diagnostic {
        Diagnostic {
            file_path: PathBuf::from("src/lib.rs"),
            rule: rule.to_string(),
            category,
            severity,
            message: format!("{rule} fired"),
            help: None,
            line: Some(1),
            column: Some(1),
            fix: None,
        }
    }

    /// A compiler-proven rule that needs no calibration artifact to score.
    fn compiler_error() -> Diagnostic {
        diagnostic("compiler-error", Category::Correctness, Severity::Error)
    }

    #[test]
    fn an_empty_scan_scores_one_hundred() {
        let (score, label, dimensions) = calculate_score(&[]).expect("embedded score model");
        assert_eq!(score, 100);
        assert_eq!(label, ScoreLabel::Great);
        assert_eq!(dimensions.security, 100);
    }

    #[test]
    fn a_confirmed_p0_can_never_be_labelled_great() {
        let (score, label, _) = calculate_score(&[compiler_error()]).expect("embedded score model");
        assert!(score <= 49, "P0 finding left the score at {score}");
        assert_eq!(label, ScoreLabel::Critical);
    }

    #[test]
    fn one_hundred_bounded_occurrences_cost_at_most_twice_the_first() {
        let single = calculate_score(&[diagnostic(
            "missing-msrv",
            Category::Cargo,
            Severity::Warning,
        )])
        .expect("embedded score model");
        let repeated: Vec<_> = (0..100)
            .map(|_| diagnostic("msrv-outdated", Category::Cargo, Severity::Warning))
            .collect();
        let many = calculate_score(&repeated).expect("embedded score model");
        let single_penalty = 100 - single.2.dependencies;
        let many_penalty = 100 - many.2.dependencies;
        assert!(
            many_penalty <= single_penalty * 2,
            "{many_penalty} exceeded twice {single_penalty}"
        );
    }

    #[test]
    fn score_ineligible_and_audit_only_findings_contribute_nothing() {
        let audit_only = calculate_score(&[diagnostic(
            "unsafe-block-audit",
            Category::Security,
            Severity::Warning,
        )])
        .expect("embedded score model");
        assert_eq!(audit_only.0, 100);

        let unknown = calculate_score(&[diagnostic(
            "clippy::some_future_lint",
            Category::Style,
            Severity::Warning,
        )])
        .expect("embedded score model");
        assert_eq!(unknown.0, 100);
    }

    #[test]
    fn optional_external_adapters_never_move_the_core_score() {
        for rule in [
            "deny-advisory",
            "unused-dependency",
            "unsafe-dependency",
            "semver-violation",
        ] {
            let (score, _, _) =
                calculate_score(&[diagnostic(rule, Category::Dependencies, Severity::Error)])
                    .expect("embedded score model");
            assert_eq!(score, 100, "{rule} moved the Core Score");
        }
    }

    #[test]
    fn adding_one_eligible_diagnostic_never_raises_a_score() {
        let base = vec![diagnostic(
            "missing-msrv",
            Category::Cargo,
            Severity::Warning,
        )];
        let (base_score, _, base_dimensions) =
            calculate_score(&base).expect("embedded score model");
        let mut extended = base;
        extended.push(compiler_error());
        let (extended_score, _, extended_dimensions) =
            calculate_score(&extended).expect("embedded score model");
        assert!(extended_score <= base_score);
        assert!(extended_dimensions.dependencies <= base_dimensions.dependencies);
        assert!(extended_dimensions.reliability <= base_dimensions.reliability);
    }

    #[test]
    fn scoring_is_independent_from_input_order() {
        let mut diagnostics = vec![
            compiler_error(),
            diagnostic("missing-msrv", Category::Cargo, Severity::Warning),
            diagnostic("msrv-outdated", Category::Cargo, Severity::Warning),
        ];
        let first = calculate_score(&diagnostics).expect("embedded score model");
        diagnostics.reverse();
        let second = calculate_score(&diagnostics).expect("embedded score model");
        assert_eq!(first.0, second.0);
        assert_eq!(first.1, second.1);
        assert_eq!(first.2.dependencies, second.2.dependencies);
    }

    #[test]
    fn category_selection_restricts_the_weighted_dimensions() {
        let diagnostics = vec![compiler_error()];
        let (full, _, _) = calculate_score(&diagnostics).expect("embedded score model");
        let (scoped, _, _) = calculate_score_for_categories(&diagnostics, &[Category::Performance])
            .expect("embedded score model");
        assert_eq!(scoped, 100);
        assert!(full < scoped);
    }

    #[test]
    fn score_label_thresholds_are_stable() {
        assert_eq!(score_label(100), ScoreLabel::Great);
        assert_eq!(score_label(95), ScoreLabel::Great);
        assert_eq!(score_label(94), ScoreLabel::NeedsWork);
        assert_eq!(score_label(50), ScoreLabel::NeedsWork);
        assert_eq!(score_label(49), ScoreLabel::Critical);
        assert_eq!(score_label(0), ScoreLabel::Critical);
    }

    /// The exported constant and the reviewed artifact must never drift.
    #[test]
    fn the_exported_model_version_matches_the_artifact() {
        assert_eq!(crate::output::score_model_version(), "2.1");
    }
}
