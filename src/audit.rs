use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::ser::{Error as _, SerializeStruct};
use serde::{Deserialize, Serialize, Serializer};

use cargo_metadata::Metadata;

use crate::execution::ScanExecution;
use crate::policy::{CorpusMeasurement, RuleTier};
use crate::report::{Diagnostic, Severity, Status};
use crate::source_kernel::SourceMeasurement;

mod ceilings;
mod density;
mod ranking;
mod packages;
mod reason;
mod share;
mod source_inventory;

pub use ceilings::ScoreCeiling;
pub use packages::PackageAudit;
pub use reason::ScoreReason;
pub use share::ShareError;

pub(crate) use ceilings::OVERALL;
use ceilings::{capped, tier_overall_ceiling, worse_tier};
use density::{DensityScope, Scale, calculate_dimensions, contributions};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SourceFileInventory {
    pub(crate) files: usize,
    pub(crate) production_lines: usize,
    pub(crate) complete: bool,
}

pub(crate) fn source_file_inventory(
    metadata: &Metadata,
    scan: Option<&ScanExecution>,
    measurement: Option<&SourceMeasurement>,
) -> SourceFileInventory {
    source_inventory::collect(metadata, scan, measurement)
}

pub const SCORE_MODEL: &str = "core-v6";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audit {
    pub source_files: usize,
    pub production_lines: usize,
    pub categories: Vec<AuditCategory>,
    pub score: Option<AuditScore>,
    inventory_is_complete: bool,
    stage_reasons: BTreeSet<ScoreReason>,
    pub packages: Vec<PackageAudit>,
}

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

    fn merge(&mut self, other: Self) {
        self.errors = self.errors.saturating_add(other.errors);
        self.warnings = self.warnings.saturating_add(other.warnings);
        self.info = self.info.saturating_add(other.info);
        self.unknown = self.unknown.saturating_add(other.unknown);
        self.total = self.total.saturating_add(other.total);
    }

    const fn is_coherent(self) -> bool {
        self.errors
            .saturating_add(self.warnings)
            .saturating_add(self.info)
            .saturating_add(self.unknown)
            == self.total
    }

    const fn covers(self, distinct: Self) -> bool {
        self.errors >= distinct.errors
            && self.warnings >= distinct.warnings
            && self.info >= distinct.info
            && self.unknown >= distinct.unknown
            && self.total >= distinct.total
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditCategory {
    pub name: AuditCategoryName,
    pub distinct: SeverityCounts,
    pub occurrences: SeverityCounts,
    pub production: SeverityCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum AuditCategoryName {
    Security,
    Bugs,
    Performance,
    Dependencies,
    Maintainability,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditScore {
    pub model: String,
    pub value: u8,
    pub label: ScoreLabel,
    pub authoritative: bool,
    pub reasons: Vec<ScoreReason>,
    pub dimensions: ScoreDimensions,
    pub worst_tier: Option<RuleTier>,
    pub applied_ceiling: Option<u8>,
    pub ceilings: Vec<ScoreCeiling>,
    pub projected_after_top_three: Option<u8>,
    pub projected_rule_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub withheld_rule_ids: Vec<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ScoreDimension {
    Security,
    Reliability,
    Maintainability,
    Performance,
    Dependencies,
}

impl ScoreDimension {
    const ALL: [Self; 5] = [
        Self::Security,
        Self::Reliability,
        Self::Maintainability,
        Self::Performance,
        Self::Dependencies,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::Reliability => "reliability",
            Self::Maintainability => "maintainability",
            Self::Performance => "performance",
            Self::Dependencies => "dependencies",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuleAggregate {
    pub(crate) id: String,
    pub(crate) effective_severity: Severity,
    pub(crate) category: Option<AuditCategoryName>,
    dimension: Option<ScoreDimension>,
    scope: DensityScope,
    tier: Option<RuleTier>,
    pub(crate) occurrences: usize,
    numerator: u64,
    owned: u64,
    noise: Option<u16>,
    contribution: u64,
}

#[derive(Debug)]
pub(crate) struct RuleAggregation {
    pub(crate) rules: Vec<RuleAggregate>,
    scale: Scale,
    voided: BTreeSet<ScoreReason>,
}

#[derive(Debug)]
struct PendingRule {
    id: String,
    severity: Severity,
    category: Option<AuditCategoryName>,
    dimension: Option<ScoreDimension>,
    scope: DensityScope,
    mapping_conflict: bool,
    tier: Option<RuleTier>,
    occurrences: usize,
    numerator: u64,
    owned: u64,
    noise: Option<u16>,
}

impl Audit {
    pub fn build(
        source_files: usize,
        production_lines: usize,
        status: Status,
        diagnostics: &[Diagnostic],
    ) -> Self {
        Self::build_with_inventory(
            SourceFileInventory {
                files: source_files,
                production_lines,
                complete: true,
            },
            1,
            status,
            if status == Status::Failed {
                BTreeSet::from([ScoreReason::StageFailed])
            } else {
                BTreeSet::new()
            },
            diagnostics,
        )
    }

    pub(crate) fn build_from_inventory(
        inventory: SourceFileInventory,
        members: Vec<(String, SourceFileInventory)>,
        unlinted: &BTreeSet<String>,
        status: Status,
        stage_reasons: BTreeSet<ScoreReason>,
        diagnostics: &[Diagnostic],
    ) -> Self {
        let mut audit =
            Self::build_with_inventory(inventory, members.len(), status, stage_reasons, diagnostics);
        audit.packages =
            packages::score(members, unlinted, status, &audit.stage_reasons, diagnostics);
        audit.withhold_when_nothing_was_linted();
        audit
    }

    fn withhold_when_nothing_was_linted(&mut self) {
        if !self.packages.is_empty() && self.packages.iter().all(|package| package.unlinted) {
            self.score = None;
        }
    }

    pub(crate) fn withhold(&mut self) {
        self.score = None;
    }

    fn build_with_inventory(
        inventory: SourceFileInventory,
        members: usize,
        status: Status,
        stage_reasons: BTreeSet<ScoreReason>,
        diagnostics: &[Diagnostic],
    ) -> Self {
        let SourceFileInventory {
            files: source_files,
            production_lines,
            complete: inventory_is_complete,
        } = inventory;
        let categories = category_tallies(diagnostics);
        let aggregation = aggregate_rules(production_lines, members, diagnostics.iter());
        let mut reasons = aggregation.voided.clone();
        reasons.extend(stage_reasons.iter().copied());
        if status == Status::Failed {
            reasons.insert(ScoreReason::StageFailed);
        } else if stage_reasons.is_empty()
            && (status != Status::Complete || !inventory_is_complete)
        {
            reasons.insert(ScoreReason::ScanIncomplete);
        }
        let score = is_scorable_workspace(source_files, production_lines)
            .then(|| score(&aggregation, reasons.into_iter().collect()));
        Self {
            source_files,
            production_lines,
            categories,
            score,
            inventory_is_complete,
            stage_reasons,
            packages: Vec::new(),
        }
    }

    pub(crate) fn rebuild_for_scope(&self, status: Status, diagnostics: &[Diagnostic]) -> Self {
        let mut rebuilt = Self::build_with_inventory(
            SourceFileInventory {
                files: self.source_files,
                production_lines: self.production_lines,
                complete: self.inventory_is_complete,
            },
            self.packages.len(),
            status,
            self.stage_reasons.clone(),
            diagnostics,
        );
        rebuilt.packages =
            packages::rescore(&self.packages, status, &self.stage_reasons, diagnostics);
        rebuilt.withhold_when_nothing_was_linted();
        if self.score.is_none() {
            rebuilt.score = None;
        }
        rebuilt
    }

    pub fn is_valid(&self) -> bool {
        let mut previous = None;
        let categories_are_valid = self.categories.iter().all(|category| {
            let ordered = previous.is_none_or(|previous| previous < category.name);
            previous = Some(category.name);
            ordered && category.is_valid()
        });
        categories_are_valid
            && (self.score.is_none()
                || is_scorable_workspace(self.source_files, self.production_lines))
            && self.score.as_ref().is_none_or(AuditScore::is_valid)
            && self.packages.iter().all(PackageAudit::is_valid)
    }

    pub fn production_totals(&self) -> SeverityCounts {
        self.categories
            .iter()
            .fold(SeverityCounts::default(), |mut production, category| {
                production.merge(category.production);
                production
            })
    }

    pub fn totals(&self) -> (SeverityCounts, SeverityCounts) {
        self.categories.iter().fold(
            (SeverityCounts::default(), SeverityCounts::default()),
            |(mut distinct, mut occurrences), category| {
                distinct.merge(category.distinct);
                occurrences.merge(category.occurrences);
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
        let mut state = serializer.serialize_struct("Audit", 5)?;
        state.serialize_field("source_files", &self.source_files)?;
        state.serialize_field("production_lines", &self.production_lines)?;
        state.serialize_field("categories", &self.categories)?;
        state.serialize_field("score", &self.score)?;
        if self.packages.is_empty() {
            state.skip_field("packages")?;
        } else {
            state.serialize_field("packages", &self.packages)?;
        }
        state.end()
    }
}

impl AuditCategory {
    fn empty(name: AuditCategoryName) -> Self {
        Self {
            name,
            distinct: SeverityCounts::default(),
            occurrences: SeverityCounts::default(),
            production: SeverityCounts::default(),
        }
    }

    fn is_valid(&self) -> bool {
        self.distinct.total > 0
            && self.distinct.is_coherent()
            && self.occurrences.is_coherent()
            && self.occurrences.covers(self.distinct)
            && self.production.is_coherent()
            && self.occurrences.covers(self.production)
    }
}

impl Serialize for AuditCategory {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("AuditCategory", 8)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("errors", &self.occurrences.errors)?;
        state.serialize_field("warnings", &self.occurrences.warnings)?;
        state.serialize_field("info", &self.occurrences.info)?;
        state.serialize_field("unknown", &self.occurrences.unknown)?;
        state.serialize_field("distinct", &self.distinct)?;
        state.serialize_field("occurrences", &self.occurrences)?;
        state.serialize_field("production", &self.production)?;
        state.end()
    }
}

impl AuditScore {
    pub fn is_valid(&self) -> bool {
        if self.model != SCORE_MODEL || self.value > 100 {
            return false;
        }
        if self.authoritative != self.reasons.is_empty() {
            return false;
        }
        if self.applied_ceiling != self.worst_tier.and_then(tier_overall_ceiling) {
            return false;
        }
        let overall = self
            .ceilings
            .iter()
            .find(|ceiling| ceiling.dimension == OVERALL)
            .map(|ceiling| (ceiling.tier, ceiling.ceiling));
        if overall != self.worst_tier.zip(self.applied_ceiling)
            || self.ceilings.iter().any(|ceiling| ceiling.rule_ids.is_empty())
        {
            return false;
        }
        if self.value != capped(weighted_score(self.dimensions), self.applied_ceiling) {
            return false;
        }
        if self.label != score_label(self.value) {
            return false;
        }
        if self.dimensions.values().into_iter().any(|value| value > 100) {
            return false;
        }
        if self
            .projected_after_top_three
            .is_some_and(|projected| projected > 100 || projected < self.value)
        {
            return false;
        }
        if !self.projected_rules_are_named_once() {
            return false;
        }
        if self.authoritative {
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

    fn projected_rules_are_named_once(&self) -> bool {
        let distinct: BTreeSet<_> = self.projected_rule_ids.iter().collect();
        self.projected_rule_ids.len() <= 3
            && distinct.len() == self.projected_rule_ids.len()
            && self.projected_rule_ids.iter().all(|rule| !rule.is_empty())
    }
}

impl ScoreDimensions {
    fn from_fn(mut score_for: impl FnMut(ScoreDimension) -> u8) -> Self {
        Self {
            security: score_for(ScoreDimension::Security),
            reliability: score_for(ScoreDimension::Reliability),
            maintainability: score_for(ScoreDimension::Maintainability),
            performance: score_for(ScoreDimension::Performance),
            dependencies: score_for(ScoreDimension::Dependencies),
        }
    }

    fn values(self) -> [u8; 5] {
        ScoreDimension::ALL.map(|dimension| self.value_for(dimension))
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

fn category_bucket(category: Option<&str>) -> AuditCategoryName {
    category
        .and_then(category_mapping)
        .map_or(AuditCategoryName::Other, |(name, _)| name)
}

const BASIS_POINTS: u64 = 10_000;

const fn is_scorable_workspace(source_files: usize, production_lines: usize) -> bool {
    source_files > 0 && production_lines > 0
}

const fn dimension_weight_twice(dimension: ScoreDimension) -> u64 {
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
        self.contribution
    }

    const fn is_scorable(&self) -> bool {
        self.numerator > 0 && self.dimension.is_some()
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
        if crate::report::DiagnosticContext::weighs(diagnostic) {
            tally
                .production
                .add(diagnostic.severity, diagnostic.occurrences);
        }
    }

    tallies.into_values().collect()
}

fn score(aggregation: &RuleAggregation, reasons: Vec<ScoreReason>) -> AuditScore {
    let scored = ScoredState::of(&aggregation.rules, &BTreeSet::new(), aggregation.scale);
    let authoritative = reasons.is_empty();
    let (projected, withheld) = aggregation.projection();
    let projected_rule_ids: Vec<String> = projected.iter().map(|rule| rule.id.clone()).collect();

    let (projected_rule_ids, withheld_rule_ids, projected_after_top_three) = if authoritative {
        let after = (!projected_rule_ids.is_empty()).then(|| {
            let removed: BTreeSet<_> = projected_rule_ids.iter().cloned().collect();
            ScoredState::of(&aggregation.rules, &removed, aggregation.scale).value
        });
        let withheld_rule_ids = withheld.iter().map(|rule| rule.id.clone()).collect();
        (projected_rule_ids, withheld_rule_ids, after)
    } else {
        (Vec::new(), Vec::new(), None)
    };

    AuditScore {
        model: SCORE_MODEL.to_owned(),
        value: scored.value,
        label: score_label(scored.value),
        authoritative,
        reasons,
        dimensions: scored.dimensions,
        worst_tier: scored.worst_tier,
        applied_ceiling: scored.applied_ceiling,
        ceilings: ceilings::ceilings(&aggregation.rules),
        projected_after_top_three,
        projected_rule_ids,
        withheld_rule_ids,
    }
}

struct ScoredState {
    dimensions: ScoreDimensions,
    worst_tier: Option<RuleTier>,
    applied_ceiling: Option<u8>,
    value: u8,
}

impl ScoredState {
    fn of(rules: &[RuleAggregate], removed: &BTreeSet<String>, scale: Scale) -> Self {
        let dimensions = calculate_dimensions(rules, removed, scale);
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
    production_lines: usize,
    members: usize,
    diagnostics: impl IntoIterator<Item = &'a Diagnostic>,
) -> RuleAggregation {
    let mut voided = BTreeSet::new();
    let mut rules = BTreeMap::<String, PendingRule>::new();
    for diagnostic in diagnostics {
        let weighs = crate::report::DiagnosticContext::weighs(diagnostic);
        let Some(rule_id) = diagnostic.code.as_deref().filter(|code| !code.is_empty()) else {
            voided.extend(void_reason(weighs, false, false));
            continue;
        };
        let definition = crate::policy::find(rule_id);
        let mapping = diagnostic.category.as_deref().and_then(category_mapping);
        let weight = density::severity_weight(diagnostic.severity);
        let scores = weighs && weight.is_some() && mapping.is_some();
        voided.extend(void_reason(weighs, definition.is_some(), scores));
        let rule = rules.entry(rule_id.to_owned()).or_insert(PendingRule {
            id: rule_id.to_owned(),
            severity: diagnostic.severity,
            category: None,
            dimension: None,
            scope: definition.map_or(DensityScope::Workspace, |definition| {
                DensityScope::of(definition.producer)
            }),
            mapping_conflict: false,
            tier: definition.map(|definition| definition.tier),
            occurrences: 0,
            numerator: 0,
            owned: 0,
            noise: crate::policy::corpus_measurement(rule_id)
                .map(CorpusMeasurement::noise_basis_points),
        });
        rule.occurrences = rule.occurrences.saturating_add(diagnostic.occurrences);
        if diagnostic.severity.rank() < rule.severity.rank() {
            rule.severity = diagnostic.severity;
        }
        if scores
            && let Some((category, dimension)) = mapping
            && let Some(weight) = weight
        {
            rule.numerator = rule.numerator.saturating_add(weight);
            if diagnostic.package.is_some() {
                rule.owned = rule.owned.saturating_add(weight);
            }
            if rule.dimension.is_some_and(|current| current != dimension) {
                rule.mapping_conflict = true;
                voided.insert(ScoreReason::CategoryMappingConflict);
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
    let scale = Scale::of(production_lines, members);
    let mut rules: Vec<RuleAggregate> = rules
        .into_values()
        .map(|rule| RuleAggregate {
            id: rule.id,
            effective_severity: rule.severity,
            category: (!rule.mapping_conflict).then_some(rule.category).flatten(),
            dimension: (!rule.mapping_conflict && rule.numerator > 0)
                .then_some(rule.dimension)
                .flatten(),
            scope: rule.scope,
            tier: rule.tier,
            occurrences: rule.occurrences,
            numerator: rule.numerator,
            owned: rule.owned,
            noise: rule.noise,
            contribution: 0,
        })
        .collect();
    let recovered = contributions(&rules, scale);
    for (rule, contribution) in rules.iter_mut().zip(recovered) {
        rule.contribution = contribution;
    }
    RuleAggregation {
        rules,
        scale,
        voided,
    }
}

const fn void_reason(weighs: bool, catalogued: bool, scores: bool) -> Option<ScoreReason> {
    match (weighs, catalogued, scores) {
        (false, _, _) | (true, true, true) => None,
        (true, false, _) => Some(ScoreReason::StageFailed),
        (true, true, false) => Some(ScoreReason::CategoryMappingConflict),
    }
}

fn weighted_score(dimensions: ScoreDimensions) -> u8 {
    let numerator = ScoreDimension::ALL.into_iter().fold(0u64, |sum, dimension| {
        sum.saturating_add(
            u64::from(dimensions.value_for(dimension)) * dimension_weight_twice(dimension),
        )
    });
    u8::try_from((numerator.saturating_add(6) / 13).min(100)).unwrap_or(100)
}

const fn score_label(score: u8) -> ScoreLabel {
    if score >= 75 {
        ScoreLabel::Great
    } else if score >= 50 {
        ScoreLabel::NeedsWork
    } else {
        ScoreLabel::Critical
    }
}

#[cfg(test)]
mod tests;
