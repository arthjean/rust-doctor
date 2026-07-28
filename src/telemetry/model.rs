use crate::diagnostics::{CheckStatus, CompletenessState, ReportV1, SuppressionCounts};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum InvocationSurface {
    Cli,
    Action,
    Mcp,
    Lsp,
}

/// Schema version of the aggregate event wire shape.
///
/// Bumped by US-013: the payload now carries the score-model version, an
/// anonymous installation cohort, per-rule aggregate counts, and trust-tier
/// counts. Every field below is always serialized, so a server reading an older
/// event sees a missing field rather than inferring a healthy analyzer state
/// (AC-8).
pub(crate) const SCHEMA_VERSION: &str = "1.2";

/// Hard payload ceiling (NFR-015).
const MAX_EVENT_BYTES: usize = 64 * 1024;

/// Number of anonymous installation cohorts. Coarse enough that a bucket
/// identifies a population, never an installation.
const COHORT_BUCKETS: u64 = 64;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AggregateEvent {
    pub(crate) schema_version: &'static str,
    pub(crate) event_id: String,
    pub(crate) event_kind: &'static str,
    pub(crate) tool_version: &'static str,
    /// Score Core version that produced the numbers in this event. Events from
    /// different models are never comparable.
    pub(crate) score_model_version: &'static str,
    /// Coarse anonymous cohort derived from the release and platform alone.
    /// Not persisted, not an installation identifier.
    pub(crate) installation_cohort: String,
    pub(crate) platform: Platform,
    pub(crate) invocation_surface: InvocationSurface,
    pub(crate) duration_bucket: &'static str,
    pub(crate) completeness: &'static str,
    pub(crate) aggregate_counts: AggregateCounts,
    /// Analyzer receipts: how many checks landed in each execution state.
    pub(crate) pass_states: BTreeMap<String, usize>,
    /// Per-rule aggregate behavior, bounded to the current catalog.
    pub(crate) rule_counts: RuleCounts,
    /// Emitted findings per trust tier.
    pub(crate) trust_tiers: BTreeMap<String, usize>,
    pub(crate) suppression_counts: SuppressionCounts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) crash_summary: Option<String>,
}

/// Per-rule activation behavior.
///
/// The five populations stay distinct: a suppressed finding is not a fired one,
/// an abstention is not a suppression, and a rule contributing to the score is
/// a strict subset of the rules that fired. Every key is a catalog rule
/// identity; anything else is counted in `unknown` without uploading its
/// identifier (AC-4, AC-7).
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct RuleCounts {
    pub(crate) fired: BTreeMap<String, usize>,
    pub(crate) suppressed: BTreeMap<String, usize>,
    pub(crate) abstained: BTreeMap<String, usize>,
    pub(crate) score_contributing: BTreeMap<String, usize>,
    /// Findings whose rule is not in the current catalog.
    pub(crate) unknown: usize,
    /// Catalog rules that ship disabled in this build.
    pub(crate) disabled: usize,
}

impl RuleCounts {
    /// Deterministic compaction step: the per-rule maps are the only unbounded
    /// part of the payload, so they go first (AC-5).
    fn compact(&mut self) {
        self.fired.clear();
        self.suppressed.clear();
        self.abstained.clear();
        self.score_contributing.clear();
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Platform {
    os: &'static str,
    architecture: &'static str,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct AggregateCounts {
    errors: usize,
    warnings: usize,
    info: usize,
    diagnostics: usize,
    planned_files: usize,
    analyzed_files: usize,
}

impl AggregateEvent {
    pub(crate) fn scan(surface: InvocationSurface, report: &ReportV1) -> Self {
        let mut pass_states = BTreeMap::new();
        let mut workspace_checks = std::collections::BTreeSet::new();
        for project in &report.projects {
            for check in &project.checks {
                if check.name.starts_with("workspace:")
                    && !workspace_checks.insert((check.name.clone(), status_name(check.status)))
                {
                    continue;
                }
                *pass_states
                    .entry(status_name(check.status).to_string())
                    .or_default() += 1;
            }
        }
        let (rule_counts, trust_tiers) = rule_aggregates(report);
        Self {
            schema_version: SCHEMA_VERSION,
            event_id: event_id(),
            event_kind: "scan",
            tool_version: env!("CARGO_PKG_VERSION"),
            score_model_version: crate::output::score_model_version(),
            installation_cohort: installation_cohort(),
            platform: platform(),
            invocation_surface: surface,
            duration_bucket: duration_bucket(Duration::from_millis(report.elapsed_ms)),
            completeness: match report.completeness.state {
                CompletenessState::Complete => "complete",
                CompletenessState::Partial => "partial",
                CompletenessState::Incomplete => "incomplete",
            },
            aggregate_counts: AggregateCounts {
                errors: report.summary.error_count,
                warnings: report.summary.warning_count,
                info: report.summary.info_count,
                diagnostics: report.summary.diagnostic_count,
                planned_files: report.completeness.planned_files,
                analyzed_files: report.completeness.analyzed_files,
            },
            pass_states,
            rule_counts,
            trust_tiers,
            suppression_counts: report.audit.suppression_counts.clone(),
            crash_summary: None,
        }
    }

    pub(crate) fn session(surface: InvocationSurface) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            event_id: event_id(),
            event_kind: "session",
            tool_version: env!("CARGO_PKG_VERSION"),
            score_model_version: crate::output::score_model_version(),
            installation_cohort: installation_cohort(),
            platform: platform(),
            invocation_surface: surface,
            duration_bucket: "not-applicable",
            completeness: "not-applicable",
            aggregate_counts: AggregateCounts::default(),
            pass_states: BTreeMap::new(),
            rule_counts: RuleCounts {
                disabled: disabled_rule_count(),
                ..RuleCounts::default()
            },
            trust_tiers: BTreeMap::new(),
            suppression_counts: SuppressionCounts::default(),
            crash_summary: None,
        }
    }

    /// Serialize within the payload ceiling, compacting deterministically.
    ///
    /// Compaction never blocks the scan and never partially truncates JSON: it
    /// drops the per-rule maps, and if the event still does not fit it is
    /// dropped entirely (AC-5).
    pub(crate) fn to_bounded_payload(&self) -> Option<Vec<u8>> {
        let value = serde_json::to_value(self).ok()?;
        // Fail closed: a payload that trips the deny-list is dropped, not sent.
        // The event type only models aggregates, so this should never fire —
        // which is exactly why it is checked before every delivery (AC-3).
        if !super::privacy::violations(&value).is_empty() {
            return None;
        }
        let serialized = serde_json::to_vec(&value).ok()?;
        if serialized.len() <= MAX_EVENT_BYTES {
            return Some(serialized);
        }
        let mut compacted = self.clone();
        compacted.rule_counts.compact();
        let serialized = serde_json::to_vec(&compacted).ok()?;
        (serialized.len() <= MAX_EVENT_BYTES).then_some(serialized)
    }

    pub(crate) fn crash(surface: InvocationSurface, summary: String) -> Self {
        let mut event = Self::session(surface);
        event.event_kind = "crash";
        event.crash_summary = Some(summary);
        event
    }
}

/// Per-rule and per-tier aggregates for one report.
///
/// Only catalog identities are ever named. A finding from an unknown or
/// dynamically discovered rule increments the unknown bucket and its identifier
/// stays local (AC-7).
fn rule_aggregates(report: &ReportV1) -> (RuleCounts, BTreeMap<String, usize>) {
    let catalog = crate::catalog::built_in_catalog().ok();
    let mut counts = RuleCounts {
        disabled: disabled_rule_count(),
        ..RuleCounts::default()
    };
    let mut tiers: BTreeMap<String, usize> = BTreeMap::new();

    let known = |rule: &str| -> Option<&'static crate::catalog::RuleDescriptor> {
        catalog.and_then(|catalog| catalog.exact(rule))
    };

    for diagnostic in &report.diagnostics {
        let Some(descriptor) = known(&diagnostic.rule) else {
            counts.unknown += 1;
            continue;
        };
        *counts
            .fired
            .entry(descriptor.canonical_id.clone())
            .or_default() += 1;
        *tiers
            .entry(descriptor.trust.tier.as_str().to_string())
            .or_default() += 1;
        if descriptor.trust.score_eligible && !diagnostic.advisory {
            *counts
                .score_contributing
                .entry(descriptor.canonical_id.clone())
                .or_default() += 1;
        }
    }
    for diagnostic in &report.audit.suppressed_security {
        match known(&diagnostic.rule) {
            Some(descriptor) => {
                *counts
                    .suppressed
                    .entry(descriptor.canonical_id.clone())
                    .or_default() += 1;
            }
            None => counts.unknown += 1,
        }
    }
    for receipt in &report.audit.abstentions {
        match known(&receipt.rule) {
            Some(descriptor) => {
                *counts
                    .abstained
                    .entry(descriptor.canonical_id.clone())
                    .or_default() += receipt.count;
            }
            None => counts.unknown += receipt.count,
        }
    }
    (counts, tiers)
}

fn disabled_rule_count() -> usize {
    crate::catalog::built_in_catalog().map_or(0, |catalog| {
        catalog
            .descriptors()
            .iter()
            .filter(|descriptor| !descriptor.default_enabled)
            .count()
    })
}

/// Coarse cohort derived from the release and platform only.
///
/// Nothing machine-specific goes in: two installations of the same build on the
/// same platform share a cohort by construction, so the value can never single
/// one out.
fn installation_cohort() -> String {
    let mut hasher = Sha256::new();
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    hasher.update(std::env::consts::OS.as_bytes());
    hasher.update(std::env::consts::ARCH.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    format!("cohort-{:02}", u64::from_le_bytes(bytes) % COHORT_BUCKETS)
}

fn event_id() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let sequence = EVENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(elapsed.as_nanos().to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(sequence.to_le_bytes());
    format!("{:x}", hasher.finalize())
}

const fn platform() -> Platform {
    Platform {
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
    }
}

const fn duration_bucket(duration: Duration) -> &'static str {
    if duration.as_millis() < 100 {
        "under-100ms"
    } else if duration.as_secs() < 1 {
        "100ms-to-1s"
    } else if duration.as_secs() < 10 {
        "1s-to-10s"
    } else if duration.as_secs() < 60 {
        "10s-to-1m"
    } else {
        "over-1m"
    }
}

const fn status_name(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Planned => "planned",
        CheckStatus::Running => "running",
        CheckStatus::Completed => "completed",
        CheckStatus::Skipped => "skipped",
        CheckStatus::Failed => "failed",
        CheckStatus::TimedOut => "timed-out",
        CheckStatus::Cancelled => "cancelled",
    }
}
