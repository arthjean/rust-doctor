mod score;
mod terminal;

pub use score::calculate_score;
pub use terminal::render_terminal;

use crate::diagnostics::{ReportV1, ScanResult};
use owo_colors::{OwoColorize, Stream};
use std::io::Write;
use std::path::Path;

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

/// Render the immutable Report V1 value to stdout or atomically to a file.
///
/// # Errors
///
/// Returns an error if serialization or the atomic destination write fails.
pub fn render_json(
    report: &ReportV1,
    compact: bool,
    destination: Option<&Path>,
) -> Result<(), crate::error::OutputError> {
    let mut bytes = if compact {
        serde_json::to_vec(report)
    } else {
        serde_json::to_vec_pretty(report)
    }
    .map_err(crate::error::OutputError::Serialize)?;
    bytes.push(b'\n');

    if let Some(path) = destination {
        let parent = path
            .parent()
            .filter(|value| !value.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|source| {
            crate::error::OutputError::Write {
                path: path.to_path_buf(),
                source,
            }
        })?;
        temporary
            .write_all(&bytes)
            .map_err(|source| crate::error::OutputError::Write {
                path: path.to_path_buf(),
                source,
            })?;
        temporary
            .flush()
            .map_err(|source| crate::error::OutputError::Write {
                path: path.to_path_buf(),
                source,
            })?;
        temporary
            .persist(path)
            .map_err(|error| crate::error::OutputError::Write {
                path: path.to_path_buf(),
                source: error.error,
            })?;
    } else {
        std::io::stdout()
            .write_all(&bytes)
            .map_err(crate::error::OutputError::Stdout)?;
    }
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
        let (score, label, dims) = calculate_score(&[]);
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
        let (score, label, dims) = calculate_score(&diags);
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
        let (score, label, dims) = calculate_score(&diags);
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
        let (score, _, dims) = calculate_score(&diags);
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
        let (score, label, dims) = calculate_score(&diags);
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
        let (score, label, dims) = calculate_score(&diags);
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
        let (score, label, dims) = calculate_score(&diags);
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
        let (_, _, dims) = calculate_score(&diags);
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
        let (score, _, _) = calculate_score(&diags);
        assert_eq!(score, 99);
    }

    #[test]
    fn test_empty_diagnostics_all_dimensions_100() {
        let (score, label, dims) = calculate_score(&[]);
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
        let (_, _, dims) = calculate_score(&diags);
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
        let (_, _, dims) = calculate_score(&diags);
        assert_eq!(dims.dependencies, 99);
        assert_eq!(dims.security, 100);
    }

    #[test]
    fn json_destination_is_atomic_and_data_matches_compact_form() {
        let report = crate::diagnostics::ReportV1::failure(
            std::path::Path::new("/repo"),
            crate::diagnostics::ScanMode::Full,
            "scan",
            "failed".to_string(),
        );
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("report.json");
        render_json(&report, false, Some(&destination)).unwrap();
        let pretty: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&destination).unwrap()).unwrap();
        let compact: serde_json::Value =
            serde_json::from_slice(&serde_json::to_vec(&report).unwrap()).unwrap();
        assert_eq!(pretty, compact);

        let unwritable = directory.path().join("missing/report.json");
        assert!(render_json(&report, false, Some(&unwritable)).is_err());
        assert!(!unwritable.exists());
    }
}
