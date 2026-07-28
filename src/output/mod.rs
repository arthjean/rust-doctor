mod prompt_theme;
mod score;
mod score_model;
mod terminal;

pub(crate) use prompt_theme::PromptTheme;
pub(crate) use score::Dimension;
#[cfg(test)]
pub(crate) use score::canonical_penalties;
pub(crate) use score::group_score_contribution;
pub use score::{calculate_score, calculate_score_for_categories};
pub(crate) use score_model::require_score_model;

/// Score Core identifier carried by reports, caches, and baselines.
///
/// Legacy values were produced by an unversioned model and are not comparable
/// with Score Core V2 output.
#[must_use]
pub fn score_model_version() -> &'static str {
    score_model::model_version()
}
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
    let decision = crate::completeness::score_decision(result);
    if decision.visibility == crate::completeness::ScoreVisibility::Absent {
        eprintln!(
            "{}",
            "No Rust source files found".if_supports_color(Stream::Stderr, |t| t.yellow())
        );
    } else if decision.published_score().is_none() {
        eprintln!(
            "{}",
            format!("Score not shown: {}.", decision.primary_reason())
                .if_supports_color(Stream::Stderr, |t| t.dimmed())
        );
    }
    if !result.skipped_passes.is_empty() && decision.published_score().is_some() {
        eprintln!(
            "Warning: {} optional pass(es) skipped; the authoritative Core Score excludes them. \
             Run: rust-doctor --install-deps",
            result.skipped_passes.len()
        );
    }
    if let Some(score) = decision.published_score() {
        println!("{score}");
    }
}

#[cfg(test)]
fn score_for_output(result: &ScanResult) -> Option<u32> {
    crate::completeness::score_decision(result).published_score()
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
            execution
                .checks
                .extend(["custom rules", "msrv"].into_iter().map(|name| CheckState {
                    name: name.to_string(),
                    required: true,
                    status: CheckStatus::Completed,
                    reason: None,
                }));
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
        let (score, label, dims) = calculate_score(&[]).expect("embedded score model");
        assert_eq!(score, 100);
        assert_eq!(label, ScoreLabel::Great);
        assert_eq!(dims.security, 100);
        assert_eq!(dims.reliability, 100);
        assert_eq!(dims.maintainability, 100);
        assert_eq!(dims.performance, 100);
        assert_eq!(dims.dependencies, 100);
    }

    // --- Score Core V2 dimension mapping ---
    //
    // Every case below uses a real catalog rule: under Score Core V2 an unknown
    // identifier is score-ineligible by contract, so a fabricated rule name
    // would silently assert nothing.

    #[test]
    fn security_rules_only_degrade_the_security_dimension() {
        let diags = vec![make_diag_with_category(
            "clippy::transmute_ptr_to_ref",
            Severity::Error,
            Category::Security,
        )];
        let (_, _, dims) = calculate_score(&diags).expect("embedded score model");
        assert!(dims.security < 100);
        assert_eq!(dims.reliability, 100);
        assert_eq!(dims.maintainability, 100);
        assert_eq!(dims.performance, 100);
        assert_eq!(dims.dependencies, 100);
    }

    #[test]
    fn cargo_and_dependency_categories_share_the_dependencies_dimension() {
        let diags = vec![
            make_diag_with_category("missing-msrv", Severity::Warning, Category::Cargo),
            make_diag_with_category("msrv-outdated", Severity::Warning, Category::Dependencies),
        ];
        let (_, _, dims) = calculate_score(&diags).expect("embedded score model");
        assert!(dims.dependencies < 100);
        assert_eq!(dims.security, 100);
    }

    #[test]
    fn repeated_occurrences_of_one_rule_stay_bounded() {
        let single = vec![make_diag_with_category(
            "missing-msrv",
            Severity::Warning,
            Category::Cargo,
        )];
        let many: Vec<_> = (0..50)
            .map(|_| make_diag_with_category("missing-msrv", Severity::Warning, Category::Cargo))
            .collect();
        let (_, _, single_dims) = calculate_score(&single).expect("embedded score model");
        let (_, _, many_dims) = calculate_score(&many).expect("embedded score model");
        let single_penalty = 100 - single_dims.dependencies;
        let many_penalty = 100 - many_dims.dependencies;
        assert!(many_penalty >= single_penalty);
        assert!(many_penalty <= single_penalty * 2);
    }

    #[test]
    fn a_p0_finding_caps_the_overall_label() {
        let diags = vec![make_diag_with_category(
            "compiler-error",
            Severity::Error,
            Category::Security,
        )];
        let (score, label, _) = calculate_score(&diags).expect("embedded score model");
        assert!(score <= 49);
        assert_eq!(label, ScoreLabel::Critical);
    }

    #[test]
    fn category_selection_is_not_diluted_by_unselected_dimensions() {
        let diags = vec![make_diag_with_category(
            "clippy::transmute_ptr_to_ref",
            Severity::Error,
            Category::Security,
        )];
        let (scoped, _, dims) = calculate_score_for_categories(&diags, &[Category::Security])
            .expect("embedded score model");
        assert_eq!(scoped, dims.security.min(49));
    }

    #[test]
    fn category_selection_ignores_sibling_categories_in_one_dimension() {
        let diags = vec![
            make_diag_with_category(
                "clippy::unwrap_used",
                Severity::Warning,
                Category::ErrorHandling,
            ),
            make_diag_with_category("compiler-error", Severity::Error, Category::Correctness),
        ];
        let (scoped, _, _) = calculate_score_for_categories(&diags, &[Category::ErrorHandling])
            .expect("embedded score model");
        let (full, _, _) = calculate_score(&diags).expect("embedded score model");
        assert!(scoped > full);
    }

    #[test]
    fn test_score_label_thresholds() {
        use score::score_label;
        assert_eq!(score_label(100), ScoreLabel::Great);
        assert_eq!(score_label(95), ScoreLabel::Great);
        assert_eq!(score_label(94), ScoreLabel::NeedsWork);
        assert_eq!(score_label(50), ScoreLabel::NeedsWork);
        assert_eq!(score_label(49), ScoreLabel::Critical);
        assert_eq!(score_label(0), ScoreLabel::Critical);
    }

    #[test]
    fn test_empty_diagnostics_all_dimensions_100() {
        let (score, label, dims) = calculate_score(&[]).expect("embedded score model");
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
        assert_eq!(score_for_output(&partial_workspace), None);
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
