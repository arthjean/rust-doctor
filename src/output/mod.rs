mod score;
mod terminal;

pub use score::calculate_score;
pub(crate) use score::calculate_score_for_categories;
pub use terminal::render_terminal;
pub(crate) use terminal::{render_terminal_for_categories, render_welcome};

use crate::diagnostics::{ReportV1, ScanResult};
use owo_colors::{OwoColorize, Stream};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static ANIMATION_CANCELLATION: OnceLock<Arc<AtomicBool>> = OnceLock::new();

pub(crate) fn register_animation_cancellation(cancellation: &Arc<AtomicBool>) {
    let _ = ANIMATION_CANCELLATION.set(Arc::clone(cancellation));
}

fn animation_cancelled() -> bool {
    ANIMATION_CANCELLATION
        .get()
        .is_some_and(|cancellation| cancellation.load(Ordering::SeqCst))
}

fn animation_sleep(duration: Duration) {
    const POLL_INTERVAL: Duration = Duration::from_millis(10);
    let mut remaining = duration;
    while !remaining.is_zero() && !animation_cancelled() {
        let step = remaining.min(POLL_INTERVAL);
        std::thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
}

/// Render `--score` mode: bare integer to stdout.
pub fn render_score(result: &ScanResult) {
    if result.source_file_count == 0 {
        eprintln!(
            "{}",
            "No Rust source files found".if_supports_color(Stream::Stderr, |t| t.yellow())
        );
    } else if !crate::completeness::score_is_reportable(result) {
        eprintln!(
            "{}",
            "Score not shown: some checks could not complete."
                .if_supports_color(Stream::Stderr, |t| t.dimmed())
        );
    }
    if !result.skipped_passes.is_empty() {
        eprintln!(
            "Warning: {} pass(es) skipped (missing tools) — score may be incomplete. \
             Run: rust-doctor --install-deps",
            result.skipped_passes.len()
        );
    }
    if score_for_output(result).is_some() {
        println!("{}", result.score);
    }
}

fn score_for_output(result: &ScanResult) -> Option<u32> {
    (result.source_file_count > 0 && crate::completeness::score_is_reportable(result))
        .then_some(result.score)
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
    let serialized = if compact {
        serde_json::to_vec(report)
    } else {
        serde_json::to_vec_pretty(report)
    };
    let mut bytes = match serialized {
        Ok(bytes) => bytes,
        Err(source) if destination.is_none() => renderer_failure_bytes(report, compact, &source)?,
        Err(source) => return Err(crate::error::OutputError::Serialize(source)),
    };
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

fn renderer_failure_bytes(
    report: &ReportV1,
    compact: bool,
    source: &serde_json::Error,
) -> Result<Vec<u8>, crate::error::OutputError> {
    let fallback = ReportV1::failure(
        Path::new(&report.requested_root),
        report.mode,
        "renderer",
        &format!("failed to serialize Report V1: {source}"),
    );
    if compact {
        serde_json::to_vec(&fallback)
    } else {
        serde_json::to_vec_pretty(&fallback)
    }
    .map_err(crate::error::OutputError::Serialize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{
        Category, CheckState, CheckStatus, Diagnostic, DimensionScores, PackageExecution,
        ScanExecution, ScoreLabel, Severity,
    };
    use std::path::PathBuf;
    use std::time::Duration;

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

    fn score_result(source_file_count: usize, authoritative: bool) -> ScanResult {
        let path = PathBuf::from("src/main.rs");
        let mut execution = ScanExecution::default();
        if authoritative {
            execution.checks.push(CheckState {
                name: "custom rules".to_string(),
                required: true,
                status: CheckStatus::Completed,
                reason: None,
            });
        }
        ScanResult {
            diagnostics: Vec::new(),
            score: 93,
            score_label: ScoreLabel::Great,
            dimension_scores: DimensionScores {
                security: 100,
                reliability: 93,
                maintainability: 100,
                performance: 100,
                dependencies: 100,
            },
            source_file_count,
            elapsed: Duration::ZERO,
            skipped_passes: Vec::new(),
            error_count: 0,
            warning_count: 0,
            info_count: 0,
            pass_timings: Vec::new(),
            suppressed_security: Vec::new(),
            planned_files: authoritative.then(|| path.clone()).into_iter().collect(),
            analyzed_files: authoritative.then_some(path).into_iter().collect(),
            compiler_evidence: Vec::new(),
            execution,
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
    fn score_mode_prints_only_an_authoritative_score() {
        assert_eq!(score_for_output(&score_result(1, true)), Some(93));
        assert_eq!(score_for_output(&score_result(1, false)), None);
        assert_eq!(score_for_output(&score_result(0, false)), None);

        let mut partial_workspace = score_result(1, false);
        partial_workspace.execution.packages = vec![
            PackageExecution {
                cargo_package_id: "scored".to_string(),
                package_root: PathBuf::from("scored"),
                planned_files: vec![PathBuf::from("scored/src/lib.rs")],
                analyzed_files: vec![PathBuf::from("scored/src/lib.rs")],
                checks: vec![CheckState {
                    name: "custom rules".to_string(),
                    required: true,
                    status: CheckStatus::Completed,
                    reason: None,
                }],
                elapsed: Duration::ZERO,
                score: Some(93),
            },
            PackageExecution {
                cargo_package_id: "failed".to_string(),
                package_root: PathBuf::from("failed"),
                planned_files: vec![PathBuf::from("failed/src/lib.rs")],
                analyzed_files: vec![],
                checks: vec![CheckState {
                    name: "custom rules".to_string(),
                    required: true,
                    status: CheckStatus::Failed,
                    reason: Some("analysis failed".to_string()),
                }],
                elapsed: Duration::ZERO,
                score: None,
            },
        ];
        assert_eq!(score_for_output(&partial_workspace), Some(93));
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
    fn category_score_is_not_diluted_by_unselected_dimensions() {
        let diags = vec![
            make_diag_with_category("sec1", Severity::Error, Category::Security),
            make_diag_with_category("sec2", Severity::Error, Category::Security),
        ];
        let (score, _, dims) = calculate_score_for_categories(&diags, &[Category::Security]);
        assert_eq!(score, 97);
        assert_eq!(dims.security, 97);
    }

    #[test]
    fn category_score_ignores_unselected_categories_in_the_same_dimension() {
        let diags = vec![
            make_diag_with_category("errors", Severity::Error, Category::ErrorHandling),
            make_diag_with_category("correctness", Severity::Error, Category::Correctness),
        ];
        let (score, _, dims) = calculate_score_for_categories(&diags, &[Category::ErrorHandling]);
        assert_eq!(score, 99);
        assert_eq!(dims.reliability, 99);
    }

    #[test]
    fn category_score_preserves_weights_across_selected_dimensions() {
        let diags = vec![
            make_diag_with_category("sec1", Severity::Error, Category::Security),
            make_diag_with_category("sec2", Severity::Error, Category::Security),
            make_diag_with_category("perf1", Severity::Error, Category::Performance),
        ];
        let (score, _, _) =
            calculate_score_for_categories(&diags, &[Category::Security, Category::Performance]);
        assert_eq!(score, 98);
    }

    #[test]
    fn json_destination_is_atomic_and_data_matches_compact_form() {
        let report = crate::diagnostics::ReportV1::failure(
            std::path::Path::new("/repo"),
            crate::diagnostics::ScanMode::Full,
            "scan",
            "failed",
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

    #[test]
    fn renderer_failure_produces_a_parseable_failure_report() {
        let report = crate::diagnostics::ReportV1::failure(
            std::path::Path::new("/repo"),
            crate::diagnostics::ScanMode::Full,
            "scan",
            "failed",
        );
        let error = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let bytes = renderer_failure_bytes(&report, true, &error).unwrap();
        let fallback: crate::diagnostics::ReportV1 = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(fallback.outcome, crate::diagnostics::ReportOutcome::Failed);
        assert_eq!(fallback.error.unwrap().kind, "renderer");
    }
}
