//! Independent project-level score-model evaluation.
//!
//! The checked review set contains no repository identity, path, message, or
//! source fragment. It is derived from the hash-bound protected corpus and an
//! explicit priority rubric that is independent from numeric score penalties.

#![expect(
    clippy::cast_precision_loss,
    reason = "project and occurrence counts stay far below the f64 mantissa"
)]

use super::manifest::{hex_digest, read_json, sha256_file, write_json_atomic};
use super::model::{CorpusRecord, EvidenceApproval};
use super::{EvalError, Result};
use crate::DecisionQualityArgs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

const REVIEW_SCHEMA_VERSION: &str = "1.0";
const REPORT_SCHEMA_VERSION: &str = "1.0";
const REVIEW_METHOD: &str = "independent-project-review-v1";
const INDEPENDENT_REVIEWER: &str = "independent-project-review-v1";
const MIN_PROJECTS: usize = 60;
const MIN_HOLDOUT_PERCENT: usize = 20;
const MIN_AGREEMENT: f64 = 0.90;
const APPROVED_CORPUS_SOURCE: &str =
    include_str!("../../../evaluation/approvals/corpus-baseline.json");
const ANCHOR_SPECS: [(&str, bool, &[&str]); 6] = [
    ("great-development", false, &[]),
    ("great-holdout", true, &[]),
    (
        "needs-development",
        false,
        &["clippy::await_holding_lock", "clippy::invalid_regex"],
    ),
    (
        "needs-holdout",
        true,
        &["clippy::almost_swapped", "clippy::eq_op"],
    ),
    ("critical-development", false, &["compiler-error"]),
    ("critical-holdout", true, &["msrv-incompatible"]),
];

#[derive(Debug, Clone, Deserialize)]
struct CatalogEntry {
    canonical_id: String,
    category: String,
    trust: CatalogTrust,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogTrust {
    priority: Option<String>,
    contributes_to_core_score: bool,
    aggregation: String,
    analyzer_availability: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct ReviewedGroup {
    rule: String,
    category: String,
    priority: String,
    aggregation: String,
    occurrences: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ProjectReview {
    project_id: String,
    repository_cluster: String,
    repository_root: String,
    language_context: String,
    framework_context: String,
    fixture_provenance: String,
    author: String,
    independent_reviewer: String,
    disposition: String,
    holdout: bool,
    reviewed_health_band: String,
    top_remediations: Vec<String>,
    groups: Vec<ReviewedGroup>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DecisionReviews {
    schema_version: String,
    dataset_version: String,
    source_corpus_sha256: String,
    review_method: String,
    reviewed_at: String,
    projects: Vec<ProjectReview>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct LabelThresholds {
    great: u32,
    needs_work: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
struct DimensionWeights {
    security: f64,
    reliability: f64,
    maintainability: f64,
    performance: f64,
    dependencies: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct PriorityPenalties {
    p0: f64,
    p1: f64,
    p2: f64,
    p3: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelCalibration {
    dataset_version: String,
    #[serde(default)]
    decision_dataset_sha256: String,
    #[serde(default)]
    migration_report: String,
    #[serde(default)]
    migration_report_sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ScoreModel {
    schema_version: String,
    model_version: String,
    label_thresholds: LabelThresholds,
    priority_penalties: PriorityPenalties,
    occurrence_multiplier_cap: f64,
    p0_score_ceiling: u32,
    dimension_weights: DimensionWeights,
    calibration: ModelCalibration,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the serialized gate artifact records independent pass/fail invariants as explicit booleans"
)]
struct CandidateMetrics {
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

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MigrationProject {
    project_id: String,
    holdout: bool,
    previous_score: u32,
    selected_score: u32,
    previous_band: String,
    selected_band: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DecisionQualityReport {
    schema_version: String,
    review_dataset_version: String,
    review_dataset_sha256: String,
    source_corpus_sha256: String,
    selected_model_version: String,
    candidates: Vec<CandidateMetrics>,
    migration: Vec<MigrationProject>,
    passed: bool,
    reasons: Vec<String>,
}

pub(crate) fn run(args: &DecisionQualityArgs) -> Result<()> {
    let records = read_corpus(&args.corpus)?;
    validate_approval(&args.corpus, &args.corpus_approval)?;
    let catalog = read_catalog(&args.binary)?;
    if args.generate_reviews {
        let reviewed_at = args.reviewed_at.as_deref().ok_or_else(|| {
            EvalError::InvalidManifest(
                "--reviewed-at is required with --generate-reviews".to_string(),
            )
        })?;
        let mut reviews = generate_reviews(&records, &catalog, reviewed_at)?;
        reviews.source_corpus_sha256 = sha256_file(&args.corpus)?;
        return write_json_atomic(&args.reviews, &reviews);
    }

    let current_path = required_path(args.model.as_deref(), "--model")?;
    let previous_path = required_path(args.previous_model.as_deref(), "--previous-model")?;
    let output = required_path(args.output.as_deref(), "--output")?;
    let reviews: DecisionReviews = read_json(&args.reviews)?;
    if reviews.source_corpus_sha256 != sha256_file(&args.corpus)? {
        return Err(EvalError::InvalidManifest(
            "decision reviews are stale relative to the approved corpus".to_string(),
        ));
    }
    validate_reviews(&reviews, &records, &catalog)?;
    let review_sha256 = sha256_file(&args.reviews)?;
    let current: ScoreModel = read_json(current_path)?;
    let previous: ScoreModel = read_json(previous_path)?;
    validate_model(&current, &reviews, &review_sha256, true)?;
    validate_model(&previous, &reviews, &review_sha256, false)?;
    if current.model_version == previous.model_version {
        return Err(EvalError::InvalidManifest(
            "selected score model must increment the previous model version".to_string(),
        ));
    }

    let previous_metrics = measure_candidate(&previous, &reviews.projects, &catalog);
    let selected_metrics = measure_candidate(&current, &reviews.projects, &catalog);
    let migration = reviews
        .projects
        .iter()
        .map(|project| {
            let (previous_score, previous_band) = score_project(&previous, project, &catalog);
            let (selected_score, selected_band) = score_project(&current, project, &catalog);
            MigrationProject {
                project_id: project.project_id.clone(),
                holdout: project.holdout,
                previous_score,
                selected_score,
                previous_band,
                selected_band,
            }
        })
        .collect();

    let mut reasons = Vec::new();
    if selected_metrics.band_agreement < MIN_AGREEMENT {
        reasons.push(format!(
            "held-out band agreement {:.3} is below {MIN_AGREEMENT:.3}",
            selected_metrics.band_agreement
        ));
    }
    if selected_metrics.top_three_remediation_overlap < MIN_AGREEMENT {
        reasons.push(format!(
            "held-out top-three overlap {:.3} is below {MIN_AGREEMENT:.3}",
            selected_metrics.top_three_remediation_overlap
        ));
    }
    if !selected_metrics.monotonic {
        reasons.push("selected model is not monotonic".to_string());
    }
    if !selected_metrics.optional_tool_invariant {
        reasons.push("optional diagnostics move Core Score".to_string());
    }
    if !selected_metrics.duplicate_stable {
        reasons.push("bounded duplicate occurrences exceed the model cap".to_string());
    }
    if !selected_metrics.reviewer_label_safe {
        reasons.push("selected model reports Great below an independent Great label".to_string());
    }
    let report = DecisionQualityReport {
        schema_version: REPORT_SCHEMA_VERSION.to_string(),
        review_dataset_version: reviews.dataset_version,
        review_dataset_sha256: review_sha256,
        source_corpus_sha256: reviews.source_corpus_sha256,
        selected_model_version: current.model_version,
        candidates: vec![previous_metrics, selected_metrics],
        migration,
        passed: reasons.is_empty(),
        reasons: reasons.clone(),
    };
    write_json_atomic(output, &report)?;
    if current.calibration.migration_report_sha256 != sha256_file(output)? {
        return Err(EvalError::GateFailed(
            "generated migration report differs from the digest bound by the selected model"
                .to_string(),
        ));
    }
    if reasons.is_empty() {
        Ok(())
    } else {
        Err(EvalError::GateFailed(reasons.join("; ")))
    }
}

fn required_path<'a>(path: Option<&'a Path>, flag: &str) -> Result<&'a Path> {
    path.ok_or_else(|| {
        EvalError::InvalidManifest(format!(
            "{flag} is required unless --generate-reviews is used"
        ))
    })
}

fn read_corpus(path: &Path) -> Result<Vec<CorpusRecord>> {
    let content =
        std::fs::read_to_string(path).map_err(|error| EvalError::io("cannot read", path, error))?;
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).map_err(|source| EvalError::Json {
                path: path.to_path_buf(),
                source,
            })
        })
        .collect()
}

fn validate_approval(corpus: &Path, approval_path: &Path) -> Result<()> {
    let approval: EvidenceApproval = read_json(approval_path)?;
    let checked: EvidenceApproval =
        serde_json::from_str(APPROVED_CORPUS_SOURCE).map_err(|source| EvalError::Json {
            path: PathBuf::from("evaluation/approvals/corpus-baseline.json"),
            source,
        })?;
    let actual = sha256_file(corpus)?;
    if approval != checked || approval.subject_sha256 != actual {
        return Err(EvalError::InvalidManifest(
            "corpus approval does not match the checked protected-CI attestation".to_string(),
        ));
    }
    Ok(())
}

fn read_catalog(binary: &Path) -> Result<BTreeMap<String, CatalogEntry>> {
    let output = Command::new(binary)
        .args(["rules", "list", "--json"])
        .output()
        .map_err(|error| EvalError::io("cannot execute rust-doctor", binary, error))?;
    if !output.status.success() {
        return Err(EvalError::Command(format!(
            "'{} rules list --json' exited with {}",
            binary.display(),
            output.status
        )));
    }
    let entries: Vec<CatalogEntry> =
        serde_json::from_slice(&output.stdout).map_err(|source| EvalError::Json {
            path: binary.to_path_buf(),
            source,
        })?;
    Ok(entries
        .into_iter()
        .map(|entry| (entry.canonical_id.clone(), entry))
        .collect())
}

fn generate_reviews(
    records: &[CorpusRecord],
    catalog: &BTreeMap<String, CatalogEntry>,
    reviewed_at: &str,
) -> Result<DecisionReviews> {
    let mut complete: Vec<_> = records.iter().filter(|record| record.complete).collect();
    complete.sort_by(|left, right| left.repository.cmp(&right.repository));
    if complete.len() < MIN_PROJECTS {
        return Err(EvalError::GateFailed(format!(
            "protected corpus has {} complete projects, expected at least {MIN_PROJECTS}",
            complete.len()
        )));
    }
    let projects = complete
        .into_iter()
        .take(MIN_PROJECTS)
        .enumerate()
        .map(|(index, record)| corpus_project_evidence(index, record, catalog))
        .chain(anchor_projects(catalog)?)
        .collect();
    Ok(DecisionReviews {
        schema_version: REVIEW_SCHEMA_VERSION.to_string(),
        dataset_version: "decision-quality-v1".to_string(),
        source_corpus_sha256: String::new(),
        review_method: REVIEW_METHOD.to_string(),
        reviewed_at: reviewed_at.to_string(),
        projects,
    })
}

fn corpus_project_evidence(
    index: usize,
    record: &CorpusRecord,
    catalog: &BTreeMap<String, CatalogEntry>,
) -> ProjectReview {
    let identity = hex_digest(format!("{}\0{}", record.repository, record.commit).as_bytes());
    let mut groups = Vec::new();
    for (rule, occurrences) in &record.per_rule_counts {
        let Some(entry) = catalog.get(rule) else {
            continue;
        };
        if !entry.trust.contributes_to_core_score || *occurrences == 0 {
            continue;
        }
        let Some(priority) = entry.trust.priority.clone() else {
            continue;
        };
        groups.push(ReviewedGroup {
            rule: rule.clone(),
            category: entry.category.clone(),
            priority,
            aggregation: entry.trust.aggregation.clone(),
            occurrences: *occurrences,
        });
    }
    groups.sort_by(review_group_order);
    ProjectReview {
        project_id: identity.clone(),
        repository_cluster: identity.clone(),
        repository_root: identity,
        language_context: "rust".to_string(),
        framework_context: "mixed-or-none".to_string(),
        fixture_provenance: "protected-public-corpus".to_string(),
        author: "upstream-public-project".to_string(),
        independent_reviewer: "unassigned".to_string(),
        disposition: "unreviewed".to_string(),
        holdout: index.is_multiple_of(5),
        reviewed_health_band: "unreviewed".to_string(),
        top_remediations: Vec::new(),
        groups,
    }
}

fn anchor_projects(catalog: &BTreeMap<String, CatalogEntry>) -> Result<Vec<ProjectReview>> {
    ANCHOR_SPECS
        .iter()
        .map(|(name, holdout, rules)| {
            let identity = hex_digest(format!("decision-anchor-v1\0{name}").as_bytes());
            let mut groups = rules
                .iter()
                .map(|rule| reviewed_group(rule, catalog))
                .collect::<Result<Vec<_>>>()?;
            groups.sort_by(review_group_order);
            Ok(ProjectReview {
                project_id: identity.clone(),
                repository_cluster: identity.clone(),
                repository_root: identity,
                language_context: "rust".to_string(),
                framework_context: "controlled-contract-fixture".to_string(),
                fixture_provenance: "reviewed-contract-fixture".to_string(),
                author: "rust-doctor-contract-fixture-v1".to_string(),
                independent_reviewer: "unassigned".to_string(),
                disposition: "unreviewed".to_string(),
                holdout: *holdout,
                reviewed_health_band: "unreviewed".to_string(),
                top_remediations: Vec::new(),
                groups,
            })
        })
        .collect()
}

fn reviewed_group(rule: &str, catalog: &BTreeMap<String, CatalogEntry>) -> Result<ReviewedGroup> {
    let entry = catalog.get(rule).ok_or_else(|| {
        EvalError::InvalidManifest(format!(
            "decision benchmark anchor references unknown rule '{rule}'"
        ))
    })?;
    if !entry.trust.contributes_to_core_score {
        return Err(EvalError::InvalidManifest(format!(
            "decision benchmark anchor rule '{rule}' is not score-authoritative"
        )));
    }
    let priority = entry.trust.priority.clone().ok_or_else(|| {
        EvalError::InvalidManifest(format!(
            "decision benchmark anchor rule '{rule}' has no priority"
        ))
    })?;
    Ok(ReviewedGroup {
        rule: rule.to_string(),
        category: entry.category.clone(),
        priority,
        aggregation: entry.trust.aggregation.clone(),
        occurrences: 1,
    })
}

fn review_group_order(left: &ReviewedGroup, right: &ReviewedGroup) -> std::cmp::Ordering {
    priority_rank(&left.priority)
        .cmp(&priority_rank(&right.priority))
        .then_with(|| right.occurrences.cmp(&left.occurrences))
        .then_with(|| left.rule.cmp(&right.rule))
}

fn priority_rank(priority: &str) -> u8 {
    match priority {
        "p0" => 0,
        "p1" => 1,
        "p2" => 2,
        "p3" => 3,
        _ => 4,
    }
}

fn validate_reviews(
    reviews: &DecisionReviews,
    records: &[CorpusRecord],
    catalog: &BTreeMap<String, CatalogEntry>,
) -> Result<()> {
    if reviews.schema_version != REVIEW_SCHEMA_VERSION
        || reviews.projects.len() != MIN_PROJECTS + ANCHOR_SPECS.len()
        || reviews.review_method != REVIEW_METHOD
        || reviews.reviewed_at.is_empty()
    {
        return Err(EvalError::InvalidManifest(
            "decision review artifact has an unsupported shape or insufficient projects"
                .to_string(),
        ));
    }
    let mut complete: Vec<_> = records.iter().filter(|record| record.complete).collect();
    complete.sort_by(|left, right| left.repository.cmp(&right.repository));
    let expected_corpus: BTreeMap<_, _> = complete
        .into_iter()
        .take(MIN_PROJECTS)
        .enumerate()
        .map(|record| {
            let (index, record) = record;
            (
                hex_digest(format!("{}\0{}", record.repository, record.commit).as_bytes()),
                corpus_project_evidence(index, record, catalog),
            )
        })
        .collect();
    let expected_anchors: BTreeMap<_, _> = anchor_projects(catalog)?
        .into_iter()
        .map(|project| (project.project_id.clone(), project))
        .collect();
    let mut identities = BTreeSet::new();
    let mut development_roots = BTreeSet::new();
    let mut holdout_roots = BTreeSet::new();
    let mut holdout_bands = BTreeSet::new();
    let mut holdout_remediations = 0usize;
    for (index, project) in reviews.projects.iter().enumerate() {
        if !identities.insert(project.project_id.as_str())
            || project.project_id != project.repository_cluster
            || project.project_id != project.repository_root
            || !is_lower_hex_sha256(&project.project_id)
            || project.independent_reviewer != INDEPENDENT_REVIEWER
            || project.author == INDEPENDENT_REVIEWER
            || project.disposition != "accepted"
            || project.language_context != "rust"
            || !matches!(
                project.fixture_provenance.as_str(),
                "protected-public-corpus" | "reviewed-contract-fixture"
            )
            || !matches!(
                project.reviewed_health_band.as_str(),
                "great" | "needs-work" | "critical"
            )
        {
            return Err(EvalError::InvalidManifest(format!(
                "project review {index} has invalid identity, provenance, or independence"
            )));
        }
        let expected = expected_corpus
            .get(&project.project_id)
            .or_else(|| expected_anchors.get(&project.project_id))
            .ok_or_else(|| {
                EvalError::InvalidManifest(format!(
                    "project review {} is absent from approved corpus evidence and contract anchors",
                    project.project_id
                ))
            })?;
        if !same_project_evidence(project, expected) || !valid_remediations(project) {
            return Err(EvalError::InvalidManifest(format!(
                "project review {} diverges from its approved evidence or remediation set",
                project.project_id
            )));
        }
        if project.holdout {
            holdout_roots.insert(project.repository_root.as_str());
            holdout_bands.insert(project.reviewed_health_band.as_str());
            holdout_remediations += project.top_remediations.len();
        } else {
            development_roots.insert(project.repository_root.as_str());
        }
    }
    if !development_roots.is_disjoint(&holdout_roots) {
        return Err(EvalError::InvalidManifest(
            "development and holdout repository roots overlap".to_string(),
        ));
    }
    if holdout_roots.len() * 100 < reviews.projects.len() * MIN_HOLDOUT_PERCENT {
        return Err(EvalError::InvalidManifest(format!(
            "holdout contains {} of {} projects, expected at least {MIN_HOLDOUT_PERCENT}%",
            holdout_roots.len(),
            reviews.projects.len()
        )));
    }
    if holdout_bands.len() != 3 || holdout_remediations == 0 {
        return Err(EvalError::InvalidManifest(
            "holdout must contain Great, Needs Work, Critical, and reviewed remediations"
                .to_string(),
        ));
    }
    Ok(())
}

fn same_project_evidence(actual: &ProjectReview, expected: &ProjectReview) -> bool {
    actual.project_id == expected.project_id
        && actual.repository_cluster == expected.repository_cluster
        && actual.repository_root == expected.repository_root
        && actual.language_context == expected.language_context
        && actual.framework_context == expected.framework_context
        && actual.fixture_provenance == expected.fixture_provenance
        && actual.author == expected.author
        && actual.holdout == expected.holdout
        && actual.groups == expected.groups
}

fn valid_remediations(project: &ProjectReview) -> bool {
    if project.top_remediations.len() > 3 {
        return false;
    }
    let available: BTreeSet<_> = project
        .groups
        .iter()
        .map(|group| group.rule.as_str())
        .collect();
    let selected: BTreeSet<_> = project
        .top_remediations
        .iter()
        .map(String::as_str)
        .collect();
    selected.len() == project.top_remediations.len()
        && selected.iter().all(|rule| available.contains(rule))
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_model(
    model: &ScoreModel,
    reviews: &DecisionReviews,
    review_sha256: &str,
    selected: bool,
) -> Result<()> {
    let approved = DimensionWeights {
        security: 2.0,
        reliability: 1.5,
        maintainability: 1.0,
        performance: 1.0,
        dependencies: 1.0,
    };
    let penalties = [
        model.priority_penalties.p0,
        model.priority_penalties.p1,
        model.priority_penalties.p2,
        model.priority_penalties.p3,
    ];
    if model.schema_version != "1.0"
        || model.model_version.is_empty()
        || model.dimension_weights != approved
        || model.label_thresholds.needs_work >= model.label_thresholds.great
        || model.label_thresholds.great > 100
        || model.p0_score_ceiling >= model.label_thresholds.great
        || !model.occurrence_multiplier_cap.is_finite()
        || model.occurrence_multiplier_cap < 1.0
        || penalties
            .iter()
            .any(|penalty| !penalty.is_finite() || *penalty < 0.0)
    {
        return Err(EvalError::InvalidManifest(format!(
            "score model {} violates the approved score contract",
            model.model_version
        )));
    }
    if selected
        && (model.calibration.dataset_version != reviews.dataset_version
            || model.calibration.decision_dataset_sha256 != review_sha256)
    {
        return Err(EvalError::InvalidManifest(
            "selected score model is stale relative to the reviewed project dataset".to_string(),
        ));
    }
    if selected {
        validate_migration_evidence(model, reviews, review_sha256)?;
    }
    Ok(())
}

fn validate_migration_evidence(
    model: &ScoreModel,
    reviews: &DecisionReviews,
    review_sha256: &str,
) -> Result<()> {
    let path = Path::new(&model.calibration.migration_report);
    if model.calibration.migration_report != "evaluation/score-model-migration-v2.1.json"
        || model.calibration.migration_report_sha256 != sha256_file(path)?
    {
        return Err(EvalError::InvalidManifest(
            "selected score model is stale relative to its migration report".to_string(),
        ));
    }
    let report: DecisionQualityReport = read_json(path)?;
    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.model_version == model.model_version)
        .ok_or_else(|| {
            EvalError::InvalidManifest(
                "migration report has no candidate for the selected score model".to_string(),
            )
        })?;
    if report.schema_version != REPORT_SCHEMA_VERSION
        || report.review_dataset_version != reviews.dataset_version
        || report.review_dataset_sha256 != review_sha256
        || report.source_corpus_sha256 != reviews.source_corpus_sha256
        || report.selected_model_version != model.model_version
        || !report.passed
        || !report.reasons.is_empty()
        || candidate.projects != reviews.projects.len()
        || candidate.holdout_projects * 100 < reviews.projects.len() * MIN_HOLDOUT_PERCENT
        || candidate.band_agreement < MIN_AGREEMENT
        || candidate.top_three_remediation_overlap < MIN_AGREEMENT
        || !candidate.monotonic
        || !candidate.optional_tool_invariant
        || !candidate.duplicate_stable
        || !candidate.reviewer_label_safe
    {
        return Err(EvalError::InvalidManifest(
            "migration report does not certify the selected score model".to_string(),
        ));
    }
    Ok(())
}

fn measure_candidate(
    model: &ScoreModel,
    projects: &[ProjectReview],
    catalog: &BTreeMap<String, CatalogEntry>,
) -> CandidateMetrics {
    let holdout: Vec<_> = projects.iter().filter(|project| project.holdout).collect();
    let band_matches = holdout
        .iter()
        .filter(|project| score_project(model, project, catalog).1 == project.reviewed_health_band)
        .count();
    let (overlap, reviewed) = holdout.iter().fold((0usize, 0usize), |acc, project| {
        let predicted = predicted_remediations(model, project, catalog);
        (
            acc.0
                + project
                    .top_remediations
                    .iter()
                    .filter(|rule| predicted.contains(rule))
                    .count(),
            acc.1 + project.top_remediations.len(),
        )
    });
    CandidateMetrics {
        model_version: model.model_version.clone(),
        projects: projects.len(),
        holdout_projects: holdout.len(),
        band_agreement: ratio_or_zero(band_matches, holdout.len()),
        top_three_remediation_overlap: ratio_or_zero(overlap, reviewed),
        monotonic: projects
            .iter()
            .all(|project| monotonic(model, project, catalog)),
        optional_tool_invariant: optional_tool_invariant(model, projects, catalog),
        duplicate_stable: duplicate_stable(model),
        reviewer_label_safe: projects.iter().all(|project| {
            project.reviewed_health_band == "great"
                || score_project(model, project, catalog).1 != "great"
        }),
    }
}

fn ratio_or_zero(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn optional_tool_invariant(
    model: &ScoreModel,
    projects: &[ProjectReview],
    catalog: &BTreeMap<String, CatalogEntry>,
) -> bool {
    let optional_rules: Vec<_> = catalog
        .values()
        .filter(|entry| entry.trust.analyzer_availability == "optional-external")
        .filter_map(|entry| {
            let priority = entry.trust.priority.clone()?;
            Some(ReviewedGroup {
                rule: entry.canonical_id.clone(),
                category: entry.category.clone(),
                priority,
                aggregation: entry.trust.aggregation.clone(),
                occurrences: 10_000,
            })
        })
        .collect();
    !optional_rules.is_empty()
        && projects.iter().all(|project| {
            let baseline = score_project(model, project, catalog);
            optional_rules.iter().all(|optional| {
                let mut with_optional = project.clone();
                with_optional.groups.push(optional.clone());
                score_project(model, &with_optional, catalog) == baseline
            })
        })
}

fn monotonic(
    model: &ScoreModel,
    project: &ProjectReview,
    catalog: &BTreeMap<String, CatalogEntry>,
) -> bool {
    let baseline = score_project(model, project, catalog).0;
    project.groups.iter().all(|group| {
        let mut changed = project.clone();
        if let Some(target) = changed
            .groups
            .iter_mut()
            .find(|item| item.rule == group.rule)
        {
            target.occurrences = target.occurrences.saturating_add(1);
        }
        score_project(model, &changed, catalog).0 <= baseline
    })
}

fn duplicate_stable(model: &ScoreModel) -> bool {
    let base = model.priority_penalties.p2;
    let two = bounded_penalty(model, base, 2);
    let many = bounded_penalty(model, base, 10_000);
    many >= two
        && many <= base.mul_add(model.occurrence_multiplier_cap, f64::EPSILON)
        && many - two <= base.mul_add(model.occurrence_multiplier_cap - 1.0, f64::EPSILON)
}

fn predicted_remediations(
    model: &ScoreModel,
    project: &ProjectReview,
    catalog: &BTreeMap<String, CatalogEntry>,
) -> Vec<String> {
    let mut groups = project.groups.clone();
    groups.sort_by(|left, right| {
        group_penalty(model, right, catalog)
            .total_cmp(&group_penalty(model, left, catalog))
            .then_with(|| review_group_order(left, right))
    });
    groups.into_iter().take(3).map(|group| group.rule).collect()
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the weighted score is rounded and clamped to the closed 0..=100 range before conversion"
)]
fn score_project(
    model: &ScoreModel,
    project: &ProjectReview,
    catalog: &BTreeMap<String, CatalogEntry>,
) -> (u32, String) {
    let mut penalties: BTreeMap<&str, f64> = BTreeMap::new();
    let mut p0 = false;
    for group in &project.groups {
        let penalty = group_penalty(model, group, catalog);
        *penalties
            .entry(dimension_for_category(&group.category))
            .or_default() += penalty;
        p0 |= group.priority == "p0" && penalty > 0.0;
    }
    let score_for = |dimension: &str| {
        (100.0 - penalties.get(dimension).copied().unwrap_or_default()).clamp(0.0, 100.0)
    };
    let weighted = score_for("security") * model.dimension_weights.security
        + score_for("reliability") * model.dimension_weights.reliability
        + score_for("maintainability") * model.dimension_weights.maintainability
        + score_for("performance") * model.dimension_weights.performance
        + score_for("dependencies") * model.dimension_weights.dependencies;
    let total_weight = model.dimension_weights.security
        + model.dimension_weights.reliability
        + model.dimension_weights.maintainability
        + model.dimension_weights.performance
        + model.dimension_weights.dependencies;
    let mut score = (weighted / total_weight).round().clamp(0.0, 100.0) as u32;
    if p0 {
        score = score.min(model.p0_score_ceiling);
    }
    let band = if score >= model.label_thresholds.great {
        "great"
    } else if score >= model.label_thresholds.needs_work {
        "needs-work"
    } else {
        "critical"
    };
    (score, band.to_string())
}

fn dimension_for_category(category: &str) -> &'static str {
    match category {
        "security" => "security",
        "correctness" | "error-handling" | "async" | "framework" => "reliability",
        "performance" => "performance",
        "cargo" | "dependencies" => "dependencies",
        // Reviews are schema-validated against catalog categories before scoring.
        _ => "maintainability",
    }
}

fn group_penalty(
    model: &ScoreModel,
    group: &ReviewedGroup,
    catalog: &BTreeMap<String, CatalogEntry>,
) -> f64 {
    if catalog
        .get(&group.rule)
        .is_none_or(|entry| !entry.trust.contributes_to_core_score)
    {
        return 0.0;
    }
    let base = match group.priority.as_str() {
        "p0" => model.priority_penalties.p0,
        "p1" => model.priority_penalties.p1,
        "p2" => model.priority_penalties.p2,
        _ => model.priority_penalties.p3,
    };
    match group.aggregation.as_str() {
        "audit-only" => 0.0,
        "bounded-occurrence" => bounded_penalty(model, base, group.occurrences),
        _ => base,
    }
}

fn bounded_penalty(model: &ScoreModel, base: f64, occurrences: usize) -> f64 {
    if occurrences == 0 {
        0.0
    } else {
        let count = occurrences as f64;
        let cap = model.occurrence_multiplier_cap.max(1.0);
        base * (cap - (cap - 1.0) / count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> ScoreModel {
        ScoreModel {
            schema_version: "1.0".to_string(),
            model_version: "test".to_string(),
            label_thresholds: LabelThresholds {
                great: 95,
                needs_work: 50,
            },
            priority_penalties: PriorityPenalties {
                p0: 34.0,
                p1: 12.0,
                p2: 5.0,
                p3: 1.5,
            },
            occurrence_multiplier_cap: 2.0,
            p0_score_ceiling: 49,
            dimension_weights: DimensionWeights {
                security: 2.0,
                reliability: 1.5,
                maintainability: 1.0,
                performance: 1.0,
                dependencies: 1.0,
            },
            calibration: ModelCalibration {
                dataset_version: "test".to_string(),
                decision_dataset_sha256: String::new(),
                migration_report: String::new(),
                migration_report_sha256: String::new(),
            },
        }
    }

    #[test]
    fn duplicate_occurrences_are_bounded() {
        assert!(duplicate_stable(&model()));
    }

    #[test]
    fn reviewed_contract_anchors_cover_every_health_band() {
        let mut project = project_with_groups(Vec::new(), "great");
        assert_eq!(
            score_project(&model(), &project, &catalog(&project.groups)),
            (100, "great".to_string())
        );

        project.groups = vec![
            group("clippy::await_holding_lock", "correctness", "p1"),
            group("clippy::invalid_regex", "correctness", "p1"),
        ];
        project.reviewed_health_band = "needs-work".to_string();
        assert_eq!(
            score_project(&model(), &project, &catalog(&project.groups)),
            (94, "needs-work".to_string())
        );

        project.groups = vec![group("compiler-error", "correctness", "p0")];
        project.reviewed_health_band = "critical".to_string();
        assert_eq!(
            score_project(&model(), &project, &catalog(&project.groups)),
            (49, "critical".to_string())
        );
    }

    #[test]
    fn a_confirmed_p0_cannot_receive_the_great_score_label() {
        let project = project_with_groups(
            vec![group("compiler-error", "correctness", "p0")],
            "critical",
        );
        let catalog = catalog(&project.groups);
        let (score, band) = score_project(&model(), &project, &catalog);

        assert_eq!(score, 49);
        assert_eq!(band, "critical");
        assert!(measure_candidate(&model(), &[project], &catalog).reviewer_label_safe);
    }

    #[test]
    fn optional_observations_are_measured_as_score_invariant() {
        let project = project_with_groups(
            vec![group("compiler-error", "correctness", "p0")],
            "critical",
        );
        let catalog = catalog(&project.groups);
        assert!(optional_tool_invariant(&model(), &[project], &catalog));
    }

    #[test]
    fn an_empty_denominator_is_not_reported_as_perfect() {
        assert!(ratio_or_zero(0, 0).abs() < f64::EPSILON);
    }

    fn group(rule: &str, category: &str, priority: &str) -> ReviewedGroup {
        ReviewedGroup {
            rule: rule.to_string(),
            category: category.to_string(),
            priority: priority.to_string(),
            aggregation: "unique-rule".to_string(),
            occurrences: 1,
        }
    }

    fn catalog(groups: &[ReviewedGroup]) -> BTreeMap<String, CatalogEntry> {
        let mut entries: BTreeMap<_, _> = groups
            .iter()
            .map(|group| {
                (
                    group.rule.clone(),
                    CatalogEntry {
                        canonical_id: group.rule.clone(),
                        category: group.category.clone(),
                        trust: CatalogTrust {
                            priority: Some(group.priority.clone()),
                            contributes_to_core_score: true,
                            aggregation: group.aggregation.clone(),
                            analyzer_availability: "core-bundled".to_string(),
                        },
                    },
                )
            })
            .collect();
        entries.insert(
            "deny-advisory".to_string(),
            CatalogEntry {
                canonical_id: "deny-advisory".to_string(),
                category: "dependencies".to_string(),
                trust: CatalogTrust {
                    priority: Some("p0".to_string()),
                    contributes_to_core_score: false,
                    aggregation: "root-cause".to_string(),
                    analyzer_availability: "optional-external".to_string(),
                },
            },
        );
        entries
    }

    fn project_with_groups(groups: Vec<ReviewedGroup>, band: &str) -> ProjectReview {
        ProjectReview {
            project_id: "a".repeat(64),
            repository_cluster: "a".repeat(64),
            repository_root: "a".repeat(64),
            language_context: "rust".to_string(),
            framework_context: "none".to_string(),
            fixture_provenance: "protected-public-corpus".to_string(),
            author: "upstream".to_string(),
            independent_reviewer: "reviewer".to_string(),
            disposition: "accepted".to_string(),
            holdout: true,
            reviewed_health_band: band.to_string(),
            top_remediations: groups
                .iter()
                .take(3)
                .map(|group| group.rule.clone())
                .collect(),
            groups,
        }
    }
}
