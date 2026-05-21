mod score;
mod terminal;

pub use score::calculate_score;
pub use terminal::render_terminal;

use crate::diagnostics::ScanResult;
use owo_colors::{OwoColorize, Stream};

/// Render `--score` mode: bare integer to stdout.
pub fn render_score(result: &ScanResult) {
    if result.source_file_count == 0 {
        eprintln!(
            "{}",
            "No Rust source files found".if_supports_color(Stream::Stderr, |t| t.yellow())
        );
    }
    if !result.skipped_passes.is_empty() {
        eprintln!(
            "Warning: {} pass(es) skipped (missing tools) — score may be incomplete. \
             Run: rust-doctor --install-deps",
            result.skipped_passes.len()
        );
    }
    println!("{}", result.score);
}

/// Render `--json` mode: full ScanResult as JSON to stdout.
///
/// # Errors
///
/// Returns an error if serialization fails.
pub fn render_json(result: &ScanResult) -> Result<(), serde_json::Error> {
    let json = serde_json::to_string_pretty(result)?;
    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Category, Diagnostic, ScoreLabel, Severity};
    use std::path::PathBuf;

    fn make_diag(rule: &str, severity: Severity) -> Diagnostic {
        make_diag_with_category(rule, severity, Category::ErrorHandling)
    }

    fn make_diag_with_category(rule: &str, severity: Severity, category: Category) -> Diagnostic {
        Diagnostic {
            file_path: PathBuf::from("src/main.rs"),
            rule: rule.to_string(),
            category,
            severity,
            message: format!("Issue: {rule}"),
            help: None,
            line: Some(1),
            column: None,
            fix: None,
        }
    }

    // --- Score calculation tests ---

    #[test]
    fn test_perfect_score() {
        let (score, label, dims) = calculate_score(&[], None);
        assert_eq!(score, 100);
        assert_eq!(label, ScoreLabel::Great);
        assert_eq!(dims.security, 100);
        assert_eq!(dims.reliability, 100);
        assert_eq!(dims.maintainability, 100);
        assert_eq!(dims.performance, 100);
        assert_eq!(dims.dependencies, 100);
    }

    #[test]
    fn test_score_with_errors_in_reliability() {
        let diags = vec![
            make_diag("rule1", Severity::Error),
            make_diag("rule2", Severity::Error),
        ];
        let (score, label, dims) = calculate_score(&diags, None);
        assert_eq!(dims.reliability, 97);
        assert_eq!(dims.security, 100);
        assert_eq!(score, 99);
        assert_eq!(label, ScoreLabel::Great);
    }

    #[test]
    fn test_score_with_warnings_in_reliability() {
        let diags = vec![
            make_diag("w1", Severity::Warning),
            make_diag("w2", Severity::Warning),
            make_diag("w3", Severity::Warning),
            make_diag("w4", Severity::Warning),
        ];
        let (score, label, dims) = calculate_score(&diags, None);
        assert_eq!(dims.reliability, 97);
        assert_eq!(score, 99);
        assert_eq!(label, ScoreLabel::Great);
    }

    #[test]
    fn test_score_duplicate_rules_counted_once() {
        let diags = vec![
            make_diag("rule1", Severity::Error),
            make_diag("rule1", Severity::Error),
            make_diag("rule1", Severity::Error),
            make_diag("rule1", Severity::Error),
            make_diag("rule1", Severity::Error),
        ];
        let (score, _, dims) = calculate_score(&diags, None);
        assert_eq!(dims.reliability, 99);
        assert_eq!(score, 100);
    }

    #[test]
    fn test_score_mixed_single_dimension() {
        let mut diags = Vec::new();
        for i in 0..10 {
            diags.push(make_diag(&format!("err{i}"), Severity::Error));
        }
        for i in 0..20 {
            diags.push(make_diag(&format!("warn{i}"), Severity::Warning));
        }
        let (score, label, dims) = calculate_score(&diags, None);
        assert_eq!(dims.reliability, 70);
        assert_eq!(score, 93);
        assert_eq!(label, ScoreLabel::Great);
    }

    #[test]
    fn test_dimension_clamped_to_zero() {
        let mut diags = Vec::new();
        for i in 0..100 {
            diags.push(make_diag(&format!("err{i}"), Severity::Error));
        }
        let (score, label, dims) = calculate_score(&diags, None);
        assert_eq!(dims.reliability, 0);
        assert_eq!(score, 77);
        assert_eq!(label, ScoreLabel::Great);
    }

    #[test]
    fn test_all_dimensions_severely_degraded() {
        let mut diags = Vec::new();
        for i in 0..100 {
            diags.push(make_diag_with_category(
                &format!("sec{i}"),
                Severity::Error,
                Category::Security,
            ));
            diags.push(make_diag_with_category(
                &format!("err{i}"),
                Severity::Error,
                Category::ErrorHandling,
            ));
            diags.push(make_diag_with_category(
                &format!("arch{i}"),
                Severity::Error,
                Category::Architecture,
            ));
            diags.push(make_diag_with_category(
                &format!("perf{i}"),
                Severity::Error,
                Category::Performance,
            ));
            diags.push(make_diag_with_category(
                &format!("dep{i}"),
                Severity::Error,
                Category::Dependencies,
            ));
        }
        let (score, label, dims) = calculate_score(&diags, None);
        assert_eq!(dims.security, 0);
        assert_eq!(dims.reliability, 0);
        assert_eq!(dims.maintainability, 0);
        assert_eq!(dims.performance, 0);
        assert_eq!(dims.dependencies, 0);
        assert_eq!(score, 0);
        assert_eq!(label, ScoreLabel::Critical);
    }

    #[test]
    fn test_score_label_thresholds() {
        use score::score_label;
        assert_eq!(score_label(100), ScoreLabel::Great);
        assert_eq!(score_label(75), ScoreLabel::Great);
        assert_eq!(score_label(74), ScoreLabel::NeedsWork);
        assert_eq!(score_label(50), ScoreLabel::NeedsWork);
        assert_eq!(score_label(49), ScoreLabel::Critical);
        assert_eq!(score_label(0), ScoreLabel::Critical);
    }

    #[test]
    fn test_security_category_only_affects_security_dimension() {
        let diags = vec![
            make_diag_with_category("sec1", Severity::Error, Category::Security),
            make_diag_with_category("sec2", Severity::Error, Category::Security),
        ];
        let (_, _, dims) = calculate_score(&diags, None);
        assert_eq!(dims.security, 97);
        assert_eq!(dims.reliability, 100);
        assert_eq!(dims.maintainability, 100);
        assert_eq!(dims.performance, 100);
        assert_eq!(dims.dependencies, 100);
    }

    #[test]
    fn test_overall_is_weighted_average() {
        let diags = vec![
            make_diag_with_category("sec1", Severity::Error, Category::Security),
            make_diag_with_category("sec2", Severity::Error, Category::Security),
        ];
        let (score, _, _) = calculate_score(&diags, None);
        assert_eq!(score, 99);
    }

    #[test]
    fn test_empty_diagnostics_all_dimensions_100() {
        let (score, label, dims) = calculate_score(&[], None);
        assert_eq!(score, 100);
        assert_eq!(label, ScoreLabel::Great);
        assert_eq!(dims.security, 100);
        assert_eq!(dims.reliability, 100);
        assert_eq!(dims.maintainability, 100);
        assert_eq!(dims.performance, 100);
        assert_eq!(dims.dependencies, 100);
    }

    #[test]
    fn test_multiple_dimensions_affected() {
        let diags = vec![
            make_diag_with_category("sec1", Severity::Error, Category::Security),
            make_diag_with_category("perf1", Severity::Warning, Category::Performance),
            make_diag_with_category("style1", Severity::Info, Category::Style),
        ];
        let (_, _, dims) = calculate_score(&diags, None);
        assert_eq!(dims.security, 99);
        assert_eq!(dims.performance, 99);
        assert_eq!(dims.maintainability, 100);
        assert_eq!(dims.reliability, 100);
        assert_eq!(dims.dependencies, 100);
    }

    #[test]
    fn test_dependencies_category_maps_to_dependencies_dimension() {
        let diags = vec![
            make_diag_with_category("dep1", Severity::Warning, Category::Dependencies),
            make_diag_with_category("cargo1", Severity::Warning, Category::Cargo),
        ];
        let (_, _, dims) = calculate_score(&diags, None);
        assert_eq!(dims.dependencies, 99);
        assert_eq!(dims.security, 100);
    }

    #[test]
    fn test_category_filtered_score() {
        use crate::cli::CategoryFilter;
        let diags = vec![
            make_diag_with_category("arch1", Severity::Error, Category::Architecture),
            make_diag_with_category("arch2", Severity::Error, Category::Architecture),
            make_diag_with_category("sec1", Severity::Error, Category::Security),
        ];
        let (score_unfiltered, _, dims_unfiltered) = calculate_score(&diags, None);
        assert_eq!(dims_unfiltered.maintainability, 97);
        assert_eq!(dims_unfiltered.security, 99);
        assert_ne!(score_unfiltered, dims_unfiltered.maintainability);

        let (score_filtered, _, dims_filtered) =
            calculate_score(&diags, Some(CategoryFilter::Architecture));
        assert_eq!(dims_filtered.maintainability, 97);
        assert_eq!(score_filtered, 97);
    }
}
