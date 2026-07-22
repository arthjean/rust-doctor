use super::manifest::{
    hex_digest, read_json, sha256_file, validate_corpus_manifest, validate_prepared,
    write_json_atomic,
};
use super::model::{
    CORPUS_SCHEMA_VERSION, CorpusManifest, CorpusRecord, EvaluationDiagnostic, FailureEvent,
    PreparedCorpus, PreparedRepository, RepositorySpec, SeverityCounts,
};
use super::process::{ResourceLimits, run_capped, run_capped_with_limits};
use super::{EvalError, Result, sandbox};
use crate::{CorpusArgs, PrepareArgs};
use rust_doctor::diagnostics::{CompletenessState, ReportV1};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::Write;
use std::path::Path;
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
    repository_timeout: Duration,
    deadline: Instant,
    output_cap: usize,
    resource_limits: ResourceLimits,
    scratch_bytes: usize,
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
    let actual = git_output(checkout, &["rev-parse", "HEAD"], timeout)?;
    if actual.trim() != spec.commit {
        return Err(EvalError::Command(format!(
            "checkout {} resolved {}, expected {}",
            spec.name,
            actual.trim(),
            spec.commit
        )));
    }
    Ok(())
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
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .env_remove("GIT_SSH")
        .env_remove("GIT_SSH_COMMAND")
        .env_remove("GIT_CONFIG_COUNT");
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
            if parsed.get("package").is_none() && parsed.get("workspace").is_none() {
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

#[expect(
    clippy::too_many_lines,
    reason = "one repository attempt retains every structured failure boundary in execution order"
)]
fn scan_repository(job: &RepositoryJob, attempt: u8, context: &RunContext<'_>) -> CorpusRecord {
    let checkout = context.checkout_root.join(&job.prepared.checkout_dir);
    if Instant::now() >= context.deadline {
        return failed_record(
            job,
            context.tool_revision,
            attempt,
            "global_budget",
            "global corpus budget expired before launch".to_string(),
            0,
        );
    }
    if let Err(error) = verify_checkout(job, &checkout) {
        return failed_record(
            job,
            context.tool_revision,
            attempt,
            "checkout",
            sanitize_message(&error.to_string(), &checkout, None),
            0,
        );
    }
    if let Err(error) = sandbox::validate_checkout_tree(&checkout) {
        return failed_record(
            job,
            context.tool_revision,
            attempt,
            "sandbox_rejected",
            sanitize_message(&error.to_string(), &checkout, None),
            0,
        );
    }
    let remaining = context.deadline.saturating_duration_since(Instant::now());
    let timeout = context.repository_timeout.min(remaining);
    if timeout.is_zero() {
        return failed_record(
            job,
            context.tool_revision,
            attempt,
            "global_budget",
            "global corpus budget expired before sandbox launch".to_string(),
            0,
        );
    }
    let scan_args = vec![
        "/workspace".to_string(),
        "--json-compact".to_string(),
        "--offline".to_string(),
        "--no-project-config".to_string(),
        "--project".to_string(),
        "*".to_string(),
        "--max-duration".to_string(),
        timeout.as_secs().max(1).to_string(),
    ];
    let command = match sandbox::command(
        &checkout,
        context.binary,
        context.cargo_home,
        context.scratch_bytes,
        &scan_args,
    ) {
        Ok(command) => command,
        Err(error) => {
            return failed_record(
                job,
                context.tool_revision,
                attempt,
                "sandbox_setup",
                sanitize_message(&error.to_string(), &checkout, None),
                0,
            );
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
            return failed_record(
                job,
                context.tool_revision,
                attempt,
                "sandbox_process",
                sanitize_message(&error.to_string(), &checkout, None),
                0,
            );
        }
    };
    let elapsed_ms = u64::try_from(output.elapsed.as_millis()).unwrap_or(u64::MAX);
    if let Some(reason) = output.resource_exhausted {
        return failed_record(
            job,
            context.tool_revision,
            attempt,
            "resource_limit",
            reason,
            elapsed_ms,
        );
    }
    if output.timed_out {
        return failed_record(
            job,
            context.tool_revision,
            attempt,
            "timeout",
            format!("sandbox exceeded {} seconds", timeout.as_secs()),
            elapsed_ms,
        );
    }
    if output.output_overflow {
        return failed_record(
            job,
            context.tool_revision,
            attempt,
            "oversized_output",
            format!("sandbox output exceeded {} bytes", context.output_cap),
            elapsed_ms,
        );
    }
    let report: ReportV1 = match serde_json::from_slice(&output.stdout) {
        Ok(report) => report,
        Err(error) => {
            let stderr = sanitize_message(
                String::from_utf8_lossy(&output.stderr).trim(),
                &checkout,
                None,
            );
            return failed_record(
                job,
                context.tool_revision,
                attempt,
                "invalid_report",
                format!("Report V1 parse failed: {error}; stderr: {stderr}"),
                elapsed_ms,
            );
        }
    };
    record_from_report(job, context.tool_revision, attempt, &report, elapsed_ms)
}

fn verify_checkout(job: &RepositoryJob, checkout: &Path) -> Result<()> {
    let actual = git_output(checkout, &["rev-parse", "HEAD"], Duration::from_secs(10))?;
    if actual.trim() != job.spec.commit {
        return Err(EvalError::Command(format!(
            "prepared checkout commit is {}, expected {}",
            actual.trim(),
            job.spec.commit
        )));
    }
    Ok(())
}

fn record_from_report(
    job: &RepositoryJob,
    tool_revision: &str,
    attempt: u8,
    report: &ReportV1,
    duration_ms: u64,
) -> CorpusRecord {
    let complete = report.completeness.state == CompletenessState::Complete
        && report
            .projects
            .iter()
            .all(|project| project.completeness.state == CompletenessState::Complete);
    let completeness = serde_json::to_value(report.completeness.state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    let mut package_roots: Vec<_> = report
        .projects
        .iter()
        .map(|project| project.package_root.clone())
        .collect();
    if package_roots.is_empty() {
        package_roots.clone_from(&job.prepared.project_roots);
    }
    package_roots.sort();
    package_roots.dedup();
    let mut per_rule_counts = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for project in &report.projects {
        for diagnostic in &project.diagnostics {
            *per_rule_counts.entry(diagnostic.rule.clone()).or_insert(0) += 1;
            let fingerprint_bytes = serde_json::to_vec(diagnostic)
                .unwrap_or_else(|_| diagnostic.baseline_key.as_bytes().to_vec());
            let fingerprint = hex_digest(&fingerprint_bytes);
            diagnostics.push(EvaluationDiagnostic {
                package_root: project.package_root.clone(),
                rule: diagnostic.rule.clone(),
                site_id: diagnostic.site_id.clone(),
                baseline_key: diagnostic.baseline_key.clone(),
                fingerprint,
            });
        }
    }
    diagnostics.sort();
    let mut failure_chain = Vec::new();
    if !complete {
        failure_chain.push(FailureEvent {
            attempt,
            kind: "incomplete_report".to_string(),
            message: report.error.as_ref().map_or_else(
                || format!("scan completeness is {completeness}"),
                |error| format!("{}: {}", error.kind, error.message),
            ),
        });
    }
    CorpusRecord {
        schema_version: CORPUS_SCHEMA_VERSION.to_string(),
        repository: job.spec.name.clone(),
        commit: job.spec.commit.clone(),
        package_roots,
        tool_revision: tool_revision.to_string(),
        complete,
        completeness,
        diagnostic_counts: SeverityCounts {
            error: report.summary.error_count,
            warning: report.summary.warning_count,
            info: report.summary.info_count,
        },
        per_rule_counts,
        duration_ms,
        attempts: attempt,
        diagnostics,
        failure_chain,
    }
}

fn failed_record(
    job: &RepositoryJob,
    tool_revision: &str,
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
        tool_revision: tool_revision.to_string(),
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
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| EvalError::io("cannot create NDJSON output directory", parent, error))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| EvalError::io("cannot create NDJSON temporary file", parent, error))?;
    for record in records {
        serde_json::to_writer(&mut temporary, record).map_err(|error| {
            EvalError::Command(format!("cannot serialize corpus record: {error}"))
        })?;
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
        assert_eq!(
            discover_project_roots(directory.path()).unwrap(),
            [".", "crate"]
        );
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
