use super::manifest::{
    evaluation_profile_sha256, hex_digest, read_json, sha256_file, validate_corpus_manifest,
    validate_prepared, write_json_atomic,
};
use super::model::{
    CORPUS_SCHEMA_VERSION, CorpusManifest, CorpusRecord, EvaluationDiagnostic, EvaluationRule,
    FailureEvent, PreparedCorpus, PreparedRepository, RepositorySpec, SeverityCounts,
};
use super::process::{ResourceLimits, run_capped, run_capped_with_limits};
use super::{EvalError, Result, sandbox};
use crate::{CorpusArgs, PrepareArgs};
use rust_doctor::diagnostics::{CompletenessState, ReportV1};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const COMMAND_OUTPUT_CAP: usize = 4 * 1024 * 1024;

#[derive(Clone)]
struct RepositoryJob {
    spec: RepositorySpec,
    prepared: PreparedRepository,
}

struct RunContext<'a> {
    checkout_root: &'a Path,
    binary: &'a Path,
    cargo_home: &'a Path,
    tool_revision: &'a str,
    evaluation_profile_sha256: &'a str,
    catalog_sha256: &'a str,
    catalog: &'a BTreeMap<String, EvaluationRule>,
    repository_timeout: Duration,
    deadline: Instant,
    output_cap: usize,
    resource_limits: ResourceLimits,
    scratch_bytes: usize,
}

struct RootScan {
    selected_root: String,
    report: ReportV1,
    project_roots: Vec<String>,
    elapsed_ms: u64,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the command handler owns its parsed Clap subcommand"
)]
pub(crate) fn prepare(args: PrepareArgs) -> Result<()> {
    let manifest: CorpusManifest = read_json(&args.manifest)?;
    validate_corpus_manifest(&manifest)?;
    let manifest_hash = sha256_file(&args.manifest)?;
    std::fs::create_dir_all(&args.checkout_root).map_err(|error| {
        EvalError::io(
            "cannot create corpus checkout root",
            &args.checkout_root,
            error,
        )
    })?;
    sandbox::initialize_evaluation_cargo_home(&args.checkout_root)?;
    let timeout = Duration::from_secs(args.repository_timeout_secs.max(1));
    let mut repositories = Vec::with_capacity(manifest.repositories.len());

    for repository in &manifest.repositories {
        let checkout = args.checkout_root.join(&repository.name);
        prepare_repository(repository, &checkout, timeout)?;
        let project_roots = discover_project_roots(&checkout)?;
        if project_roots.len() < repository.minimum_project_roots {
            return Err(EvalError::InvalidManifest(format!(
                "{} exposes {} Cargo roots at {}, expected at least {}",
                repository.name,
                project_roots.len(),
                repository.commit,
                repository.minimum_project_roots
            )));
        }
        repositories.push(PreparedRepository {
            name: repository.name.clone(),
            commit: repository.commit.clone(),
            checkout_dir: repository.name.clone(),
            project_roots,
            tree_digest: git_output(&checkout, &["rev-parse", "HEAD^{tree}"], timeout)?
                .trim()
                .to_string(),
            submodule_status: submodule_status(&checkout, timeout)?,
        });
    }

    let total_roots: usize = repositories
        .iter()
        .map(|repository| repository.project_roots.len())
        .sum();
    if total_roots < 250 {
        return Err(EvalError::InvalidManifest(format!(
            "prepared corpus contains {total_roots} Cargo roots, expected at least 250"
        )));
    }
    let prepared = PreparedCorpus {
        schema_version: CORPUS_SCHEMA_VERSION.to_string(),
        manifest_sha256: manifest_hash,
        repositories,
    };
    write_json_atomic(&args.prepared_out, &prepared)
}

#[expect(
    clippy::too_many_lines,
    reason = "corpus validation, bounded scheduling and atomic gate output form one transaction"
)]
pub(crate) fn run(args: CorpusArgs) -> Result<()> {
    let manifest: CorpusManifest = read_json(&args.manifest)?;
    validate_corpus_manifest(&manifest)?;
    let manifest_hash = sha256_file(&args.manifest)?;
    let prepared: PreparedCorpus = read_json(&args.prepared)?;
    validate_prepared(&prepared, &manifest, &manifest_hash)?;
    let binary = args.binary.canonicalize().map_err(|error| {
        EvalError::io(
            "cannot canonicalize rust-doctor binary",
            &args.binary,
            error,
        )
    })?;
    let checkout_root = args.checkout_root.canonicalize().map_err(|error| {
        EvalError::io(
            "cannot canonicalize corpus checkout root",
            &args.checkout_root,
            error,
        )
    })?;
    let cargo_home = sandbox::validate_evaluation_cargo_home(&checkout_root)?;
    let tool_revision = args
        .tool_revision
        .map_or_else(|| binary_revision(&binary), Ok)?;
    if tool_revision.trim().is_empty() {
        return Err(EvalError::InvalidManifest(
            "tool revision cannot be empty".to_string(),
        ));
    }
    let evaluation_profile_sha256 = evaluation_profile_sha256(&manifest.evaluation_profile)?;
    let catalog = read_evaluation_catalog(&binary, &checkout_root)?;
    let catalog_bytes = serde_json::to_vec(&catalog).map_err(|error| {
        EvalError::Command(format!("cannot fingerprint evaluation catalog: {error}"))
    })?;
    let catalog_sha256 = hex_digest(&catalog_bytes);
    let available = thread::available_parallelism().map_or(1, usize::from);
    let concurrency = args.concurrency.unwrap_or_else(|| available.min(8));
    if concurrency == 0 {
        return Err(EvalError::InvalidManifest(
            "corpus concurrency must be greater than zero".to_string(),
        ));
    }
    let output_cap = args
        .output_limit_mib
        .checked_mul(1024 * 1024)
        .ok_or_else(|| EvalError::InvalidManifest("output cap is too large".to_string()))?;
    if output_cap == 0 {
        return Err(EvalError::InvalidManifest(
            "output cap must be greater than zero".to_string(),
        ));
    }
    let memory_bytes = mib_to_bytes(args.sandbox_memory_mib, "sandbox memory")?;
    let scratch_bytes = mib_to_bytes(args.sandbox_scratch_mib, "sandbox scratch")?;
    if args.sandbox_process_limit == 0 {
        return Err(EvalError::InvalidManifest(
            "sandbox process limit must be greater than zero".to_string(),
        ));
    }
    let prepared_by_name: HashMap<_, _> = prepared
        .repositories
        .into_iter()
        .map(|repository| (repository.name.clone(), repository))
        .collect();
    let jobs: Vec<_> = manifest
        .repositories
        .into_iter()
        .map(|spec| {
            let prepared = prepared_by_name.get(&spec.name).cloned().ok_or_else(|| {
                EvalError::InvalidManifest(format!("{} was not prepared", spec.name))
            })?;
            Ok(RepositoryJob { spec, prepared })
        })
        .collect::<Result<_>>()?;
    let context = RunContext {
        checkout_root: &checkout_root,
        binary: &binary,
        cargo_home: &cargo_home,
        tool_revision: &tool_revision,
        evaluation_profile_sha256: &evaluation_profile_sha256,
        catalog_sha256: &catalog_sha256,
        catalog: &catalog,
        repository_timeout: Duration::from_secs(args.repository_timeout_secs.max(1)),
        deadline: Instant::now()
            .checked_add(Duration::from_secs(args.global_timeout_secs.max(1)))
            .ok_or_else(|| EvalError::InvalidManifest("global timeout is too large".to_string()))?,
        output_cap,
        resource_limits: ResourceLimits {
            max_resident_bytes: u64::try_from(memory_bytes).unwrap_or(u64::MAX),
            max_processes: args.sandbox_process_limit,
        },
        scratch_bytes,
    };

    let mut records = run_wave(jobs.clone(), concurrency, 1, &context);
    for attempt in 2..=3 {
        let retry_jobs: Vec<_> = jobs
            .iter()
            .filter(|job| {
                records
                    .iter()
                    .find(|record| record.repository == job.spec.name)
                    .is_some_and(|record| !record.complete)
            })
            .cloned()
            .collect();
        if retry_jobs.is_empty() || Instant::now() >= context.deadline {
            break;
        }
        for mut retried in run_wave(retry_jobs, 1, attempt, &context) {
            if let Some(previous) = records
                .iter_mut()
                .find(|record| record.repository == retried.repository)
            {
                let mut failures = std::mem::take(&mut previous.failure_chain);
                failures.append(&mut retried.failure_chain);
                retried.failure_chain = failures;
                *previous = retried;
            }
        }
    }
    records.sort_by(|left, right| left.repository.cmp(&right.repository));
    write_ndjson_atomic(&args.output, &records)?;
    let incomplete: Vec<_> = records
        .iter()
        .filter(|record| !record.complete)
        .map(|record| record.repository.as_str())
        .collect();
    if incomplete.is_empty() {
        Ok(())
    } else {
        Err(EvalError::GateFailed(format!(
            "{} corpus repositories remain incomplete after bounded retries: {}",
            incomplete.len(),
            incomplete.join(", ")
        )))
    }
}

fn prepare_repository(spec: &RepositorySpec, checkout: &Path, timeout: Duration) -> Result<()> {
    if checkout.exists() {
        if !checkout.join(".git").is_dir() {
            return Err(EvalError::Command(format!(
                "existing checkout path '{}' is not a Git repository",
                checkout.display()
            )));
        }
        let status = git_output(checkout, &["status", "--porcelain"], timeout)?;
        if !status.trim().is_empty() {
            return Err(EvalError::Command(format!(
                "prepared checkout '{}' has local changes; refusing to overwrite them",
                spec.name
            )));
        }
        let remote = git_output(checkout, &["remote", "get-url", "origin"], timeout)?;
        if remote.trim() != spec.url {
            return Err(EvalError::Command(format!(
                "prepared checkout '{}' has an unexpected origin",
                spec.name
            )));
        }
    } else {
        let parent = checkout.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|error| EvalError::io("cannot create checkout parent", parent, error))?;
        run_git(
            parent,
            &[
                "clone",
                "--quiet",
                "--filter=blob:none",
                "--no-checkout",
                &spec.url,
                checkout.to_string_lossy().as_ref(),
            ],
            timeout,
        )?;
    }
    run_git(
        checkout,
        &["fetch", "--quiet", "--depth", "1", "origin", &spec.commit],
        timeout,
    )?;
    run_git(
        checkout,
        &["checkout", "--quiet", "--detach", &spec.commit],
        timeout,
    )?;
    validate_local_git_config(checkout, timeout)?;
    update_submodules_securely(checkout, checkout, timeout, 0, &mut HashSet::new())?;
    let actual = git_output(checkout, &["rev-parse", "HEAD"], timeout)?;
    if actual.trim() != spec.commit {
        return Err(EvalError::Command(format!(
            "checkout {} resolved {}, expected {}",
            spec.name,
            actual.trim(),
            spec.commit
        )));
    }
    verify_clean_checkout(checkout, timeout)?;
    Ok(())
}

fn verify_clean_checkout(checkout: &Path, timeout: Duration) -> Result<()> {
    let status = git_output(
        checkout,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        timeout,
    )?;
    if !status.trim().is_empty() {
        return Err(EvalError::Command(format!(
            "prepared checkout '{}' has a dirty index or worktree",
            checkout.display()
        )));
    }
    let submodules = submodule_status(checkout, timeout)?;
    if submodules
        .iter()
        .any(|status| matches!(status.as_bytes().first(), Some(b'-' | b'+' | b'U')))
    {
        return Err(EvalError::Command(format!(
            "prepared checkout '{}' has an uninitialized or divergent submodule",
            checkout.display()
        )));
    }
    Ok(())
}

fn submodule_status(checkout: &Path, timeout: Duration) -> Result<Vec<String>> {
    let output = git_output(checkout, &["submodule", "status", "--recursive"], timeout)?;
    Ok(output
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn validate_local_git_config(checkout: &Path, timeout: Duration) -> Result<()> {
    let forbidden = git_config_output(
        checkout,
        None,
        &[
            "--get-regexp",
            "^(url\\..*\\.insteadOf|credential\\.|http\\..*\\.extraHeader|core\\.sshCommand)",
        ],
        timeout,
    )?;
    if !forbidden.trim().is_empty() {
        return Err(EvalError::Command(format!(
            "prepared checkout '{}' has credential-bearing or URL-rewriting local Git configuration",
            checkout.display()
        )));
    }
    let submodule_urls = git_config_output(
        checkout,
        None,
        &["--get-regexp", "^submodule\\..*\\.url$"],
        timeout,
    )?;
    if submodule_urls.lines().any(|line| {
        line.split_once(char::is_whitespace)
            .is_none_or(|(_, url)| !canonical_github_url(url.trim()))
    }) {
        return Err(EvalError::Command(format!(
            "prepared checkout '{}' has a non-canonical local submodule URL",
            checkout.display()
        )));
    }
    let submodule_updates = git_config_output(
        checkout,
        None,
        &["--get-regexp", "^submodule\\..*\\.update$"],
        timeout,
    )?;
    if submodule_updates.lines().any(|line| {
        line.split_once(char::is_whitespace)
            .is_some_and(|(_, update)| update.trim().starts_with('!'))
    }) {
        return Err(EvalError::Command(format!(
            "prepared checkout '{}' has an executable local submodule update",
            checkout.display()
        )));
    }
    Ok(())
}

fn update_submodules_securely(
    repository: &Path,
    checkout: &Path,
    timeout: Duration,
    depth: usize,
    visited: &mut HashSet<std::path::PathBuf>,
) -> Result<()> {
    const MAX_SUBMODULE_DEPTH: usize = 8;
    if depth > MAX_SUBMODULE_DEPTH {
        return Err(EvalError::Command(format!(
            "submodule nesting exceeds {MAX_SUBMODULE_DEPTH} levels"
        )));
    }
    let modules = repository.join(".gitmodules");
    if !modules.exists() {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(&modules)
        .map_err(|error| EvalError::io("cannot inspect .gitmodules", &modules, error))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1024 * 1024 {
        return Err(EvalError::Command(format!(
            "repository '{}' has an unsafe .gitmodules file",
            repository.display()
        )));
    }
    let url_keys = git_config_keys(repository, &modules, "^submodule\\..*\\.url$", timeout)?;
    let path_keys = git_config_keys(repository, &modules, "^submodule\\..*\\.path$", timeout)?;
    if url_keys.is_empty() || url_keys.len() != path_keys.len() {
        return Err(EvalError::Command(format!(
            "repository '{}' has incomplete submodule URL/path declarations",
            repository.display()
        )));
    }
    for key in git_config_keys(repository, &modules, "^submodule\\..*\\.update$", timeout)? {
        let update = git_config_value(repository, &modules, &key, timeout)?;
        if update.starts_with('!') {
            return Err(EvalError::Command(format!(
                "repository '{}' declares an executable submodule update",
                repository.display()
            )));
        }
    }
    let mut paths = Vec::with_capacity(url_keys.len());
    for url_key in url_keys {
        let url = git_config_value(repository, &modules, &url_key, timeout)?;
        if !canonical_github_url(&url) {
            return Err(EvalError::Command(format!(
                "repository '{}' declares a non-canonical submodule URL",
                repository.display()
            )));
        }
        let path_key = format!("{}.path", url_key.trim_end_matches(".url"));
        let path = git_config_value(repository, &modules, &path_key, timeout)?;
        let relative = Path::new(&path);
        if path.is_empty()
            || path.contains('\\')
            || !relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
        {
            return Err(EvalError::Command(format!(
                "repository '{}' declares an unsafe submodule path",
                repository.display()
            )));
        }
        paths.push(relative.to_path_buf());
    }
    run_git(
        repository,
        &[
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "credential.helper=",
            "submodule",
            "update",
            "--init",
            "--depth",
            "1",
            "--jobs",
            "1",
        ],
        timeout,
    )?;
    for relative in paths {
        let nested = repository.join(relative).canonicalize().map_err(|error| {
            EvalError::io("cannot canonicalize prepared submodule", repository, error)
        })?;
        if !nested.starts_with(checkout) || !visited.insert(nested.clone()) {
            return Err(EvalError::Command(format!(
                "repository '{}' has an escaping or repeated submodule path",
                repository.display()
            )));
        }
        validate_local_git_config(&nested, timeout)?;
        update_submodules_securely(&nested, checkout, timeout, depth + 1, visited)?;
    }
    Ok(())
}

#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "canonical GitHub repository URLs deliberately require the lowercase .git suffix"
)]
fn canonical_github_url(url: &str) -> bool {
    let Some(path) = url.strip_prefix("https://github.com/") else {
        return false;
    };
    let mut segments = path.split('/');
    let owner = segments.next().unwrap_or_default();
    let repository = segments.next().unwrap_or_default();
    !owner.is_empty()
        && repository.ends_with(".git")
        && repository.len() > ".git".len()
        && segments.next().is_none()
        && !url.contains(['@', '?', '#', '\\'])
}

fn git_config_keys(
    repository: &Path,
    modules: &Path,
    pattern: &str,
    timeout: Duration,
) -> Result<Vec<String>> {
    Ok(git_config_output(
        repository,
        Some(modules),
        &["--name-only", "--get-regexp", pattern],
        timeout,
    )?
    .lines()
    .map(str::trim)
    .filter(|key| !key.is_empty())
    .map(str::to_string)
    .collect())
}

fn git_config_value(
    repository: &Path,
    modules: &Path,
    key: &str,
    timeout: Duration,
) -> Result<String> {
    let value = git_config_output(repository, Some(modules), &["--get", key], timeout)?;
    let value = value.trim();
    if value.is_empty() {
        return Err(EvalError::Command(format!(
            "repository '{}' has an empty submodule declaration",
            repository.display()
        )));
    }
    Ok(value.to_string())
}

fn git_config_output(
    directory: &Path,
    file: Option<&Path>,
    arguments: &[&str],
    timeout: Duration,
) -> Result<String> {
    let mut command = Command::new("git");
    command.current_dir(directory).arg("config");
    if let Some(file) = file {
        command.arg("--file").arg(file);
    } else {
        command.arg("--local");
    }
    command.args(arguments);
    harden_git_environment(&mut command);
    let output = run_capped(command, timeout, COMMAND_OUTPUT_CAP)?;
    if output.timed_out || (!output.status.success() && output.status.code() != Some(1)) {
        return Err(EvalError::Command(format!(
            "cannot inspect Git configuration in '{}': {}",
            directory.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_git(directory: &Path, arguments: &[&str], timeout: Duration) -> Result<()> {
    let mut command = Command::new("git");
    command.current_dir(directory).args(arguments);
    harden_git_environment(&mut command);
    let output = run_capped(command, timeout, COMMAND_OUTPUT_CAP)?;
    if output.timed_out {
        return Err(EvalError::Command(format!(
            "git command timed out in '{}'",
            directory.display()
        )));
    }
    if !output.status.success() {
        return Err(EvalError::Command(format!(
            "git command failed in '{}': {}",
            directory.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn git_output(directory: &Path, arguments: &[&str], timeout: Duration) -> Result<String> {
    let mut command = Command::new("git");
    command.current_dir(directory).args(arguments);
    harden_git_environment(&mut command);
    let output = run_capped(command, timeout, COMMAND_OUTPUT_CAP)?;
    if output.timed_out || !output.status.success() {
        return Err(EvalError::Command(format!(
            "git command failed in '{}': {}",
            directory.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn harden_git_environment(command: &mut Command) {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("HOME")
        .env_remove("SSH_AUTH_SOCK")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .env_remove("GIT_SSH")
        .env_remove("GIT_SSH_COMMAND")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy");
}

fn discover_project_roots(checkout: &Path) -> Result<Vec<String>> {
    let mut roots = Vec::new();
    let mut pending = vec![checkout.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| EvalError::io("cannot inspect prepared checkout", &directory, error))?
        {
            let entry = entry.map_err(|error| {
                EvalError::io("cannot inspect prepared checkout entry", &directory, error)
            })?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| EvalError::io("cannot inspect prepared path", &path, error))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                let name = entry.file_name();
                if !matches!(
                    name.to_str(),
                    Some(".git" | "target" | "vendor" | "node_modules")
                ) {
                    pending.push(path);
                }
                continue;
            }
            if entry.file_name() != "Cargo.toml" || metadata.len() > 1024 * 1024 {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .map_err(|error| EvalError::io("cannot read Cargo manifest", &path, error))?;
            let parsed: toml::Value = match toml::from_str(&content) {
                Ok(value) => value,
                Err(_) => continue,
            };
            // Report V1 emits package roots. A virtual workspace manifest is the
            // owner used to reach its members, not a separately reported root.
            if parsed.get("package").is_none() {
                continue;
            }
            let root = path.parent().unwrap_or(checkout);
            let relative = root.strip_prefix(checkout).unwrap_or(root);
            let normalized = if relative.as_os_str().is_empty() {
                ".".to_string()
            } else {
                relative.to_string_lossy().replace('\\', "/")
            };
            roots.push(normalized);
        }
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn binary_revision(binary: &Path) -> Result<String> {
    let mut command = Command::new(binary);
    command.arg("--version");
    let output = run_capped(command, Duration::from_secs(10), 64 * 1024)?;
    if !output.status.success() || output.timed_out {
        return Err(EvalError::Command(format!(
            "cannot read version from '{}': {}",
            binary.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn read_evaluation_catalog(
    binary: &Path,
    working_directory: &Path,
) -> Result<BTreeMap<String, EvaluationRule>> {
    let mut command = Command::new(binary);
    command
        .args(["rules", "list"])
        .arg(working_directory)
        .arg("--json");
    let output = run_capped(command, Duration::from_secs(30), COMMAND_OUTPUT_CAP)?;
    if output.timed_out || output.output_overflow || !output.status.success() {
        return Err(EvalError::Command(format!(
            "cannot read candidate rule catalog: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let entries: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).map_err(|error| {
            EvalError::Command(format!("candidate rule catalog is invalid JSON: {error}"))
        })?;
    let mut catalog = BTreeMap::new();
    for entry in entries {
        let id = entry["canonical_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                EvalError::Command("candidate rule catalog contains an invalid ID".to_string())
            })?;
        let rule = EvaluationRule {
            category: entry["category"]
                .as_str()
                .ok_or_else(|| EvalError::Command(format!("catalog rule {id} has no category")))?
                .to_string(),
            default_severity: entry["default_severity"]
                .as_str()
                .ok_or_else(|| {
                    EvalError::Command(format!("catalog rule {id} has no default severity"))
                })?
                .to_string(),
            default_enabled: entry["default_enabled"].as_bool().ok_or_else(|| {
                EvalError::Command(format!("catalog rule {id} has no activation state"))
            })?,
        };
        if catalog.insert(id.to_string(), rule).is_some() {
            return Err(EvalError::Command(format!(
                "candidate rule catalog repeats {id}"
            )));
        }
    }
    if catalog.is_empty() {
        return Err(EvalError::Command(
            "candidate rule catalog is empty".to_string(),
        ));
    }
    Ok(catalog)
}

fn run_wave(
    jobs: Vec<RepositoryJob>,
    concurrency: usize,
    attempt: u8,
    context: &RunContext<'_>,
) -> Vec<CorpusRecord> {
    let count = jobs.len();
    let queue = Arc::new(Mutex::new(VecDeque::from(jobs)));
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..concurrency.min(count.max(1)) {
            let queue = Arc::clone(&queue);
            let sender = sender.clone();
            scope.spawn(move || {
                loop {
                    let job = queue
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .pop_front();
                    let Some(job) = job else {
                        break;
                    };
                    let record = scan_repository(&job, attempt, context);
                    if sender.send(record).is_err() {
                        break;
                    }
                }
            });
        }
    });
    drop(sender);
    receiver.into_iter().take(count).collect()
}

fn scan_repository(job: &RepositoryJob, attempt: u8, context: &RunContext<'_>) -> CorpusRecord {
    let checkout = context.checkout_root.join(&job.prepared.checkout_dir);
    if Instant::now() >= context.deadline {
        return failed_record(
            job,
            context,
            attempt,
            "global_budget",
            "global corpus budget expired before launch".to_string(),
            0,
        );
    }
    if let Err(error) = verify_checkout(job, &checkout) {
        return failed_record(
            job,
            context,
            attempt,
            "checkout",
            sanitize_message(&error.to_string(), &checkout, None),
            0,
        );
    }
    if let Err(error) = sandbox::validate_checkout_tree(&checkout) {
        return failed_record(
            job,
            context,
            attempt,
            "sandbox_rejected",
            sanitize_message(&error.to_string(), &checkout, None),
            0,
        );
    }
    let repository_deadline = Instant::now()
        .checked_add(context.repository_timeout)
        .map_or(context.deadline, |deadline| deadline.min(context.deadline));
    let mut scans = Vec::new();
    let mut attempted_roots = BTreeSet::new();
    let mut reported_roots = BTreeSet::new();
    let mut failures = Vec::new();
    for root in &job.prepared.project_roots {
        if reported_roots.contains(root) {
            continue;
        }
        attempted_roots.insert(root.clone());
        let timeout = repository_deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            failures.push(FailureEvent {
                attempt,
                kind: "repository_budget".to_string(),
                message: "repository budget expired before every Cargo root was covered"
                    .to_string(),
            });
            break;
        }
        match scan_root(context, &checkout, root, attempt, timeout) {
            Ok(scan) => {
                attempted_roots.extend(scan.project_roots.iter().cloned());
                reported_roots.extend(scan.project_roots.iter().cloned());
                scans.push(scan);
            }
            Err(failure) => {
                failures.push(failure);
                break;
            }
        }
    }
    record_from_scans(
        job,
        context,
        attempt,
        &scans,
        attempted_roots.into_iter().collect(),
        reported_roots.into_iter().collect(),
        failures,
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "one root scan retains every structured failure boundary in execution order"
)]
fn scan_root(
    context: &RunContext<'_>,
    checkout: &Path,
    root: &str,
    attempt: u8,
    timeout: Duration,
) -> std::result::Result<RootScan, FailureEvent> {
    let workspace_root = if root == "." {
        "/workspace".to_string()
    } else {
        format!("/workspace/{root}")
    };
    let scan_args = vec![
        workspace_root,
        "--json-compact".to_string(),
        "--offline".to_string(),
        "--no-project-config".to_string(),
        "--no-respect-inline-disables".to_string(),
        "--evaluation-profile".to_string(),
        "--project".to_string(),
        "*".to_string(),
        "--max-duration".to_string(),
        timeout.as_secs().max(1).to_string(),
    ];
    let command = match sandbox::command(
        checkout,
        context.binary,
        context.cargo_home,
        context.scratch_bytes,
        &scan_args,
    ) {
        Ok(command) => command,
        Err(error) => {
            return Err(FailureEvent {
                attempt,
                kind: "sandbox_setup".to_string(),
                message: sanitize_message(&error.to_string(), checkout, None),
            });
        }
    };
    let output = match run_capped_with_limits(
        command,
        timeout,
        context.output_cap,
        context.resource_limits,
    ) {
        Ok(output) => output,
        Err(error) => {
            return Err(FailureEvent {
                attempt,
                kind: "sandbox_process".to_string(),
                message: sanitize_message(&error.to_string(), checkout, None),
            });
        }
    };
    let elapsed_ms = u64::try_from(output.elapsed.as_millis()).unwrap_or(u64::MAX);
    if let Some(reason) = output.resource_exhausted {
        return Err(FailureEvent {
            attempt,
            kind: "resource_limit".to_string(),
            message: reason,
        });
    }
    if output.timed_out {
        return Err(FailureEvent {
            attempt,
            kind: "timeout".to_string(),
            message: format!(
                "Cargo root {root} exceeded the remaining {} second repository budget",
                timeout.as_secs()
            ),
        });
    }
    if output.output_overflow {
        return Err(FailureEvent {
            attempt,
            kind: "oversized_output".to_string(),
            message: format!(
                "Cargo root {root} exceeded the {} byte output cap",
                context.output_cap
            ),
        });
    }
    let report: ReportV1 = match serde_json::from_slice(&output.stdout) {
        Ok(report) => report,
        Err(error) => {
            let stderr = sanitize_message(
                String::from_utf8_lossy(&output.stderr).trim(),
                checkout,
                None,
            );
            return Err(FailureEvent {
                attempt,
                kind: "invalid_report".to_string(),
                message: format!(
                    "Cargo root {root} emitted invalid Report V1: {error}; stderr: {stderr}"
                ),
            });
        }
    };
    let project_roots = report
        .projects
        .iter()
        .map(|project| repository_relative_root(&report, &project.package_root))
        .collect::<Result<Vec<_>>>()
        .map_err(|error| FailureEvent {
            attempt,
            kind: "invalid_report_root".to_string(),
            message: format!("Cargo root {root}: {error}"),
        })?;
    Ok(RootScan {
        selected_root: root.to_string(),
        report,
        project_roots,
        elapsed_ms,
    })
}

fn repository_relative_root(report: &ReportV1, package_root: &str) -> Result<String> {
    let resolved = report.resolved_root.as_deref().ok_or_else(|| {
        EvalError::Command("successful Report V1 has no resolved root".to_string())
    })?;
    let resolved = Path::new(resolved);
    let workspace = Path::new("/workspace");
    let prefix = resolved.strip_prefix(workspace).map_err(|_| {
        EvalError::Command(format!(
            "resolved report root '{}' escapes /workspace",
            resolved.display()
        ))
    })?;
    let package = Path::new(package_root);
    if package_root != "."
        && !package
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(EvalError::Command(format!(
            "reported package root {package_root:?} is not a safe relative path"
        )));
    }
    let combined: PathBuf = if package_root == "." {
        prefix.to_path_buf()
    } else {
        prefix.join(package)
    };
    if combined.as_os_str().is_empty() {
        Ok(".".to_string())
    } else {
        Ok(combined.to_string_lossy().replace('\\', "/"))
    }
}

fn verify_checkout(job: &RepositoryJob, checkout: &Path) -> Result<()> {
    verify_clean_checkout(checkout, Duration::from_secs(10))?;
    let actual = git_output(checkout, &["rev-parse", "HEAD"], Duration::from_secs(10))?;
    if actual.trim() != job.spec.commit {
        return Err(EvalError::Command(format!(
            "prepared checkout commit is {}, expected {}",
            actual.trim(),
            job.spec.commit
        )));
    }
    let tree_digest = git_output(
        checkout,
        &["rev-parse", "HEAD^{tree}"],
        Duration::from_secs(10),
    )?;
    if tree_digest.trim() != job.prepared.tree_digest {
        return Err(EvalError::Command(format!(
            "prepared checkout tree digest is {}, expected {}",
            tree_digest.trim(),
            job.prepared.tree_digest
        )));
    }
    if submodule_status(checkout, Duration::from_secs(10))? != job.prepared.submodule_status {
        return Err(EvalError::Command(
            "prepared checkout submodule state changed after preparation".to_string(),
        ));
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "aggregate report construction keeps root coverage and diagnostic identity together"
)]
fn record_from_scans(
    job: &RepositoryJob,
    context: &RunContext<'_>,
    attempt: u8,
    scans: &[RootScan],
    mut attempted_roots: Vec<String>,
    mut reported_roots: Vec<String>,
    mut failure_chain: Vec<FailureEvent>,
) -> CorpusRecord {
    let mut expected_roots = job.prepared.project_roots.clone();
    expected_roots.sort();
    expected_roots.dedup();
    attempted_roots.sort();
    attempted_roots.dedup();
    reported_roots.sort();
    reported_roots.dedup();
    let roots_match = expected_roots == attempted_roots && attempted_roots == reported_roots;
    let reports_complete = scans.iter().all(|scan| {
        scan.report.completeness.state == CompletenessState::Complete
            && scan
                .report
                .projects
                .iter()
                .all(|project| project.completeness.state == CompletenessState::Complete)
    });
    let complete = roots_match && reports_complete && failure_chain.is_empty();
    let completeness = if complete { "complete" } else { "incomplete" }.to_string();
    let package_roots = expected_roots.clone();
    let mut per_rule_counts = BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut diagnostic_counts = SeverityCounts::default();
    let mut duration_ms = 0u64;
    for scan in scans {
        duration_ms = duration_ms.saturating_add(scan.elapsed_ms);
        diagnostic_counts.error = diagnostic_counts
            .error
            .saturating_add(scan.report.summary.error_count);
        diagnostic_counts.warning = diagnostic_counts
            .warning
            .saturating_add(scan.report.summary.warning_count);
        diagnostic_counts.info = diagnostic_counts
            .info
            .saturating_add(scan.report.summary.info_count);
        for (project, package_root) in scan.report.projects.iter().zip(&scan.project_roots) {
            for diagnostic in &project.diagnostics {
                *per_rule_counts.entry(diagnostic.rule.clone()).or_insert(0) += 1;
                let fingerprint_bytes = serde_json::to_vec(diagnostic)
                    .unwrap_or_else(|_| diagnostic.baseline_key.as_bytes().to_vec());
                diagnostics.push(EvaluationDiagnostic {
                    repository: job.spec.name.clone(),
                    package_root: package_root.clone(),
                    rule: diagnostic.rule.clone(),
                    site_id: diagnostic.site_id.clone(),
                    baseline_key: diagnostic.baseline_key.clone(),
                    fingerprint: hex_digest(&fingerprint_bytes),
                });
            }
        }
        if scan.report.completeness.state != CompletenessState::Complete
            || scan
                .report
                .projects
                .iter()
                .any(|project| project.completeness.state != CompletenessState::Complete)
        {
            failure_chain.push(FailureEvent {
                attempt,
                kind: "incomplete_report".to_string(),
                message: scan.report.error.as_ref().map_or_else(
                    || {
                        format!(
                            "Cargo root {} returned incomplete required work",
                            scan.selected_root
                        )
                    },
                    |error| format!("{}: {}", error.kind, error.message),
                ),
            });
        }
    }
    diagnostics.sort();
    if !roots_match {
        failure_chain.push(FailureEvent {
            attempt,
            kind: "root_coverage_mismatch".to_string(),
            message: format!(
                "expected={expected_roots:?}; attempted={attempted_roots:?}; reported={reported_roots:?}"
            ),
        });
    }
    CorpusRecord {
        schema_version: CORPUS_SCHEMA_VERSION.to_string(),
        repository: job.spec.name.clone(),
        commit: job.spec.commit.clone(),
        package_roots,
        expected_roots,
        attempted_roots,
        reported_roots,
        tool_revision: context.tool_revision.to_string(),
        evaluation_profile_sha256: context.evaluation_profile_sha256.to_string(),
        catalog_sha256: context.catalog_sha256.to_string(),
        catalog: context.catalog.clone(),
        tree_digest: job.prepared.tree_digest.clone(),
        complete,
        completeness,
        diagnostic_counts,
        per_rule_counts,
        duration_ms,
        attempts: attempt,
        diagnostics,
        failure_chain,
    }
}

fn failed_record(
    job: &RepositoryJob,
    context: &RunContext<'_>,
    attempt: u8,
    kind: &str,
    message: String,
    duration_ms: u64,
) -> CorpusRecord {
    CorpusRecord {
        schema_version: CORPUS_SCHEMA_VERSION.to_string(),
        repository: job.spec.name.clone(),
        commit: job.spec.commit.clone(),
        package_roots: job.prepared.project_roots.clone(),
        expected_roots: job.prepared.project_roots.clone(),
        attempted_roots: Vec::new(),
        reported_roots: Vec::new(),
        tool_revision: context.tool_revision.to_string(),
        evaluation_profile_sha256: context.evaluation_profile_sha256.to_string(),
        catalog_sha256: context.catalog_sha256.to_string(),
        catalog: context.catalog.clone(),
        tree_digest: job.prepared.tree_digest.clone(),
        complete: false,
        completeness: "incomplete".to_string(),
        diagnostic_counts: SeverityCounts::default(),
        per_rule_counts: BTreeMap::new(),
        duration_ms,
        attempts: attempt,
        diagnostics: Vec::new(),
        failure_chain: vec![FailureEvent {
            attempt,
            kind: kind.to_string(),
            message,
        }],
    }
}

fn sanitize_message(message: &str, checkout: &Path, scratch: Option<&Path>) -> String {
    let mut sanitized = message.replace(&checkout.to_string_lossy().to_string(), "<repository>");
    if let Some(scratch) = scratch {
        sanitized = sanitized.replace(&scratch.to_string_lossy().to_string(), "<sandbox>");
    }
    sanitized.chars().take(2_048).collect()
}

fn mib_to_bytes(value: usize, label: &str) -> Result<usize> {
    if value == 0 {
        return Err(EvalError::InvalidManifest(format!(
            "{label} limit must be greater than zero"
        )));
    }
    value
        .checked_mul(1024 * 1024)
        .ok_or_else(|| EvalError::InvalidManifest(format!("{label} limit is too large")))
}

fn write_ndjson_atomic(path: &Path, records: &[CorpusRecord]) -> Result<()> {
    let schema_value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../evaluation/schemas/corpus-record-v1.schema.json"
    ))
    .map_err(|error| {
        EvalError::Command(format!("corpus record schema is invalid JSON: {error}"))
    })?;
    let schema = jsonschema::validator_for(&schema_value)
        .map_err(|error| EvalError::Command(format!("corpus record schema is invalid: {error}")))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| EvalError::io("cannot create NDJSON output directory", parent, error))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| EvalError::io("cannot create NDJSON temporary file", parent, error))?;
    for record in records {
        let value = serde_json::to_value(record).map_err(|error| {
            EvalError::Command(format!("cannot serialize corpus record: {error}"))
        })?;
        let schema_errors: Vec<_> = schema
            .iter_errors(&value)
            .take(8)
            .map(|error| error.to_string())
            .collect();
        if !schema_errors.is_empty() {
            return Err(EvalError::Command(format!(
                "corpus record for {} violates schema: {}",
                record.repository,
                schema_errors.join("; ")
            )));
        }
        serde_json::to_writer(&mut temporary, &value)
            .map_err(|error| EvalError::Command(format!("cannot write corpus record: {error}")))?;
        temporary
            .write_all(b"\n")
            .map_err(|error| EvalError::io("cannot write corpus record", path, error))?;
    }
    temporary
        .as_file_mut()
        .sync_all()
        .map_err(|error| EvalError::io("cannot sync corpus NDJSON", path, error))?;
    temporary
        .persist(path)
        .map_err(|error| EvalError::io("cannot persist corpus NDJSON", path, error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_root_discovery_ignores_vendor_and_invalid_manifests() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\n",
        )
        .unwrap();
        std::fs::create_dir_all(directory.path().join("crate")).unwrap();
        std::fs::write(
            directory.path().join("crate/Cargo.toml"),
            "[package]\nname = \"crate\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(directory.path().join("vendor/ignored")).unwrap();
        std::fs::write(
            directory.path().join("vendor/ignored/Cargo.toml"),
            "[package]\nname = \"ignored\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert_eq!(discover_project_roots(directory.path()).unwrap(), ["crate"]);
    }

    #[test]
    fn failure_messages_remove_host_paths() {
        let checkout = Path::new("/host/secret/repository");
        let scratch = Path::new("/host/secret/scratch");
        let sanitized = sanitize_message(
            "/host/secret/repository failed in /host/secret/scratch",
            checkout,
            Some(scratch),
        );
        assert_eq!(sanitized, "<repository> failed in <sandbox>");
    }
}
