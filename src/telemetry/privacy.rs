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

#[cfg(test)]
mod tests {
    use super::*;

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
