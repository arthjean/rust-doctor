use super::model::AggregateEvent;
use std::time::Duration;

pub(crate) fn validate_endpoint(endpoint: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(endpoint).map_err(|error| error.to_string())?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("endpoint must not contain credentials, a query, or a fragment".to_string());
    }
    let loopback_http = url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !loopback_http {
        return Err("endpoint must use HTTPS; HTTP is accepted only on loopback".to_string());
    }
    Ok(())
}

pub(crate) fn deliver(endpoint: &str, event: &AggregateEvent) -> Result<(), String> {
    validate_endpoint(endpoint)?;
    // An event that cannot be compacted under the ceiling is dropped rather
    // than truncated: a partial payload would be unparseable, and delivery is
    // never allowed to delay the scan (NFR-015).
    let payload = event
        .to_bounded_payload()
        .ok_or_else(|| "telemetry event exceeds the payload ceiling".to_string())?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("rust-doctor/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(endpoint)
        .header("x-rust-doctor-schema", event.schema_version)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload)
        .send()
        .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("collector returned HTTP {}", response.status()))
    }
}
