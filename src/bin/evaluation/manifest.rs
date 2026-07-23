use super::model::{
    CORPUS_SCHEMA_VERSION, CorpusManifest, EvaluationProfile, PreparedCorpus, RepositorySpec,
};
use super::{EvalError, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Component, Path};

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).map_err(|error| EvalError::io("cannot read", path, error))?;
    serde_json::from_slice(&bytes).map_err(|source| EvalError::Json {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| EvalError::io("cannot create output directory", parent, error))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| EvalError::io("cannot create temporary output", parent, error))?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), value).map_err(|source| {
        EvalError::Command(format!("cannot serialize '{}': {source}", path.display()))
    })?;
    temporary
        .as_file_mut()
        .write_all(b"\n")
        .map_err(|error| EvalError::io("cannot finish temporary output", path, error))?;
    temporary
        .as_file_mut()
        .sync_all()
        .map_err(|error| EvalError::io("cannot sync temporary output", path, error))?;
    temporary
        .persist(path)
        .map_err(|error| EvalError::io("cannot persist output", path, error.error))?;
    Ok(())
}

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|error| EvalError::io("cannot hash", path, error))?;
    Ok(hex_digest(&bytes))
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn evaluation_profile_sha256(profile: &EvaluationProfile) -> Result<String> {
    let bytes = serde_json::to_vec(profile).map_err(|error| {
        EvalError::InvalidManifest(format!("evaluation profile cannot be serialized: {error}"))
    })?;
    Ok(hex_digest(&bytes))
}

pub(crate) fn validate_corpus_manifest(manifest: &CorpusManifest) -> Result<()> {
    if manifest.schema_version != CORPUS_SCHEMA_VERSION {
        return Err(EvalError::InvalidManifest(format!(
            "corpus schema must be {CORPUS_SCHEMA_VERSION}, got {}",
            manifest.schema_version
        )));
    }
    if manifest.repositories.len() < 100 {
        return Err(EvalError::InvalidManifest(format!(
            "corpus must contain at least 100 repositories, got {}",
            manifest.repositories.len()
        )));
    }
    validate_evaluation_profile(&manifest.evaluation_profile)?;
    let mut names = HashSet::new();
    let mut urls = HashSet::new();
    let mut total_roots = 0usize;
    for repository in &manifest.repositories {
        validate_repository(repository)?;
        if !names.insert(repository.name.as_str()) {
            return Err(EvalError::InvalidManifest(format!(
                "duplicate repository name {}",
                repository.name
            )));
        }
        if !urls.insert(repository.url.as_str()) {
            return Err(EvalError::InvalidManifest(format!(
                "duplicate repository URL {}",
                repository.url
            )));
        }
        total_roots = total_roots.saturating_add(repository.minimum_project_roots);
    }
    if total_roots < 250 {
        return Err(EvalError::InvalidManifest(format!(
            "corpus must declare at least 250 Cargo project roots, got {total_roots}"
        )));
    }
    Ok(())
}

fn validate_evaluation_profile(profile: &EvaluationProfile) -> Result<()> {
    let required_adapters = ["clippy", "custom-rules", "dependencies"];
    if profile.version != "1.1"
        || profile.normalized_severity != "warning"
        || !profile.force_candidate_rules
        || profile.respect_inline_suppressions
        || profile.respect_project_config
        || !profile.offline
        || required_adapters
            .iter()
            .any(|adapter| !profile.adapter_policy.contains_key(*adapter))
    {
        return Err(EvalError::InvalidManifest(
            "evaluation profile must force all candidate rules at warning severity, disable project and inline suppression policy, run offline, and pin every adapter policy"
                .to_string(),
        ));
    }
    let expected = [
        ("clippy", "excluded-environment-dependent"),
        ("custom-rules", "required-all-candidates"),
        ("dependencies", "excluded-environment-dependent"),
    ];
    if profile.adapter_policy.len() != expected.len()
        || expected.iter().any(|(adapter, policy)| {
            profile.adapter_policy.get(*adapter).map(String::as_str) != Some(*policy)
        })
    {
        return Err(EvalError::InvalidManifest(
            "evaluation adapter policy must require all custom rules and exclude environment-dependent adapters"
                .to_string(),
        ));
    }
    if profile
        .adapter_policy
        .values()
        .any(|policy| policy.trim().is_empty())
    {
        return Err(EvalError::InvalidManifest(
            "evaluation adapter policies cannot be empty".to_string(),
        ));
    }
    Ok(())
}

#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "canonical corpus URLs deliberately require the lowercase .git suffix"
)]
fn validate_repository(repository: &RepositorySpec) -> Result<()> {
    if repository.name.is_empty()
        || !repository
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(EvalError::InvalidManifest(format!(
            "unsafe repository name {:?}",
            repository.name
        )));
    }
    if !repository.url.starts_with("https://github.com/") || !repository.url.ends_with(".git") {
        return Err(EvalError::InvalidManifest(format!(
            "repository {} must use a canonical GitHub HTTPS .git URL",
            repository.name
        )));
    }
    if repository.commit.len() != 40
        || !repository
            .commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(EvalError::InvalidManifest(format!(
            "repository {} is not pinned to a full 40-character commit",
            repository.name
        )));
    }
    if repository.minimum_project_roots == 0 {
        return Err(EvalError::InvalidManifest(format!(
            "repository {} must declare at least one project root",
            repository.name
        )));
    }
    Ok(())
}

pub(crate) fn validate_prepared(
    prepared: &PreparedCorpus,
    manifest: &CorpusManifest,
    manifest_hash: &str,
) -> Result<()> {
    if prepared.schema_version != CORPUS_SCHEMA_VERSION {
        return Err(EvalError::InvalidManifest(format!(
            "prepared corpus schema must be {CORPUS_SCHEMA_VERSION}"
        )));
    }
    if prepared.manifest_sha256 != manifest_hash {
        return Err(EvalError::InvalidManifest(
            "prepared corpus does not match the pinned manifest hash".to_string(),
        ));
    }
    if prepared.repositories.len() != manifest.repositories.len() {
        return Err(EvalError::InvalidManifest(
            "prepared corpus repository count differs from manifest".to_string(),
        ));
    }
    let expected: std::collections::HashMap<_, _> = manifest
        .repositories
        .iter()
        .map(|repository| (repository.name.as_str(), repository))
        .collect();
    let mut prepared_names = HashSet::new();
    for repository in &prepared.repositories {
        if !prepared_names.insert(repository.name.as_str()) {
            return Err(EvalError::InvalidManifest(format!(
                "duplicate prepared repository {}",
                repository.name
            )));
        }
        let spec = expected.get(repository.name.as_str()).ok_or_else(|| {
            EvalError::InvalidManifest(format!(
                "prepared repository {} is absent from manifest",
                repository.name
            ))
        })?;
        if repository.commit != spec.commit {
            return Err(EvalError::InvalidManifest(format!(
                "prepared repository {} has commit {}, expected {}",
                repository.name, repository.commit, spec.commit
            )));
        }
        if repository.checkout_dir != spec.name {
            return Err(EvalError::InvalidManifest(format!(
                "prepared repository {} has an unsafe checkout directory",
                repository.name
            )));
        }
        let mut roots = HashSet::new();
        for root in &repository.project_roots {
            if !roots.insert(root.as_str()) || !safe_project_root(root) {
                return Err(EvalError::InvalidManifest(format!(
                    "prepared repository {} has an invalid or duplicate project root",
                    repository.name
                )));
            }
        }
        if repository.project_roots.len() < spec.minimum_project_roots {
            return Err(EvalError::InvalidManifest(format!(
                "prepared repository {} has {} roots, expected at least {}",
                repository.name,
                repository.project_roots.len(),
                spec.minimum_project_roots
            )));
        }
        if repository.tree_digest.is_empty()
            || repository
                .submodule_status
                .iter()
                .any(|status| status.trim().is_empty())
        {
            return Err(EvalError::InvalidManifest(format!(
                "prepared repository {} is missing tree or submodule provenance",
                repository.name
            )));
        }
    }
    Ok(())
}

fn safe_project_root(root: &str) -> bool {
    root == "."
        || (!root.is_empty()
            && !root.contains('\\')
            && Path::new(root)
                .components()
                .all(|component| matches!(component, Component::Normal(_))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_corpus_has_verified_pin_and_root_cardinality() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluation/corpus-v1.json");
        let manifest: CorpusManifest = read_json(&path).unwrap();
        validate_corpus_manifest(&manifest).unwrap();
        assert_eq!(manifest.repositories.len(), 100);
        assert!(
            manifest
                .repositories
                .iter()
                .map(|repository| repository.minimum_project_roots)
                .sum::<usize>()
                >= 250
        );
        assert!(manifest.repositories.iter().all(|repository| {
            repository.commit.len() == 40
                && repository
                    .commit
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
        }));
    }

    #[test]
    fn prepared_roots_cannot_escape_the_checkout() {
        assert!(safe_project_root("."));
        assert!(safe_project_root("crates/member"));
        assert!(!safe_project_root("../outside"));
        assert!(!safe_project_root("/absolute"));
        assert!(!safe_project_root("crates\\member"));
    }
}
