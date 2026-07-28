use super::TelemetryError;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Consent {
    pub(crate) schema_version: String,
    pub(crate) enabled: bool,
    pub(crate) endpoint: String,
}

pub(crate) fn load() -> Result<Option<Consent>, TelemetryError> {
    load_from(&super::config_root()?)
}

pub(crate) fn store(endpoint: &str) -> Result<(), TelemetryError> {
    store_at(
        &super::config_root()?,
        &Consent {
            schema_version: "1.1".to_string(),
            enabled: true,
            endpoint: endpoint.to_string(),
        },
    )
}

pub(crate) fn remove() -> Result<(), TelemetryError> {
    let path = consent_path(&super::config_root()?);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(TelemetryError::Io { path, source }),
    }
}

fn consent_path(root: &Path) -> PathBuf {
    root.join("telemetry-consent.json")
}

fn load_from(root: &Path) -> Result<Option<Consent>, TelemetryError> {
    let path = consent_path(root);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(TelemetryError::Io { path, source }),
    };
    let stored_consent: Consent =
        serde_json::from_str(&content).map_err(|source| TelemetryError::Json { path, source })?;
    if stored_consent.schema_version != "1.1" {
        return Err(TelemetryError::InvalidConsent);
    }
    Ok(Some(stored_consent))
}

fn store_at(root: &Path, consent: &Consent) -> Result<(), TelemetryError> {
    std::fs::create_dir_all(root).map_err(|source| TelemetryError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let destination = consent_path(root);
    let mut temporary =
        tempfile::NamedTempFile::new_in(root).map_err(|source| TelemetryError::Io {
            path: root.to_path_buf(),
            source,
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|source| TelemetryError::Io {
                path: temporary.path().to_path_buf(),
                source,
            })?;
    }
    serde_json::to_writer_pretty(&mut temporary, consent).map_err(|source| {
        TelemetryError::Json {
            path: destination.clone(),
            source,
        }
    })?;
    temporary
        .write_all(b"\n")
        .map_err(|source| TelemetryError::Io {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    temporary.flush().map_err(|source| TelemetryError::Io {
        path: temporary.path().to_path_buf(),
        source,
    })?;
    temporary
        .persist(&destination)
        .map_err(|error| TelemetryError::Io {
            path: destination,
            source: error.error,
        })?;
    Ok(())
}

/// Payload content the schema must never carry.
///
/// This is the deny-list side of the contract: the event type only models
/// aggregates, and this check proves that no aggregate smuggled a path, a
/// message, a package name, or an exact timestamp through (US-013 AC-3).
pub(crate) fn violations(payload: &serde_json::Value) -> Vec<String> {
    let mut found = Vec::new();
    walk(payload, &mut Vec::new(), &mut found);
    found.sort();
    found.dedup();
    found
}

fn walk(value: &serde_json::Value, path: &mut Vec<String>, found: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                // A prohibited name only leaks when it carries content. An
                // aggregate count named `path` is a number of path-scoped
                // suppressions, not a path.
                let carries_content = !matches!(
                    child,
                    serde_json::Value::Number(_)
                        | serde_json::Value::Bool(_)
                        | serde_json::Value::Null
                );
                if carries_content && is_prohibited_key(key) {
                    found.push(format!("prohibited field '{key}'"));
                }
                path.push(key.clone());
                walk(child, path, found);
                path.pop();
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk(item, path, found);
            }
        }
        serde_json::Value::String(text) => {
            // Rule identities are the one string population allowed to be
            // free-form, and they are already bounded to the catalog.
            let in_rule_map = path.iter().any(|segment| segment == "rule_counts");
            if !in_rule_map && let Some(reason) = prohibited_text(text) {
                found.push(format!(
                    "{reason} in '{}'",
                    path.last().map_or("<root>", String::as_str)
                ));
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn is_prohibited_key(key: &str) -> bool {
    const PROHIBITED: &[&str] = &[
        "path",
        "file",
        "file_path",
        "source",
        "source_text",
        "message",
        "diagnostic_message",
        "package",
        "package_name",
        "crate",
        "repository",
        "git_remote",
        "remote",
        "command",
        "command_line",
        "argv",
        "environment",
        "env",
        "timestamp",
        "user",
        "hostname",
        "machine_id",
        "installation_id",
        "persistent_id",
    ];
    PROHIBITED.contains(&key)
}

fn prohibited_text(text: &str) -> Option<&'static str> {
    if text.contains('/') || text.contains('\\') {
        // Documented exception: the cohort and version banners never contain
        // separators, so any separator here is a path.
        return Some("path separator");
    }
    if text.contains("://") {
        return Some("URL");
    }
    if text.contains('@') {
        return Some("address-like token");
    }
    // Not a filesystem lookup: this is a deny-list over payload text, so a
    // case-insensitive suffix match on the raw string is exactly the check.
    let lowered = text.to_ascii_lowercase();
    if [".rs", ".toml"]
        .iter()
        .any(|suffix| lowered.as_str().ends_with(suffix))
    {
        return Some("file name");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deny_list_rejects_paths_messages_and_identifiers() {
        let leaky = serde_json::json!({
            "schema_version": "1.2",
            "file_path": "/home/user/project/src/lib.rs",
        });
        let found = violations(&leaky);
        assert!(
            found.iter().any(|entry| entry.contains("file_path")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|entry| entry.contains("path separator")),
            "{found:?}"
        );

        assert!(
            violations(&serde_json::json!({ "note": "src/lib.rs" }))
                .iter()
                .any(|entry| entry.contains("path separator"))
        );
        assert!(
            violations(&serde_json::json!({ "remote": "origin" }))
                .iter()
                .any(|entry| entry.contains("remote"))
        );
        // An aggregate count that happens to be named `path` carries no path.
        assert!(violations(&serde_json::json!({ "suppression_counts": { "path": 3 } })).is_empty());
        assert!(violations(&serde_json::json!({ "ok": "complete" })).is_empty());
    }

    #[test]
    fn consent_round_trip_is_versioned_without_event_storage() {
        let root = tempfile::tempdir().unwrap();
        let consent = Consent {
            schema_version: "1.1".to_string(),
            enabled: true,
            endpoint: "https://telemetry.example/events".to_string(),
        };
        store_at(root.path(), &consent).unwrap();
        let loaded = load_from(root.path()).unwrap().unwrap();
        assert_eq!(loaded.endpoint, consent.endpoint);
        assert_eq!(loaded.schema_version, "1.1");
    }
}
