use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use serde::ser::{Error as _, SerializeStruct};
use serde::{Deserialize, Serialize, Serializer};

use cargo_metadata::Metadata;

use crate::execution::ScanExecution;
use crate::policy::RuleTier;
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

pub const SCORE_MODEL: &str = "core-v2";
const SHARE_BASE_URL: &str = "https://rust-doctor.vercel.app/share";
const MAX_SHARED_COUNT: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audit {
    pub source_files: usize,
    pub categories: Vec<AuditCategory>,
    pub score: Option<AuditScore>,
}

/// Per-severity count of a single quantity.
///
/// The report publishes two distinct quantities: the number of distinct
/// diagnostics and the number of occurrences. Every surface exposes both under
/// explicit names, and `total` is always the sum of the four severities.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SeverityCounts {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub unknown: usize,
    pub total: usize,
}

impl SeverityCounts {
    pub(crate) fn add(&mut self, severity: Severity, count: usize) {
        let bucket = match severity {
            Severity::Error => &mut self.errors,
            Severity::Warning => &mut self.warnings,
            Severity::Info => &mut self.info,
            Severity::Unknown => &mut self.unknown,
        };
        *bucket = bucket.saturating_add(count);
        self.total = self.total.saturating_add(count);
    }

    const fn is_coherent(self) -> bool {
        self.errors
            .saturating_add(self.warnings)
            .saturating_add(self.info)
            .saturating_add(self.unknown)
            == self.total
    }

    /// At least one occurrence per distinct diagnostic, never the reverse.
    const fn covers(self, distinct: Self) -> bool {
        self.errors >= distinct.errors
            && self.warnings >= distinct.warnings
            && self.info >= distinct.info
            && self.unknown >= distinct.unknown
            && self.total >= distinct.total
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditCategory {
    pub name: AuditCategoryName,
    /// Historical alias of `occurrences.errors`, kept for consumers of the
    /// previous schema.
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub unknown: usize,
    pub distinct: SeverityCounts,
    pub occurrences: SeverityCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum AuditCategoryName {
    Security,
    Bugs,
    Performance,
    Dependencies,
    Maintainability,
    /// Diagnostic with no catalog category, a compilation error for instance.
    /// The bucket exists so that no diagnostic disappears between `summary`
    /// and `audit.categories`.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditScore {
    pub model: String,
    pub value: u8,
    pub label: ScoreLabel,
    pub authoritative: bool,
    pub dimensions: ScoreDimensions,
    /// Worst tier observed across all dimensions, or `null` when no scored
    /// rule is catalogued.
    pub worst_tier: Option<RuleTier>,
    /// Global cap effectively applied to `value`, or `null` when the worst tier
    /// imposes none. Published so that a score drop can be explained without
    /// recomputation.
    pub applied_ceiling: Option<u8>,
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
    tier: Option<RuleTier>,
    occurrences: usize,
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
    tier: Option<RuleTier>,
    occurrences: usize,
}

impl Audit {
    pub fn build(source_files: usize, status: Status, diagnostics: &[Diagnostic]) -> Self {
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
        // Both quantities count everything the report publishes, FR-06
        // requires them to equal `summary`. Only the score sets aside
        // non-production diagnostics: they stay visible and counted, they stop
        // costing points.
        let aggregation = aggregate_rules(
            diagnostics
                .iter()
                .filter(|diagnostic| crate::report::DiagnosticContext::weighs(diagnostic)),
        );
        let score = (source_files > 0).then(|| score(&aggregation, analysis_is_authoritative));
        Self {
            source_files,
            categories,
            score,
        }
    }

    pub(crate) fn rebuild_for_scope(&self, status: Status, diagnostics: &[Diagnostic]) -> Self {
        let inventory_is_authoritative =
            self.score.as_ref().is_some_and(|score| score.authoritative);
        Self::build_with_authority(
            self.source_files,
            status == Status::Complete && inventory_is_authoritative,
            diagnostics,
        )
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
            ordered && category.is_valid()
        });
        categories_are_valid
            && (self.source_files > 0) == self.score.is_some()
            && self.score.as_ref().is_none_or(AuditScore::is_valid)
    }

    /// Both quantities aggregated over every category of the block.
    pub fn totals(&self) -> (SeverityCounts, SeverityCounts) {
        self.categories.iter().fold(
            (SeverityCounts::default(), SeverityCounts::default()),
            |(mut distinct, mut occurrences), category| {
                for (target, source) in [
                    (&mut distinct, category.distinct),
                    (&mut occurrences, category.occurrences),
                ] {
                    target.errors = target.errors.saturating_add(source.errors);
                    target.warnings = target.warnings.saturating_add(source.warnings);
                    target.info = target.info.saturating_add(source.info);
                    target.unknown = target.unknown.saturating_add(source.unknown);
                    target.total = target.total.saturating_add(source.total);
                }
                (distinct, occurrences)
            },
        )
    }
}

impl Serialize for Audit {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if !self.is_valid() {
            return Err(S::Error::custom("invalid audit state"));
        }
        let mut state = serializer.serialize_struct("Audit", 3)?;
        state.serialize_field("source_files", &self.source_files)?;
        state.serialize_field("categories", &self.categories)?;
        state.serialize_field("score", &self.score)?;
        state.end()
    }
}

impl AuditCategory {
    fn empty(name: AuditCategoryName) -> Self {
        Self {
            name,
            errors: 0,
            warnings: 0,
            info: 0,
            unknown: 0,
            distinct: SeverityCounts::default(),
            occurrences: SeverityCounts::default(),
        }
    }

    fn is_valid(&self) -> bool {
        self.distinct.total > 0
            && self.distinct.is_coherent()
            && self.occurrences.is_coherent()
            && self.occurrences.covers(self.distinct)
            && self.errors == self.occurrences.errors
            && self.warnings == self.occurrences.warnings
            && self.info == self.occurrences.info
            && self.unknown == self.occurrences.unknown
    }
}

impl AuditScore {
    pub fn is_valid(&self) -> bool {
        self.model == SCORE_MODEL
            && self.value <= 100
            && self.applied_ceiling == self.worst_tier.and_then(tier_overall_ceiling)
            && self.value == capped(weighted_score(self.dimensions), self.applied_ceiling)
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
            Self::Other => "Other",
        }
    }
}

const ORDERED_CATEGORIES: [AuditCategoryName; 6] = [
    AuditCategoryName::Security,
    AuditCategoryName::Bugs,
    AuditCategoryName::Performance,
    AuditCategoryName::Dependencies,
    AuditCategoryName::Maintainability,
    AuditCategoryName::Other,
];

const fn category_position(category: AuditCategoryName) -> u8 {
    match category {
        AuditCategoryName::Security => 0,
        AuditCategoryName::Bugs => 1,
        AuditCategoryName::Performance => 2,
        AuditCategoryName::Dependencies => 3,
        AuditCategoryName::Maintainability => 4,
        AuditCategoryName::Other => 5,
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

/// Every diagnostic falls into exactly one bucket, including those no catalog
/// category covers. Without that, `summary` and `audit.categories` would count
/// different populations.
pub(crate) fn category_bucket(category: Option<&str>) -> AuditCategoryName {
    category
        .and_then(category_mapping)
        .map_or(AuditCategoryName::Other, |(name, _)| name)
}

/// Cap imposed on a dimension by the worst tier it contains.
pub(crate) const fn tier_dimension_ceiling(tier: RuleTier) -> Option<u8> {
    match tier {
        RuleTier::P0 => Some(20),
        RuleTier::P1 => Some(50),
        RuleTier::P2 => Some(75),
        RuleTier::P3 => None,
    }
}

/// Cap imposed on the overall score by the worst tier across all dimensions.
pub(crate) const fn tier_overall_ceiling(tier: RuleTier) -> Option<u8> {
    match tier {
        RuleTier::P0 => Some(40),
        RuleTier::P1 => Some(65),
        RuleTier::P2 | RuleTier::P3 => None,
    }
}

/// Occurrence steps applied to a rule's penalty, published as inclusive upper
/// bounds.
pub(crate) const OCCURRENCE_STEPS: [(usize, u64); 4] = [(1, 1), (5, 2), (20, 3), (usize::MAX, 4)];

/// Saturating multiplier: a rule can never go past the last step, whatever its
/// occurrence count.
pub(crate) const fn occurrence_multiplier(occurrences: usize) -> u64 {
    let mut index = 0;
    while index < OCCURRENCE_STEPS.len() {
        if occurrences <= OCCURRENCE_STEPS[index].0 {
            return OCCURRENCE_STEPS[index].1;
        }
        index += 1;
    }
    OCCURRENCE_STEPS[OCCURRENCE_STEPS.len() - 1].1
}

const fn capped(value: u8, ceiling: Option<u8>) -> u8 {
    match ceiling {
        Some(ceiling) if ceiling < value => ceiling,
        _ => value,
    }
}

/// The worst tier is the minimum, `P0` being declared first.
fn worse_tier(current: Option<RuleTier>, candidate: Option<RuleTier>) -> Option<RuleTier> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.min(candidate)),
        (current, candidate) => current.or(candidate),
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
    /// Penalty in quarter points, scored severity times occurrence step.
    pub(crate) const fn penalty_quarters(&self) -> u64 {
        match self.scored_severity {
            Some(severity) => severity_penalty_quarters(severity)
                .saturating_mul(occurrence_multiplier(self.occurrences)),
            None => 0,
        }
    }

    pub(crate) const fn contribution(&self) -> u64 {
        match self.dimension {
            Some(dimension) => self
                .penalty_quarters()
                .saturating_mul(dimension_weight_twice(dimension)),
            None => 0,
        }
    }

    /// A non-scorable rule caps nothing: a tier cannot act without a retained
    /// dimension and severity.
    const fn scoring_tier(&self) -> Option<RuleTier> {
        if self.is_scorable() { self.tier } else { None }
    }

    const fn is_scorable(&self) -> bool {
        self.scored_severity.is_some() && self.dimension.is_some()
    }
}

fn category_tallies(diagnostics: &[Diagnostic]) -> Vec<AuditCategory> {
    let mut tallies = BTreeMap::<AuditCategoryName, AuditCategory>::new();
    for diagnostic in diagnostics {
        let name = category_bucket(diagnostic.category.as_deref());
        let tally = tallies
            .entry(name)
            .or_insert_with(|| AuditCategory::empty(name));
        tally.distinct.add(diagnostic.severity, 1);
        tally
            .occurrences
            .add(diagnostic.severity, diagnostic.occurrences);
    }

    ORDERED_CATEGORIES
        .into_iter()
        .filter_map(|name| tallies.remove(&name))
        .map(|mut tally| {
            tally.errors = tally.occurrences.errors;
            tally.warnings = tally.occurrences.warnings;
            tally.info = tally.occurrences.info;
            tally.unknown = tally.occurrences.unknown;
            tally
        })
        .collect()
}

fn score(aggregation: &RuleAggregation, scan_complete: bool) -> AuditScore {
    let scored = ScoredState::of(&aggregation.rules, &BTreeSet::new());

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
        ScoredState::of(&aggregation.rules, &removed).value
    });
    let projected_rule_ids = if authoritative {
        projected_rule_ids
    } else {
        Vec::new()
    };

    AuditScore {
        model: SCORE_MODEL.to_owned(),
        value: scored.value,
        label: score_label(scored.value),
        authoritative,
        dimensions: scored.dimensions,
        worst_tier: scored.worst_tier,
        applied_ceiling: scored.applied_ceiling,
        projected_after_top_three,
        projected_rule_ids,
    }
}

/// Capped score and its cause, for a given set of rules.
struct ScoredState {
    dimensions: ScoreDimensions,
    worst_tier: Option<RuleTier>,
    applied_ceiling: Option<u8>,
    value: u8,
}

impl ScoredState {
    fn of(rules: &[RuleAggregate], removed: &BTreeSet<String>) -> Self {
        let dimensions = calculate_dimensions(rules, removed);
        let worst_tier = rules
            .iter()
            .filter(|rule| !removed.contains(&rule.id))
            .fold(None, |worst, rule| worse_tier(worst, rule.scoring_tier()));
        let applied_ceiling = worst_tier.and_then(tier_overall_ceiling);
        Self {
            dimensions,
            worst_tier,
            applied_ceiling,
            value: capped(weighted_score(dimensions), applied_ceiling),
        }
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
            tier: crate::policy::find(rule_id).map(|definition| definition.tier),
            occurrences: 0,
        });
        rule.occurrences = rule.occurrences.saturating_add(diagnostic.occurrences);
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
                tier: rule.tier,
                occurrences: rule.occurrences,
            })
            .collect(),
        diagnostics_are_authoritative,
    }
}

fn calculate_dimensions(rules: &[RuleAggregate], removed: &BTreeSet<String>) -> ScoreDimensions {
    let score_for = |dimension| {
        let (penalty, worst) = rules
            .iter()
            .filter(|rule| rule.dimension == Some(dimension) && !removed.contains(&rule.id))
            .fold((0u64, None), |(penalty, worst), rule| {
                (
                    penalty.saturating_add(rule.penalty_quarters()),
                    worse_tier(worst, rule.scoring_tier()),
                )
            });
        capped(
            dimension_score(penalty),
            worst.and_then(tier_dimension_ceiling),
        )
    };
    ScoreDimensions {
        security: score_for(ScoreDimension::Security),
        reliability: score_for(ScoreDimension::Reliability),
        maintainability: score_for(ScoreDimension::Maintainability),
        performance: score_for(ScoreDimension::Performance),
        dependencies: score_for(ScoreDimension::Dependencies),
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
    use crate::policy::CATALOG;
    use crate::report::{DiagnosticSource, DiagnosticSpan};

    #[derive(Debug, Deserialize)]
    struct Oracle {
        schema_version: u8,
        model: String,
        uncategorized_bucket: String,
        category_mappings: BTreeMap<String, CategoryExpectation>,
        rule_tiers: BTreeMap<String, String>,
        tier_ceilings: Vec<TierCeilingExpectation>,
        occurrence_steps: Vec<OccurrenceStepExpectation>,
        score_boundaries: Vec<ScoreBoundaryExpectation>,
        rounding_cases: Vec<RoundingExpectation>,
        score_cases: Vec<ScoreCase>,
        share_cases: Vec<ShareCase>,
    }

    #[derive(Debug, Deserialize)]
    struct TierCeilingExpectation {
        tier: String,
        dimension: Option<u8>,
        overall: Option<u8>,
    }

    #[derive(Debug, Deserialize)]
    struct OccurrenceStepExpectation {
        occurrences: usize,
        multiplier: u64,
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
            "../tests/fixtures/local-cli-experience/audit-core-v2.json"
        ))
        .expect("audit oracle should be valid")
    }

    fn diagnostic(input: &OracleDiagnostic, index: usize) -> Diagnostic {
        Diagnostic {
            context: None,
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
            related: Vec::new(),
            similarity_basis_points: None,
            complexity: None,
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
        assert_eq!(oracle.schema_version, 2);
        assert_eq!(oracle.model, SCORE_MODEL);
        for (category, expected) in oracle.category_mappings {
            let (display, dimension) = category_mapping(&category).expect("mapped category");
            assert_eq!(display.to_string(), expected.display, "{category}");
            assert_eq!(format!("{dimension:?}"), expected.dimension, "{category}");
            assert_eq!(category_bucket(Some(&category)), display, "{category}");
        }
        for unmapped in [None, Some("future"), Some(""), Some("style")] {
            assert_eq!(
                category_bucket(unmapped).to_string(),
                oracle.uncategorized_bucket
            );
        }

        let tiers: BTreeMap<_, _> = CATALOG
            .iter()
            .map(|definition| {
                (
                    definition.id.to_owned(),
                    definition.tier.as_str().to_owned(),
                )
            })
            .collect();
        assert_eq!(tiers, oracle.rule_tiers);

        let mut previous: Option<(u8, u8)> = None;
        for expected in oracle.tier_ceilings {
            let tier = RuleTier::parse(&expected.tier).expect("published tier should be known");
            assert_eq!(tier_dimension_ceiling(tier), expected.dimension, "{tier:?}");
            assert_eq!(tier_overall_ceiling(tier), expected.overall, "{tier:?}");
            let current = (
                expected.dimension.unwrap_or(100),
                expected.overall.unwrap_or(100),
            );
            if let Some(previous) = previous {
                assert!(
                    previous.0 < current.0 && previous.1 <= current.1,
                    "caps must decrease strictly with gravity: {previous:?} then {current:?}",
                );
            }
            previous = Some(current);
        }

        for expected in oracle.occurrence_steps {
            assert_eq!(
                occurrence_multiplier(expected.occurrences),
                expected.multiplier,
                "{} occurrences",
                expected.occurrences
            );
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
                worst_tier: None,
                applied_ceiling: None,
                projected_after_top_three: None,
                projected_rule_ids: Vec::new(),
            }),
        };

        assert!(!audit.is_valid());
        assert_eq!(audit.share_url(), Err(ShareError::InvalidPayload));
        assert!(serde_json::to_vec(&audit).is_err());
        assert_eq!(
            build_share_url(100, 1_000_001, 0, 0, 1),
            Err(ShareError::InvalidPayload)
        );
    }

    fn catalog_diagnostics(occurrences: usize) -> Vec<Diagnostic> {
        CATALOG
            .iter()
            .enumerate()
            .map(|(index, definition)| Diagnostic {
                context: None,
                id: format!("finding-{index}"),
                source: DiagnosticSource::Clippy,
                code: Some(definition.id.to_owned()),
                base_severity: Severity::Error,
                severity: Severity::Error,
                category: Some(definition.category.to_owned()),
                message: "worst case".to_owned(),
                help: None,
                package: None,
                target: None,
                path: None,
                span: None,
                related: Vec::new(),
                similarity_basis_points: None,
                complexity: None,
                occurrences,
            })
            .collect()
    }

    /// What the real catalog produces, as opposed to the injected dimensions
    /// of the oracle.
    ///
    /// Under `core-v1`, saturating the twelve rules left the score at 96,
    /// label `Great`: the additive scale was structurally unable to go down.
    /// Under `core-v2` the worst observed tier caps the score, so the same
    /// catalog reaches the `Critical` band.
    #[test]
    fn the_catalog_drives_the_score_out_of_its_top_label() {
        let diagnostics = catalog_diagnostics(1);
        let audit = Audit::build(1, Status::Complete, &diagnostics);
        let score = audit.score.expect("a scored audit should exist");

        assert_eq!(score.value, 40);
        assert_eq!(score.label, ScoreLabel::Critical);
        assert_eq!(score.worst_tier, Some(RuleTier::P0));
        assert_eq!(score.applied_ceiling, Some(40));
        assert!(score.authoritative);

        assert_eq!(score.dimensions.security, 20);
        assert_eq!(score.dimensions.reliability, 50);
        assert_eq!(score.dimensions.maintainability, 79);
        // EP-024 opens `performance` and `dependencies`: no dimension stays
        // frozen at 100, so no weight of the scale is inert any more.
        assert_eq!(score.dimensions.performance, 75);
        assert_eq!(score.dimensions.dependencies, 50);
        assert!(
            score
                .dimensions
                .values()
                .into_iter()
                .all(|value| value < 100),
            "{:?}",
            score.dimensions
        );
    }

    fn diagnostics_for(rules: &[(&str, &str, Severity, usize)]) -> Vec<Diagnostic> {
        rules
            .iter()
            .enumerate()
            .map(
                |(index, (code, category, severity, occurrences))| Diagnostic {
                    context: None,
                    id: format!("finding-{index}"),
                    source: DiagnosticSource::Clippy,
                    code: Some((*code).to_owned()),
                    base_severity: *severity,
                    severity: *severity,
                    category: Some((*category).to_owned()),
                    message: format!("finding {index}"),
                    help: None,
                    package: None,
                    target: None,
                    path: None,
                    span: None,
                    related: Vec::new(),
                    similarity_basis_points: None,
                    complexity: None,
                    occurrences: *occurrences,
                },
            )
            .collect()
    }

    fn scored(rules: &[(&str, &str, Severity, usize)]) -> AuditScore {
        Audit::build(1, Status::Complete, &diagnostics_for(rules))
            .score
            .expect("a scored audit should exist")
    }

    /// A clean codebase takes no cap, and a cap is not invented out of a rule
    /// outside the catalog.
    #[test]
    fn a_clean_codebase_scores_one_hundred_without_any_ceiling() {
        let clean = Audit::build(1, Status::Complete, &[])
            .score
            .expect("a scored audit should exist");
        assert_eq!(clean.value, 100);
        assert_eq!(clean.worst_tier, None);
        assert_eq!(clean.applied_ceiling, None);

        let uncatalogued = scored(&[("clippy::unknown_rule", "security", Severity::Error, 1)]);
        assert_eq!(uncatalogued.worst_tier, None);
        assert_eq!(uncatalogued.applied_ceiling, None);
    }

    /// A tier only caps when it acts: a rule switched off by the policy carries
    /// an unknown severity, hence neither penalty nor cap.
    #[test]
    fn a_disabled_rule_neither_penalizes_nor_caps() {
        let disabled = scored(&[(
            "rust_doctor::source::dynamic_shell_command",
            "security",
            Severity::Unknown,
            1,
        )]);
        assert_eq!(disabled.value, 100);
        assert_eq!(disabled.worst_tier, None);
        assert_eq!(disabled.applied_ceiling, None);
        assert_eq!(disabled.dimensions.security, 100);
    }

    /// The worst tier of a dimension overrides the others, and a graver tier in
    /// another dimension still brings the overall score down.
    #[test]
    fn only_the_worst_tier_applies_per_dimension_and_overall() {
        let mixed = scored(&[
            ("clippy::todo", "correctness", Severity::Warning, 1),
            ("clippy::unimplemented", "correctness", Severity::Warning, 1),
            ("clippy::dbg_macro", "maintainability", Severity::Warning, 1),
            (
                "rust_doctor::source::disabled_tls_verification",
                "security",
                Severity::Warning,
                1,
            ),
        ]);

        assert_eq!(mixed.dimensions.reliability, 50, "P1 overrides P2");
        assert_eq!(mixed.dimensions.security, 20, "P0 caps its dimension");
        assert!(mixed.dimensions.maintainability > 75, "P3 does not cap");
        assert_eq!(mixed.worst_tier, Some(RuleTier::P0));
        assert_eq!(mixed.applied_ceiling, Some(40));
        assert_eq!(mixed.value, 40);
    }

    /// The steps tell an isolated occurrence from a systematic practice,
    /// without letting a single rule saturate its dimension.
    #[test]
    fn occurrence_steps_grow_then_saturate_without_panicking() {
        let single = scored(&[("clippy::stepped", "security", Severity::Error, 1)]);
        let fifty = scored(&[("clippy::stepped", "security", Severity::Error, 50)]);
        assert!(
            fifty.value < single.value,
            "{} should be under {}",
            fifty.value,
            single.value
        );

        let thousand = scored(&[("clippy::stepped", "security", Severity::Error, 1_000)]);
        let saturated = scored(&[("clippy::stepped", "security", Severity::Error, usize::MAX)]);
        assert_eq!(thousand.dimensions.security, fifty.dimensions.security);
        assert_eq!(saturated.dimensions.security, fifty.dimensions.security);
        assert!(
            saturated.dimensions.security > 0,
            "a bounded step cannot saturate a dimension on its own",
        );

        let ceiling = severity_penalty_quarters(Severity::Error)
            * OCCURRENCE_STEPS[OCCURRENCE_STEPS.len() - 1].1;
        assert_eq!(dimension_score(ceiling), saturated.dimensions.security);
    }

    /// Codebase size does not enter the scale: same rule profile and same
    /// occurrences, same score.
    #[test]
    fn the_score_is_invariant_to_codebase_size() {
        let rules = [
            ("clippy::todo", "correctness", Severity::Warning, 12),
            ("clippy::dbg_macro", "maintainability", Severity::Warning, 3),
        ];
        let small = Audit::build(4, Status::Complete, &diagnostics_for(&rules));
        let large = Audit::build(4_000, Status::Complete, &diagnostics_for(&rules));
        assert_eq!(
            small.score.map(|score| score.value),
            large.score.map(|score| score.value)
        );
    }

    /// A rule's penalty recomputes from the published fields alone: severity,
    /// occurrences, category and tier.
    #[test]
    fn a_rule_penalty_is_reproducible_from_published_fields() {
        let rules = [
            ("clippy::todo", "correctness", Severity::Warning, 7),
            (
                "rust_doctor::source::dynamic_shell_command",
                "security",
                Severity::Error,
                2,
            ),
        ];
        let diagnostics = diagnostics_for(&rules);
        let aggregation = aggregate_rules(&diagnostics);

        for (code, _, severity, occurrences) in rules {
            let aggregate = aggregation
                .rules
                .iter()
                .find(|rule| rule.id == code)
                .expect("the rule should be aggregated");
            let expected = severity_penalty_quarters(severity) * occurrence_multiplier(occurrences);
            assert_eq!(aggregate.penalty_quarters(), expected, "{code}");
        }
    }

    /// An incomplete scan stays capped and stays non-authoritative.
    #[test]
    fn an_incomplete_scan_is_capped_and_stays_non_authoritative() {
        let diagnostics = diagnostics_for(&[(
            "rust_doctor::source::dynamic_shell_command",
            "security",
            Severity::Warning,
            1,
        )]);
        let audit = Audit::build(1, Status::Incomplete, &diagnostics);
        let score = audit.score.expect("a scored audit should exist");

        assert_eq!(score.value, 40);
        assert_eq!(score.applied_ceiling, Some(40));
        assert!(!score.authoritative);
        assert_eq!(score.projected_after_top_three, None);
        assert!(score.projected_rule_ids.is_empty());
    }

    /// A diagnostic with no catalog category stays counted: both quantities of
    /// the categories reconstitute the report population exactly.
    #[test]
    fn every_diagnostic_lands_in_exactly_one_bucket() {
        let mut diagnostics = diagnostics_for(&[
            ("clippy::todo", "correctness", Severity::Warning, 3),
            ("clippy::dbg_macro", "maintainability", Severity::Info, 1),
        ]);
        diagnostics.push(Diagnostic {
            context: None,
            id: "compiler".to_owned(),
            source: DiagnosticSource::Rustc,
            code: Some("E0433".to_owned()),
            base_severity: Severity::Error,
            severity: Severity::Error,
            category: None,
            message: "unresolved import".to_owned(),
            help: None,
            package: None,
            target: None,
            path: None,
            span: None,
            related: Vec::new(),
            similarity_basis_points: None,
            complexity: None,
            occurrences: 2,
        });

        let audit = Audit::build(1, Status::Incomplete, &diagnostics);
        let (distinct, occurrences) = audit.totals();

        assert_eq!(distinct.total, 3);
        assert_eq!(occurrences.total, 6);
        assert_eq!(distinct.errors, 1);
        assert_eq!(occurrences.errors, 2);
        assert_eq!(
            audit
                .categories
                .iter()
                .map(|category| category.name)
                .collect::<Vec<_>>(),
            [
                AuditCategoryName::Bugs,
                AuditCategoryName::Maintainability,
                AuditCategoryName::Other,
            ]
        );
        assert!(audit.is_valid());
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
