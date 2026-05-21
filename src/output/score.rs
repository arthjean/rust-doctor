use crate::cli::CategoryFilter;
use crate::diagnostics::{Category, Diagnostic, DimensionScores, ScoreLabel, Severity};
use std::collections::{HashMap, HashSet};

// --- Score constants ---

pub(super) const SCORE_GOOD_THRESHOLD: u32 = 75;
pub(super) const SCORE_OK_THRESHOLD: u32 = 50;

const ERROR_RULE_PENALTY: f64 = 1.5;
const WARNING_RULE_PENALTY: f64 = 0.75;
const INFO_RULE_PENALTY: f64 = 0.25;

// --- Dimension weights ---

const WEIGHT_SECURITY: f64 = 2.0;
const WEIGHT_RELIABILITY: f64 = 1.5;
const WEIGHT_MAINTAINABILITY: f64 = 1.0;
const WEIGHT_PERFORMANCE: f64 = 1.0;
const WEIGHT_DEPENDENCIES: f64 = 1.0;

// --- Score calculation ---

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimension {
    Security,
    Reliability,
    Maintainability,
    Performance,
    Dependencies,
}

pub const fn filter_dimension(filter: CategoryFilter) -> Dimension {
    match filter {
        CategoryFilter::Security => Dimension::Security,
        CategoryFilter::Correctness
        | CategoryFilter::ErrorHandling
        | CategoryFilter::Async
        | CategoryFilter::Framework => Dimension::Reliability,
        CategoryFilter::Architecture | CategoryFilter::Style => Dimension::Maintainability,
        CategoryFilter::Performance => Dimension::Performance,
        CategoryFilter::Dependencies | CategoryFilter::Cargo => Dimension::Dependencies,
    }
}

/// Compute the score for a single dimension from its unique rules by severity.
fn dimension_score(error_count: usize, warning_count: usize, info_count: usize) -> u32 {
    let penalty = (info_count as f64).mul_add(
        INFO_RULE_PENALTY,
        (error_count as f64).mul_add(
            ERROR_RULE_PENALTY,
            warning_count as f64 * WARNING_RULE_PENALTY,
        ),
    );
    (100.0 - penalty).round().clamp(0.0, 100.0) as u32
}

/// Calculate health score from diagnostics using weighted dimension scoring.
///
/// Each diagnostic is assigned to a dimension based on its category.
/// Each dimension is scored independently: `100 - (unique_rules × severity_penalty)`.
/// The overall score is the weighted average of all dimension scores.
///
/// Returns `(score, label, dimension_scores)`.
#[allow(clippy::option_if_let_else)]
pub fn calculate_score(
    diagnostics: &[Diagnostic],
    category_filter: Option<CategoryFilter>,
) -> (u32, ScoreLabel, DimensionScores) {
    // Collect unique rules per (dimension, severity).
    let mut dim_errors: HashMap<Dimension, HashSet<&str>> = HashMap::new();
    let mut dim_warnings: HashMap<Dimension, HashSet<&str>> = HashMap::new();
    let mut dim_infos: HashMap<Dimension, HashSet<&str>> = HashMap::new();

    for d in diagnostics {
        let dim = category_dimension(&d.category);
        match d.severity {
            Severity::Error => {
                dim_errors.entry(dim).or_default().insert(d.rule.as_str());
            }
            Severity::Warning => {
                dim_warnings.entry(dim).or_default().insert(d.rule.as_str());
            }
            Severity::Info => {
                dim_infos.entry(dim).or_default().insert(d.rule.as_str());
            }
        }
    }

    let score_for = |dim: Dimension| -> u32 {
        dimension_score(
            dim_errors.get(&dim).map_or(0, HashSet::len),
            dim_warnings.get(&dim).map_or(0, HashSet::len),
            dim_infos.get(&dim).map_or(0, HashSet::len),
        )
    };

    let security = score_for(Dimension::Security);
    let reliability = score_for(Dimension::Reliability);
    let maintainability = score_for(Dimension::Maintainability);
    let performance = score_for(Dimension::Performance);
    let dependencies = score_for(Dimension::Dependencies);

    let dimensions = DimensionScores {
        security,
        reliability,
        maintainability,
        performance,
        dependencies,
    };

    // Calculate score based on category filtering or weighted average
    let score = if let Some(filter) = category_filter {
        let active_dim = filter_dimension(filter);
        match active_dim {
            Dimension::Security => security,
            Dimension::Reliability => reliability,
            Dimension::Maintainability => maintainability,
            Dimension::Performance => performance,
            Dimension::Dependencies => dependencies,
        }
    } else {
        let total_weight = WEIGHT_SECURITY
            + WEIGHT_RELIABILITY
            + WEIGHT_MAINTAINABILITY
            + WEIGHT_PERFORMANCE
            + WEIGHT_DEPENDENCIES;
        let weighted_sum = f64::from(security).mul_add(
            WEIGHT_SECURITY,
            f64::from(reliability).mul_add(
                WEIGHT_RELIABILITY,
                f64::from(maintainability).mul_add(
                    WEIGHT_MAINTAINABILITY,
                    f64::from(performance).mul_add(
                        WEIGHT_PERFORMANCE,
                        f64::from(dependencies) * WEIGHT_DEPENDENCIES,
                    ),
                ),
            ),
        );
        (weighted_sum / total_weight).round().clamp(0.0, 100.0) as u32
    };

    let label = score_label(score);

    (score, label, dimensions)
}

pub(super) const fn score_label(score: u32) -> ScoreLabel {
    if score >= SCORE_GOOD_THRESHOLD {
        ScoreLabel::Great
    } else if score >= SCORE_OK_THRESHOLD {
        ScoreLabel::NeedsWork
    } else {
        ScoreLabel::Critical
    }
}
