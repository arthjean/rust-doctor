use super::manifest::{hex_digest, read_json, sha256_file, write_json_atomic};
use super::process::run_capped;
use super::{EvalError, Result};
use crate::BenchmarkArgs;
use rust_doctor::diagnostics::ReportV1;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const BENCHMARK_SCHEMA_VERSION: &str = "1.0";
const COMMAND_OUTPUT_CAP: usize = 32 * 1024 * 1024;
const MIN_REPETITIONS: usize = 3;
const MIN_ABSOLUTE_REGRESSION_MS: u64 = 50;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkManifest {
    schema_version: String,
    fixtures: Vec<FixtureSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FixtureSpec {
    name: String,
    source_files: usize,
    lines_per_file: usize,
    workspace_members: usize,
    modes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkRecord {
    fixture: String,
    fixture_fingerprint: String,
    mode: String,
    temperature: String,
    repetition: usize,
    wall_ms: u64,
    cpu_ms: u64,
    peak_rss_bytes: u64,
    files_per_second: f64,
    cache_hit_rate: f64,
    pass_timings_ms: BTreeMap<String, u64>,
    diagnostic_sha256: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkGate {
    blocked: bool,
    reasons: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkReport {
    schema_version: String,
    manifest_sha256: String,
    tool_revision: String,
    binary_sha256: String,
    host_class: String,
    toolchain: String,
    repetitions: usize,
    diagnostic_sha256: String,
    approval: Option<BenchmarkApproval>,
    records: Vec<BenchmarkRecord>,
    gate: BenchmarkGate,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkApproval {
    reviewed_by: String,
    reviewed_at: String,
    review_source: String,
}

#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    reason = "the command handler owns its parsed Clap subcommand"
)]
pub(crate) fn run(args: BenchmarkArgs) -> Result<()> {
    if args.repetitions < MIN_REPETITIONS {
        return Err(EvalError::InvalidManifest(format!(
            "benchmark gates require at least {MIN_REPETITIONS} repetitions"
        )));
    }
    if args.baseline.is_none() && !args.record {
        return Err(EvalError::InvalidManifest(
            "benchmark gate requires --baseline; use --record only to generate a review candidate"
                .to_string(),
        ));
    }
    let manifest: BenchmarkManifest = read_json(&args.manifest)?;
    validate_manifest(&manifest)?;
    let manifest_sha256 = sha256_file(&args.manifest)?;
    let binary = args.binary.canonicalize().map_err(|error| {
        EvalError::io("cannot canonicalize benchmark binary", &args.binary, error)
    })?;
    ensure_gnu_time()?;
    let tool_revision = binary_revision(&binary)?;
    let binary_sha256 = sha256_file(&binary)?;
    let host_class = std::env::var("RUST_DOCTOR_BENCHMARK_HOST_CLASS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            EvalError::InvalidManifest(
                "RUST_DOCTOR_BENCHMARK_HOST_CLASS must identify the benchmark runner".to_string(),
            )
        })?;
    let toolchain = rustc_identity()?;
    let mut records = Vec::new();

    for fixture in &manifest.fixtures {
        let materialized = materialize(fixture)?;
        for mode in &fixture.modes {
            for repetition in 0..args.repetitions {
                remove_scan_caches(materialized.root.path())?;
                records.push(run_once(
                    fixture,
                    &materialized,
                    mode,
                    "cold",
                    repetition,
                    &binary,
                )?);
                records.push(run_once(
                    fixture,
                    &materialized,
                    mode,
                    "warm",
                    repetition,
                    &binary,
                )?);
            }
        }
    }
    let mut gate = BenchmarkGate::default();
    for record in &records {
        if record.fixture == "hundred-k-lines" && record.peak_rss_bytes > 512 * 1024 * 1024 {
            gate.reasons.push(format!(
                "{} {} {} used {} MiB, above 512 MiB",
                record.fixture,
                record.mode,
                record.temperature,
                record.peak_rss_bytes / (1024 * 1024)
            ));
        }
    }
    let diagnostic_sha256 = diagnostic_matrix_sha256(&records)?;
    if let Some(baseline_path) = &args.baseline {
        let baseline: BenchmarkReport = read_json(baseline_path)?;
        if baseline.schema_version != BENCHMARK_SCHEMA_VERSION
            || baseline.manifest_sha256 != manifest_sha256
            || baseline.host_class != host_class
            || baseline.toolchain != toolchain
            || baseline.repetitions != args.repetitions
            || baseline.diagnostic_sha256 != diagnostic_sha256
            || baseline.binary_sha256.len() != 64
        {
            return Err(EvalError::InvalidManifest(
                "benchmark baseline schema, fixtures, diagnostics, host, toolchain, repetitions, or binary identity is incompatible"
                    .to_string(),
            ));
        }
        validate_approval(baseline.approval.as_ref())?;
        gate.reasons
            .extend(compare_runtime(&baseline.records, &records));
    }
    gate.blocked = !gate.reasons.is_empty();
    let report = BenchmarkReport {
        schema_version: BENCHMARK_SCHEMA_VERSION.to_string(),
        manifest_sha256,
        tool_revision,
        binary_sha256,
        host_class,
        toolchain,
        repetitions: args.repetitions,
        diagnostic_sha256,
        approval: None,
        records,
        gate,
    };
    write_json_atomic(&args.output, &report)?;
    if report.gate.blocked {
        Err(EvalError::GateFailed(report.gate.reasons.join("; ")))
    } else {
        Ok(())
    }
}

struct MaterializedFixture {
    root: tempfile::TempDir,
    source_files: Vec<PathBuf>,
}

fn validate_manifest(manifest: &BenchmarkManifest) -> Result<()> {
    if manifest.schema_version != BENCHMARK_SCHEMA_VERSION {
        return Err(EvalError::InvalidManifest(format!(
            "benchmark schema must be {BENCHMARK_SCHEMA_VERSION}"
        )));
    }
    let names: BTreeSet<_> = manifest
        .fixtures
        .iter()
        .map(|fixture| fixture.name.as_str())
        .collect();
    for required in [
        "small",
        "medium",
        "large",
        "workspace-20",
        "hundred-k-lines",
    ] {
        if !names.contains(required) {
            return Err(EvalError::InvalidManifest(format!(
                "benchmark manifest is missing fixed fixture {required}"
            )));
        }
    }
    for fixture in &manifest.fixtures {
        if fixture.source_files == 0
            || fixture.lines_per_file == 0
            || fixture.workspace_members == 0
        {
            return Err(EvalError::InvalidManifest(format!(
                "fixture {} dimensions must be positive",
                fixture.name
            )));
        }
        if fixture.name == "workspace-20" && fixture.workspace_members != 20 {
            return Err(EvalError::InvalidManifest(
                "workspace-20 must contain exactly 20 members".to_string(),
            ));
        }
        let modes: BTreeSet<_> = fixture.modes.iter().map(String::as_str).collect();
        for mode in &modes {
            if !matches!(*mode, "full" | "files" | "lines" | "baseline") {
                return Err(EvalError::InvalidManifest(format!(
                    "fixture {} has unknown mode {mode}",
                    fixture.name
                )));
            }
        }
        if fixture.name != "hundred-k-lines"
            && ["full", "files", "lines", "baseline"]
                .iter()
                .any(|mode| !modes.contains(mode))
        {
            return Err(EvalError::InvalidManifest(format!(
                "fixture {} must cover full, files, lines, and baseline",
                fixture.name
            )));
        }
    }
    let Some(hundred_k) = manifest
        .fixtures
        .iter()
        .find(|fixture| fixture.name == "hundred-k-lines")
    else {
        return Err(EvalError::InvalidManifest(
            "benchmark manifest is missing fixed fixture hundred-k-lines".to_string(),
        ));
    };
    if hundred_k
        .source_files
        .saturating_mul(hundred_k.lines_per_file)
        < 100_000
    {
        return Err(EvalError::InvalidManifest(
            "hundred-k-lines must materialize at least 100,000 lines".to_string(),
        ));
    }
    Ok(())
}

fn materialize(spec: &FixtureSpec) -> Result<MaterializedFixture> {
    let root = tempfile::Builder::new()
        .prefix("rust-doctor-benchmark-")
        .tempdir()
        .map_err(|error| EvalError::io("cannot create benchmark fixture", ".", error))?;
    let mut sources = Vec::with_capacity(spec.source_files);
    if spec.workspace_members == 1 {
        write_package(root.path(), "fixture")?;
        write_sources(
            root.path(),
            spec.source_files,
            spec.lines_per_file,
            &mut sources,
        )?;
    } else {
        let members: Vec<_> = (0..spec.workspace_members)
            .map(|index| format!("crates/member-{index:02}"))
            .collect();
        let members_toml = members
            .iter()
            .map(|member| format!("\"{member}\""))
            .collect::<Vec<_>>()
            .join(", ");
        write(
            &root.path().join("Cargo.toml"),
            &format!("[workspace]\nresolver = \"3\"\nmembers = [{members_toml}]\n"),
        )?;
        let files_per_member = spec.source_files.div_ceil(spec.workspace_members);
        let mut remaining = spec.source_files;
        for (index, member) in members.iter().enumerate() {
            let package_root = root.path().join(member);
            write_package(&package_root, &format!("member_{index:02}"))?;
            let count = remaining.min(files_per_member);
            write_sources(&package_root, count, spec.lines_per_file, &mut sources)?;
            remaining = remaining.saturating_sub(count);
        }
    }
    write(
        &root.path().join("rust-doctor.toml"),
        "lint = true\ndependencies = false\n",
    )?;
    git(root.path(), &["init", "--quiet"])?;
    git(
        root.path(),
        &["config", "user.email", "benchmark@localhost"],
    )?;
    git(root.path(), &["config", "user.name", "Benchmark"])?;
    git(root.path(), &["add", "."])?;
    git(root.path(), &["commit", "--quiet", "-m", "fixture"])?;
    let first = sources.first().ok_or_else(|| {
        EvalError::InvalidManifest(format!("fixture {} has no source file", spec.name))
    })?;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(first)
        .map_err(|error| EvalError::io("cannot mutate benchmark fixture", first, error))?;
    file.write_all(b"// changed line\n")
        .map_err(|error| EvalError::io("cannot mutate benchmark fixture", first, error))?;
    Ok(MaterializedFixture {
        root,
        source_files: sources,
    })
}

fn write_package(root: &Path, name: &str) -> Result<()> {
    write(
        &root.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.85\"\n"
        ),
    )
}

fn write_sources(
    package_root: &Path,
    count: usize,
    lines_per_file: usize,
    sources: &mut Vec<PathBuf>,
) -> Result<()> {
    let source_root = package_root.join("src");
    std::fs::create_dir_all(&source_root)
        .map_err(|error| EvalError::io("cannot create benchmark source", &source_root, error))?;
    for index in 0..count {
        let path = if index == 0 {
            source_root.join("lib.rs")
        } else {
            source_root.join(format!("generated_{index:04}.rs"))
        };
        let mut content = format!("pub fn value_{index}() -> usize {{ {index} }}\n");
        for line in 1..lines_per_file {
            writeln!(content, "// fixture line {line}").map_err(|error| {
                EvalError::Command(format!("cannot format benchmark fixture: {error}"))
            })?;
        }
        write(&path, &content)?;
        sources.push(path);
    }
    Ok(())
}

fn write(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| EvalError::io("cannot create benchmark directory", parent, error))?;
    }
    std::fs::write(path, content)
        .map_err(|error| EvalError::io("cannot write benchmark fixture", path, error))
}

fn git(root: &Path, arguments: &[&str]) -> Result<()> {
    let mut command = Command::new("git");
    command.current_dir(root).args(arguments);
    let output = run_capped(command, Duration::from_secs(30), 1024 * 1024)?;
    if output.timed_out || !output.status.success() {
        return Err(EvalError::Command(format!(
            "benchmark Git setup failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn run_once(
    spec: &FixtureSpec,
    materialized: &MaterializedFixture,
    mode: &str,
    temperature: &str,
    repetition: usize,
    binary: &Path,
) -> Result<BenchmarkRecord> {
    let selected = selected_sources(materialized, mode);
    let hit_rate = cache_hit_rate(materialized.root.path(), &selected);
    let metrics = tempfile::NamedTempFile::new()
        .map_err(|error| EvalError::io("cannot create benchmark metrics file", ".", error))?;
    let mut command = Command::new("/usr/bin/time");
    command
        .args(["-f", "%U\t%S\t%M", "-o"])
        .arg(metrics.path())
        .arg(binary)
        .arg(materialized.root.path())
        .args(["--json-compact", "--offline"])
        .args(mode_arguments(materialized, mode)?);
    let output = run_capped(command, Duration::from_mins(30), COMMAND_OUTPUT_CAP)?;
    if output.timed_out || !output.status.success() {
        return Err(EvalError::Command(format!(
            "benchmark {} {mode} {temperature} failed: {}",
            spec.name,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let report: ReportV1 = serde_json::from_slice(&output.stdout).map_err(|error| {
        EvalError::Command(format!(
            "benchmark {} emitted invalid Report V1: {error}",
            spec.name
        ))
    })?;
    let metrics_content = std::fs::read_to_string(metrics.path()).map_err(|error| {
        EvalError::io(
            "cannot read benchmark process metrics",
            metrics.path(),
            error,
        )
    })?;
    let (cpu_ms, peak_rss_bytes) = parse_time_metrics(&metrics_content)?;
    let analyzed = u32::try_from(report.completeness.analyzed_files)
        .map_err(|_| EvalError::Command("benchmark analyzed-file count exceeds u32".to_string()))?;
    let elapsed_seconds = output.elapsed.as_secs_f64().max(0.001);
    let diagnostic_sha256 = report_diagnostic_sha256(&report)?;
    Ok(BenchmarkRecord {
        fixture: spec.name.clone(),
        fixture_fingerprint: fixture_fingerprint(spec),
        mode: mode.to_string(),
        temperature: temperature.to_string(),
        repetition,
        wall_ms: u64::try_from(output.elapsed.as_millis()).unwrap_or(u64::MAX),
        cpu_ms,
        peak_rss_bytes,
        files_per_second: f64::from(analyzed) / elapsed_seconds,
        cache_hit_rate: hit_rate,
        pass_timings_ms: report.pass_timings_ms,
        diagnostic_sha256,
    })
}

fn mode_arguments(materialized: &MaterializedFixture, mode: &str) -> Result<Vec<String>> {
    match mode {
        "full" => Ok(Vec::new()),
        "files" => {
            let first = materialized.source_files.first().ok_or_else(|| {
                EvalError::InvalidManifest("files benchmark has no source".to_string())
            })?;
            let relative = first
                .strip_prefix(materialized.root.path())
                .unwrap_or(first)
                .to_string_lossy()
                .replace('\\', "/");
            Ok(vec![
                "--scope".to_string(),
                "files".to_string(),
                "--files".to_string(),
                relative,
            ])
        }
        "lines" => Ok(vec![
            "--scope".to_string(),
            "lines".to_string(),
            "--base".to_string(),
            "HEAD".to_string(),
        ]),
        "baseline" => Ok(vec![
            "--baseline".to_string(),
            "--base".to_string(),
            "HEAD".to_string(),
        ]),
        other => Err(EvalError::InvalidManifest(format!(
            "unknown benchmark mode {other}"
        ))),
    }
}

fn selected_sources(materialized: &MaterializedFixture, mode: &str) -> Vec<PathBuf> {
    if mode == "full" {
        materialized.source_files.clone()
    } else {
        materialized
            .source_files
            .first()
            .cloned()
            .into_iter()
            .collect()
    }
}

fn cache_hit_rate(root: &Path, selected: &[PathBuf]) -> f64 {
    if selected.is_empty() {
        return 0.0;
    }
    let hits = selected
        .iter()
        .filter(|source| source_cache_hit(root, source))
        .count();
    let hits = u32::try_from(hits).unwrap_or(u32::MAX);
    let selected = u32::try_from(selected.len()).unwrap_or(u32::MAX);
    f64::from(hits) / f64::from(selected)
}

fn source_cache_hit(root: &Path, source: &Path) -> bool {
    let Ok(content) = std::fs::read(source) else {
        return false;
    };
    let hash = hex_digest(&content);
    let mut directory = source.parent();
    while let Some(candidate_root) = directory {
        if !candidate_root.starts_with(root) {
            break;
        }
        let cache_path = candidate_root.join(".rust-doctor-cache.json");
        if let Ok(cache) = std::fs::read(&cache_path)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&cache)
        {
            let relative = source
                .strip_prefix(candidate_root)
                .unwrap_or(source)
                .to_string_lossy()
                .replace('\\', "/");
            if value["files"][&relative]["hash"].as_str() == Some(hash.as_str()) {
                return true;
            }
        }
        if candidate_root == root {
            break;
        }
        directory = candidate_root.parent();
    }
    false
}

fn remove_scan_caches(root: &Path) -> Result<()> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| EvalError::io("cannot inspect benchmark cache", &directory, error))?
        {
            let entry = entry.map_err(|error| {
                EvalError::io("cannot inspect benchmark cache", &directory, error)
            })?;
            let path = entry.path();
            if entry.file_name() == ".rust-doctor-cache.json" {
                std::fs::remove_file(&path)
                    .map_err(|error| EvalError::io("cannot clear benchmark cache", &path, error))?;
            } else if path.is_dir() && entry.file_name() != ".git" && entry.file_name() != "target"
            {
                pending.push(path);
            }
        }
    }
    Ok(())
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "validated non-negative GNU time seconds are rounded to millisecond counters"
)]
fn parse_time_metrics(metrics: &str) -> Result<(u64, u64)> {
    let fields: Vec<_> = metrics.split_whitespace().collect();
    if fields.len() != 3 {
        return Err(EvalError::Command(format!(
            "unexpected GNU time metrics {metrics:?}"
        )));
    }
    let user = fields[0]
        .parse::<f64>()
        .map_err(|error| EvalError::Command(format!("invalid user CPU metric: {error}")))?;
    let system = fields[1]
        .parse::<f64>()
        .map_err(|error| EvalError::Command(format!("invalid system CPU metric: {error}")))?;
    let rss_kib = fields[2]
        .parse::<u64>()
        .map_err(|error| EvalError::Command(format!("invalid RSS metric: {error}")))?;
    let cpu_ms = ((user + system) * 1_000.0).round().max(0.0) as u64;
    Ok((cpu_ms, rss_kib.saturating_mul(1024)))
}

fn fixture_fingerprint(spec: &FixtureSpec) -> String {
    let serialized = serde_json::to_vec(spec).unwrap_or_else(|_| spec.name.as_bytes().to_vec());
    let mut identity = Vec::with_capacity(
        serialized
            .len()
            .saturating_add(include_str!("benchmark.rs").len()),
    );
    identity.extend_from_slice(b"rust-doctor-benchmark-generator-v1\0");
    identity.extend_from_slice(&serialized);
    identity.push(0);
    identity.extend_from_slice(include_bytes!("benchmark.rs"));
    hex_digest(&identity)
}

fn compare_runtime(baseline: &[BenchmarkRecord], candidate: &[BenchmarkRecord]) -> Vec<String> {
    type Key = (String, String, String, String);
    let groups = |records: &[BenchmarkRecord]| -> HashMap<Key, Vec<u64>> {
        let mut grouped: HashMap<Key, Vec<u64>> = HashMap::new();
        for record in records {
            grouped
                .entry((
                    record.fixture.clone(),
                    record.fixture_fingerprint.clone(),
                    record.mode.clone(),
                    record.temperature.clone(),
                ))
                .or_default()
                .push(record.wall_ms);
        }
        grouped
    };
    let baseline = groups(baseline);
    let candidate = groups(candidate);
    let mut reasons = Vec::new();
    if baseline.keys().collect::<BTreeSet<_>>() != candidate.keys().collect::<BTreeSet<_>>() {
        reasons.push("candidate benchmark matrix differs from the approved baseline".to_string());
        return reasons;
    }
    for (key, baseline_values) in baseline {
        let Some(candidate_values) = candidate.get(&key) else {
            reasons.push(format!("candidate is missing benchmark {key:?}"));
            continue;
        };
        if baseline_values.len() != candidate_values.len()
            || baseline_values.len() < MIN_REPETITIONS
        {
            reasons.push(format!(
                "benchmark {key:?} does not preserve the approved repetition matrix"
            ));
            continue;
        }
        for (label, quantile) in [("median", 50), ("p95", 95)] {
            let base = percentile(&baseline_values, quantile);
            let current = percentile(candidate_values, quantile);
            let regression = percent_change(base, current);
            if regression > 10.0 && current.saturating_sub(base) >= MIN_ABSOLUTE_REGRESSION_MS {
                reasons.push(format!(
                    "{} {} {} {label} regressed {regression:.3}%",
                    key.0, key.2, key.3
                ));
            }
        }
    }
    reasons
}

fn report_diagnostic_sha256(report: &ReportV1) -> Result<String> {
    let mut diagnostics = report.diagnostics.clone();
    diagnostics.sort_by(|left, right| {
        left.rule
            .cmp(&right.rule)
            .then(left.site_id.cmp(&right.site_id))
            .then(left.baseline_key.cmp(&right.baseline_key))
    });
    let bytes = serde_json::to_vec(&diagnostics).map_err(|error| {
        EvalError::Command(format!("cannot fingerprint benchmark diagnostics: {error}"))
    })?;
    Ok(hex_digest(&bytes))
}

fn diagnostic_matrix_sha256(records: &[BenchmarkRecord]) -> Result<String> {
    let matrix: Vec<_> = records
        .iter()
        .map(|record| {
            (
                &record.fixture,
                &record.fixture_fingerprint,
                &record.mode,
                &record.temperature,
                record.repetition,
                &record.diagnostic_sha256,
            )
        })
        .collect();
    let bytes = serde_json::to_vec(&matrix).map_err(|error| {
        EvalError::Command(format!("cannot fingerprint benchmark matrix: {error}"))
    })?;
    Ok(hex_digest(&bytes))
}

fn validate_approval(approval: Option<&BenchmarkApproval>) -> Result<()> {
    let approval = approval.ok_or_else(|| {
        EvalError::InvalidManifest(
            "benchmark baseline has no protected approval metadata".to_string(),
        )
    })?;
    if approval.reviewed_by.trim().is_empty()
        || approval.reviewed_at.trim().is_empty()
        || !matches!(
            approval.review_source.as_str(),
            "protected-ci" | "codeowners"
        )
    {
        return Err(EvalError::InvalidManifest(
            "benchmark approval must identify a protected CI or CODEOWNERS review".to_string(),
        ));
    }
    Ok(())
}

fn rustc_identity() -> Result<String> {
    let mut command = Command::new("rustc");
    command.arg("-Vv");
    let output = run_capped(command, Duration::from_secs(10), 64 * 1024)?;
    if output.timed_out || !output.status.success() {
        return Err(EvalError::Command(
            "cannot identify benchmark rustc toolchain".to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = (sorted.len() - 1).saturating_mul(percentile).div_ceil(100);
    sorted[index.min(sorted.len() - 1)]
}

#[expect(
    clippy::cast_precision_loss,
    reason = "benchmark millisecond durations are bounded far below f64 integer precision"
)]
fn percent_change(baseline: u64, candidate: u64) -> f64 {
    if baseline == 0 {
        0.0
    } else {
        ((candidate as f64 / baseline as f64) - 1.0) * 100.0
    }
}

fn ensure_gnu_time() -> Result<()> {
    let mut command = Command::new("/usr/bin/time");
    command.arg("--version");
    let output = run_capped(command, Duration::from_secs(5), 64 * 1024)?;
    if output.status.success() && String::from_utf8_lossy(&output.stdout).contains("GNU") {
        Ok(())
    } else {
        Err(EvalError::Unsupported(
            "performance benchmarks require GNU /usr/bin/time for CPU and peak RSS metrics"
                .to_string(),
        ))
    }
}

fn binary_revision(binary: &Path) -> Result<String> {
    let mut command = Command::new(binary);
    command.arg("--version");
    let output = run_capped(command, Duration::from_secs(10), 64 * 1024)?;
    if !output.status.success() {
        return Err(EvalError::Command(format!(
            "cannot read benchmark binary version: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnu_time_metrics_include_cpu_and_peak_rss() {
        assert_eq!(
            parse_time_metrics("0.25\t0.05\t1024\n").unwrap(),
            (300, 1_048_576)
        );
    }

    #[test]
    fn median_or_p95_above_ten_percent_blocks() {
        let make = |wall_ms| BenchmarkRecord {
            fixture: "small".to_string(),
            fixture_fingerprint: "hash".to_string(),
            mode: "full".to_string(),
            temperature: "cold".to_string(),
            repetition: 0,
            wall_ms,
            cpu_ms: wall_ms,
            peak_rss_bytes: 1,
            files_per_second: 1.0,
            cache_hit_rate: 0.0,
            pass_timings_ms: BTreeMap::new(),
            diagnostic_sha256: "d".repeat(64),
        };
        let baseline = vec![make(100), make(100), make(100)];
        let candidate = vec![make(100), make(160), make(170)];
        assert!(!compare_runtime(&baseline, &candidate).is_empty());
    }

    #[test]
    fn fixture_fingerprint_changes_with_dimensions() {
        let mut fixture = FixtureSpec {
            name: "small".to_string(),
            source_files: 1,
            lines_per_file: 10,
            workspace_members: 1,
            modes: vec!["full".to_string()],
        };
        let before = fixture_fingerprint(&fixture);
        fixture.lines_per_file = 11;
        assert_ne!(before, fixture_fingerprint(&fixture));
    }
}
