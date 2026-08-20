use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use serde::ser::{Error as _, SerializeStruct};
use serde::{Deserialize, Serialize, Serializer};

use cargo_metadata::Metadata;

use crate::execution::ScanExecution;
use crate::policy::RuleTier;
use crate::report::{Diagnostic, Severity, Status};
use crate::source_kernel::SourceMeasurement;

mod source_inventory;

/// The workspace the score is computed against: how much of it there is, and
/// whether that is the workspace or a floor on it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SourceFileInventory {
    pub(crate) files: usize,
    /// Lines of production Rust the walk counted. Test code is excluded, since
    /// the score charges for what ships.
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

pub const SCORE_MODEL: &str = "core-v2";
const SHARE_BASE_URL: &str = "https://rust-doctor.com/share";
const MAX_SHARED_COUNT: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audit {
    pub source_files: usize,
    /// Lines of production Rust the scan counted, test code excluded. Zero when
    /// the workspace holds none, and a floor on it whenever
    /// `inventory_is_complete` is false.
    pub production_lines: usize,
    pub categories: Vec<AuditCategory>,
    pub score: Option<AuditScore>,
    /// Whether `source_files` counted the workspace or is a floor on it.
    ///
    /// Rebuilding the block for a narrower scope needs this fact and nothing else about the
    /// original scan, and the block used to recover it from `score.authoritative`, which also
    /// carries the status and whether every diagnostic was catalogued. One uncatalogued rule
    /// anywhere therefore made every later scope non-authoritative, for a reason that had
    /// nothing to do with the inventory. It stays private because it is not published: the
    /// wire shape is the three members above.
    inventory_is_complete: bool,
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

    /// Adds every bucket of another count into this one.
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

    /// At least one occurrence per distinct diagnostic, never the reverse.
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
}

/// The categories the report publishes, and the order it publishes them in.
///
/// The declaration order is that order: `Ord` derives from it, the tally map is keyed by it, and
/// `Audit::is_valid` checks it. A second list restating the same sequence is a second place for
/// it to be wrong, which is why there is none.
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
    /// Rules that fired here and that the corpus adjudicated at no true
    /// positive, in descending cost order, hence absent from
    /// `projected_rule_ids` whatever their volume.
    ///
    /// Published so the omission reads as a measurement rather than a defect:
    /// the rule with the most findings is often the one the corpus found most
    /// often wrong, and a list that silently drops it is impossible to trust.
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

impl ScoreDimension {
    /// Every dimension, in the order the report publishes them.
    ///
    /// This is the one list the scoring walks: `every_dimension_is_listed_once` stops compiling
    /// when a dimension is declared and forgotten here.
    const ALL: [Self; 5] = [
        Self::Security,
        Self::Reliability,
        Self::Maintainability,
        Self::Performance,
        Self::Dependencies,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuleAggregate {
    pub(crate) id: String,
    pub(crate) effective_severity: Severity,
    scored_severity: Option<Severity>,
    pub(crate) category: Option<AuditCategoryName>,
    dimension: Option<ScoreDimension>,
    tier: Option<RuleTier>,
    /// Every occurrence the report publishes for this rule, which is what the reader is shown.
    pub(crate) occurrences: usize,
    /// The subset of them the score charges for. The two differ by exactly the diagnostics
    /// outside production code: counted and displayed, never penalized.
    scored_occurrences: usize,
    /// Adjudicated false-positive rate on the pinned corpus, in basis points.
    /// It ranks what to repair first and enters no penalty: what a rule costs
    /// the score is what it reported here, whatever it costs elsewhere.
    noise: Option<u16>,
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
    scored_occurrences: usize,
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
            status,
            diagnostics,
        )
    }

    pub(crate) fn build_from_inventory(
        inventory: SourceFileInventory,
        status: Status,
        diagnostics: &[Diagnostic],
    ) -> Self {
        Self::build_with_inventory(inventory, status, diagnostics)
    }

    fn build_with_inventory(
        inventory: SourceFileInventory,
        status: Status,
        diagnostics: &[Diagnostic],
    ) -> Self {
        let SourceFileInventory {
            files: source_files,
            production_lines,
            complete: inventory_is_complete,
        } = inventory;
        // Both quantities count everything the report publishes, FR-06 requires them to equal
        // `summary`. What the score sets aside is decided inside `aggregate_rules`, so the two
        // callers of it cannot disagree on the population.
        let categories = category_tallies(diagnostics);
        let aggregation = aggregate_rules(diagnostics.iter());
        let scan_is_complete = status == Status::Complete && inventory_is_complete;
        let score = (source_files > 0).then(|| score(&aggregation, scan_is_complete));
        Self {
            source_files,
            production_lines,
            categories,
            score,
            inventory_is_complete,
        }
    }

    pub(crate) fn rebuild_for_scope(&self, status: Status, diagnostics: &[Diagnostic]) -> Self {
        Self::build_with_inventory(
            SourceFileInventory {
                files: self.source_files,
                production_lines: self.production_lines,
                complete: self.inventory_is_complete,
            },
            status,
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

        // The counts the block already publishes, not a third summation over the same
        // categories that nothing kept in step with the first two.
        let (_, occurrences) = self.totals();
        build_share_url(
            score.value,
            occurrences.errors,
            occurrences.warnings,
            occurrences.info,
            self.source_files,
            self.production_lines,
        )
    }

    pub fn is_valid(&self) -> bool {
        let mut previous = None;
        let categories_are_valid = self.categories.iter().all(|category| {
            let ordered = previous.is_none_or(|previous| previous < category.name);
            previous = Some(category.name);
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
        let mut state = serializer.serialize_struct("Audit", 4)?;
        state.serialize_field("source_files", &self.source_files)?;
        state.serialize_field("production_lines", &self.production_lines)?;
        state.serialize_field("categories", &self.categories)?;
        state.serialize_field("score", &self.score)?;
        state.end()
    }
}

impl AuditCategory {
    fn empty(name: AuditCategoryName) -> Self {
        Self {
            name,
            distinct: SeverityCounts::default(),
            occurrences: SeverityCounts::default(),
        }
    }

    fn is_valid(&self) -> bool {
        self.distinct.total > 0
            && self.distinct.is_coherent()
            && self.occurrences.is_coherent()
            && self.occurrences.covers(self.distinct)
    }
}

impl Serialize for AuditCategory {
    /// The four bare severity members are the schema-v7 spelling of `occurrences`, projected
    /// from it rather than stored beside it.
    ///
    /// Held as fields, they were a second copy of one fact: a recopy pass wrote them, four
    /// clauses of `is_valid` checked they still agreed, and `share_url` summed the copy while
    /// `totals` summed the original. A projection cannot disagree with its source.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("AuditCategory", 7)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("errors", &self.occurrences.errors)?;
        state.serialize_field("warnings", &self.occurrences.warnings)?;
        state.serialize_field("info", &self.occurrences.info)?;
        state.serialize_field("unknown", &self.occurrences.unknown)?;
        state.serialize_field("distinct", &self.distinct)?;
        state.serialize_field("occurrences", &self.occurrences)?;
        state.end()
    }
}

impl AuditScore {
    /// Whether every published member still follows from the others.
    ///
    /// `Audit`'s `Serialize` refuses a block that fails this, so each clause is the one thing it
    /// names rather than a term in one long conjunction: when it refuses, the caller can be told
    /// which fact stopped holding.
    pub fn is_valid(&self) -> bool {
        if self.model != SCORE_MODEL || self.value > 100 {
            return false;
        }
        if self.applied_ceiling != self.worst_tier.and_then(tier_overall_ceiling) {
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
        // Repairing the top three can only raise the score, never lower it.
        if self
            .projected_after_top_three
            .is_some_and(|projected| projected > 100 || projected < self.value)
        {
            return false;
        }
        if !self.projected_rules_are_named_once() {
            return false;
        }
        // An authoritative score names what to fix and what fixing it would be worth, or names
        // neither. A non-authoritative one names nothing at all.
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

    /// At most three rules, none of them empty and none of them named twice.
    fn projected_rules_are_named_once(&self) -> bool {
        let distinct: BTreeSet<_> = self.projected_rule_ids.iter().collect();
        self.projected_rule_ids.len() <= 3
            && distinct.len() == self.projected_rule_ids.len()
            && self.projected_rule_ids.iter().all(|rule| !rule.is_empty())
    }
}

impl ScoreDimensions {
    /// The five dimensions, each scored by the same function.
    ///
    /// This and `value_for` are the two halves of the one mapping between a dimension and the
    /// field the report publishes it under. Everything else walks `ScoreDimension::ALL`.
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

/// Every diagnostic falls into exactly one bucket, including those no catalog
/// category covers. Without that, `summary` and `audit.categories` would count
/// different populations.
fn category_bucket(category: Option<&str>) -> AuditCategoryName {
    category
        .and_then(category_mapping)
        .map_or(AuditCategoryName::Other, |(name, _)| name)
}

/// Cap imposed on a dimension by the worst tier it contains.
const fn tier_dimension_ceiling(tier: RuleTier) -> Option<u8> {
    match tier {
        RuleTier::P0 => Some(20),
        RuleTier::P1 => Some(50),
        RuleTier::P2 => Some(75),
        RuleTier::P3 => None,
    }
}

/// Cap imposed on the overall score by the worst tier across all dimensions.
const fn tier_overall_ceiling(tier: RuleTier) -> Option<u8> {
    match tier {
        RuleTier::P0 => Some(40),
        RuleTier::P1 => Some(65),
        RuleTier::P2 | RuleTier::P3 => None,
    }
}

/// Occurrence steps applied to a rule's penalty, as inclusive upper bounds.
const OCCURRENCE_STEPS: [(usize, u64); 3] = [(1, 1), (5, 2), (20, 3)];

/// What a count past the last step multiplies by. Naming the saturation is what removed the
/// `usize::MAX` sentinel step, and with it the walk that had to index the table back out to
/// find its own last row.
const OCCURRENCE_CEILING: u64 = 4;

/// Full scale of an adjudicated rate, matching the basis points the corpus
/// publishes.
const BASIS_POINTS: u64 = 10_000;

/// Saturating multiplier: a rule can never go past the ceiling, whatever its occurrence count.
fn occurrence_multiplier(occurrences: usize) -> u64 {
    OCCURRENCE_STEPS
        .into_iter()
        .find_map(|(bound, multiplier)| (occurrences <= bound).then_some(multiplier))
        .unwrap_or(OCCURRENCE_CEILING)
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

const fn severity_penalty_quarters(severity: Severity) -> u64 {
    match severity {
        Severity::Error => 6,
        Severity::Warning => 3,
        Severity::Info => 1,
        Severity::Unknown => 0,
    }
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
    /// Penalty in quarter points, scored severity times occurrence step.
    fn penalty_quarters(&self) -> u64 {
        match self.scored_severity {
            Some(severity) => severity_penalty_quarters(severity)
                .saturating_mul(occurrence_multiplier(self.scored_occurrences)),
            None => 0,
        }
    }

    pub(crate) fn contribution(&self) -> u64 {
        match self.dimension {
            Some(dimension) => self
                .penalty_quarters()
                .saturating_mul(dimension_weight_twice(dimension)),
            None => 0,
        }
    }

    /// What repairing this rule is expected to be worth, which is what it costs
    /// the score discounted by how often the corpus found it wrong.
    ///
    /// A rule the corpus adjudicated at 100 % false positives is expected to be
    /// worth nothing to repair, whatever its volume, and volume is exactly what
    /// the noisiest rules have the most of. Ranking by contribution alone told
    /// the user to fix the rule that fires most, which is not the same question
    /// and, on a rule measured at zero true positives, is advice to change
    /// correct code. An unmeasured rule keeps its full contribution: no
    /// measurement is not evidence of noise.
    pub(crate) fn expected_repair_value(&self) -> u64 {
        let kept = match self.noise {
            Some(noise) => BASIS_POINTS.saturating_sub(noise as u64),
            None => BASIS_POINTS,
        };
        self.contribution().saturating_mul(kept) / BASIS_POINTS
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

/// One tally per category that fired, in the order the report publishes them.
///
/// The map is keyed by the category itself, whose `Ord` is its declaration order, so it already
/// comes out ordered: sorting it a second time against a hand-written list is how the two orders
/// used to be able to disagree.
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

    tallies.into_values().collect()
}

fn score(aggregation: &RuleAggregation, scan_complete: bool) -> AuditScore {
    let scored = ScoredState::of(&aggregation.rules, &BTreeSet::new());
    let authoritative = scan_complete && aggregation.diagnostics_are_authoritative;
    let (projected, withheld) = rank_repairs(&aggregation.rules);
    let projected_rule_ids: Vec<String> = projected
        .iter()
        .take(3)
        .map(|rule| rule.id.clone())
        .collect();

    // A ranking is only worth as much as the set of diagnostics it ranked, and that set is
    // exactly what a scan that did not complete cannot vouch for. So the projection, the rules
    // it names and the rules it withheld are published together or not at all.
    let (projected_rule_ids, withheld_rule_ids, projected_after_top_three) = if authoritative {
        let after = (!projected_rule_ids.is_empty()).then(|| {
            let removed: BTreeSet<_> = projected_rule_ids.iter().cloned().collect();
            ScoredState::of(&aggregation.rules, &removed).value
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
        dimensions: scored.dimensions,
        worst_tier: scored.worst_tier,
        applied_ceiling: scored.applied_ceiling,
        projected_after_top_three,
        projected_rule_ids,
        withheld_rule_ids,
    }
}

/// The rules that cost the score anything, split by whether repairing them is worth something,
/// each half ordered by what it is worth to the reader.
///
/// The two halves are one question asked once. A rule the corpus adjudicated at no true positive
/// is expected to be worth nothing to repair, whatever its volume, and volume is exactly what the
/// noisiest rules have the most of: it is withheld rather than ranked last, because naming it
/// would still be telling the reader to go and change correct code. It is published all the same,
/// loudest first, so its absence from the projection reads as a measurement and not as a defect.
fn rank_repairs(rules: &[RuleAggregate]) -> (Vec<&RuleAggregate>, Vec<&RuleAggregate>) {
    let (mut projected, mut withheld): (Vec<_>, Vec<_>) = rules
        .iter()
        .filter(|rule| rule.is_scorable() && rule.contribution() > 0)
        .partition(|rule| rule.expected_repair_value() > 0);
    projected.sort_by(|left, right| {
        right
            .expected_repair_value()
            .cmp(&left.expected_repair_value())
            .then_with(|| right.contribution().cmp(&left.contribution()))
            .then_with(|| left.id.cmp(&right.id))
    });
    withheld.sort_by(|left, right| {
        right
            .contribution()
            .cmp(&left.contribution())
            .then_with(|| left.id.cmp(&right.id))
    });
    (projected, withheld)
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

/// One aggregate per rule that fired, over every diagnostic the report publishes.
///
/// The score and the report body read the same aggregates. What separates them is not the
/// population but which figures they use: a diagnostic outside production code is counted in
/// `occurrences` and left out of `scored_occurrences`, so it stays visible and costs no points,
/// and it never decides whether the diagnostics are authoritative. That set-aside used to live
/// at one of the two call sites, so the contribution the body ranked by was not the one the
/// score charged, and a rule that only ever fired in a test was ranked as if it had.
pub(crate) fn aggregate_rules<'a>(
    diagnostics: impl IntoIterator<Item = &'a Diagnostic>,
) -> RuleAggregation {
    let mut diagnostics_are_authoritative = true;
    let mut rules = BTreeMap::<String, PendingRule>::new();
    for diagnostic in diagnostics {
        let weighs = crate::report::DiagnosticContext::weighs(diagnostic);
        let Some(rule_id) = diagnostic.code.as_deref().filter(|code| !code.is_empty()) else {
            diagnostics_are_authoritative &= !weighs;
            continue;
        };
        let mapping = diagnostic.category.as_deref().and_then(category_mapping);
        let scores = weighs && diagnostic.severity != Severity::Unknown && mapping.is_some();
        if weighs && !scores {
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
            scored_occurrences: 0,
            noise: crate::policy::corpus_noise(rule_id),
        });
        rule.occurrences = rule.occurrences.saturating_add(diagnostic.occurrences);
        // Every occurrence in production code moves the step, including one the score can put no
        // severity on: what a rule costs grows with how often it fired, and an occurrence the
        // catalog could not read is still an occurrence.
        if weighs {
            rule.scored_occurrences = rule
                .scored_occurrences
                .saturating_add(diagnostic.occurrences);
        }
        if diagnostic.severity.rank() < rule.severity.rank() {
            rule.severity = diagnostic.severity;
        }
        if scores && let Some((category, dimension)) = mapping {
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
                scored_occurrences: rule.scored_occurrences,
                noise: rule.noise,
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
    ScoreDimensions::from_fn(score_for)
}

fn dimension_score(penalty_quarters: u64) -> u8 {
    let remaining_quarters = 400u64.saturating_sub(penalty_quarters);
    u8::try_from((remaining_quarters.saturating_add(2) / 4).min(100)).unwrap_or(100)
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

fn build_share_url(
    score: u8,
    errors: usize,
    warnings: usize,
    info: usize,
    source_files: usize,
    production_lines: usize,
) -> Result<String, ShareError> {
    if score > 100
        || [errors, warnings, info, source_files, production_lines]
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
        ("l", production_lines),
    ] {
        if count > 0 {
            let _ = write!(url, "&{key}={count}");
        }
    }
    Ok(url)
}

#[cfg(test)]
mod tests;
