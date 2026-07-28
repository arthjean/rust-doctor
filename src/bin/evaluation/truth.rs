//! Versioned truth dataset validation and the per-rule calibration baseline.
//!
//! The dataset in `evaluation/truth-dataset-v1.json` models positive
//! opportunities and negative contexts, not just emitted findings, so recall is
//! measurable. Labels are re-derived from the fixtures on every run: a fixture
//! edit that is not reflected in the dataset fails the job instead of silently
//! changing the measured population.
#![expect(
    clippy::cast_precision_loss,
    reason = "sample counts stay far below the f64 mantissa when computing ratios"
)]

use super::manifest::{hex_digest, read_json, sha256_file, write_json_atomic};
use super::{EvalError, Result};
use rust_doctor::api::{ScanOptions, ScanRequest, ScanScope};
use rust_doctor::config::{AdapterPolicy, FileConfig};
use rust_doctor::diagnostics::{CheckStatus, Diagnostic, DiagnosticLocation, ReportV1};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const DATASET_SCHEMA_VERSION: &str = "1.0";
/// Baseline artifact shape. Bumped independently of the dataset: US-010 added
/// the observation populations (emitted, suppressed, abstained, uncovered) that
/// the earlier 1.0 baseline could not represent.
const BASELINE_SCHEMA_VERSION: &str = "1.1";
const CALIBRATION_SCHEMA_VERSION: &str = "1.0";
const CONFIDENCE_LEVEL: f64 = 0.95;
const MAX_FALSE_POSITIVE_RATE: f64 = 0.02;
const MIN_RECALL: f64 = 0.80;
const MIN_POSITIVE_SAMPLES: usize = 50;
const MIN_NEGATIVE_SAMPLES: usize = 149;
const MIN_CONTEXT_COVERAGE: f64 = 0.90;

/// Reviewer states whose cases are excluded from every pass statistic.
const EXCLUDED_REVIEWER_STATES: &[&str] = &["unreviewed", "disputed", "stale", "unknown"];
/// Every reviewer state the schema accepts. A value outside this set is a
/// malformed label, not a silently excluded one (US-010 AC-8).
const REVIEWER_STATES: &[&str] = &["reviewed", "unreviewed", "disputed", "stale", "unknown"];
const CASE_KINDS: &[&str] = &["positive-opportunity", "negative-context"];
const APPLICABILITY_STATES: &[&str] = &["applicable", "not-applicable"];
const LABEL_PROVENANCES: &[&str] = &["authored-fixture", "corpus-review", "upstream-report"];
const DISPOSITIONS: &[&str] = &["accepted", "rejected", "unavailable"];

#[derive(Debug, Deserialize)]
pub(crate) struct TruthDataset {
    pub(crate) schema_version: String,
    pub(crate) dataset_version: String,
    pub(crate) msrv: String,
    pub(crate) rules: Vec<String>,
    pub(crate) fixtures: Vec<TruthFixture>,
    pub(crate) cases: Vec<TruthCase>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct TruthFixture {
    pub(crate) path: PathBuf,
    pub(crate) rule: String,
    pub(crate) crate_role: String,
    pub(crate) source_surface: String,
    pub(crate) target_path: String,
    pub(crate) edition: String,
    pub(crate) msrv: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CaseLocation {
    pub(crate) line: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CaseContext {
    pub(crate) crate_role: String,
    pub(crate) source_surface: String,
    pub(crate) target_path: String,
    pub(crate) edition: String,
    pub(crate) msrv: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct TruthCase {
    pub(crate) case_id: String,
    pub(crate) rule: String,
    pub(crate) fixture: PathBuf,
    pub(crate) location: CaseLocation,
    pub(crate) kind: String,
    pub(crate) expected_applicability: String,
    pub(crate) expected_emission: bool,
    pub(crate) expected_priority: Option<String>,
    pub(crate) context: CaseContext,
    pub(crate) label_provenance: String,
    pub(crate) reviewer_state: String,
    pub(crate) repository_cluster: String,
    pub(crate) repository_root: String,
    pub(crate) language_context: String,
    pub(crate) framework_context: Option<String>,
    pub(crate) author: String,
    pub(crate) independent_reviewer: String,
    pub(crate) disposition: String,
    pub(crate) holdout: bool,
}

impl TruthCase {
    fn counted(&self) -> bool {
        !EXCLUDED_REVIEWER_STATES.contains(&self.reviewer_state.as_str())
            && self.disposition == "accepted"
            && !self.author.is_empty()
            && !self.independent_reviewer.is_empty()
            && self.author != self.independent_reviewer
            && !self.repository_cluster.is_empty()
            && !self.repository_root.is_empty()
            && !self.language_context.is_empty()
    }
}

/// Measured evidence for one rule on the labeled dataset.
///
/// The six populations of US-010 AC-1 stay independent here: labeled positive
/// opportunities and negative contexts come from the dataset, while emitted,
/// suppressed, abstained, and uncovered come from the scan. None of them is
/// derived from another, so a quiet rule cannot look accurate by never firing.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct RuleMetrics {
    pub(crate) rule: String,
    /// Counted labels, i.e. the denominator every ratio below is reported over.
    pub(crate) sample_size: usize,
    /// Counted labels whose fixture completed analysis.
    pub(crate) complete_sample_size: usize,
    /// Counted labels whose fixture did not complete: never merged into the
    /// pass population (AC-6).
    pub(crate) incomplete_sample_size: usize,
    pub(crate) positive_samples: usize,
    pub(crate) negative_samples: usize,
    pub(crate) excluded_labels: usize,
    pub(crate) true_positives: usize,
    pub(crate) false_positives: usize,
    pub(crate) false_negatives: usize,
    pub(crate) true_negatives: usize,
    /// Emissions landing on a label the reviewer has not settled. Neither a
    /// true nor a false positive (AC-4).
    pub(crate) unknown_emissions: usize,
    /// Extra emissions on an opportunity already recovered. Recall counts the
    /// opportunity once; the duplicates are reported here (AC-7).
    pub(crate) duplicate_emissions: usize,
    pub(crate) suppressed_findings: usize,
    pub(crate) abstentions: usize,
    /// Labels the scan never reached, because their fixture did not complete.
    pub(crate) uncovered_contexts: usize,
    pub(crate) precision: Option<f64>,
    pub(crate) recall: Option<f64>,
    pub(crate) false_positive_rate: Option<f64>,
    pub(crate) false_positive_upper_bound: Option<f64>,
    pub(crate) confidence_level: f64,
    /// Positive opportunities the scan actually reached, over all of them.
    pub(crate) opportunity_coverage: Option<f64>,
    /// Required-context coverage: counted labels whose fixture completed and
    /// where the rule did not abstain, over all counted labels.
    pub(crate) context_coverage: f64,
    pub(crate) abstention_rate: Option<f64>,
    pub(crate) emitted_count: usize,
    pub(crate) score_contribution: f64,
    pub(crate) complete_fixtures: usize,
    pub(crate) incomplete_fixtures: usize,
    pub(crate) scan_complete: bool,
    pub(crate) evidence_complete: bool,
    pub(crate) decision: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct FixtureRun {
    pub(crate) fixture: PathBuf,
    pub(crate) rule: String,
    pub(crate) analyzed: bool,
    pub(crate) completeness_state: String,
    pub(crate) score: Option<u32>,
    pub(crate) emitted_for_rule: usize,
    pub(crate) planned_files: usize,
    pub(crate) suppressed: usize,
    pub(crate) abstentions: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub(crate) struct BaselineThresholds {
    pub(crate) confidence_level: f64,
    pub(crate) max_false_positive_rate: f64,
    pub(crate) min_recall: f64,
    pub(crate) min_positive_samples: usize,
    pub(crate) min_negative_samples: usize,
    pub(crate) min_context_coverage: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct TruthBaseline {
    pub(crate) schema_version: String,
    pub(crate) dataset_version: String,
    pub(crate) dataset_sha256: String,
    pub(crate) msrv: String,
    pub(crate) toolchain: String,
    pub(crate) tool_version: String,
    pub(crate) tool_revision: String,
    pub(crate) configuration_sha256: String,
    pub(crate) catalog_sha256: String,
    pub(crate) score_model_version: String,
    pub(crate) thresholds: BaselineThresholds,
    pub(crate) rules: Vec<RuleMetrics>,
    pub(crate) fixtures: Vec<FixtureRun>,
}

/// Calibration artifact regenerated from a measured baseline.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct CalibrationArtifact {
    schema_version: String,
    calibration_version: String,
    dataset_version: String,
    toolchain: String,
    thresholds: BaselineThresholds,
    rules: Vec<CalibrationRecord>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CalibrationRecord {
    rule: String,
    decision: String,
    evidence_complete: bool,
    true_positives: usize,
    false_positives: usize,
    false_negatives: usize,
    positive_samples: usize,
    negative_samples: usize,
    precision: Option<f64>,
    recall: Option<f64>,
    false_positive_rate: Option<f64>,
    false_positive_upper_bound: Option<f64>,
    confidence_level: f64,
    context_coverage: f64,
    reviewer: String,
    reviewed_at: String,
    rationale: String,
}

pub(crate) struct TruthArgs {
    pub(crate) dataset: PathBuf,
    pub(crate) binary: PathBuf,
    pub(crate) output: PathBuf,
    pub(crate) tool_revision: String,
    pub(crate) calibration_out: Option<PathBuf>,
    pub(crate) calibration_version: Option<String>,
    pub(crate) reviewer: Option<String>,
    pub(crate) reviewed_at: Option<String>,
}

#[expect(
    clippy::too_many_lines,
    reason = "the baseline job keeps dataset validation, measurement, and artifact writing in one readable sequence"
)]
pub(crate) fn run(args: &TruthArgs) -> Result<()> {
    let dataset: TruthDataset = read_json(&args.dataset)?;
    validate_dataset(&dataset)?;
    verify_labels_match_fixtures(&dataset)?;

    let catalog_sha256 = catalog_fingerprint(&args.binary)?;
    let configuration_sha256 = hex_digest(scan_configuration_descriptor().as_bytes());

    let mut observations = Observations::default();

    for fixture in &dataset.fixtures {
        let outcome = scan_fixture(fixture)?;
        if outcome.analyzed {
            observations.analyzed.insert(fixture.path.clone());
        }
        let entry = observations
            .emissions
            .entry(fixture.path.clone())
            .or_default();
        for diagnostic in &outcome.diagnostics {
            entry.push((diagnostic.rule.clone(), diagnostic.line.unwrap_or(0)));
        }
        *observations
            .score_contribution
            .entry(fixture.rule.clone())
            .or_default() += rule_score_contribution(&outcome.diagnostics, &fixture.rule)?;
        let abstentions = outcome
            .abstentions
            .get(&fixture.rule)
            .copied()
            .unwrap_or_default();
        if abstentions > 0 {
            observations.abstained.insert(fixture.path.clone());
        }
        observations.fixtures.push(FixtureRun {
            fixture: fixture.path.clone(),
            rule: fixture.rule.clone(),
            analyzed: outcome.analyzed,
            completeness_state: outcome.completeness_state,
            score: outcome.score,
            emitted_for_rule: outcome
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.rule == fixture.rule)
                .count(),
            planned_files: outcome.planned_files,
            suppressed: outcome.suppressed,
            abstentions,
        });
    }

    let rules = dataset
        .rules
        .iter()
        .map(|rule| measure_rule(rule, &dataset, &observations))
        .collect::<Vec<_>>();

    let baseline = TruthBaseline {
        schema_version: BASELINE_SCHEMA_VERSION.to_string(),
        dataset_version: dataset.dataset_version.clone(),
        dataset_sha256: sha256_file(&args.dataset)?,
        msrv: dataset.msrv.clone(),
        toolchain: toolchain_identifier()?,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        tool_revision: args.tool_revision.clone(),
        configuration_sha256,
        catalog_sha256,
        score_model_version: rust_doctor::output::score_model_version().to_string(),
        thresholds: thresholds(),
        rules: rules.clone(),
        fixtures: observations.fixtures,
    };
    write_json_atomic(&args.output, &baseline)?;

    if let Some(path) = &args.calibration_out {
        let measured_rules = rules
            .iter()
            .map(|metrics| CalibrationRecord {
                rule: metrics.rule.clone(),
                decision: if metrics.decision == "score-eligible-default" {
                    metrics.decision.clone()
                } else {
                    "non-scoring-default".to_string()
                },
                evidence_complete: metrics.evidence_complete,
                true_positives: metrics.true_positives,
                false_positives: metrics.false_positives,
                false_negatives: metrics.false_negatives,
                positive_samples: metrics.positive_samples,
                negative_samples: metrics.negative_samples,
                precision: metrics.precision,
                recall: metrics.recall,
                false_positive_rate: metrics.false_positive_rate,
                false_positive_upper_bound: metrics.false_positive_upper_bound,
                confidence_level: metrics.confidence_level,
                context_coverage: metrics.context_coverage,
                reviewer: args
                    .reviewer
                    .clone()
                    .unwrap_or_else(|| "unassigned".to_string()),
                reviewed_at: args.reviewed_at.clone().unwrap_or_default(),
                rationale: rationale_for(metrics),
            })
            .collect();
        let artifact = CalibrationArtifact {
            schema_version: CALIBRATION_SCHEMA_VERSION.to_string(),
            calibration_version: args
                .calibration_version
                .clone()
                .unwrap_or_else(|| dataset.dataset_version.clone()),
            dataset_version: dataset.dataset_version,
            toolchain: baseline.msrv,
            thresholds: thresholds(),
            rules: merge_calibration_records(path, measured_rules)?,
        };
        write_json_atomic(path, &artifact)?;
    }

    report(&rules);
    Ok(())
}

/// Preserve reviewed decisions for rules outside the selected truth slice.
///
/// A truth dataset may deliberately remeasure only a subset of the catalog.
/// Regeneration replaces those measured records and carries every other
/// reviewed decision forward, so a partial measurement cannot silently erase
/// activation policy for unrelated rules.
fn merge_calibration_records(
    path: &Path,
    measured: Vec<CalibrationRecord>,
) -> Result<Vec<CalibrationRecord>> {
    let mut records: BTreeMap<String, CalibrationRecord> = if path.exists() {
        let previous: CalibrationArtifact = read_json(path)?;
        previous
            .rules
            .into_iter()
            .map(|record| (record.rule.clone(), record))
            .collect()
    } else {
        BTreeMap::new()
    };
    for record in measured {
        records.insert(record.rule.clone(), record);
    }
    Ok(records.into_values().collect())
}

const fn thresholds() -> BaselineThresholds {
    BaselineThresholds {
        confidence_level: CONFIDENCE_LEVEL,
        max_false_positive_rate: MAX_FALSE_POSITIVE_RATE,
        min_recall: MIN_RECALL,
        min_positive_samples: MIN_POSITIVE_SAMPLES,
        min_negative_samples: MIN_NEGATIVE_SAMPLES,
        min_context_coverage: MIN_CONTEXT_COVERAGE,
    }
}

fn rationale_for(metrics: &RuleMetrics) -> String {
    match metrics.decision.as_str() {
        "score-eligible-default" => format!(
            "{} of {} opportunities recovered with {} false positives on {} negative contexts",
            metrics.true_positives,
            metrics.positive_samples,
            metrics.false_positives,
            metrics.negative_samples
        ),
        _ => format!(
            "gate not satisfied: {} positives, {} negatives, recall {:?}, exact 95% false-positive upper bound {:?}, context coverage {:.2}",
            metrics.positive_samples,
            metrics.negative_samples,
            metrics.recall,
            metrics.false_positive_upper_bound,
            metrics.context_coverage
        ),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "every label rejection stays in one place so a failure names the offending case"
)]
fn validate_dataset(dataset: &TruthDataset) -> Result<()> {
    if dataset.schema_version != DATASET_SCHEMA_VERSION {
        return Err(EvalError::InvalidManifest(format!(
            "truth dataset schema must be {DATASET_SCHEMA_VERSION}, got {}",
            dataset.schema_version
        )));
    }
    if dataset.rules.len() < 10 {
        return Err(EvalError::InvalidManifest(format!(
            "truth dataset must cover at least 10 rules, got {}",
            dataset.rules.len()
        )));
    }
    let declared: BTreeSet<&str> = dataset.rules.iter().map(String::as_str).collect();
    for fixture in &dataset.fixtures {
        if !declared.contains(fixture.rule.as_str()) {
            return Err(EvalError::InvalidManifest(format!(
                "fixture '{}' targets undeclared rule '{}'",
                fixture.path.display(),
                fixture.rule
            )));
        }
        if fixture.msrv != dataset.msrv {
            return Err(EvalError::InvalidManifest(format!(
                "fixture '{}' declares MSRV {} but the dataset pins {}",
                fixture.path.display(),
                fixture.msrv,
                dataset.msrv
            )));
        }
    }
    let mut identifiers = BTreeSet::new();
    let mut root_partitions = BTreeMap::new();
    // Every rejection below names the offending case identifiers so a failure
    // is addressable without re-deriving which label broke (US-010 AC-8).
    let mut sites: BTreeMap<(&Path, u32), &TruthCase> = BTreeMap::new();
    for case in &dataset.cases {
        if !identifiers.insert(case.case_id.as_str()) {
            return Err(EvalError::InvalidManifest(format!(
                "duplicate truth case identifier '{}'",
                case.case_id
            )));
        }
        for (field, value, allowed) in [
            ("kind", case.kind.as_str(), CASE_KINDS),
            (
                "expected_applicability",
                case.expected_applicability.as_str(),
                APPLICABILITY_STATES,
            ),
            (
                "label_provenance",
                case.label_provenance.as_str(),
                LABEL_PROVENANCES,
            ),
            (
                "reviewer_state",
                case.reviewer_state.as_str(),
                REVIEWER_STATES,
            ),
            ("disposition", case.disposition.as_str(), DISPOSITIONS),
        ] {
            if !allowed.contains(&value) {
                return Err(EvalError::InvalidManifest(format!(
                    "case '{}' has malformed {field} '{value}'; expected one of {}",
                    case.case_id,
                    allowed.join(", ")
                )));
            }
        }
        if case.expected_emission != (case.kind == "positive-opportunity") {
            return Err(EvalError::InvalidManifest(format!(
                "case '{}' contradicts itself: kind '{}' with expected_emission {}",
                case.case_id, case.kind, case.expected_emission
            )));
        }
        if case.expected_applicability == "not-applicable" && case.expected_emission {
            return Err(EvalError::InvalidManifest(format!(
                "case '{}' expects an emission from a context it declares not-applicable",
                case.case_id
            )));
        }
        if !declared.contains(case.rule.as_str()) {
            return Err(EvalError::InvalidManifest(format!(
                "case '{}' targets undeclared rule '{}'",
                case.case_id, case.rule
            )));
        }
        if case.counted()
            && (!is_lower_hex_sha256(&case.repository_cluster)
                || !is_lower_hex_sha256(&case.repository_root))
        {
            return Err(EvalError::InvalidManifest(format!(
                "case '{}' must use privacy-safe SHA-256 repository identities",
                case.case_id
            )));
        }
        if case.counted()
            && let Some(previous) =
                root_partitions.insert(case.repository_root.as_str(), case.holdout)
            && previous != case.holdout
        {
            return Err(EvalError::InvalidManifest(format!(
                "case '{}' puts repository root {} in both development and holdout data",
                case.case_id, case.repository_root
            )));
        }
        // Two labels on one site must agree. Contradictory expectations would
        // let the same emission be scored as both correct and incorrect.
        if let Some(previous) = sites.insert((case.fixture.as_path(), case.location.line), case) {
            if previous.rule == case.rule {
                return Err(EvalError::InvalidManifest(format!(
                    "cases '{}' and '{}' duplicate rule '{}' at {}:{}",
                    previous.case_id,
                    case.case_id,
                    case.rule,
                    case.fixture.display(),
                    case.location.line
                )));
            }
            if previous.expected_emission != case.expected_emission {
                return Err(EvalError::InvalidManifest(format!(
                    "cases '{}' and '{}' contradict each other at {}:{}",
                    previous.case_id,
                    case.case_id,
                    case.fixture.display(),
                    case.location.line
                )));
            }
        }
    }
    Ok(())
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Everything the scan observed, kept separate from everything the dataset
/// labeled. Metrics are the join of the two, never a reinterpretation of one.
#[derive(Default)]
struct Observations {
    fixtures: Vec<FixtureRun>,
    /// Rule identity and line of every emitted finding, per fixture.
    emissions: BTreeMap<PathBuf, Vec<(String, u32)>>,
    /// Fixtures whose analysis completed.
    analyzed: BTreeSet<PathBuf>,
    /// Fixtures where the owning rule declined to decide.
    abstained: BTreeSet<PathBuf>,
    score_contribution: BTreeMap<String, f64>,
}

/// Re-derive every label from its fixture and refuse a dataset that drifted.
/// The same pass is the MSRV parse gate: a fixture that no longer parses fails
/// the job with its path and the exact parser reason.
fn verify_labels_match_fixtures(dataset: &TruthDataset) -> Result<()> {
    for fixture in &dataset.fixtures {
        let content = std::fs::read_to_string(&fixture.path)
            .map_err(|error| EvalError::io("cannot read truth fixture", &fixture.path, error))?;
        if hex_digest(content.as_bytes()) != fixture.sha256 {
            return Err(EvalError::GateFailed(format!(
                "truth fixture '{}' changed without a dataset update",
                fixture.path.display()
            )));
        }
        if let Err(error) = syn::parse_file(&content) {
            return Err(EvalError::GateFailed(format!(
                "truth fixture '{}' does not parse on MSRV {}: {error}",
                fixture.path.display(),
                fixture.msrv
            )));
        }
        let derived = derive_labels(&content);
        let declared: Vec<(u32, bool)> = dataset
            .cases
            .iter()
            .filter(|case| case.fixture == fixture.path)
            .map(|case| (case.location.line, case.expected_emission))
            .collect();
        if derived != declared {
            return Err(EvalError::GateFailed(format!(
                "truth fixture '{}' declares {} labels but the dataset records {}",
                fixture.path.display(),
                derived.len(),
                declared.len()
            )));
        }
    }
    Ok(())
}

fn derive_labels(content: &str) -> Vec<(u32, bool)> {
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let positive = line.contains("//~ pos");
            let negative = line.contains("//~ neg");
            if !positive && !negative {
                return None;
            }
            u32::try_from(index + 1)
                .ok()
                .map(|line_number| (line_number, positive))
        })
        .collect()
}

struct FixtureOutcome {
    analyzed: bool,
    completeness_state: String,
    score: Option<u32>,
    diagnostics: Vec<Diagnostic>,
    planned_files: usize,
    suppressed: usize,
    /// Abstention receipts keyed by the rule that declined to decide.
    abstentions: BTreeMap<String, usize>,
}

fn scan_fixture(fixture: &TruthFixture) -> Result<FixtureOutcome> {
    let directory = tempfile::TempDir::new()
        .map_err(|error| EvalError::io("cannot create fixture workspace", ".", error))?;
    let root = directory.path();
    materialize(root, fixture)?;

    let report = rust_doctor::api::scan(ScanRequest {
        root: root.to_path_buf(),
        options: scan_options(),
    })
    .map_err(|error| {
        EvalError::Command(format!(
            "truth scan failed for '{}': {error}",
            fixture.path.display()
        ))
    })?;
    Ok(outcome_from_report(&report, fixture))
}

fn scan_options() -> ScanOptions {
    ScanOptions {
        scope: ScanScope::Full,
        adapters: AdapterPolicy {
            compiler_lint: false,
            custom_ast: true,
            supply_chain: false,
            quality: false,
            network: false,
        },
        use_project_config: false,
        config_overrides: FileConfig::default(),
        ..ScanOptions::default()
    }
}

/// Stable description of the scan policy used by the baseline, so a policy
/// change invalidates the recorded configuration fingerprint.
fn scan_configuration_descriptor() -> String {
    "scope=full;adapters=custom-ast;project-config=off;network=off".to_string()
}

fn outcome_from_report(report: &ReportV1, fixture: &TruthFixture) -> FixtureOutcome {
    let analyzed = report.projects.iter().any(|project| {
        project
            .analyzed_files
            .iter()
            .any(|path| path == &fixture.target_path)
    }) && report
        .projects
        .iter()
        .flat_map(|project| project.checks.iter())
        .any(|check| check.name == "custom rules" && check.status == CheckStatus::Completed);
    let diagnostics = report
        .diagnostics
        .iter()
        .filter_map(|diagnostic| {
            let DiagnosticLocation::Source { path, range } = &diagnostic.location else {
                return None;
            };
            if path != &fixture.target_path {
                return None;
            }
            Some(Diagnostic {
                file_path: PathBuf::from(path),
                rule: diagnostic.rule.clone(),
                category: diagnostic.category.clone(),
                severity: diagnostic.severity,
                message: diagnostic.message.clone(),
                help: None,
                line: Some(range.start.line),
                column: Some(range.start.column),
                fix: None,
            })
        })
        .collect();
    let mut abstentions = BTreeMap::new();
    for receipt in &report.audit.abstentions {
        *abstentions.entry(receipt.rule.clone()).or_default() += receipt.count;
    }
    FixtureOutcome {
        analyzed,
        completeness_state: format!("{:?}", report.completeness.state).to_lowercase(),
        score: report.summary.score,
        diagnostics,
        planned_files: report.completeness.planned_files,
        suppressed: report.audit.suppression_counts.total(),
        abstentions,
    }
}

/// Exact score impact of one rule on one fixture: the difference between the
/// score computed from every diagnostic and the score computed without this
/// rule's diagnostics.
fn rule_score_contribution(diagnostics: &[Diagnostic], rule: &str) -> Result<f64> {
    let (with_rule, _, _) = rust_doctor::output::calculate_score(diagnostics).ok_or_else(|| {
        EvalError::InvalidManifest("checked score model could not score truth evidence".to_string())
    })?;
    let without: Vec<Diagnostic> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule != rule)
        .cloned()
        .collect();
    let (without_rule, _, _) = rust_doctor::output::calculate_score(&without).ok_or_else(|| {
        EvalError::InvalidManifest(
            "checked score model could not score truth counterfactual".to_string(),
        )
    })?;
    Ok(f64::from(without_rule) - f64::from(with_rule))
}

fn materialize(root: &Path, fixture: &TruthFixture) -> Result<()> {
    let manifest = format!(
        "[package]\nname = \"truth-fixture\"\nversion = \"0.0.0\"\nedition = \"{}\"\nrust-version = \"{}\"\n\n[dependencies]\n",
        fixture.edition, fixture.msrv
    );
    write(&root.join("Cargo.toml"), &manifest)?;
    let content = std::fs::read_to_string(&fixture.path)
        .map_err(|error| EvalError::io("cannot read truth fixture", &fixture.path, error))?;
    let target = root.join(&fixture.target_path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| EvalError::io("cannot create fixture directory", parent, error))?;
    }
    write(&target, &content)?;
    // A module fixture needs a crate root that declares it, otherwise Cargo
    // reports no target and the file would be analyzed without its surface.
    if fixture.target_path != "src/lib.rs" && fixture.target_path != "src/main.rs" {
        let module = Path::new(&fixture.target_path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        write(&root.join("src/lib.rs"), &format!("pub mod {module};\n"))?;
    }
    Ok(())
}

fn write(path: &Path, content: &str) -> Result<()> {
    std::fs::write(path, content)
        .map_err(|error| EvalError::io("cannot write fixture file", path, error))
}

/// Labeled sites for one rule, indexed by fixture and line.
///
/// Counted positives, counted negatives, and labels the reviewer has not
/// settled are three separate sets: an emission landing on an excluded label is
/// unknown, not a false positive, and never a true positive (US-010 AC-4).
#[derive(Default)]
struct LabelledSites<'dataset> {
    positives: BTreeMap<&'dataset Path, BTreeSet<u32>>,
    negatives: BTreeMap<&'dataset Path, BTreeSet<u32>>,
    excluded: BTreeMap<&'dataset Path, BTreeSet<u32>>,
}

impl<'dataset> LabelledSites<'dataset> {
    fn insert(&mut self, case: &'dataset TruthCase, included: bool) {
        let target = if !included {
            &mut self.excluded
        } else if case.expected_emission {
            &mut self.positives
        } else {
            &mut self.negatives
        };
        target
            .entry(case.fixture.as_path())
            .or_default()
            .insert(case.location.line);
    }

    fn contains(map: &BTreeMap<&Path, BTreeSet<u32>>, fixture: &Path, line: u32) -> bool {
        map.get(fixture).is_some_and(|lines| lines.contains(&line))
    }
}

/// Counted label populations and the emission verdicts they produce.
#[derive(Default)]
struct RuleTally {
    positive_samples: usize,
    negative_samples: usize,
    true_positives: usize,
    false_negatives: usize,
    negatives_fired: usize,
    complete_sample_size: usize,
    context_covered: usize,
    uncovered_contexts: usize,
    opportunities_reached: usize,
}

/// Emission verdicts derived by joining the scan against the labels.
#[derive(Default)]
struct EmissionTally {
    emitted: usize,
    false_positives: usize,
    unknown: usize,
    duplicates: usize,
}

#[expect(
    clippy::too_many_lines,
    reason = "per-rule metric assembly keeps every counted population in one auditable place"
)]
fn measure_rule(rule: &str, dataset: &TruthDataset, observed: &Observations) -> RuleMetrics {
    let cases: Vec<&TruthCase> = dataset
        .cases
        .iter()
        .filter(|case| case.rule == rule)
        .collect();
    let mut independent = BTreeSet::new();
    let counted: Vec<&&TruthCase> = cases
        .iter()
        .filter(|case| {
            case.counted()
                && independent.insert((
                    case.repository_cluster.as_str(),
                    case.kind.as_str(),
                    case.context.crate_role.as_str(),
                    case.context.source_surface.as_str(),
                ))
        })
        .collect();
    let excluded_labels = cases.len().saturating_sub(counted.len());

    let counted_case_ids: BTreeSet<_> = counted.iter().map(|case| case.case_id.as_str()).collect();
    let mut sites = LabelledSites::default();
    for case in &cases {
        sites.insert(case, counted_case_ids.contains(case.case_id.as_str()));
    }

    let tally = tally_labels(rule, &counted, observed);
    let emissions = tally_emissions(rule, dataset, observed, &sites);

    let fixtures: Vec<&TruthFixture> = dataset
        .fixtures
        .iter()
        .filter(|fixture| fixture.rule == rule)
        .collect();
    let complete_fixtures = fixtures
        .iter()
        .filter(|fixture| observed.analyzed.contains(&fixture.path))
        .count();
    let incomplete_fixtures = fixtures.len() - complete_fixtures;
    let scan_complete = incomplete_fixtures == 0;

    let abstentions: usize = observed
        .fixtures
        .iter()
        .filter(|run| run.rule == rule)
        .map(|run| run.abstentions)
        .sum();
    let suppressed_findings: usize = observed
        .fixtures
        .iter()
        .filter(|run| run.rule == rule)
        .map(|run| run.suppressed)
        .sum();
    let planned_decisions: usize = observed
        .fixtures
        .iter()
        .filter(|run| run.rule == rule)
        .map(|run| run.planned_files)
        .sum();

    let true_negatives = tally.negative_samples.saturating_sub(tally.negatives_fired);
    let precision = ratio(
        tally.true_positives,
        tally.true_positives + emissions.false_positives,
    );
    let recall = ratio(tally.true_positives, tally.positive_samples);
    let false_positive_rate = ratio(
        emissions.false_positives,
        emissions.false_positives + true_negatives,
    );
    let false_positive_upper_bound = clopper_pearson_upper(
        emissions.false_positives,
        emissions.false_positives + true_negatives,
        CONFIDENCE_LEVEL,
    );
    let opportunity_coverage = ratio(tally.opportunities_reached, tally.positive_samples);
    let context_coverage = if counted.is_empty() {
        0.0
    } else {
        tally.context_covered as f64 / counted.len() as f64
    };
    let abstention_rate = ratio(abstentions, planned_decisions);

    // Recall is unavailable without a labeled opportunity, and an unavailable
    // metric never satisfies a gate (AC-3).
    let evidence_complete = scan_complete
        && recall.is_some()
        && false_positive_upper_bound.is_some()
        && tally.positive_samples >= MIN_POSITIVE_SAMPLES
        && tally.negative_samples >= MIN_NEGATIVE_SAMPLES;
    let passes = evidence_complete
        && recall.is_some_and(|value| value >= MIN_RECALL)
        && false_positive_upper_bound.is_some_and(|value| value <= MAX_FALSE_POSITIVE_RATE)
        && context_coverage >= MIN_CONTEXT_COVERAGE;
    let decision = if !evidence_complete {
        "evidence-incomplete".to_string()
    } else if passes {
        "score-eligible-default".to_string()
    } else {
        "non-scoring-default".to_string()
    };

    RuleMetrics {
        rule: rule.to_string(),
        sample_size: counted.len(),
        complete_sample_size: tally.complete_sample_size,
        incomplete_sample_size: counted.len() - tally.complete_sample_size,
        positive_samples: tally.positive_samples,
        negative_samples: tally.negative_samples,
        excluded_labels,
        true_positives: tally.true_positives,
        false_positives: emissions.false_positives,
        false_negatives: tally.false_negatives,
        true_negatives,
        unknown_emissions: emissions.unknown,
        duplicate_emissions: emissions.duplicates,
        suppressed_findings,
        abstentions,
        uncovered_contexts: tally.uncovered_contexts,
        precision,
        recall,
        false_positive_rate,
        false_positive_upper_bound,
        confidence_level: CONFIDENCE_LEVEL,
        opportunity_coverage,
        context_coverage,
        abstention_rate,
        emitted_count: emissions.emitted,
        score_contribution: observed
            .score_contribution
            .get(rule)
            .copied()
            .unwrap_or_default(),
        complete_fixtures,
        incomplete_fixtures,
        scan_complete,
        evidence_complete,
        decision,
    }
}

/// Walk the counted labels once and record every population they belong to.
///
/// A labeled positive with no emitted finding is a false negative even though
/// the scanner produced no diagnostic to inspect (AC-5), and a label whose
/// fixture never completed is uncovered rather than silently correct (AC-6).
fn tally_labels(rule: &str, counted: &[&&TruthCase], observed: &Observations) -> RuleTally {
    let mut tally = RuleTally::default();
    for case in counted {
        let analyzed = observed.analyzed.contains(&case.fixture);
        let abstained = observed.abstained.contains(&case.fixture);
        let fired = observed.emissions.get(&case.fixture).is_some_and(|fired| {
            fired
                .iter()
                .any(|(id, line)| id == rule && *line == case.location.line)
        });
        if analyzed {
            tally.complete_sample_size += 1;
        } else {
            tally.uncovered_contexts += 1;
        }
        // Required context is present only when the fixture completed and the
        // rule did not decline to decide on it.
        if analyzed && !abstained {
            tally.context_covered += 1;
        }
        if case.expected_emission {
            tally.positive_samples += 1;
            if analyzed {
                tally.opportunities_reached += 1;
            }
            if fired {
                tally.true_positives += 1;
            } else {
                tally.false_negatives += 1;
            }
        } else {
            tally.negative_samples += 1;
            if fired {
                tally.negatives_fired += 1;
            }
        }
    }
    tally
}

/// Classify every emitted finding against the labels.
///
/// An emission on an already-recovered opportunity is a duplicate, not a second
/// true positive (AC-7); an emission on a label the reviewer has not settled is
/// unknown (AC-4); anything else unlabeled is a false positive.
fn tally_emissions(
    rule: &str,
    dataset: &TruthDataset,
    observed: &Observations,
    sites: &LabelledSites<'_>,
) -> EmissionTally {
    let mut tally = EmissionTally::default();
    for fixture in dataset
        .fixtures
        .iter()
        .filter(|fixture| fixture.rule == rule)
    {
        let Some(fired) = observed.emissions.get(&fixture.path) else {
            continue;
        };
        let mut recovered: BTreeSet<u32> = BTreeSet::new();
        for (id, line) in fired {
            if id != rule {
                continue;
            }
            tally.emitted += 1;
            if LabelledSites::contains(&sites.positives, &fixture.path, *line) {
                if !recovered.insert(*line) {
                    tally.duplicates += 1;
                }
            } else if LabelledSites::contains(&sites.excluded, &fixture.path, *line)
                && !LabelledSites::contains(&sites.negatives, &fixture.path, *line)
            {
                tally.unknown += 1;
            } else {
                tally.false_positives += 1;
            }
        }
    }
    tally
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

/// Exact one-sided Clopper-Pearson upper bound for a binomial proportion.
fn clopper_pearson_upper(observed: usize, trials: usize, confidence: f64) -> Option<f64> {
    if trials == 0 || observed > trials || !(0.0..1.0).contains(&confidence) {
        return None;
    }
    if observed == trials {
        return Some(1.0);
    }
    if observed == 0 {
        return Some(1.0 - (1.0 - confidence).powf(1.0 / trials as f64));
    }
    let target = 1.0 - confidence;
    let mut low = observed as f64 / trials as f64;
    let mut high = 1.0;
    for _ in 0..80 {
        let midpoint = f64::midpoint(low, high);
        if binomial_cdf(observed, trials, midpoint) > target {
            low = midpoint;
        } else {
            high = midpoint;
        }
    }
    Some(high)
}

fn binomial_cdf(observed: usize, trials: usize, probability: f64) -> f64 {
    if probability <= 0.0 {
        return 1.0;
    }
    if probability >= 1.0 {
        return f64::from(observed == trials);
    }
    let failure = 1.0 - probability;
    let mut term = failure.powf(trials as f64);
    let mut total = term;
    for successes in 0..observed {
        term *= (trials - successes) as f64 / (successes + 1) as f64;
        term *= probability / failure;
        total += term;
    }
    total.clamp(0.0, 1.0)
}

fn catalog_fingerprint(binary: &Path) -> Result<String> {
    let output = std::process::Command::new(binary)
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
    Ok(hex_digest(&output.stdout))
}

fn toolchain_identifier() -> Result<String> {
    let output = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .map_err(|error| EvalError::io("cannot execute rustc", "rustc", error))?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn report(rules: &[RuleMetrics]) {
    for metrics in rules {
        let format_ratio = |value: Option<f64>| {
            value.map_or_else(|| "n/a".to_string(), |value| format!("{value:.3}"))
        };
        println!(
            "{:<32} n={:<3} (complete={:<3} incomplete={:<3}) pos={:<3} neg={:<3} tp={:<3} fp={:<3} fn={:<3} unknown={:<3} dup={:<3} precision={} recall={} fpr={} opportunity={} context={:.2} abstention={} → {}",
            metrics.rule,
            metrics.sample_size,
            metrics.complete_sample_size,
            metrics.incomplete_sample_size,
            metrics.positive_samples,
            metrics.negative_samples,
            metrics.true_positives,
            metrics.false_positives,
            metrics.false_negatives,
            metrics.unknown_emissions,
            metrics.duplicate_emissions,
            format_ratio(metrics.precision),
            format_ratio(metrics.recall),
            format_ratio(metrics.false_positive_rate),
            format_ratio(metrics.opportunity_coverage),
            metrics.context_coverage,
            format_ratio(metrics.abstention_rate),
            metrics.decision,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(id: &str, rule: &str, line: u32, kind: &str, reviewer_state: &str) -> TruthCase {
        let positive = kind == "positive-opportunity";
        TruthCase {
            case_id: id.to_string(),
            rule: rule.to_string(),
            fixture: PathBuf::from("evaluation/truth/fixtures/demo.rs"),
            location: CaseLocation { line },
            kind: kind.to_string(),
            expected_applicability: if positive {
                "applicable".to_string()
            } else {
                "not-applicable".to_string()
            },
            expected_emission: positive,
            expected_priority: None,
            context: CaseContext {
                crate_role: "library".to_string(),
                source_surface: "library".to_string(),
                target_path: "src/lib.rs".to_string(),
                edition: "2024".to_string(),
                msrv: "1.97".to_string(),
            },
            label_provenance: "authored-fixture".to_string(),
            reviewer_state: reviewer_state.to_string(),
            repository_cluster: "1".repeat(64),
            repository_root: "2".repeat(64),
            language_context: "rust".to_string(),
            framework_context: None,
            author: "fixture-author".to_string(),
            independent_reviewer: "independent-review".to_string(),
            disposition: "accepted".to_string(),
            holdout: false,
        }
    }

    fn fixture(rule: &str) -> TruthFixture {
        TruthFixture {
            path: PathBuf::from("evaluation/truth/fixtures/demo.rs"),
            rule: rule.to_string(),
            crate_role: "library".to_string(),
            source_surface: "library".to_string(),
            target_path: "src/lib.rs".to_string(),
            edition: "2024".to_string(),
            msrv: "1.97".to_string(),
            sha256: "0".repeat(64),
        }
    }

    fn dataset(cases: Vec<TruthCase>) -> TruthDataset {
        TruthDataset {
            schema_version: DATASET_SCHEMA_VERSION.to_string(),
            dataset_version: "test".to_string(),
            msrv: "1.97".to_string(),
            // The dataset contract requires at least ten covered rules; only
            // `demo-rule` carries fixtures and labels in these tests.
            rules: std::iter::once("demo-rule".to_string())
                .chain((1..10).map(|index| format!("filler-rule-{index}")))
                .collect(),
            fixtures: vec![fixture("demo-rule")],
            cases,
        }
    }

    fn observations(emissions: Vec<(&str, u32)>, analyzed: bool) -> Observations {
        let path = PathBuf::from("evaluation/truth/fixtures/demo.rs");
        let mut observed = Observations::default();
        observed.emissions.insert(
            path.clone(),
            emissions
                .into_iter()
                .map(|(rule, line)| (rule.to_string(), line))
                .collect(),
        );
        if analyzed {
            observed.analyzed.insert(path.clone());
        }
        observed.fixtures.push(FixtureRun {
            fixture: path,
            rule: "demo-rule".to_string(),
            analyzed,
            completeness_state: "complete".to_string(),
            score: Some(100),
            emitted_for_rule: 0,
            planned_files: 4,
            suppressed: 0,
            abstentions: 0,
        });
        observed
    }

    #[test]
    fn a_rule_without_a_positive_opportunity_reports_recall_as_unavailable() {
        let data = dataset(vec![case(
            "c1",
            "demo-rule",
            3,
            "negative-context",
            "reviewed",
        )]);
        let metrics = measure_rule("demo-rule", &data, &observations(vec![], true));
        assert!(metrics.recall.is_none());
        assert!(metrics.opportunity_coverage.is_none());
        assert!(!metrics.evidence_complete);
        assert_eq!(metrics.decision, "evidence-incomplete");
    }

    #[test]
    fn a_labelled_opportunity_without_an_emission_is_a_false_negative() {
        let data = dataset(vec![case(
            "c1",
            "demo-rule",
            7,
            "positive-opportunity",
            "reviewed",
        )]);
        let metrics = measure_rule("demo-rule", &data, &observations(vec![], true));
        assert_eq!(metrics.false_negatives, 1);
        assert_eq!(metrics.true_positives, 0);
        assert_eq!(metrics.emitted_count, 0);
    }

    #[test]
    fn repeated_emissions_on_one_opportunity_count_recall_once() {
        let data = dataset(vec![case(
            "c1",
            "demo-rule",
            7,
            "positive-opportunity",
            "reviewed",
        )]);
        let metrics = measure_rule(
            "demo-rule",
            &data,
            &observations(vec![("demo-rule", 7), ("demo-rule", 7)], true),
        );
        assert_eq!(metrics.true_positives, 1);
        assert_eq!(metrics.duplicate_emissions, 1);
        assert_eq!(metrics.false_positives, 0);
        assert_eq!(metrics.emitted_count, 2);
    }

    #[test]
    fn an_emission_on_an_unsettled_label_is_unknown_and_never_a_true_positive() {
        let data = dataset(vec![
            case("c1", "demo-rule", 7, "positive-opportunity", "reviewed"),
            case("c2", "demo-rule", 11, "positive-opportunity", "disputed"),
        ]);
        let metrics = measure_rule(
            "demo-rule",
            &data,
            &observations(vec![("demo-rule", 7), ("demo-rule", 11)], true),
        );
        assert_eq!(metrics.unknown_emissions, 1);
        assert_eq!(metrics.true_positives, 1);
        assert_eq!(metrics.false_positives, 0);
        assert_eq!(metrics.excluded_labels, 1);
    }

    #[test]
    fn an_emission_on_an_unlabelled_line_is_a_false_positive() {
        let data = dataset(vec![case(
            "c1",
            "demo-rule",
            7,
            "positive-opportunity",
            "reviewed",
        )]);
        let metrics = measure_rule(
            "demo-rule",
            &data,
            &observations(vec![("demo-rule", 42)], true),
        );
        assert_eq!(metrics.false_positives, 1);
        assert_eq!(metrics.unknown_emissions, 0);
    }

    #[test]
    fn an_incomplete_fixture_separates_the_population_and_reports_denominators() {
        let data = dataset(vec![
            case("c1", "demo-rule", 7, "positive-opportunity", "reviewed"),
            case("c2", "demo-rule", 9, "negative-context", "reviewed"),
        ]);
        let metrics = measure_rule("demo-rule", &data, &observations(vec![], false));
        assert_eq!(metrics.sample_size, 2);
        assert_eq!(metrics.complete_sample_size, 0);
        assert_eq!(metrics.incomplete_sample_size, 2);
        assert_eq!(metrics.uncovered_contexts, 2);
        assert_eq!(metrics.incomplete_fixtures, 1);
        assert!(!metrics.scan_complete);
        assert_eq!(metrics.opportunity_coverage, Some(0.0));
    }

    #[test]
    fn abstention_removes_required_context_and_is_reported_as_a_rate() {
        let data = dataset(vec![case(
            "c1",
            "demo-rule",
            7,
            "positive-opportunity",
            "reviewed",
        )]);
        let mut observed = observations(vec![], true);
        observed
            .abstained
            .insert(PathBuf::from("evaluation/truth/fixtures/demo.rs"));
        if let Some(run) = observed.fixtures.first_mut() {
            run.abstentions = 1;
        }
        let metrics = measure_rule("demo-rule", &data, &observed);
        assert!(metrics.context_coverage.abs() < f64::EPSILON);
        assert_eq!(metrics.abstentions, 1);
        assert_eq!(metrics.abstention_rate, Some(0.25));
    }

    #[test]
    fn malformed_duplicate_and_contradictory_labels_fail_with_case_identifiers() {
        let mut malformed = case("c1", "demo-rule", 7, "positive-opportunity", "reviewed");
        malformed.reviewer_state = "approved".to_string();
        let error = validate_dataset(&dataset(vec![malformed]))
            .expect_err("an unknown reviewer state is malformed");
        assert!(error.to_string().contains("c1"), "{error}");
        assert!(error.to_string().contains("reviewer_state"), "{error}");

        let duplicate = vec![
            case("c1", "demo-rule", 7, "positive-opportunity", "reviewed"),
            case("c2", "demo-rule", 7, "positive-opportunity", "reviewed"),
        ];
        let error = validate_dataset(&dataset(duplicate)).expect_err("one site, one label");
        assert!(
            error.to_string().contains("c1") && error.to_string().contains("c2"),
            "{error}"
        );

        let contradictory = vec![
            case("c1", "demo-rule", 7, "positive-opportunity", "reviewed"),
            {
                let mut other = case("c2", "other-rule", 7, "negative-context", "reviewed");
                other.rule = "demo-rule-b".to_string();
                other
            },
        ];
        let mut data = dataset(contradictory);
        data.rules.push("demo-rule-b".to_string());
        let error = validate_dataset(&data).expect_err("labels on one site must agree");
        assert!(
            error.to_string().contains("c1") && error.to_string().contains("c2"),
            "{error}"
        );
        assert!(error.to_string().contains("contradict"), "{error}");
    }

    #[test]
    fn a_case_targeting_an_undeclared_rule_is_rejected() {
        let mut stray = case("c9", "demo-rule", 7, "positive-opportunity", "reviewed");
        stray.rule = "not-in-catalog".to_string();
        let error = validate_dataset(&dataset(vec![stray])).expect_err("rules must be declared");
        assert!(error.to_string().contains("c9"), "{error}");
    }

    #[test]
    fn exact_upper_bound_requires_149_clean_negatives_at_95_percent() {
        let bound_148 = clopper_pearson_upper(0, 148, 0.95).expect("bound");
        let bound_149 = clopper_pearson_upper(0, 149, 0.95).expect("bound");
        assert!(bound_148 > 0.02, "{bound_148}");
        assert!(bound_149 <= 0.02, "{bound_149}");
    }

    #[test]
    fn same_author_and_correlated_labels_are_excluded() {
        let mut same_author = case(
            "same-author",
            "demo-rule",
            7,
            "positive-opportunity",
            "reviewed",
        );
        same_author.independent_reviewer = same_author.author.clone();
        let data = dataset(vec![
            same_author,
            case(
                "correlated",
                "demo-rule",
                9,
                "positive-opportunity",
                "reviewed",
            ),
            case(
                "independent",
                "demo-rule",
                11,
                "positive-opportunity",
                "reviewed",
            ),
        ]);
        let metrics = measure_rule(
            "demo-rule",
            &data,
            &observations(vec![("demo-rule", 11)], true),
        );
        assert_eq!(metrics.positive_samples, 1);
        assert_eq!(metrics.excluded_labels, 2);
        assert_eq!(metrics.unknown_emissions, 1);
        assert_eq!(metrics.false_positives, 0);
    }
}
