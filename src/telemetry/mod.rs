#![expect(
    clippy::redundant_pub_crate,
    reason = "telemetry contracts are shared by sibling modules while the parent module remains crate-private"
)]

mod model;
mod privacy;
mod transport;

use crate::diagnostics::ReportV1;
use model::{AggregateEvent, InvocationSurface};
use std::path::{Path, PathBuf};

pub(crate) use transport::validate_endpoint;

#[derive(Debug, thiserror::Error)]
pub(crate) enum TelemetryError {
    #[error("telemetry config directory is unavailable")]
    ConfigHome,
    #[error("failed to access telemetry state '{}': {source}", path.display())]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid telemetry JSON in '{}': {source}", path.display())]
    Json {
        path: std::path::PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("stored telemetry consent has an unsupported schema")]
    InvalidConsent,
    #[error("invalid telemetry endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("telemetry consent prompt failed: {0}")]
    Prompt(#[source] dialoguer::Error),
}

pub(crate) fn config_root() -> Result<PathBuf, TelemetryError> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("rust-doctor"));
    }
    if let Some(path) = std::env::var_os("APPDATA") {
        return Ok(PathBuf::from(path).join("rust-doctor"));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".config/rust-doctor"))
        .ok_or(TelemetryError::ConfigHome)
}

pub(crate) fn enable(endpoint: &str) -> Result<(), TelemetryError> {
    validate_endpoint(endpoint).map_err(TelemetryError::InvalidEndpoint)?;
    privacy::store(endpoint)
}

pub(crate) fn disable() -> Result<(), TelemetryError> {
    privacy::remove()
}

pub(crate) fn status() -> String {
    if environment_disabled() {
        return "Telemetry disabled by RUST_DOCTOR_TELEMETRY=0.".to_string();
    }
    match privacy::load() {
        Ok(Some(consent)) if consent.enabled => format!(
            "Telemetry enabled for {}. Events are attempted once and never queued locally.",
            sanitized_endpoint(&consent.endpoint)
        ),
        Ok(_) => "Telemetry disabled. No network request is made by default.".to_string(),
        Err(error) => format!("Telemetry disabled because consent is invalid: {error}"),
    }
}

pub(crate) fn record_scan(no_telemetry: bool, offline: bool, report: &ReportV1) {
    let surface = if std::env::var_os("GITHUB_ACTIONS").is_some() {
        InvocationSurface::Action
    } else {
        InvocationSurface::Cli
    };
    deliver_if_enabled(
        no_telemetry,
        offline,
        &AggregateEvent::scan(surface, report),
    );
}

pub(crate) fn record_session(no_telemetry: bool, offline: bool, surface: InvocationSurface) {
    deliver_if_enabled(no_telemetry, offline, &AggregateEvent::session(surface));
}

pub(crate) fn install_panic_hook(
    no_telemetry: bool,
    offline: bool,
    surface: InvocationSurface,
    project_root: &Path,
) {
    if enabled_endpoint(no_telemetry, offline).is_none() {
        return;
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel::<AggregateEvent>(8);
    let worker = std::thread::Builder::new()
        .name("rust-doctor-crash-telemetry".to_string())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                if let Some(endpoint) = enabled_endpoint(no_telemetry, offline) {
                    let _ = transport::deliver(&endpoint, &event);
                }
            }
        });
    if worker.is_err() {
        return;
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from);
    let project_root = project_root.to_path_buf();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        let message = information.payload().downcast_ref::<&str>().map_or_else(
            || {
                information
                    .payload()
                    .downcast_ref::<String>()
                    .map_or("panic", String::as_str)
            },
            |message| *message,
        );
        let summary = scrub_crash(message, home.as_deref(), &project_root);
        let event = AggregateEvent::crash(surface, summary);
        eprintln!("Rust Doctor crash event ID: {}", event.event_id);
        let _ = sender.try_send(event);
        previous(information);
    }));
}

pub(crate) const fn mcp_surface() -> InvocationSurface {
    InvocationSurface::Mcp
}

pub(crate) const fn lsp_surface() -> InvocationSurface {
    InvocationSurface::Lsp
}

pub(crate) const fn cli_surface() -> InvocationSurface {
    InvocationSurface::Cli
}

fn deliver_if_enabled(no_telemetry: bool, offline: bool, event: &AggregateEvent) {
    if let Some(endpoint) = enabled_endpoint(no_telemetry, offline) {
        let _ = transport::deliver(&endpoint, event);
    }
}

fn enabled_endpoint(no_telemetry: bool, offline: bool) -> Option<String> {
    if !(DeliveryPolicy {
        no_telemetry,
        offline,
        environment_disabled: environment_disabled(),
        has_consent: true,
    })
    .allows()
    {
        return None;
    }
    privacy::load()
        .ok()
        .flatten()
        .filter(|consent| consent.enabled)
        .map(|consent| consent.endpoint)
}

#[derive(Clone, Copy)]
struct DeliveryPolicy {
    no_telemetry: bool,
    offline: bool,
    environment_disabled: bool,
    has_consent: bool,
}

impl DeliveryPolicy {
    const fn allows(self) -> bool {
        self.has_consent && !self.no_telemetry && !self.offline && !self.environment_disabled
    }
}

fn environment_disabled() -> bool {
    std::env::var("RUST_DOCTOR_TELEMETRY").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off"
        )
    })
}

fn sanitized_endpoint(endpoint: &str) -> String {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return "<invalid endpoint>".to_string();
    };
    let Some(host) = url.host_str() else {
        return "<invalid endpoint>".to_string();
    };
    let port = url
        .port()
        .map_or_else(String::new, |port| format!(":{port}"));
    let path = if url.path() == "/" {
        "/"
    } else {
        "/<redacted>"
    };
    format!("{}://{host}{port}{path}", url.scheme())
}

fn scrub_crash(message: &str, home: Option<&Path>, project_root: &Path) -> String {
    let mut scrubbed = message.to_string();
    for prefix in home.into_iter().chain(std::iter::once(project_root)) {
        let display = prefix.to_string_lossy();
        if !display.is_empty() {
            scrubbed = scrubbed.replace(display.as_ref(), "<path>");
        }
    }
    let scrubbed = scrubbed
        .split_whitespace()
        .map(|token| {
            if token.contains('/') || token.contains('\\') {
                "<path>"
            } else if token.contains('=') {
                "<value>"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let path_removed = scrubbed.contains("<path>");
    let value_removed = scrubbed.contains("<value>");
    format!(
        "panic payload redacted; path-prefixes-removed={path_removed}; values-removed={value_removed}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_report() -> ReportV1 {
        let mut report: ReportV1 = serde_json::from_value(serde_json::json!({
            "schema_version": "1",
            "tool_version": "0.0.0",
            "report_constructed": true,
            "outcome": "clean",
            "requested_root": ".",
            "resolved_root": null,
            "mode": "full",
            "gate_result": "passed",
            "completeness": {
                "state": "complete",
                "planned_files": 2,
                "analyzed_files": 2,
                "completed_checks": 1,
                "skipped_checks": 0,
                "failed_checks": 0,
                "timed_out_checks": 0,
                "cancelled_checks": 0
            },
            "projects": [],
            "diagnostics": [],
            "summary": {
                "score": 100,
                "score_label": "Great",
                "diagnostic_count": 0,
                "error_count": 0,
                "warning_count": 0,
                "info_count": 0
            },
            "elapsed_ms": 12,
            "audit": {"suppressed_security": []},
            "score": 100,
            "score_label": "Great",
            "dimension_scores": null,
            "source_file_count": 2,
            "elapsed": 0.012,
            "skipped_passes": [],
            "error_count": 0,
            "warning_count": 0,
            "info_count": 0
        }))
        .expect("report fixture parses");
        report.audit.abstentions = vec![crate::diagnostics::AbstentionReceipt {
            rule: "unwrap-in-production".to_string(),
            reason: "unsupported-source-surface".to_string(),
            count: 3,
        }];
        report
    }

    #[test]
    fn a_disabled_telemetry_configuration_attempts_no_network_request() {
        // The policy is the only thing standing between a scan and a request,
        // so it is asserted directly for every reason it can deny (AC-1).
        assert!(enabled_endpoint(true, false).is_none());
        assert!(enabled_endpoint(false, true).is_none());
    }

    #[test]
    fn a_scan_event_carries_the_score_model_and_an_anonymous_cohort() {
        let event = AggregateEvent::scan(InvocationSurface::Cli, &scan_report());
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["schema_version"], model::SCHEMA_VERSION);
        assert_eq!(
            value["score_model_version"],
            crate::output::score_model_version()
        );
        let cohort = value["installation_cohort"].as_str().expect("cohort");
        assert!(cohort.starts_with("cohort-"), "{cohort}");
        // Two events from the same build share a cohort: it identifies a
        // population, never an installation.
        let second = AggregateEvent::scan(InvocationSurface::Mcp, &scan_report());
        assert_eq!(
            serde_json::to_value(&second).unwrap()["installation_cohort"],
            value["installation_cohort"]
        );
        assert_ne!(second.event_id, event.event_id);
    }

    #[test]
    fn rule_populations_stay_distinct_and_bounded_to_the_catalog() {
        let event = AggregateEvent::scan(InvocationSurface::Cli, &scan_report());
        let counts = &event.rule_counts;
        assert_eq!(counts.abstained.get("unwrap-in-production"), Some(&3));
        assert!(
            counts.fired.is_empty(),
            "no diagnostic fired in the fixture"
        );
        assert!(counts.suppressed.is_empty());
        assert!(counts.score_contributing.is_empty());
        assert!(counts.disabled > 0, "the catalog ships opt-in rules");
        let catalog = crate::catalog::built_in_catalog().expect("catalog");
        for rule in counts
            .fired
            .keys()
            .chain(counts.abstained.keys())
            .chain(counts.suppressed.keys())
            .chain(counts.score_contributing.keys())
        {
            assert!(
                catalog.exact(rule).is_some(),
                "{rule} is not a catalog rule"
            );
        }
    }

    #[test]
    fn an_unknown_rule_is_bucketed_without_uploading_its_identifier() {
        let mut report = scan_report();
        report
            .audit
            .abstentions
            .push(crate::diagnostics::AbstentionReceipt {
                rule: "vendor::secret-internal-rule".to_string(),
                reason: "missing-context".to_string(),
                count: 2,
            });
        let event = AggregateEvent::scan(InvocationSurface::Cli, &report);
        assert_eq!(event.rule_counts.unknown, 2);
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(
            !serialized.contains("secret-internal-rule"),
            "an unknown identifier must not be uploaded"
        );
    }

    #[test]
    fn a_payload_over_the_ceiling_is_compacted_deterministically_then_dropped() {
        let mut event = AggregateEvent::scan(InvocationSurface::Cli, &scan_report());
        // A pathological rule map that cannot fit under the 64 KiB ceiling.
        for index in 0..20_000 {
            event
                .rule_counts
                .fired
                .insert(format!("rule-{index:06}"), index);
        }
        let payload = event.to_bounded_payload().expect("compaction succeeds");
        assert!(payload.len() <= 64 * 1024);
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        // Compaction drops the per-rule maps first and keeps the aggregates.
        assert!(
            value["rule_counts"]["fired"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        assert_eq!(value["completeness"], "complete");
        assert!(value["pass_states"].is_object());

        let compacted = event.to_bounded_payload().expect("deterministic");
        assert_eq!(payload, compacted);
    }

    #[test]
    fn every_delivered_payload_passes_privacy_validation() {
        for event in [
            AggregateEvent::scan(InvocationSurface::Cli, &scan_report()),
            AggregateEvent::session(InvocationSurface::Action),
            AggregateEvent::crash(InvocationSurface::Lsp, "panic payload redacted".to_string()),
        ] {
            let value = serde_json::to_value(&event).unwrap();
            let violations = privacy::violations(&value);
            assert!(violations.is_empty(), "{violations:?}");
            assert!(event.to_bounded_payload().is_some());
        }
    }

    #[test]
    fn analyzer_state_is_always_serialized_so_absence_is_never_read_as_healthy() {
        // AC-8: an older client omitting a field must be distinguishable from a
        // client reporting a healthy analyzer, so nothing here is skipped.
        let value = serde_json::to_value(AggregateEvent::session(InvocationSurface::Cli)).unwrap();
        for required in [
            "schema_version",
            "score_model_version",
            "completeness",
            "pass_states",
            "rule_counts",
            "trust_tiers",
            "aggregate_counts",
            "suppression_counts",
        ] {
            assert!(value.get(required).is_some(), "{required} was skipped");
        }
        assert_eq!(value["completeness"], "not-applicable");
    }

    #[test]
    fn event_schema_cannot_serialize_prohibited_fields() {
        let event = AggregateEvent::session(InvocationSurface::Cli);
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["schema_version"], model::SCHEMA_VERSION);
        let serialized = value.to_string();
        for prohibited in [
            "repository",
            "source_text",
            "diagnostic_message",
            "git_remote",
            "environment",
            "command_argument",
            "persistent_id",
        ] {
            assert!(!serialized.contains(prohibited), "found {prohibited}");
        }
        assert_eq!(value["suppression_counts"]["path"], 0);
        assert!(!serialized.contains("/home/"));
        assert!(!serialized.contains("src/lib.rs"));
    }

    #[test]
    fn every_surface_uses_the_same_fail_closed_delivery_policy() {
        let allowed = DeliveryPolicy {
            no_telemetry: false,
            offline: false,
            environment_disabled: false,
            has_consent: true,
        };
        assert!(allowed.allows());
        for denied in [
            DeliveryPolicy {
                has_consent: false,
                ..allowed
            },
            DeliveryPolicy {
                no_telemetry: true,
                ..allowed
            },
            DeliveryPolicy {
                offline: true,
                ..allowed
            },
            DeliveryPolicy {
                environment_disabled: true,
                ..allowed
            },
        ] {
            assert!(!denied.allows());
        }

        for surface in [
            InvocationSurface::Cli,
            InvocationSurface::Action,
            InvocationSurface::Mcp,
            InvocationSurface::Lsp,
        ] {
            let event = serde_json::to_value(AggregateEvent::session(surface)).unwrap();
            assert_eq!(event["schema_version"], model::SCHEMA_VERSION);
        }
    }

    #[test]
    fn crash_events_use_the_same_current_schema_as_session_events() {
        let event =
            AggregateEvent::crash(InvocationSurface::Cli, "panic payload redacted".to_string());
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["schema_version"], model::SCHEMA_VERSION);
        assert_eq!(value["event_kind"], "crash");
        assert!(value["suppression_counts"].is_object());
    }

    #[test]
    fn crash_scrubbing_removes_paths_and_environment_values() {
        let scrubbed = scrub_crash(
            "failed /home/user/project/src/lib.rs TOKEN=secret",
            Some(Path::new("/home/user")),
            Path::new("/home/user/project"),
        );
        assert!(!scrubbed.contains("/home/user"));
        assert!(!scrubbed.contains("secret"));
        assert!(scrubbed.contains("path-prefixes-removed=true"));
        assert!(scrubbed.contains("values-removed=true"));
    }

    #[test]
    fn endpoint_requires_https_except_loopback() {
        assert!(validate_endpoint("https://telemetry.example/events").is_ok());
        assert!(validate_endpoint("http://127.0.0.1:3000/events").is_ok());
        assert!(validate_endpoint("http://example.com/events").is_err());
        assert!(validate_endpoint("https://user:secret@example.com/events").is_err());
    }
}
