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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AggregateEvent {
    pub(crate) schema_version: &'static str,
    pub(crate) event_id: String,
    pub(crate) event_kind: &'static str,
    pub(crate) tool_version: &'static str,
    pub(crate) platform: Platform,
    pub(crate) invocation_surface: InvocationSurface,
    pub(crate) duration_bucket: &'static str,
    pub(crate) completeness: &'static str,
    pub(crate) aggregate_counts: AggregateCounts,
    pub(crate) pass_states: BTreeMap<String, usize>,
    pub(crate) suppression_counts: SuppressionCounts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) crash_summary: Option<String>,
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
        Self {
            schema_version: "1.1",
            event_id: event_id(),
            event_kind: "scan",
            tool_version: env!("CARGO_PKG_VERSION"),
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
            suppression_counts: report.audit.suppression_counts.clone(),
            crash_summary: None,
        }
    }

    pub(crate) fn session(surface: InvocationSurface) -> Self {
        Self {
            schema_version: "1.1",
            event_id: event_id(),
            event_kind: "session",
            tool_version: env!("CARGO_PKG_VERSION"),
            platform: platform(),
            invocation_surface: surface,
            duration_bucket: "not-applicable",
            completeness: "not-applicable",
            aggregate_counts: AggregateCounts::default(),
            pass_states: BTreeMap::new(),
            suppression_counts: SuppressionCounts::default(),
            crash_summary: None,
        }
    }

    pub(crate) fn crash(surface: InvocationSurface, summary: String) -> Self {
        let mut event = Self::session(surface);
        event.event_kind = "crash";
        event.crash_summary = Some(summary);
        event
    }
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
