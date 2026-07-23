use crate::diagnostics::{CheckStatus, ScanResult};
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
    pub(crate) suppression_count: usize,
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
    pub(crate) fn scan(surface: InvocationSurface, result: &ScanResult) -> Self {
        let score_authoritative = crate::completeness::score_is_authoritative(result);
        let mut pass_states = BTreeMap::new();
        for check in &result.execution.checks {
            *pass_states
                .entry(status_name(check.status).to_string())
                .or_default() += 1;
        }
        Self {
            schema_version: "1.0",
            event_id: event_id(),
            event_kind: "scan",
            tool_version: env!("CARGO_PKG_VERSION"),
            platform: platform(),
            invocation_surface: surface,
            duration_bucket: duration_bucket(result.elapsed),
            completeness: if score_authoritative {
                "complete"
            } else {
                "incomplete"
            },
            aggregate_counts: AggregateCounts {
                errors: result.error_count,
                warnings: result.warning_count,
                info: result.info_count,
                diagnostics: result.diagnostics.len(),
                planned_files: result.planned_files.len(),
                analyzed_files: result.analyzed_files.len(),
            },
            pass_states,
            suppression_count: result.suppressed_security.len(),
            crash_summary: None,
        }
    }

    pub(crate) fn session(surface: InvocationSurface) -> Self {
        Self {
            schema_version: "1.0",
            event_id: event_id(),
            event_kind: "session",
            tool_version: env!("CARGO_PKG_VERSION"),
            platform: platform(),
            invocation_surface: surface,
            duration_bucket: "not-applicable",
            completeness: "not-applicable",
            aggregate_counts: AggregateCounts::default(),
            pass_states: BTreeMap::new(),
            suppression_count: 0,
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
