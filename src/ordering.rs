//! One canonical priority, grouping, and truncation contract.
//!
//! Terminal, JSON, SARIF, MCP, CI annotations, plans, and handoffs all render
//! the same immutable report, so they must also agree on what comes first. This
//! module owns that decision: every consumer sorts through [`compare`] and
//! groups through [`root_cause_groups`] instead of reimplementing a local
//! severity heuristic (US-015 AC-1).
//!
//! The comparator is total and deterministic. Every key it reads is either
//! present on the canonical diagnostic or has an explicit fallback, so two runs
//! over the same report produce byte-identical order even when paths, lines, or
//! priorities are missing (US-015 AC-6).

#![expect(
    clippy::redundant_pub_crate,
    reason = "the ordering contract is consumed by sibling modules through this private module"
)]

use crate::diagnostics::{
    CanonicalDiagnostic, Category, DiagnosticLocation, DiagnosticOwnership, RootCauseGroup,
    ScoreImpact, Severity,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const LOOKUP_CACHE_SLOTS: usize = 4_096;
const FINGERPRINT_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FINGERPRINT_PRIME: u64 = 0x0000_0100_0000_01b3;
const FINGERPRINT_MIX_1: u64 = 0xff51_afd7_ed55_8ccd;
const FINGERPRINT_MIX_2: u64 = 0xc4ce_b9fe_1a85_ec53;

/// Findings needed before output switches to migration grouping.
pub(crate) const MIGRATION_FINDING_THRESHOLD: usize = 50;
/// Distinct files needed alongside the finding threshold.
pub(crate) const MIGRATION_FILE_THRESHOLD: usize = 10;
/// Root-cause groups needed alongside the finding threshold.
pub(crate) const MIGRATION_GROUP_THRESHOLD: usize = 5;
/// Root-cause groups a consumer must never displace when truncating.
pub(crate) const PROTECTED_ROOT_CAUSE_GROUPS: usize = 3;

/// Product urgency rank. Unranked findings sort last without being dropped.
const fn priority_rank(priority: Option<&str>) -> u8 {
    match priority {
        Some(value) => match value.as_bytes() {
            b"p0" => 0,
            b"p1" => 1,
            b"p2" => 2,
            b"p3" => 3,
            _ => 4,
        },
        None => 4,
    }
}

/// Evidence authority rank: what the finding actually did to the score.
const fn authority_rank(impact: ScoreImpact) -> u8 {
    match impact {
        ScoreImpact::Scored => 0,
        ScoreImpact::Advisory => 1,
        ScoreImpact::Ineligible => 2,
        ScoreImpact::Suppressed => 3,
    }
}

/// Provenance rank: compiler evidence outranks file-local inference.
fn trust_rank(tier: &str) -> u8 {
    match tier {
        "compiler-proven" => 0,
        "advisory-backed" => 1,
        "calibrated-heuristic" => 2,
        "audit-only" => 3,
        _ => 4,
    }
}

/// Stable category order, independent from the `Display` label.
const fn category_rank(category: &Category) -> u8 {
    match category {
        Category::Security => 0,
        Category::Correctness => 1,
        Category::ErrorHandling => 2,
        Category::Async => 3,
        Category::Framework => 4,
        Category::Dependencies => 5,
        Category::Cargo => 6,
        Category::Performance => 7,
        Category::Architecture => 8,
        Category::Style => 9,
    }
}

const fn package_key(ownership: &DiagnosticOwnership) -> &str {
    match ownership {
        DiagnosticOwnership::Package { package_id } => package_id.as_str(),
        DiagnosticOwnership::Workspace => "",
        DiagnosticOwnership::Unowned => "\u{7f}",
    }
}

/// Path, line, and column, with project-level findings sorting first.
const fn location_key(location: &DiagnosticLocation) -> (&str, u32, u32) {
    match location {
        DiagnosticLocation::Source { path, range } => {
            (path.as_str(), range.start.line, range.start.column)
        }
        DiagnosticLocation::Project => ("", 0, 0),
    }
}

const fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Error => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    }
}

/// Occurrence counts per root-cause key, used as the impact term of the sort.
///
/// Built once per report so ordering stays O(n log n) rather than recomputing
/// group sizes inside the comparator.
#[derive(Debug, Default, Clone)]
pub(crate) struct RootCauseImpact {
    occurrences: HashMap<String, usize>,
    cache: Vec<Option<ImpactCacheEntry>>,
}

#[derive(Debug, Clone)]
struct ImpactCacheEntry {
    fingerprint: u64,
    key: String,
    weight: usize,
}

impl RootCauseImpact {
    pub(crate) fn measure<'a>(
        diagnostics: impl IntoIterator<Item = &'a CanonicalDiagnostic>,
    ) -> Self {
        let mut occurrences: HashMap<String, usize> = HashMap::new();
        for diagnostic in diagnostics {
            if let Some(key) = diagnostic.root_cause_key.as_ref() {
                if let Some(count) = occurrences.get_mut(key.as_str()) {
                    *count += 1;
                } else {
                    occurrences.insert(key.clone(), 1);
                }
            }
        }
        let mut cache = vec![None; LOOKUP_CACHE_SLOTS];
        for (key, weight) in &occurrences {
            let fingerprint = fast_fingerprint(&[key]);
            cache[fingerprint as usize % LOOKUP_CACHE_SLOTS] = Some(ImpactCacheEntry {
                fingerprint,
                key: key.clone(),
                weight: *weight,
            });
        }
        Self { occurrences, cache }
    }

    fn weight(&self, diagnostic: &CanonicalDiagnostic) -> usize {
        let Some(key) = diagnostic.root_cause_key.as_ref() else {
            return 0;
        };
        let fingerprint = fast_fingerprint(&[key]);
        self.cache
            .get(fingerprint as usize % LOOKUP_CACHE_SLOTS)
            .and_then(Option::as_ref)
            .filter(|entry| entry.fingerprint == fingerprint && entry.key == *key)
            .map_or_else(
                || self.occurrences.get(key).copied().unwrap_or_default(),
                |entry| entry.weight,
            )
    }
}

fn fast_fingerprint(values: &[&str]) -> u64 {
    let mut fingerprint = FINGERPRINT_OFFSET;
    for value in values {
        let bytes = value.as_bytes();
        let edge_len = bytes.len().min(8);
        let mut prefix = [0; 8];
        prefix[..edge_len].copy_from_slice(&bytes[..edge_len]);
        let mut suffix = [0; 8];
        suffix[..edge_len].copy_from_slice(&bytes[bytes.len() - edge_len..]);
        fingerprint ^= u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        fingerprint = fingerprint.wrapping_mul(FINGERPRINT_PRIME);
        fingerprint ^= u64::from_le_bytes(prefix);
        fingerprint = fingerprint.wrapping_mul(FINGERPRINT_PRIME);
        fingerprint ^= u64::from_le_bytes(suffix);
        fingerprint = fingerprint.wrapping_mul(FINGERPRINT_PRIME);
    }
    fingerprint ^= fingerprint >> 33;
    fingerprint = fingerprint.wrapping_mul(FINGERPRINT_MIX_1);
    fingerprint ^= fingerprint >> 33;
    fingerprint = fingerprint.wrapping_mul(FINGERPRINT_MIX_2);
    fingerprint ^= fingerprint >> 33;
    fingerprint
}

/// The canonical comparator.
///
/// Order: priority, authority, trust tier, root-cause impact, category, rule
/// ID, package, path, location, then stable tie-breakers. Severity is only a
/// tie-breaker: it describes presentation, not urgency.
pub(crate) fn compare(
    left: &CanonicalDiagnostic,
    right: &CanonicalDiagnostic,
    impact: &RootCauseImpact,
) -> Ordering {
    let left_key = DiagnosticSortKey::new(left, impact.weight(left));
    let right_key = DiagnosticSortKey::new(right, impact.weight(right));
    left_key.compare(&right_key)
}

struct DiagnosticSortKey<'a> {
    priority: u8,
    authority: u8,
    trust: u8,
    root_cause_impact: usize,
    category: u8,
    rule: &'a str,
    package: &'a str,
    path: &'a str,
    line: u32,
    column: u32,
    severity: u8,
    message: &'a str,
    site_id: &'a str,
}

impl<'a> DiagnosticSortKey<'a> {
    fn new(diagnostic: &'a CanonicalDiagnostic, root_cause_impact: usize) -> Self {
        let (path, line, column) = location_key(&diagnostic.location);
        Self {
            priority: priority_rank(diagnostic.priority.as_deref()),
            authority: authority_rank(diagnostic.score_impact),
            trust: trust_rank(&diagnostic.trust_tier),
            root_cause_impact,
            category: category_rank(&diagnostic.category),
            rule: &diagnostic.rule,
            package: package_key(&diagnostic.ownership),
            path,
            line,
            column,
            severity: severity_rank(diagnostic.severity),
            message: &diagnostic.message,
            site_id: &diagnostic.site_id,
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.authority.cmp(&other.authority))
            .then_with(|| self.trust.cmp(&other.trust))
            // Higher root-cause impact first: one defect behind many
            // occurrences outranks an isolated finding of the same priority.
            .then_with(|| other.root_cause_impact.cmp(&self.root_cause_impact))
            .then_with(|| self.category.cmp(&other.category))
            .then_with(|| self.rule.cmp(other.rule))
            .then_with(|| self.package.cmp(other.package))
            .then_with(|| self.path.cmp(other.path))
            .then_with(|| self.line.cmp(&other.line))
            .then_with(|| self.column.cmp(&other.column))
            .then_with(|| self.severity.cmp(&other.severity))
            .then_with(|| self.message.cmp(other.message))
            // `site_id` carries no meaning, but it is unique and stable, so it
            // is the last resort that makes the order total.
            .then_with(|| self.site_id.cmp(other.site_id))
    }
}

struct OwnedDiagnosticSortKey<'a> {
    decision_rank: u128,
    rule_rank: usize,
    package_rank: usize,
    path_rank: usize,
    line: u32,
    column: u32,
    severity: u8,
    message_rank: usize,
    site_id: &'a str,
}

impl<'a> OwnedDiagnosticSortKey<'a> {
    fn new(
        diagnostic: &'a CanonicalDiagnostic,
        root_cause_impact: usize,
        lexical_id: usize,
    ) -> Self {
        let (_, line, column) = location_key(&diagnostic.location);
        Self {
            decision_rank: decision_rank(diagnostic, root_cause_impact),
            rule_rank: lexical_id,
            package_rank: 0,
            path_rank: 0,
            line,
            column,
            severity: severity_rank(diagnostic.severity),
            message_rank: 0,
            site_id: &diagnostic.site_id,
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        self.decision_rank
            .cmp(&other.decision_rank)
            .then_with(|| self.rule_rank.cmp(&other.rule_rank))
            .then_with(|| self.package_rank.cmp(&other.package_rank))
            .then_with(|| self.path_rank.cmp(&other.path_rank))
            .then_with(|| self.line.cmp(&other.line))
            .then_with(|| self.column.cmp(&other.column))
            .then_with(|| self.severity.cmp(&other.severity))
            .then_with(|| self.message_rank.cmp(&other.message_rank))
            .then_with(|| self.site_id.cmp(other.site_id))
    }
}

fn decision_rank(diagnostic: &CanonicalDiagnostic, root_cause_impact: usize) -> u128 {
    let mut rank = u128::from(priority_rank(diagnostic.priority.as_deref()));
    rank = (rank << 2) | u128::from(authority_rank(diagnostic.score_impact));
    rank = (rank << 3) | u128::from(trust_rank(&diagnostic.trust_tier));
    rank = (rank << usize::BITS) | (usize::MAX.saturating_sub(root_cause_impact) as u128);
    (rank << 4) | u128::from(category_rank(&diagnostic.category))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct LexicalValues<'a> {
    rule: &'a str,
    package: &'a str,
    path: &'a str,
    message: &'a str,
}

fn lexical_field_ranks<'a>(
    values: &[LexicalValues<'a>],
    field: impl Fn(LexicalValues<'a>) -> &'a str,
) -> Vec<usize> {
    let mut order: Vec<_> = (0..values.len()).collect();
    order.sort_unstable_by(|left, right| field(values[*left]).cmp(field(values[*right])));

    let mut ranks = vec![0; values.len()];
    let mut rank = 0;
    let mut previous = None;
    for index in order {
        let value = field(values[index]);
        if previous.is_some_and(|previous| previous != value) {
            rank += 1;
        }
        ranks[index] = rank;
        previous = Some(value);
    }
    ranks
}

fn owned_sort_keys<'a>(
    diagnostics: &'a [CanonicalDiagnostic],
    impact: &RootCauseImpact,
) -> Vec<OwnedDiagnosticSortKey<'a>> {
    let mut lexical_ids = HashMap::new();
    let mut lexical_values = Vec::new();
    let mut cache = vec![None; LOOKUP_CACHE_SLOTS];
    let mut keys = Vec::with_capacity(diagnostics.len());
    for diagnostic in diagnostics {
        let (path, _, _) = location_key(&diagnostic.location);
        let values = LexicalValues {
            rule: &diagnostic.rule,
            package: package_key(&diagnostic.ownership),
            path,
            message: &diagnostic.message,
        };
        let fingerprint =
            fast_fingerprint(&[values.rule, values.package, values.path, values.message]);
        let slot = fingerprint as usize % LOOKUP_CACHE_SLOTS;
        let lexical_id = if let Some((cached_fingerprint, cached_values, cached_id)) = cache[slot]
            && cached_fingerprint == fingerprint
            && cached_values == values
        {
            cached_id
        } else {
            let id = match lexical_ids.entry(values) {
                std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let id = lexical_values.len();
                    lexical_values.push(values);
                    entry.insert(id);
                    id
                }
            };
            cache[slot] = Some((fingerprint, values, id));
            id
        };
        keys.push(OwnedDiagnosticSortKey::new(
            diagnostic,
            impact.weight(diagnostic),
            lexical_id,
        ));
    }

    let rule_ranks = lexical_field_ranks(&lexical_values, |values| values.rule);
    let package_ranks = lexical_field_ranks(&lexical_values, |values| values.package);
    let path_ranks = lexical_field_ranks(&lexical_values, |values| values.path);
    let message_ranks = lexical_field_ranks(&lexical_values, |values| values.message);
    for key in &mut keys {
        let lexical_id = key.rule_rank;
        key.rule_rank = rule_ranks[lexical_id];
        key.package_rank = package_ranks[lexical_id];
        key.path_rank = path_ranks[lexical_id];
        key.message_rank = message_ranks[lexical_id];
    }
    keys
}

/// Sort borrowed diagnostics through the canonical comparator.
pub(crate) fn sort_refs(diagnostics: &mut [&CanonicalDiagnostic], impact: &RootCauseImpact) {
    diagnostics.sort_unstable_by(|left, right| compare(left, right, impact));
}

/// Sort owned diagnostics through the canonical comparator.
pub(crate) fn sort_owned(diagnostics: &mut [CanonicalDiagnostic], impact: &RootCauseImpact) {
    #[cfg(test)]
    let started = std::time::Instant::now();
    let keys = owned_sort_keys(diagnostics, impact);
    #[cfg(test)]
    let keys_elapsed = started.elapsed();
    let mut order: Vec<_> = (0..diagnostics.len()).collect();
    order.sort_unstable_by(|left, right| keys[*left].compare(&keys[*right]));
    drop(keys);
    #[cfg(test)]
    let order_elapsed = started.elapsed().saturating_sub(keys_elapsed);

    // `order[new] = old`; invert it so each swap carries the destination
    // alongside the diagnostic it moves.
    let mut destinations = vec![0; diagnostics.len()];
    for (new, old) in order.into_iter().enumerate() {
        destinations[old] = new;
    }
    for current in 0..diagnostics.len() {
        while destinations[current] != current {
            let target = destinations[current];
            diagnostics.swap(current, target);
            destinations.swap(current, target);
        }
    }
    #[cfg(test)]
    eprintln!(
        "canonical sort phases: keys {keys_elapsed:?}, order {order_elapsed:?}, permutation {:?}",
        started
            .elapsed()
            .saturating_sub(keys_elapsed)
            .saturating_sub(order_elapsed)
    );
}

/// Build the canonical root-cause groups behind a set of diagnostics.
///
/// One group owns the priority, trust tier, aggregation policy, and score
/// impact of its occurrences; the occurrences themselves stay inspectable
/// through `site_ids` (US-014 AC-3).
pub(crate) fn root_cause_groups<'a>(
    diagnostics: impl IntoIterator<Item = &'a CanonicalDiagnostic>,
) -> Vec<RootCauseGroup> {
    let mut indices = HashMap::new();
    let mut accumulators = Vec::new();
    let mut last = None;
    for diagnostic in diagnostics {
        let Some(key) = diagnostic.root_cause_key.as_deref() else {
            // Unmapped rules have no root cause to own; they remain visible in
            // `diagnostics` and are never invented into a group.
            continue;
        };
        let index = if let Some((last_key, index)) = last
            && last_key == key
        {
            index
        } else {
            *indices.entry(key).or_insert_with(|| {
                let index = accumulators.len();
                accumulators.push((key, RootCauseAccumulator::new(diagnostic)));
                index
            })
        };
        accumulators[index].1.push(diagnostic);
        last = Some((key, index));
    }

    let mut groups: Vec<RootCauseGroup> = accumulators
        .into_iter()
        .map(|(key, entry)| entry.finish(key))
        .collect();
    groups.sort_by(|left, right| {
        priority_rank(left.priority.as_deref())
            .cmp(&priority_rank(right.priority.as_deref()))
            .then_with(|| {
                authority_rank(left.score_impact).cmp(&authority_rank(right.score_impact))
            })
            .then_with(|| trust_rank(&left.trust_tier).cmp(&trust_rank(&right.trust_tier)))
            .then_with(|| right.occurrences.cmp(&left.occurrences))
            .then_with(|| category_rank(&left.category).cmp(&category_rank(&right.category)))
            .then_with(|| left.rule.cmp(&right.rule))
            .then_with(|| left.key.cmp(&right.key))
    });
    groups
}

struct RootCauseAccumulator<'a> {
    representative: &'a CanonicalDiagnostic,
    files: Vec<&'a str>,
    site_ids: Vec<&'a str>,
    occurrences: usize,
}

impl<'a> RootCauseAccumulator<'a> {
    const fn new(diagnostic: &'a CanonicalDiagnostic) -> Self {
        Self {
            representative: diagnostic,
            files: Vec::new(),
            site_ids: Vec::new(),
            occurrences: 0,
        }
    }

    fn push(&mut self, diagnostic: &'a CanonicalDiagnostic) {
        self.occurrences += 1;
        self.site_ids.push(diagnostic.site_id.as_str());
        if let DiagnosticLocation::Source { path, .. } = &diagnostic.location {
            self.files.push(path.as_str());
        }
        // Highest-priority occurrence represents the group so that a mixed
        // group never advertises a weaker urgency than one of its members.
        if priority_rank(diagnostic.priority.as_deref())
            < priority_rank(self.representative.priority.as_deref())
        {
            self.representative = diagnostic;
        }
    }

    fn finish(mut self, key: &str) -> RootCauseGroup {
        self.files.sort_unstable();
        self.files.dedup();
        self.site_ids.sort_unstable();
        self.site_ids.dedup();
        let contribution = crate::output::group_score_contribution(
            &self.representative.rule,
            &self.representative.category,
            self.occurrences,
        );
        RootCauseGroup {
            key: key.to_string(),
            title: self.representative.title.clone(),
            rule: self.representative.rule.clone(),
            category: self.representative.category.clone(),
            priority: self.representative.priority.clone(),
            trust_tier: self.representative.trust_tier.clone(),
            aggregation_policy: self.representative.aggregation_policy.clone(),
            score_impact: self.representative.score_impact,
            occurrences: self.occurrences,
            file_count: self.files.len(),
            score_dimension: contribution
                .as_ref()
                .map(|value| value.dimension.to_string()),
            current_penalty: contribution.as_ref().map(|value| value.current_penalty),
            maximum_penalty: contribution.as_ref().map(|value| value.maximum_penalty),
            remediation_title: contribution.map(|value| value.remediation_title),
            site_ids: self.site_ids.into_iter().map(str::to_string).collect(),
        }
    }
}

/// Whether output should switch from a flat priority list to migration
/// grouping (US-015 AC-3 and AC-4).
pub(crate) fn use_migration_grouping(diagnostic_count: usize, groups: &[RootCauseGroup]) -> bool {
    if diagnostic_count < MIGRATION_FINDING_THRESHOLD {
        return false;
    }
    let files: usize = distinct_file_count(groups);
    files >= MIGRATION_FILE_THRESHOLD || groups.len() >= MIGRATION_GROUP_THRESHOLD
}

/// Upper bound on distinct files across groups.
///
/// Groups can overlap on a file, so this counts each group's own files. It is
/// only used against a threshold, never reported as an exact file total.
fn distinct_file_count(groups: &[RootCauseGroup]) -> usize {
    groups
        .iter()
        .map(|group| group.file_count)
        .max()
        .map_or(0, |largest| {
            if groups.len() == 1 {
                largest
            } else {
                groups.iter().map(|group| group.file_count).sum()
            }
        })
}

/// What a consumer dropped when its output limit was reached.
///
/// Truncation is reported, never silent: an omitted count by priority and by
/// category is the difference between "nothing else matters" and "there is
/// more" (US-015 AC-5).
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct TruncationSummary {
    pub(crate) omitted: usize,
    pub(crate) by_priority: BTreeMap<String, usize>,
    pub(crate) by_category: BTreeMap<String, usize>,
}

impl TruncationSummary {
    /// Summarize the tail a consumer did not render.
    pub(crate) fn measure<'a>(omitted: impl IntoIterator<Item = &'a CanonicalDiagnostic>) -> Self {
        let mut summary = Self::default();
        for diagnostic in omitted {
            summary.omitted += 1;
            let priority = diagnostic
                .priority
                .clone()
                .unwrap_or_else(|| "unranked".to_string());
            *summary.by_priority.entry(priority).or_default() += 1;
            *summary
                .by_category
                .entry(diagnostic.category.to_string())
                .or_default() += 1;
        }
        summary
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.omitted == 0
    }

    /// Deterministic one-line rendering shared by every text surface.
    pub(crate) fn describe(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let priorities: Vec<String> = self
            .by_priority
            .iter()
            .map(|(priority, count)| format!("{priority} {count}"))
            .collect();
        let categories: Vec<String> = self
            .by_category
            .iter()
            .map(|(category, count)| format!("{category} {count}"))
            .collect();
        format!(
            "{} more not shown (by priority: {}; by category: {})",
            self.omitted,
            priorities.join(", "),
            categories.join(", ")
        )
    }
}

/// Split diagnostics into the rendered head and the omitted tail, keeping the
/// top root-cause groups regardless of the limit.
///
/// The first occurrence of each of the top [`PROTECTED_ROOT_CAUSE_GROUPS`]
/// groups is always retained, so a flood of lower-priority findings can never
/// displace the three most important root causes (US-015 AC-5).
pub(crate) fn truncate<'a>(
    diagnostics: &[&'a CanonicalDiagnostic],
    groups: &[RootCauseGroup],
    limit: usize,
) -> (Vec<&'a CanonicalDiagnostic>, TruncationSummary) {
    if diagnostics.len() <= limit {
        return (diagnostics.to_vec(), TruncationSummary::default());
    }
    let protected: BTreeSet<&str> = groups
        .iter()
        .take(PROTECTED_ROOT_CAUSE_GROUPS)
        .map(|group| group.key.as_str())
        .collect();

    let mut kept: Vec<&CanonicalDiagnostic> = Vec::with_capacity(limit);
    let mut omitted: Vec<&CanonicalDiagnostic> = Vec::new();
    let mut seen_protected: BTreeSet<&str> = BTreeSet::new();
    for diagnostic in diagnostics {
        // A protected group's first occurrence is kept even past the limit, so
        // a flood of lower-priority findings cannot displace it.
        let represents_protected = diagnostic
            .root_cause_key
            .as_deref()
            .is_some_and(|key| protected.contains(key) && seen_protected.insert(key));
        if kept.len() < limit || represents_protected {
            kept.push(diagnostic);
        } else {
            omitted.push(diagnostic);
        }
    }
    // A protected group can push the head one past the limit; that is
    // deliberate and bounded by PROTECTED_ROOT_CAUSE_GROUPS.
    let summary = TruncationSummary::measure(omitted);
    (kept, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{SourcePosition, SourceRange};

    fn diagnostic(
        rule: &str,
        priority: Option<&str>,
        path: &str,
        line: u32,
    ) -> CanonicalDiagnostic {
        CanonicalDiagnostic {
            provider: "rust-doctor".to_string(),
            rule: rule.to_string(),
            title: rule.to_string(),
            category: Category::Correctness,
            severity: Severity::Warning,
            message: format!("{rule} fired"),
            help: None,
            url: String::new(),
            tags: Vec::new(),
            analysis_kind: "syn_ast".to_string(),
            confidence: "medium".to_string(),
            original_level: "warning".to_string(),
            ownership: DiagnosticOwnership::Workspace,
            source_surface: crate::diagnostics::SourceSurface::Library,
            location: DiagnosticLocation::Source {
                path: path.to_string(),
                range: SourceRange {
                    start: SourcePosition {
                        line,
                        column: 1,
                        byte_offset: None,
                    },
                    end: SourcePosition {
                        line,
                        column: 1,
                        byte_offset: None,
                    },
                },
            },
            related_locations: Vec::new(),
            macro_expansion: None,
            fixes: Vec::new(),
            visible_on: Vec::new(),
            site_id: format!("{rule}-{path}-{line}"),
            baseline_key: format!("{rule}-key"),
            namespace_fallback: false,
            advisory: false,
            priority: priority.map(str::to_string),
            trust_tier: "calibrated-heuristic".to_string(),
            score_eligible: priority.is_some(),
            score_impact: if priority.is_some() {
                ScoreImpact::Scored
            } else {
                ScoreImpact::Ineligible
            },
            aggregation_policy: "bounded-occurrence".to_string(),
            root_cause_key: Some(format!("rule:{rule}")),
            evidence_summary: String::new(),
            limitations: Vec::new(),
            fix_recipe: None,
            suppressed: false,
        }
    }

    #[test]
    fn priority_outranks_severity_and_path() {
        let mut low = diagnostic("style-rule", Some("p3"), "a.rs", 1);
        low.severity = Severity::Error;
        let high = diagnostic("security-rule", Some("p0"), "z.rs", 900);
        let impact = RootCauseImpact::measure([&low, &high]);
        assert_eq!(compare(&high, &low, &impact), Ordering::Less);
    }

    #[test]
    fn unranked_findings_sort_last_without_disappearing() {
        let unranked = diagnostic("clippy::future_lint", None, "a.rs", 1);
        let ranked = diagnostic("unwrap-in-production", Some("p2"), "z.rs", 400);
        let impact = RootCauseImpact::measure([&unranked, &ranked]);
        let mut order = vec![&unranked, &ranked];
        sort_refs(&mut order, &impact);
        assert_eq!(order[0].rule, "unwrap-in-production");
        assert_eq!(order.len(), 2);
    }

    #[test]
    fn ordering_is_stable_across_input_permutations() {
        let items = [
            diagnostic("a-rule", Some("p1"), "b.rs", 5),
            diagnostic("b-rule", Some("p1"), "a.rs", 5),
            diagnostic("c-rule", None, "c.rs", 1),
            diagnostic("a-rule", Some("p1"), "b.rs", 1),
        ];
        let impact = RootCauseImpact::measure(items.iter());
        let mut forward: Vec<&CanonicalDiagnostic> = items.iter().collect();
        let mut backward: Vec<&CanonicalDiagnostic> = items.iter().rev().collect();
        sort_refs(&mut forward, &impact);
        sort_refs(&mut backward, &impact);
        let forward_ids: Vec<_> = forward.iter().map(|value| &value.site_id).collect();
        let backward_ids: Vec<_> = backward.iter().map(|value| &value.site_id).collect();
        assert_eq!(forward_ids, backward_ids);
    }

    #[test]
    fn owned_sort_matches_the_canonical_comparator_across_cache_collisions() {
        let mut items = vec![
            diagnostic("same-rule", Some("p2"), "aaaaaaaaXbbbbbbbb", 2),
            diagnostic("same-rule", Some("p2"), "aaaaaaaaYbbbbbbbb", 1),
            diagnostic("security-rule", Some("p0"), "z.rs", 90),
            diagnostic("unranked-rule", None, "a.rs", 1),
            diagnostic("same-rule", Some("p2"), "aaaaaaaaXbbbbbbbb", 1),
        ];
        items[0].site_id = "site-z".to_string();
        items[1].site_id = "site-a".to_string();
        items[2].category = Category::Security;
        items[2].trust_tier = "compiler-proven".to_string();
        items[3].ownership = DiagnosticOwnership::Unowned;
        items[4].message = "different tie-breaker".to_string();

        assert_eq!(
            fast_fingerprint(&["same-rule", "", "aaaaaaaaXbbbbbbbb", "same-rule fired"]),
            fast_fingerprint(&["same-rule", "", "aaaaaaaaYbbbbbbbb", "same-rule fired"]),
            "the fixture must exercise the secure collision fallback"
        );

        let impact = RootCauseImpact::measure(&items);
        let mut borrowed: Vec<_> = items.iter().collect();
        sort_refs(&mut borrowed, &impact);
        let expected: Vec<_> = borrowed
            .iter()
            .map(|diagnostic| diagnostic.site_id.clone())
            .collect();

        let mut owned = items.clone();
        sort_owned(&mut owned, &impact);
        let actual: Vec<_> = owned
            .iter()
            .map(|diagnostic| diagnostic.site_id.clone())
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn missing_locations_still_produce_a_total_order() {
        let mut project_level = diagnostic("compiler-error", Some("p0"), "", 0);
        project_level.location = DiagnosticLocation::Project;
        let source_level = diagnostic("compiler-error", Some("p0"), "src/lib.rs", 3);
        let impact = RootCauseImpact::measure([&project_level, &source_level]);
        assert_eq!(
            compare(&project_level, &source_level, &impact),
            Ordering::Less
        );
        assert_eq!(
            compare(&project_level, &project_level.clone(), &impact),
            Ordering::Equal
        );
    }

    #[test]
    fn one_root_cause_owns_its_occurrences() {
        let diagnostics: Vec<_> = (1..=4)
            .map(|line| diagnostic("unwrap-in-production", Some("p2"), "src/lib.rs", line))
            .collect();
        let groups = root_cause_groups(&diagnostics);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].key, "rule:unwrap-in-production");
        assert_eq!(groups[0].occurrences, 4);
        assert_eq!(groups[0].file_count, 1);
        assert_eq!(groups[0].site_ids.len(), 4);
        assert_eq!(groups[0].priority.as_deref(), Some("p2"));
    }

    #[test]
    fn root_cause_order_matches_canonical_diagnostic_order() {
        let mut correctness = diagnostic("unwrap-in-production", Some("p2"), "src/lib.rs", 2);
        correctness.category = Category::Correctness;
        let mut style = diagnostic("missing-panics-doc", Some("p2"), "src/lib.rs", 1);
        style.category = Category::Style;
        let mut diagnostics = vec![style, correctness];
        let impact = RootCauseImpact::measure(&diagnostics);
        sort_owned(&mut diagnostics, &impact);
        let groups = root_cause_groups(&diagnostics);

        assert_eq!(diagnostics[0].rule, "unwrap-in-production");
        assert_eq!(groups[0].rule, diagnostics[0].rule);
    }

    #[test]
    fn displayed_group_penalties_reconcile_before_dimension_rounding() {
        let diagnostics = [
            diagnostic("missing-msrv", Some("p1"), "Cargo.toml", 1),
            diagnostic("msrv-outdated", Some("p2"), "Cargo.toml", 2),
            diagnostic("msrv-outdated", Some("p2"), "Cargo.toml", 3),
            diagnostic("msrv-outdated", Some("p2"), "Cargo.toml", 4),
        ];
        let groups = root_cause_groups(&diagnostics);
        let displayed: f64 = groups
            .iter()
            .map(|group| {
                let current = group
                    .current_penalty
                    .expect("every scored group exposes its current penalty");
                let maximum = group
                    .maximum_penalty
                    .expect("every scored group exposes its maximum penalty");
                assert!(maximum + f64::EPSILON >= current);
                assert!(
                    group
                        .remediation_title
                        .as_deref()
                        .is_some_and(|title| !title.is_empty())
                );
                current
            })
            .sum();
        let penalties = crate::output::canonical_penalties(&diagnostics);
        let scored = penalties
            .get(&crate::output::Dimension::Reliability)
            .copied()
            .expect("the reliability dimension has a penalty");

        assert!((displayed - scored).abs() <= 0.01);
    }

    #[test]
    fn an_unmapped_rule_never_becomes_a_root_cause_group() {
        let mut unmapped = diagnostic("clippy::future_lint", None, "src/lib.rs", 1);
        unmapped.root_cause_key = None;
        assert!(root_cause_groups(&[unmapped]).is_empty());
    }

    #[test]
    fn migration_grouping_needs_scale_not_just_volume() {
        let single_file: Vec<_> = (1..=60)
            .map(|line| diagnostic("excessive-clone", Some("p2"), "src/lib.rs", line))
            .collect();
        let groups = root_cause_groups(&single_file);
        assert!(!use_migration_grouping(single_file.len(), &groups));

        let spread: Vec<_> = (1..=60)
            .map(|index| {
                diagnostic(
                    "excessive-clone",
                    Some("p2"),
                    &format!("src/file{index}.rs"),
                    1,
                )
            })
            .collect();
        let spread_groups = root_cause_groups(&spread);
        assert!(use_migration_grouping(spread.len(), &spread_groups));
    }

    #[test]
    fn truncation_reports_omissions_and_protects_top_groups() {
        let mut diagnostics: Vec<CanonicalDiagnostic> = (1..=20)
            .map(|line| diagnostic("excessive-clone", Some("p2"), "src/perf.rs", line))
            .collect();
        diagnostics.push(diagnostic("compiler-error", Some("p0"), "src/lib.rs", 1));
        diagnostics.push(diagnostic("panic-in-library", Some("p1"), "src/lib.rs", 2));
        diagnostics.push(diagnostic("msrv-outdated", Some("p3"), "Cargo.toml", 1));

        let impact = RootCauseImpact::measure(diagnostics.iter());
        let groups = root_cause_groups(&diagnostics);
        let mut ordered: Vec<&CanonicalDiagnostic> = diagnostics.iter().collect();
        sort_refs(&mut ordered, &impact);

        let (kept, summary) = truncate(&ordered, &groups, 3);
        assert!(summary.omitted > 0);
        assert!(summary.describe().contains("more not shown"));
        for group in groups.iter().take(PROTECTED_ROOT_CAUSE_GROUPS) {
            assert!(
                kept.iter()
                    .any(|diagnostic| diagnostic.root_cause_key.as_deref() == Some(&group.key)),
                "top group {} was displaced",
                group.key
            );
        }
    }

    #[test]
    fn truncation_below_the_limit_omits_nothing() {
        let diagnostics = vec![diagnostic("compiler-error", Some("p0"), "src/lib.rs", 1)];
        let groups = root_cause_groups(&diagnostics);
        let ordered: Vec<&CanonicalDiagnostic> = diagnostics.iter().collect();
        let (kept, summary) = truncate(&ordered, &groups, 10);
        assert_eq!(kept.len(), 1);
        assert!(summary.is_empty());
        assert!(summary.describe().is_empty());
    }

    #[cfg(target_os = "linux")]
    fn resident_bytes() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").expect("read process status");
        status
            .lines()
            .find_map(|line| {
                let kib = line.strip_prefix("VmRSS:")?.split_whitespace().next()?;
                kib.parse::<u64>().ok()
            })
            .unwrap_or(0)
            .saturating_mul(1024)
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "release certification measures the 100,000-diagnostic decision budget"]
    fn canonical_decision_scale_gate() {
        const DIAGNOSTICS: usize = 100_000;
        const PACKAGES: usize = 200;
        const REPETITIONS: usize = 5;
        let make_diagnostics = || {
            let mut diagnostics = Vec::with_capacity(DIAGNOSTICS);
            for index in 0..DIAGNOSTICS {
                let group = index % 100;
                let package = index % PACKAGES;
                let mut item = diagnostic(
                    "unwrap-in-production",
                    Some("p2"),
                    &format!("crates/member-{package:03}/src/lib.rs"),
                    u32::try_from(index / PACKAGES + 1).expect("bounded line"),
                );
                item.ownership = DiagnosticOwnership::Package {
                    package_id: format!("member-{package:03}"),
                };
                item.root_cause_key = Some(format!("rule:unwrap-in-production:{group:03}"));
                item.site_id = format!("site-{index:06}");
                diagnostics.push(item);
            }
            diagnostics
        };

        let mut elapsed_samples = Vec::with_capacity(REPETITIONS);
        let mut max_added_resident = 0;
        for _ in 0..REPETITIONS {
            let mut diagnostics = make_diagnostics();
            let resident_before = resident_bytes();
            let started = std::time::Instant::now();
            let impact = RootCauseImpact::measure(&diagnostics);
            let impact_elapsed = started.elapsed();
            sort_owned(&mut diagnostics, &impact);
            let sort_elapsed = started.elapsed().saturating_sub(impact_elapsed);
            let groups = root_cause_groups(&diagnostics);
            let elapsed = started.elapsed();
            let grouping_elapsed = elapsed
                .saturating_sub(impact_elapsed)
                .saturating_sub(sort_elapsed);
            eprintln!(
                "canonical decision phases: impact {impact_elapsed:?}, sort {sort_elapsed:?}, grouping {grouping_elapsed:?}, total {elapsed:?}"
            );
            elapsed_samples.push(elapsed);
            max_added_resident =
                max_added_resident.max(resident_bytes().saturating_sub(resident_before));

            assert_eq!(groups.len(), 100);
            let packages: BTreeSet<_> = diagnostics
                .iter()
                .map(|diagnostic| package_key(&diagnostic.ownership))
                .collect();
            assert_eq!(packages.len(), PACKAGES);
        }

        elapsed_samples.sort_unstable();
        let median_elapsed = elapsed_samples[REPETITIONS / 2];
        assert!(
            median_elapsed <= std::time::Duration::from_millis(100),
            "canonical decision median took {median_elapsed:?}, over 100 ms: {elapsed_samples:?}"
        );
        assert!(
            max_added_resident <= 64 * 1024 * 1024,
            "canonical decision added {} MiB resident memory",
            max_added_resident / (1024 * 1024)
        );
    }
}
