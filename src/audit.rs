use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use cargo_metadata::Metadata;

use crate::execution::ScanExecution;
use crate::report::{Diagnostic, Severity, Status};
use crate::source_kernel::SourceScan;

mod source_inventory;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SourceFileInventory {
    pub(crate) files: usize,
    pub(crate) complete: bool,
}

pub(crate) fn source_file_inventory(
    metadata: &Metadata,
    scan: Option<&ScanExecution>,
    source: Option<&SourceScan>,
) -> SourceFileInventory {
    source_inventory::collect(metadata, scan, source)
}

pub const SCORE_MODEL: &str = "core-v1";
const SHARE_BASE_URL: &str = "https://rust-doctor.vercel.app/share";
const MAX_SHARED_COUNT: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Audit {
    pub source_files: usize,
    pub categories: Vec<AuditCategory>,
    pub score: Option<AuditScore>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditCategory {
    pub name: AuditCategoryName,
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum AuditCategoryName {
    Security,
    Bugs,
    Performance,
    Dependencies,
    Maintainability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditScore {
    pub model: String,
    pub value: u8,
    pub label: ScoreLabel,
    pub authoritative: bool,
    pub dimensions: ScoreDimensions,
    pub projected_after_top_three: Option<u8>,
    pub projected_rule_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct ScoreDimensions {
    pub security: u8,
    pub reliability: u8,
    pub maintainability: u8,
    pub performance: u8,
    pub dependencies: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ScoreLabel {
    Great,
    #[serde(rename = "Needs work")]
    NeedsWork,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareError {
    ScoreUnavailable,
    NonAuthoritative,
    InvalidPayload,
}

impl fmt::Display for ShareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ScoreUnavailable => "the audit does not contain a score",
            Self::NonAuthoritative => "the audit score is not authoritative",
            Self::InvalidPayload => "the audit exceeds the public share bounds",
        })
    }
}

impl std::error::Error for ShareError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ScoreDimension {
    Security,
    Reliability,
    Maintainability,
    Performance,
    Dependencies,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuleAggregate {
    pub(crate) id: String,
    pub(crate) effective_severity: Severity,
    scored_severity: Option<Severity>,
    pub(crate) category: Option<AuditCategoryName>,
    dimension: Option<ScoreDimension>,
}

#[derive(Debug)]
pub(crate) struct RuleAggregation {
    pub(crate) rules: Vec<RuleAggregate>,
    diagnostics_are_authoritative: bool,
}

#[derive(Debug)]
struct PendingRule {
    id: String,
    severity: Severity,
    category: Option<AuditCategoryName>,
    dimension: Option<ScoreDimension>,
    scored_severity: Option<Severity>,
    mapping_conflict: bool,
}

impl Audit {
    pub(crate) fn build(source_files: usize, status: Status, diagnostics: &[Diagnostic]) -> Self {
        Self::build_with_authority(source_files, status == Status::Complete, diagnostics)
    }

    pub(crate) fn build_from_inventory(
        inventory: SourceFileInventory,
        status: Status,
        diagnostics: &[Diagnostic],
    ) -> Self {
        Self::build_with_authority(
            inventory.files,
            status == Status::Complete && inventory.complete,
            diagnostics,
        )
    }

    fn build_with_authority(
        source_files: usize,
        analysis_is_authoritative: bool,
        diagnostics: &[Diagnostic],
    ) -> Self {
        let categories = category_tallies(diagnostics);
        let aggregation = aggregate_rules(diagnostics);
        let score = (source_files > 0).then(|| score(&aggregation, analysis_is_authoritative));
        Self {
            source_files,
            categories,
            score,
        }
    }

    pub fn share_url(&self) -> Result<String, ShareError> {
        let score = self.score.as_ref().ok_or(ShareError::ScoreUnavailable)?;
        if !score.authoritative {
            return Err(ShareError::NonAuthoritative);
        }
        if !self.is_valid() {
            return Err(ShareError::InvalidPayload);
        }

        let mut errors = 0usize;
        let mut warnings = 0usize;
        let mut info = 0usize;
        for category in &self.categories {
            errors = errors.saturating_add(category.errors);
            warnings = warnings.saturating_add(category.warnings);
            info = info.saturating_add(category.info);
        }
        build_share_url(score.value, errors, warnings, info, self.source_files)
    }

    pub fn is_valid(&self) -> bool {
        let mut previous = None;
        let categories_are_valid = self.categories.iter().all(|category| {
            let position = category_position(category.name);
            let ordered = previous.is_none_or(|previous| previous < position);
            previous = Some(position);
            ordered
                && category
                    .errors
                    .saturating_add(category.warnings)
                    .saturating_add(category.info)
                    .saturating_add(category.unknown)
                    > 0
        });
        categories_are_valid
            && (self.source_files > 0) == self.score.is_some()
            && self.score.as_ref().is_none_or(AuditScore::is_valid)
    }
}

impl AuditScore {
    pub fn is_valid(&self) -> bool {
        self.model == SCORE_MODEL
            && self.value <= 100
            && self.label == score_label(self.value)
            && self
                .dimensions
                .values()
                .into_iter()
                .all(|value| value <= 100)
            && self
                .projected_after_top_three
                .is_none_or(|value| value <= 100)
            && self
                .projected_after_top_three
                .is_none_or(|projected| projected >= self.value)
            && self.projected_rule_ids.len() <= 3
            && self
                .projected_rule_ids
                .iter()
                .all(|rule_id| !rule_id.is_empty())
            && self
                .projected_rule_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                == self.projected_rule_ids.len()
            && if self.authoritative {
                matches!(
                    (
                        self.projected_after_top_three,
                        self.projected_rule_ids.is_empty()
                    ),
                    (None, true) | (Some(_), false)
                )
            } else {
                self.projected_after_top_three.is_none() && self.projected_rule_ids.is_empty()
            }
    }
}

impl ScoreDimensions {
    fn values(self) -> [u8; 5] {
        [
            self.security,
            self.reliability,
            self.maintainability,
            self.performance,
            self.dependencies,
        ]
    }

    fn value_for(self, dimension: ScoreDimension) -> u8 {
        match dimension {
            ScoreDimension::Security => self.security,
            ScoreDimension::Reliability => self.reliability,
            ScoreDimension::Maintainability => self.maintainability,
            ScoreDimension::Performance => self.performance,
            ScoreDimension::Dependencies => self.dependencies,
        }
    }
}

impl ScoreLabel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Great => "Great",
            Self::NeedsWork => "Needs work",
            Self::Critical => "Critical",
        }
    }
}

impl fmt::Display for ScoreLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AuditCategoryName {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Security => "Security",
            Self::Bugs => "Bugs",
            Self::Performance => "Performance",
            Self::Dependencies => "Dependencies",
            Self::Maintainability => "Maintainability",
        }
    }
}

const fn category_position(category: AuditCategoryName) -> u8 {
    match category {
        AuditCategoryName::Security => 0,
        AuditCategoryName::Bugs => 1,
        AuditCategoryName::Performance => 2,
        AuditCategoryName::Dependencies => 3,
        AuditCategoryName::Maintainability => 4,
    }
}

impl fmt::Display for AuditCategoryName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub(crate) fn category_mapping(category: &str) -> Option<(AuditCategoryName, ScoreDimension)> {
    match category {
        "security" => Some((AuditCategoryName::Security, ScoreDimension::Security)),
        "correctness" | "reliability" => {
            Some((AuditCategoryName::Bugs, ScoreDimension::Reliability))
        }
        "performance" => Some((AuditCategoryName::Performance, ScoreDimension::Performance)),
        "cargo" | "dependencies" => Some((
            AuditCategoryName::Dependencies,
            ScoreDimension::Dependencies,
        )),
        "maintainability" => Some((
            AuditCategoryName::Maintainability,
            ScoreDimension::Maintainability,
        )),
        _ => None,
    }
}

pub(crate) const fn severity_penalty_quarters(severity: Severity) -> u64 {
    match severity {
        Severity::Error => 6,
        Severity::Warning => 3,
        Severity::Info => 1,
        Severity::Unknown => 0,
    }
}

pub(crate) const fn dimension_weight_twice(dimension: ScoreDimension) -> u64 {
    match dimension {
        ScoreDimension::Security => 4,
        ScoreDimension::Reliability => 3,
        ScoreDimension::Maintainability
        | ScoreDimension::Performance
        | ScoreDimension::Dependencies => 2,
    }
}

impl RuleAggregate {
    pub(crate) const fn contribution(&self) -> u64 {
        match (self.scored_severity, self.dimension) {
            (Some(severity), Some(dimension)) => {
                severity_penalty_quarters(severity) * dimension_weight_twice(dimension)
            }
            _ => 0,
        }
    }

    const fn is_scorable(&self) -> bool {
        self.scored_severity.is_some() && self.dimension.is_some()
    }
}

fn category_tallies(diagnostics: &[Diagnostic]) -> Vec<AuditCategory> {
    let mut tallies = BTreeMap::<AuditCategoryName, AuditCategory>::new();
    for diagnostic in diagnostics {
        let Some((name, _)) = diagnostic.category.as_deref().and_then(category_mapping) else {
            continue;
        };
        let tally = tallies.entry(name).or_insert(AuditCategory {
            name,
            errors: 0,
            warnings: 0,
            info: 0,
            unknown: 0,
        });
        let count = diagnostic.occurrences;
        match diagnostic.severity {
            Severity::Error => tally.errors = tally.errors.saturating_add(count),
            Severity::Warning => tally.warnings = tally.warnings.saturating_add(count),
            Severity::Info => tally.info = tally.info.saturating_add(count),
            Severity::Unknown => tally.unknown = tally.unknown.saturating_add(count),
        }
    }

    [
        AuditCategoryName::Security,
        AuditCategoryName::Bugs,
        AuditCategoryName::Performance,
        AuditCategoryName::Dependencies,
        AuditCategoryName::Maintainability,
    ]
    .into_iter()
    .filter_map(|name| tallies.remove(&name))
    .collect()
}

fn score(aggregation: &RuleAggregation, scan_complete: bool) -> AuditScore {
    let dimensions = calculate_dimensions(&aggregation.rules, &BTreeSet::new());
    let value = weighted_score(dimensions);

    let mut projected_rules: Vec<_> = aggregation
        .rules
        .iter()
        .filter(|rule| rule.is_scorable())
        .cloned()
        .collect();
    projected_rules.sort_by(|left, right| {
        right
            .contribution()
            .cmp(&left.contribution())
            .then_with(|| left.id.cmp(&right.id))
    });
    let projected_rule_ids: Vec<_> = projected_rules
        .iter()
        .take(3)
        .map(|rule| rule.id.clone())
        .collect();
    let authoritative = scan_complete && aggregation.diagnostics_are_authoritative;
    let projected_after_top_three = (authoritative && !projected_rule_ids.is_empty()).then(|| {
        let removed: BTreeSet<_> = projected_rule_ids.iter().cloned().collect();
        weighted_score(calculate_dimensions(&aggregation.rules, &removed))
    });
    let projected_rule_ids = if authoritative {
        projected_rule_ids
    } else {
        Vec::new()
    };

    AuditScore {
        model: SCORE_MODEL.to_owned(),
        value,
        label: score_label(value),
        authoritative,
        dimensions,
        projected_after_top_three,
        projected_rule_ids,
    }
}

pub(crate) fn aggregate_rules<'a>(
    diagnostics: impl IntoIterator<Item = &'a Diagnostic>,
) -> RuleAggregation {
    let mut diagnostics_are_authoritative = true;
    let mut rules = BTreeMap::<String, PendingRule>::new();
    for diagnostic in diagnostics {
        let Some(rule_id) = diagnostic.code.as_deref().filter(|code| !code.is_empty()) else {
            diagnostics_are_authoritative = false;
            continue;
        };
        let mapping = diagnostic.category.as_deref().and_then(category_mapping);
        let valid = diagnostic.severity != Severity::Unknown && mapping.is_some();
        if !valid {
            diagnostics_are_authoritative = false;
        }
        let rule = rules.entry(rule_id.to_owned()).or_insert(PendingRule {
            id: rule_id.to_owned(),
            severity: diagnostic.severity,
            category: None,
            dimension: None,
            scored_severity: None,
            mapping_conflict: false,
        });
        if diagnostic.severity.rank() < rule.severity.rank() {
            rule.severity = diagnostic.severity;
        }
        if valid && let Some((category, dimension)) = mapping {
            rule.scored_severity =
                Some(rule.scored_severity.map_or(diagnostic.severity, |current| {
                    if diagnostic.severity.rank() < current.rank() {
                        diagnostic.severity
                    } else {
                        current
                    }
                }));
            if rule.dimension.is_some_and(|current| current != dimension) {
                rule.mapping_conflict = true;
                diagnostics_are_authoritative = false;
            } else if rule.dimension.is_none() {
                rule.category = Some(category);
                rule.dimension = Some(dimension);
            }
        } else if rule.category.is_none()
            && let Some((category, _)) = mapping
        {
            rule.category = Some(category);
        }
    }
    RuleAggregation {
        rules: rules
            .into_values()
            .map(|rule| RuleAggregate {
                id: rule.id,
                effective_severity: rule.severity,
                scored_severity: rule.scored_severity,
                category: (!rule.mapping_conflict).then_some(rule.category).flatten(),
                dimension: (!rule.mapping_conflict && rule.scored_severity.is_some())
                    .then_some(rule.dimension)
                    .flatten(),
            })
            .collect(),
        diagnostics_are_authoritative,
    }
}

fn calculate_dimensions(rules: &[RuleAggregate], removed: &BTreeSet<String>) -> ScoreDimensions {
    let penalty_for = |dimension| {
        rules
            .iter()
            .filter(|rule| rule.dimension == Some(dimension) && !removed.contains(&rule.id))
            .fold(0u64, |penalty, rule| {
                penalty.saturating_add(rule.scored_severity.map_or(0, severity_penalty_quarters))
            })
    };
    ScoreDimensions {
        security: dimension_score(penalty_for(ScoreDimension::Security)),
        reliability: dimension_score(penalty_for(ScoreDimension::Reliability)),
        maintainability: dimension_score(penalty_for(ScoreDimension::Maintainability)),
        performance: dimension_score(penalty_for(ScoreDimension::Performance)),
        dependencies: dimension_score(penalty_for(ScoreDimension::Dependencies)),
    }
}

fn dimension_score(penalty_quarters: u64) -> u8 {
    let remaining_quarters = 400u64.saturating_sub(penalty_quarters);
    u8::try_from((remaining_quarters.saturating_add(2) / 4).min(100)).unwrap_or(100)
}

fn weighted_score(dimensions: ScoreDimensions) -> u8 {
    let numerator = [
        ScoreDimension::Security,
        ScoreDimension::Reliability,
        ScoreDimension::Maintainability,
        ScoreDimension::Performance,
        ScoreDimension::Dependencies,
    ]
    .into_iter()
    .fold(0u64, |sum, dimension| {
        sum.saturating_add(
            u64::from(dimensions.value_for(dimension)) * dimension_weight_twice(dimension),
        )
    });
    u8::try_from((numerator.saturating_add(6) / 13).min(100)).unwrap_or(100)
}

pub(crate) const fn score_label(score: u8) -> ScoreLabel {
    if score >= 75 {
        ScoreLabel::Great
    } else if score >= 50 {
        ScoreLabel::NeedsWork
    } else {
        ScoreLabel::Critical
    }
}

fn build_share_url(
    score: u8,
    errors: usize,
    warnings: usize,
    info: usize,
    source_files: usize,
) -> Result<String, ShareError> {
    if score > 100
        || [errors, warnings, info, source_files]
            .into_iter()
            .any(|count| count > MAX_SHARED_COUNT)
    {
        return Err(ShareError::InvalidPayload);
    }

    let mut url = format!("{SHARE_BASE_URL}?s={score}");
    for (key, count) in [
        ("e", errors),
        ("w", warnings),
        ("i", info),
        ("f", source_files),
    ] {
        if count > 0 {
            let _ = write!(url, "&{key}={count}");
        }
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::Value;

    use super::*;
    use crate::report::{DiagnosticSource, DiagnosticSpan};

    #[derive(Debug, Deserialize)]
    struct Oracle {
        schema_version: u8,
        model: String,
        category_mappings: BTreeMap<String, CategoryExpectation>,
        score_boundaries: Vec<ScoreBoundaryExpectation>,
        rounding_cases: Vec<RoundingExpectation>,
        score_cases: Vec<ScoreCase>,
        share_cases: Vec<ShareCase>,
    }

    #[derive(Debug, Deserialize)]
    struct CategoryExpectation {
        display: String,
        dimension: String,
    }

    #[derive(Debug, Deserialize)]
    struct ScoreBoundaryExpectation {
        name: String,
        dimensions: ScoreDimensions,
        expected_value: u8,
        expected_label: String,
        expected_counted_rule_ids: Vec<String>,
    }

    #[derive(Debug, Deserialize)]
    struct RoundingExpectation {
        penalty_quarters: u64,
        score: u8,
    }

    #[derive(Debug, Deserialize)]
    struct ScoreCase {
        name: String,
        source_files: usize,
        complete: bool,
        diagnostics: Vec<OracleDiagnostic>,
        expected_audit: Value,
        expected_counted_rule_ids: Vec<String>,
    }

    #[derive(Debug, Deserialize)]
    struct OracleDiagnostic {
        rule_id: Option<String>,
        category: Option<String>,
        severity: String,
        occurrences: usize,
    }

    #[derive(Debug, Deserialize)]
    struct ShareCase {
        score: u8,
        errors: usize,
        warnings: usize,
        info: usize,
        files: usize,
        expected: Option<String>,
    }

    fn oracle() -> Oracle {
        serde_json::from_str(include_str!(
            "../tests/fixtures/local-cli-experience/audit-core-v1.json"
        ))
        .expect("audit oracle should be valid")
    }

    fn diagnostic(input: &OracleDiagnostic, index: usize) -> Diagnostic {
        Diagnostic {
            id: format!("finding-{index}"),
            source: DiagnosticSource::Clippy,
            code: input.rule_id.clone(),
            base_severity: severity(&input.severity).expect("oracle severity should be known"),
            severity: severity(&input.severity).expect("oracle severity should be known"),
            category: input.category.clone(),
            message: format!("oracle finding {index}"),
            help: None,
            package: None,
            target: None,
            path: Some(format!("src/{index}.rs")),
            span: Some(DiagnosticSpan {
                line_start: 1,
                column_start: 1,
                line_end: 1,
                column_end: 2,
            }),
            occurrences: input.occurrences,
        }
    }

    fn severity(value: &str) -> Option<Severity> {
        match value {
            "error" => Some(Severity::Error),
            "warning" => Some(Severity::Warning),
            "info" => Some(Severity::Info),
            "unknown" => Some(Severity::Unknown),
            _ => None,
        }
    }

    #[test]
    fn versioned_oracle_covers_categories_labels_scores_and_rule_identity() {
        let oracle = oracle();
        assert_eq!(oracle.schema_version, 1);
        assert_eq!(oracle.model, SCORE_MODEL);
        for (category, expected) in oracle.category_mappings {
            let (display, dimension) = category_mapping(&category).expect("mapped category");
            assert_eq!(display.to_string(), expected.display, "{category}");
            assert_eq!(format!("{dimension:?}"), expected.dimension, "{category}");
        }
        for expected in oracle.score_boundaries {
            assert_eq!(
                weighted_score(expected.dimensions),
                expected.expected_value,
                "{}",
                expected.name
            );
            assert_eq!(
                score_label(expected.expected_value).to_string(),
                expected.expected_label,
                "{}",
                expected.name
            );
            assert!(
                expected.expected_counted_rule_ids.is_empty(),
                "dimension-only boundary cases must not invent Rule IDs: {}",
                expected.name
            );
        }
        for expected in oracle.rounding_cases {
            assert_eq!(
                dimension_score(expected.penalty_quarters),
                expected.score,
                "penalty {} quarters",
                expected.penalty_quarters
            );
            let dimensions = ScoreDimensions {
                security: expected.score,
                reliability: expected.score,
                maintainability: expected.score,
                performance: expected.score,
                dependencies: expected.score,
            };
            assert_eq!(weighted_score(dimensions), expected.score);
        }
        for case in oracle.score_cases {
            let diagnostics: Vec<_> = case
                .diagnostics
                .iter()
                .enumerate()
                .map(|(index, input)| diagnostic(input, index))
                .collect();
            let status = if case.complete {
                Status::Complete
            } else {
                Status::Incomplete
            };
            let audit = Audit::build(case.source_files, status, &diagnostics);
            assert_eq!(
                serde_json::to_value(&audit).unwrap(),
                case.expected_audit,
                "{}",
                case.name
            );
            let input = aggregate_rules(&diagnostics);
            let counted: Vec<_> = input
                .rules
                .into_iter()
                .filter(RuleAggregate::is_scorable)
                .map(|rule| rule.id)
                .collect();
            assert_eq!(counted, case.expected_counted_rule_ids, "{}", case.name);
            assert!(audit.is_valid(), "{}", case.name);
        }
    }

    #[test]
    fn versioned_share_oracle_matches_public_query_bounds() {
        for case in oracle().share_cases {
            let actual = build_share_url(
                case.score,
                case.errors,
                case.warnings,
                case.info,
                case.files,
            )
            .ok();
            assert_eq!(actual, case.expected);
        }
        assert_eq!(score_label(52), ScoreLabel::NeedsWork);
    }

    #[test]
    fn invalid_score_state_is_rejected_before_sharing() {
        let audit = Audit {
            source_files: 1,
            categories: Vec::new(),
            score: Some(AuditScore {
                model: SCORE_MODEL.to_owned(),
                value: 101,
                label: ScoreLabel::Great,
                authoritative: true,
                dimensions: ScoreDimensions {
                    security: 100,
                    reliability: 100,
                    maintainability: 100,
                    performance: 100,
                    dependencies: 100,
                },
                projected_after_top_three: None,
                projected_rule_ids: Vec::new(),
            }),
        };

        assert!(!audit.is_valid());
        assert_eq!(audit.share_url(), Err(ShareError::InvalidPayload));
        assert_eq!(
            build_share_url(100, 1_000_001, 0, 0, 1),
            Err(ShareError::InvalidPayload)
        );
    }

    #[test]
    fn incomplete_source_inventory_never_emits_an_authoritative_score() {
        let audit = Audit::build_from_inventory(
            SourceFileInventory {
                files: 1,
                complete: false,
            },
            Status::Complete,
            &[],
        );

        assert_eq!(audit.score.as_ref().map(|score| score.value), Some(100));
        assert_eq!(
            audit.score.as_ref().map(|score| score.authoritative),
            Some(false)
        );
    }
}
