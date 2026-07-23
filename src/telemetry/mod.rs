#![expect(
    clippy::redundant_pub_crate,
    reason = "telemetry contracts are shared by sibling modules while the parent module remains crate-private"
)]

mod model;
mod privacy;
mod transport;

use crate::diagnostics::ScanResult;
use model::{AggregateEvent, InvocationSurface};
use std::path::Path;

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

pub(crate) fn record_scan(no_telemetry: bool, offline: bool, result: &ScanResult) {
    let surface = if std::env::var_os("GITHUB_ACTIONS").is_some() {
        InvocationSurface::Action
    } else {
        InvocationSurface::Cli
    };
    deliver_if_enabled(
        no_telemetry,
        offline,
        &AggregateEvent::scan(surface, result),
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
    if no_telemetry || offline || environment_disabled() {
        return None;
    }
    privacy::load()
        .ok()
        .flatten()
        .filter(|consent| consent.enabled)
        .map(|consent| consent.endpoint)
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

    #[test]
    fn event_schema_cannot_serialize_prohibited_fields() {
        let event = AggregateEvent::session(InvocationSurface::Cli);
        let value = serde_json::to_value(event).unwrap();
        let serialized = value.to_string();
        for prohibited in [
            "path",
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
