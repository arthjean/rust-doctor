//! Versioned Score Core V2 policy artifact.
//!
//! The exact penalties, thresholds, occurrence bounds, and dimension weights
//! live in `evaluation/score-model-v2.json` and are compiled into the binary.
//! Keeping them in a reviewable artifact means a score change is a reviewed
//! data change with an explicit model identifier, not an edit buried in
//! arithmetic.

#![expect(
    clippy::redundant_pub_crate,
    reason = "the score model is consumed by sibling output modules through this private module"
)]

use crate::trust::Priority;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

/// Approved dimension weights. Changing them requires explicit product
/// approval, so any candidate model that disagrees is rejected at load time.
pub(crate) const APPROVED_WEIGHTS: DimensionWeights = DimensionWeights {
    security: 2.0,
    reliability: 1.5,
    maintainability: 1.0,
    performance: 1.0,
    dependencies: 1.0,
};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub(crate) struct DimensionWeights {
    pub(crate) security: f64,
    pub(crate) reliability: f64,
    pub(crate) maintainability: f64,
    pub(crate) performance: f64,
    pub(crate) dependencies: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct LabelThresholds {
    pub(crate) great: u32,
    pub(crate) needs_work: u32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct PriorityPenalties {
    pub(crate) p0: f64,
    pub(crate) p1: f64,
    pub(crate) p2: f64,
    pub(crate) p3: f64,
}

impl PriorityPenalties {
    pub(crate) const fn for_priority(&self, priority: Priority) -> f64 {
        match priority {
            Priority::P0 => self.p0,
            Priority::P1 => self.p1,
            Priority::P2 => self.p2,
            Priority::P3 => self.p3,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ScoreModel {
    pub(crate) schema_version: String,
    pub(crate) model_version: String,
    pub(crate) label_thresholds: LabelThresholds,
    pub(crate) priority_penalties: PriorityPenalties,
    pub(crate) occurrence_multiplier_cap: f64,
    pub(crate) p0_score_ceiling: u32,
    pub(crate) dimension_weights: DimensionWeights,
    calibration: ModelCalibration,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelCalibration {
    dataset_version: String,
    decision_dataset: String,
    decision_dataset_sha256: String,
    migration_report: String,
    migration_report_sha256: String,
}

#[derive(Debug, Deserialize)]
struct MigrationReport {
    schema_version: String,
    review_dataset_version: String,
    review_dataset_sha256: String,
    selected_model_version: String,
    candidates: Vec<MigrationCandidate>,
    passed: bool,
    reasons: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MigrationCandidate {
    model_version: String,
    projects: usize,
    holdout_projects: usize,
    band_agreement: f64,
    top_three_remediation_overlap: f64,
    monotonic: bool,
    optional_tool_invariant: bool,
    duplicate_stable: bool,
    reviewer_label_safe: bool,
}

impl ScoreModel {
    /// Total penalty for one aggregation group.
    ///
    /// A bounded-occurrence group saturates at `cap × base`: with `cap = 2` a
    /// hundred occurrences cost at most twice the first one, which is the
    /// invariant Score Core V2 promises to consumers.
    pub(crate) fn bounded_penalty(&self, base: f64, occurrences: usize) -> f64 {
        if occurrences == 0 {
            return 0.0;
        }
        let count = occurrences as f64;
        let cap = self.occurrence_multiplier_cap.max(1.0);
        base * (cap - (cap - 1.0) / count)
    }

    pub(crate) const fn label(&self, score: u32) -> crate::diagnostics::ScoreLabel {
        if score >= self.label_thresholds.great {
            crate::diagnostics::ScoreLabel::Great
        } else if score >= self.label_thresholds.needs_work {
            crate::diagnostics::ScoreLabel::NeedsWork
        } else {
            crate::diagnostics::ScoreLabel::Critical
        }
    }

    fn validate(&self) -> Result<(), String> {
        let penalties = [
            self.priority_penalties.p0,
            self.priority_penalties.p1,
            self.priority_penalties.p2,
            self.priority_penalties.p3,
        ];
        if self.schema_version != "1.0" {
            return Err(format!(
                "unsupported score model schema '{}'",
                self.schema_version
            ));
        }
        if self.dimension_weights != APPROVED_WEIGHTS {
            return Err("candidate score model changes the approved dimension weights".to_string());
        }
        if !self.occurrence_multiplier_cap.is_finite() || self.occurrence_multiplier_cap < 1.0 {
            return Err("occurrence multiplier cap must be at least 1.0".to_string());
        }
        if penalties
            .iter()
            .any(|penalty| !penalty.is_finite() || *penalty < 0.0)
        {
            return Err("priority penalties must be finite and non-negative".to_string());
        }
        if self.label_thresholds.needs_work >= self.label_thresholds.great {
            return Err("label thresholds must be strictly ordered".to_string());
        }
        if self.label_thresholds.great > 100 {
            return Err("label thresholds must stay inside the 0-100 score range".to_string());
        }
        if self.p0_score_ceiling >= self.label_thresholds.great {
            return Err(
                "a confirmed P0 finding must keep the score below the Great threshold".to_string(),
            );
        }
        if self.calibration.dataset_version != "decision-quality-v1"
            || self.calibration.decision_dataset != "evaluation/decision-quality-v1.json"
            || self.calibration.decision_dataset_sha256 != decision_dataset_sha256()
        {
            return Err(
                "score model is stale relative to the checked decision-quality dataset".to_string(),
            );
        }
        self.validate_migration_report()?;
        Ok(())
    }

    fn validate_migration_report(&self) -> Result<(), String> {
        if self.calibration.migration_report != "evaluation/score-model-migration-v2.1.json"
            || self.calibration.migration_report_sha256 != migration_report_sha256()
        {
            return Err(
                "score model is stale relative to the checked migration report".to_string(),
            );
        }
        let report: MigrationReport = serde_json::from_str(MIGRATION_REPORT_SOURCE)
            .map_err(|error| format!("score migration report is invalid: {error}"))?;
        let candidate = report
            .candidates
            .iter()
            .find(|candidate| candidate.model_version == self.model_version)
            .ok_or_else(|| "score migration report has no selected-model candidate".to_string())?;
        if report.schema_version != "1.0"
            || report.review_dataset_version != self.calibration.dataset_version
            || report.review_dataset_sha256 != self.calibration.decision_dataset_sha256
            || report.selected_model_version != self.model_version
            || !report.passed
            || !report.reasons.is_empty()
            || candidate.projects < 60
            || candidate.holdout_projects * 100 < candidate.projects * 20
            || candidate.band_agreement < 0.90
            || candidate.top_three_remediation_overlap < 0.90
            || !candidate.monotonic
            || !candidate.optional_tool_invariant
            || !candidate.duplicate_stable
            || !candidate.reviewer_label_safe
        {
            return Err("score migration report does not certify the selected model".to_string());
        }
        Ok(())
    }
}

const SCORE_MODEL_SOURCE: &str = include_str!("../../evaluation/score-model-v2.json");
const DECISION_DATASET_SOURCE: &str = include_str!("../../evaluation/decision-quality-v1.json");
const MIGRATION_REPORT_SOURCE: &str =
    include_str!("../../evaluation/score-model-migration-v2.1.json");

fn decision_dataset_sha256() -> String {
    format!("{:x}", Sha256::digest(DECISION_DATASET_SOURCE.as_bytes()))
}

fn migration_report_sha256() -> String {
    format!("{:x}", Sha256::digest(MIGRATION_REPORT_SOURCE.as_bytes()))
}

static SCORE_MODEL: LazyLock<Result<ScoreModel, String>> = LazyLock::new(|| {
    let model: ScoreModel = serde_json::from_str(SCORE_MODEL_SOURCE)
        .map_err(|error| format!("score model artifact is invalid: {error}"))?;
    model.validate()?;
    Ok(model)
});

/// The active, validated score model.
pub(crate) fn score_model() -> Option<&'static ScoreModel> {
    SCORE_MODEL.as_ref().ok()
}

/// Reject a scan before it can publish a synthetic value from a missing,
/// malformed, or stale checked model.
pub(crate) fn require_score_model() -> Result<(), String> {
    SCORE_MODEL
        .as_ref()
        .map(|_| ())
        .map_err(std::clone::Clone::clone)
}

/// Model identifier carried by reports, caches, and baselines.
pub(super) fn model_version() -> &'static str {
    score_model().map_or("unknown", |model| model.model_version.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checked_model_loads_and_keeps_the_approved_weights() {
        let model = score_model().expect("score model must load");
        assert_eq!(model.model_version, "2.1");
        assert_eq!(model.dimension_weights, APPROVED_WEIGHTS);
    }

    #[test]
    fn bounded_occurrences_saturate_at_the_multiplier_cap() {
        let model = score_model().expect("score model must load");
        let base = model.priority_penalties.p2;
        let first = model.bounded_penalty(base, 1);
        let hundred = model.bounded_penalty(base, 100);
        assert!((first - base).abs() < f64::EPSILON);
        assert!(hundred <= base.mul_add(2.0, f64::EPSILON));
        assert!(hundred > model.bounded_penalty(base, 2));
    }

    #[test]
    fn a_model_that_changes_dimension_weights_is_rejected() {
        let mut model: ScoreModel =
            serde_json::from_str(SCORE_MODEL_SOURCE).expect("artifact parses");
        model.dimension_weights.security = 3.0;
        assert!(model.validate().is_err());
    }

    #[test]
    fn a_model_that_lets_p0_stay_great_is_rejected() {
        let mut model: ScoreModel =
            serde_json::from_str(SCORE_MODEL_SOURCE).expect("artifact parses");
        model.p0_score_ceiling = model.label_thresholds.great;
        assert!(model.validate().is_err());
    }

    #[test]
    fn the_model_binds_the_checked_project_review_dataset() {
        let model = score_model().expect("score model must load");
        assert_eq!(
            model.calibration.decision_dataset_sha256,
            decision_dataset_sha256()
        );
    }

    #[test]
    fn the_model_binds_a_passing_migration_report() {
        let model = score_model().expect("score model must load");
        assert_eq!(
            model.calibration.migration_report_sha256,
            migration_report_sha256()
        );
        assert!(model.validate_migration_report().is_ok());
    }
}
