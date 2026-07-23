//! Bounded incremental cache for custom AST rules.
//!
//! Entries are keyed by a complete analyzer fingerprint and file content. The
//! cache is best effort: corrupt, stale, oversized, or future data is ignored.

use crate::diagnostics::Diagnostic;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const CACHE_VERSION: u32 = 2;
const REPORT_SCHEMA_VERSION: &str = "1.0";
const CACHE_FILENAME: &str = ".rust-doctor-cache.json";
const DEFAULT_CACHE_MAX_BYTES: usize = 512 * 1024 * 1024;
const MAX_TRANSIENT_CACHE_BYTES: u64 = DEFAULT_CACHE_MAX_BYTES as u64;
const RULE_IMPLEMENTATION_SOURCES: &[&str] = &[
    include_str!("passes/static_analysis/rules/mod.rs"),
    include_str!("passes/static_analysis/rules/async_rules.rs"),
    include_str!("passes/static_analysis/rules/complexity.rs"),
    include_str!("passes/static_analysis/rules/error_handling.rs"),
    include_str!("passes/static_analysis/rules/framework.rs"),
    include_str!("passes/static_analysis/rules/framework_packs.rs"),
    include_str!("passes/static_analysis/rules/performance/mod.rs"),
    include_str!("passes/static_analysis/rules/performance/allocation.rs"),
    include_str!("passes/static_analysis/rules/performance/clone.rs"),
    include_str!("passes/static_analysis/rules/performance/collect_iterate.rs"),
    include_str!("passes/static_analysis/rules/performance/large_enum.rs"),
    include_str!("passes/static_analysis/rules/performance/string_literal.rs"),
    include_str!("passes/static_analysis/rules/reliability.rs"),
    include_str!("passes/static_analysis/rules/security.rs"),
    include_str!("passes/static_analysis/rules/tranche.rs"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileEntry {
    hash: String,
    diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    last_accessed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanCache {
    version: u32,
    config_hash: String,
    #[serde(default)]
    access_clock: u64,
    #[serde(skip, default = "default_cache_max_bytes")]
    max_bytes: usize,
    files: HashMap<PathBuf, FileEntry>,
}

impl ScanCache {
    pub fn new(config_hash: String) -> Self {
        Self {
            version: CACHE_VERSION,
            config_hash,
            access_clock: 0,
            max_bytes: DEFAULT_CACHE_MAX_BYTES,
            files: HashMap::new(),
        }
    }

    /// Load a compatible cache without ever reading beyond the transient cap.
    pub fn load(project_root: &Path, config_hash: &str) -> Option<Self> {
        let cache_path = project_root.join(CACHE_FILENAME);
        tracing::debug!(path = %cache_path.display(), "loading scan cache");
        let metadata = std::fs::metadata(&cache_path).ok()?;
        if metadata.len() > MAX_TRANSIENT_CACHE_BYTES {
            tracing::debug!(
                bytes = metadata.len(),
                "cache miss: cache exceeds size budget"
            );
            return None;
        }
        let content = std::fs::read(&cache_path).ok()?;
        let mut cache: Self = serde_json::from_slice(&content).ok()?;
        if cache.version != CACHE_VERSION {
            tracing::debug!("cache miss: version mismatch");
            return None;
        }
        if cache.config_hash != config_hash {
            tracing::debug!("cache miss: analyzer fingerprint changed");
            return None;
        }
        cache.max_bytes = DEFAULT_CACHE_MAX_BYTES;
        if content.len() > cache.max_bytes {
            cache.enforce_limit();
        }
        tracing::debug!(files = cache.files.len(), "cache loaded");
        Some(cache)
    }

    /// Persist through a same-directory temporary file, preserving the last
    /// valid cache if serialization, sync, or replacement fails.
    pub fn save(&mut self, project_root: &Path) {
        self.enforce_limit();
        let cache_path = project_root.join(CACHE_FILENAME);
        let Ok(json) = serde_json::to_vec(self) else {
            tracing::debug!("cache save skipped: serialization failed");
            return;
        };
        if json.len() > self.max_bytes {
            tracing::debug!(bytes = json.len(), "cache save skipped: size cap exceeded");
            return;
        }
        let Ok(mut temporary) = tempfile::NamedTempFile::new_in(project_root) else {
            tracing::debug!("cache save skipped: temporary file unavailable");
            return;
        };
        if temporary.write_all(&json).is_err() || temporary.as_file_mut().sync_all().is_err() {
            tracing::debug!("cache save skipped: temporary write failed");
            return;
        }
        if let Err(error) = temporary.persist(&cache_path) {
            tracing::debug!(%error, "cache save skipped: atomic replacement failed");
            return;
        }
        tracing::debug!(path = %cache_path.display(), files = self.files.len(), bytes = json.len(), "cache saved");
    }

    pub fn is_fresh_with_hash(&mut self, path: &Path, content: &str) -> (bool, String) {
        let hash = hash_content(content);
        let fresh = self.files.get(path).is_some_and(|entry| entry.hash == hash);
        if fresh {
            let accessed = self.next_access();
            if let Some(entry) = self.files.get_mut(path) {
                entry.last_accessed = accessed;
            }
        }
        (fresh, hash)
    }

    pub fn get_cached_diagnostics(&self, path: &Path) -> Option<&[Diagnostic]> {
        self.files
            .get(path)
            .map(|entry| entry.diagnostics.as_slice())
    }

    pub fn update_with_hash(&mut self, path: &Path, hash: String, diagnostics: Vec<Diagnostic>) {
        let last_accessed = self.next_access();
        let entry = FileEntry {
            hash,
            diagnostics,
            last_accessed,
        };
        if !self.reserve_entry(path, &entry) {
            self.files.remove(path);
            return;
        }
        self.files.insert(path.to_path_buf(), entry);
    }

    const fn next_access(&mut self) -> u64 {
        self.access_clock = self.access_clock.saturating_add(1);
        self.access_clock
    }

    fn enforce_limit(&mut self) {
        let Ok(mut serialized_bytes) = serde_json::to_vec(self).map(|value| value.len()) else {
            return;
        };
        if serialized_bytes <= self.max_bytes || self.files.is_empty() {
            return;
        }
        let mut candidates: Vec<_> = self
            .files
            .iter()
            .filter_map(|(path, entry)| {
                let key_bytes = serde_json::to_vec(path).ok()?.len();
                let value_bytes = serde_json::to_vec(entry).ok()?.len();
                Some((
                    path.clone(),
                    entry.last_accessed,
                    key_bytes.saturating_add(value_bytes).saturating_add(1),
                ))
            })
            .collect();
        candidates.sort_by(
            |(left_path, left_access, _), (right_path, right_access, _)| {
                left_access
                    .cmp(right_access)
                    .then_with(|| left_path.cmp(right_path))
            },
        );
        let mut remaining = self.files.len();
        for (path, _, entry_bytes) in candidates {
            if serialized_bytes <= self.max_bytes {
                break;
            }
            if self.files.remove(&path).is_some() {
                let comma = usize::from(remaining > 1);
                serialized_bytes =
                    serialized_bytes.saturating_sub(entry_bytes.saturating_add(comma));
                remaining = remaining.saturating_sub(1);
            }
        }
    }

    fn reserve_entry(&mut self, path: &Path, entry: &FileEntry) -> bool {
        let Ok(entry_bytes) = serialized_entry_component_bytes(path, entry) else {
            return false;
        };
        let Ok(current_bytes) = serde_json::to_vec(self).map(|value| value.len()) else {
            return false;
        };
        let mut projected = if let Some(existing) = self.files.get(path) {
            let Ok(replaced_bytes) = serialized_entry_component_bytes(path, existing) else {
                return false;
            };
            current_bytes
                .saturating_sub(replaced_bytes)
                .saturating_add(entry_bytes)
        } else {
            current_bytes
                .saturating_add(entry_bytes)
                .saturating_add(usize::from(!self.files.is_empty()))
        };
        if projected <= self.max_bytes {
            return true;
        }
        let mut candidates: Vec<_> = self
            .files
            .iter()
            .filter(|(candidate, _)| candidate.as_path() != path)
            .filter_map(|(candidate, existing)| {
                Some((
                    candidate.clone(),
                    existing.last_accessed,
                    serialized_entry_component_bytes(candidate, existing).ok()?,
                ))
            })
            .collect();
        candidates.sort_by(
            |(left_path, left_access, _), (right_path, right_access, _)| {
                left_access
                    .cmp(right_access)
                    .then_with(|| left_path.cmp(right_path))
            },
        );
        for (candidate, _, bytes) in candidates {
            if projected <= self.max_bytes {
                break;
            }
            if self.files.remove(&candidate).is_some() {
                // The pending entry remains, so each eviction also removes one
                // comma from the serialized map.
                projected = projected.saturating_sub(bytes.saturating_add(1));
            }
        }
        projected <= self.max_bytes
    }

    #[cfg(test)]
    const fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }
}

fn serialized_entry_component_bytes(
    path: &Path,
    entry: &FileEntry,
) -> Result<usize, serde_json::Error> {
    Ok(serde_json::to_vec(path)?
        .len()
        .saturating_add(serde_json::to_vec(entry)?.len())
        .saturating_add(1))
}

const fn default_cache_max_bytes() -> usize {
    DEFAULT_CACHE_MAX_BYTES
}

pub fn hash_content(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

/// Fingerprint every input capable of changing custom-rule output.
pub fn compute_config_hash(
    project_root: &Path,
    ignore_rules: &[String],
    ignore_files: &[String],
    enable_rules: &[String],
    active_rule_fingerprints: &[String],
) -> String {
    let mut digest = Sha256::new();
    update_digest(&mut digest, &CACHE_VERSION.to_le_bytes());
    update_digest(&mut digest, REPORT_SCHEMA_VERSION.as_bytes());
    update_digest(&mut digest, env!("CARGO_PKG_VERSION").as_bytes());
    for source in RULE_IMPLEMENTATION_SOURCES {
        update_digest(&mut digest, source.as_bytes());
    }
    update_digest(&mut digest, rustc_identity().as_bytes());
    update_digest(&mut digest, target_identity(project_root).as_bytes());
    update_list(&mut digest, ignore_rules);
    update_list(&mut digest, ignore_files);
    update_list(&mut digest, enable_rules);
    update_list(&mut digest, active_rule_fingerprints);
    format!("{:x}", digest.finalize())
}

fn target_identity(project_root: &Path) -> String {
    let host = rustc_identity()
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap_or("unknown-host");
    let target = std::env::var("CARGO_BUILD_TARGET").unwrap_or_else(|_| host.to_string());
    let mut digest = Sha256::new();
    update_digest(&mut digest, target.as_bytes());

    let mut cargo_configs = Vec::new();
    for ancestor in project_root.ancestors() {
        for name in ["config.toml", "config"] {
            let candidate = ancestor.join(".cargo").join(name);
            if candidate.is_file() {
                cargo_configs.push(candidate);
            }
        }
    }
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")));
    if let Some(cargo_home) = cargo_home {
        for name in ["config.toml", "config"] {
            let candidate = cargo_home.join(name);
            if candidate.is_file() {
                cargo_configs.push(candidate);
            }
        }
    }
    cargo_configs.sort();
    cargo_configs.dedup();
    for config in cargo_configs {
        if std::fs::metadata(&config).is_ok_and(|metadata| metadata.len() <= 1024 * 1024)
            && let Ok(content) = std::fs::read(&config)
        {
            update_digest(&mut digest, &content);
        }
    }
    format!("{:x}", digest.finalize())
}

fn update_list(digest: &mut Sha256, values: &[String]) {
    update_digest(digest, &values.len().to_le_bytes());
    for value in values {
        update_digest(digest, value.as_bytes());
    }
}

fn update_digest(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_le_bytes());
    digest.update(value);
}

fn rustc_identity() -> &'static str {
    static IDENTITY: LazyLock<String> = LazyLock::new(|| {
        std::process::Command::new("rustc")
            .arg("-Vv")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .unwrap_or_else(|| {
                format!(
                    "rustc-unavailable:{}-{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )
            })
    });
    IDENTITY.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Category, Severity};

    fn sample_diagnostic(file: &str, rule: &str, message_size: usize) -> Diagnostic {
        Diagnostic {
            file_path: PathBuf::from(file),
            rule: rule.to_string(),
            category: Category::ErrorHandling,
            severity: Severity::Warning,
            message: "x".repeat(message_size),
            help: None,
            line: Some(1),
            column: None,
            fix: None,
        }
    }

    fn fingerprint() -> String {
        compute_config_hash(Path::new("."), &[], &[], &[], &[])
    }

    #[test]
    fn atomic_roundtrip_preserves_fresh_entries() {
        let directory = tempfile::tempdir().unwrap();
        let fingerprint = fingerprint();
        let mut cache = ScanCache::new(fingerprint.clone());
        cache.update_with_hash(
            Path::new("src/main.rs"),
            hash_content("fn main() {}"),
            vec![sample_diagnostic("src/main.rs", "unwrap-in-production", 8)],
        );
        cache.save(directory.path());

        let mut loaded = ScanCache::load(directory.path(), &fingerprint).unwrap();
        assert!(
            loaded
                .is_fresh_with_hash(Path::new("src/main.rs"), "fn main() {}")
                .0
        );
        assert_eq!(
            loaded
                .get_cached_diagnostics(Path::new("src/main.rs"))
                .unwrap()
                .len(),
            1
        );
        let leftovers: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name() != CACHE_FILENAME)
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn content_config_rule_toolchain_and_schema_inputs_are_stable() {
        assert_eq!(hash_content("same"), hash_content("same"));
        assert_ne!(hash_content("same"), hash_content("different"));
        assert_eq!(hash_content("same").len(), 64);
        let base = compute_config_hash(Path::new("."), &[], &[], &[], &["rule:a".to_string()]);
        let changed_rule =
            compute_config_hash(Path::new("."), &[], &[], &[], &["rule:b".to_string()]);
        let changed_config = compute_config_hash(
            Path::new("."),
            &["off".to_string()],
            &[],
            &[],
            &["rule:a".to_string()],
        );
        assert_ne!(base, changed_rule);
        assert_ne!(base, changed_config);
        assert_eq!(base.len(), 64);
        assert!(rustc_identity().contains("host:") || rustc_identity().contains("unavailable"));
    }

    #[test]
    fn cargo_target_configuration_changes_the_fingerprint() {
        let directory = tempfile::tempdir().unwrap();
        let cargo = directory.path().join(".cargo");
        std::fs::create_dir_all(&cargo).unwrap();
        let config = cargo.join("config.toml");
        std::fs::write(&config, "[build]\ntarget = \"x86_64-unknown-linux-gnu\"\n").unwrap();
        let before = compute_config_hash(directory.path(), &[], &[], &[], &[]);
        std::fs::write(&config, "[build]\ntarget = \"aarch64-unknown-linux-gnu\"\n").unwrap();
        let after = compute_config_hash(directory.path(), &[], &[], &[], &[]);
        assert_ne!(before, after);
    }

    #[test]
    fn corrupt_future_and_wrong_fingerprint_caches_recompute() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CACHE_FILENAME);
        std::fs::write(&path, "not json").unwrap();
        assert!(ScanCache::load(directory.path(), "fingerprint").is_none());

        std::fs::write(
            &path,
            serde_json::json!({
                "version": CACHE_VERSION + 1,
                "config_hash": "fingerprint",
                "access_clock": 0,
                "files": {}
            })
            .to_string(),
        )
        .unwrap();
        assert!(ScanCache::load(directory.path(), "fingerprint").is_none());

        let mut cache = ScanCache::new("old".to_string());
        cache.save(directory.path());
        assert!(ScanCache::load(directory.path(), "new").is_none());
    }

    #[test]
    fn lru_eviction_keeps_recent_entries_under_the_cap() {
        let directory = tempfile::tempdir().unwrap();
        let mut cache = ScanCache::new(fingerprint()).with_max_bytes(1_200);
        cache.update_with_hash(
            Path::new("old.rs"),
            hash_content("old"),
            vec![sample_diagnostic("old.rs", "old", 500)],
        );
        cache.update_with_hash(
            Path::new("recent.rs"),
            hash_content("recent"),
            vec![sample_diagnostic("recent.rs", "recent", 500)],
        );
        assert!(serde_json::to_vec(&cache).unwrap().len() <= 1_200);
        assert!(cache.is_fresh_with_hash(Path::new("recent.rs"), "recent").0);
        cache.save(directory.path());
        let bytes = std::fs::read(directory.path().join(CACHE_FILENAME)).unwrap();
        assert!(bytes.len() <= 1_200);
        let persisted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(persisted["files"].get("recent.rs").is_some());
        assert!(persisted["files"].get("old.rs").is_none());

        let mut cycling = ScanCache::new(fingerprint()).with_max_bytes(1_200);
        for index in 0..20 {
            let path = PathBuf::from(format!("replacement-{index}.rs"));
            cycling.update_with_hash(
                &path,
                hash_content("replacement"),
                vec![sample_diagnostic(
                    path.to_str().unwrap(),
                    "replacement",
                    300,
                )],
            );
            assert!(serde_json::to_vec(&cycling).unwrap().len() <= 1_200);
        }
        assert!(
            cycling
                .is_fresh_with_hash(Path::new("replacement-19.rs"), "replacement")
                .0
        );
    }

    #[test]
    fn modified_content_is_never_returned_as_a_hit() {
        let mut cache = ScanCache::new(fingerprint());
        cache.update_with_hash(Path::new("src/lib.rs"), hash_content("old"), Vec::new());
        assert!(!cache.is_fresh_with_hash(Path::new("src/lib.rs"), "new").0);
    }
}
