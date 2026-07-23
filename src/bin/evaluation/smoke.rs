use super::manifest::hex_digest;
use super::process::{ProcessOutput, run_capped};
use super::{EvalError, Result};
use crate::SmokeArgs;
use rust_doctor::diagnostics::{ReportOutcome, ReportV1, ScanMode};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const OUTPUT_CAP: usize = 32 * 1024 * 1024;
const ARCHIVE_ENTRY_LIMIT: usize = 4_096;
const ARCHIVE_EXPANDED_CAP: usize = 256 * 1024 * 1024;

#[expect(
    clippy::needless_pass_by_value,
    reason = "the command handler owns its parsed Clap subcommand"
)]
pub(crate) fn run(args: SmokeArgs) -> Result<()> {
    let binary = canonical_binary(&args.binary, "default-feature binary")?;
    let no_default = canonical_binary(&args.no_default_binary, "no-default-features binary")?;
    let schema = load_schema(&args.schema)?;
    let fixture = materialize_fixture()?;
    let timeout = Duration::from_secs(args.timeout_secs.max(1));

    smoke_terminal(&binary, fixture.path(), timeout)?;
    smoke_score(&binary, fixture.path(), timeout)?;
    smoke_json(&binary, fixture.path(), &schema, timeout)?;
    smoke_sarif(&binary, fixture.path(), timeout)?;
    smoke_baseline(&binary, fixture.path(), &schema, timeout)?;
    smoke_failures(&binary, fixture.path(), &schema, timeout)?;
    smoke_no_default(&binary, &no_default, fixture.path(), &schema, timeout)?;
    smoke_mcp(&binary, fixture.path(), timeout)?;
    smoke_npm(&binary, &args.npm_root, &args.bun, timeout)?;
    smoke_archives(&binary, &args.archives, timeout)?;
    smoke_crate(&args.crate_package, timeout)?;
    Ok(())
}

fn smoke_terminal(binary: &Path, fixture: &Path, timeout: Duration) -> Result<()> {
    let output = invoke(binary, fixture, &["--offline"], timeout)?;
    require_success("terminal", &output)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains("rust-doctor") {
        return Err(EvalError::Command(
            "terminal smoke did not render the score surface".to_string(),
        ));
    }
    Ok(())
}

fn smoke_score(binary: &Path, fixture: &Path, timeout: Duration) -> Result<()> {
    let output = invoke(binary, fixture, &["--score", "--offline"], timeout)?;
    require_success("score", &output)?;
    let score = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|error| {
            EvalError::Command(format!("score smoke emitted a non-integer: {error}"))
        })?;
    if score > 100 {
        return Err(EvalError::Command(format!(
            "score smoke emitted out-of-range score {score}"
        )));
    }
    Ok(())
}

fn smoke_json(
    binary: &Path,
    fixture: &Path,
    schema: &jsonschema::Validator,
    timeout: Duration,
) -> Result<()> {
    let output = invoke(binary, fixture, &["--json-compact", "--offline"], timeout)?;
    require_success("JSON", &output)?;
    let report = parse_report("JSON", &output.stdout, schema)?;
    if report.diagnostics.is_empty() {
        return Err(EvalError::Command(
            "JSON smoke fixture produced no canonical diagnostic".to_string(),
        ));
    }
    Ok(())
}

fn smoke_sarif(binary: &Path, fixture: &Path, timeout: Duration) -> Result<()> {
    let output = invoke(binary, fixture, &["--sarif", "--offline"], timeout)?;
    require_success("SARIF", &output)?;
    let sarif: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        EvalError::Command(format!("SARIF smoke emitted invalid JSON: {error}"))
    })?;
    if sarif["version"] != "2.1.0" {
        return Err(EvalError::Command(
            "SARIF smoke emitted an unsupported version".to_string(),
        ));
    }
    let results = sarif["runs"][0]["results"]
        .as_array()
        .ok_or_else(|| EvalError::Command("SARIF smoke omitted runs[0].results".to_string()))?;
    if results.is_empty()
        || results.iter().any(|result| {
            result["ruleId"].as_str().is_none_or(str::is_empty)
                || result["partialFingerprints"]
                    .as_object()
                    .is_none_or(serde_json::Map::is_empty)
        })
    {
        return Err(EvalError::Command(
            "SARIF smoke requires stable rule IDs and partial fingerprints".to_string(),
        ));
    }
    Ok(())
}

fn smoke_baseline(
    binary: &Path,
    fixture: &Path,
    schema: &jsonschema::Validator,
    timeout: Duration,
) -> Result<()> {
    let output = invoke(
        binary,
        fixture,
        &[
            "--baseline",
            "--base",
            "HEAD",
            "--json-compact",
            "--offline",
        ],
        timeout,
    )?;
    require_success("baseline", &output)?;
    let report = parse_report("baseline", &output.stdout, schema)?;
    if report.mode != ScanMode::Baseline || report.baseline.is_none() {
        return Err(EvalError::Command(
            "baseline smoke omitted baseline comparison metadata".to_string(),
        ));
    }
    Ok(())
}

fn smoke_failures(
    binary: &Path,
    fixture: &Path,
    schema: &jsonschema::Validator,
    timeout: Duration,
) -> Result<()> {
    let malformed = tempfile::tempdir()
        .map_err(|error| EvalError::io("cannot create malformed fixture", ".", error))?;
    std::fs::write(malformed.path().join("Cargo.toml"), "not valid TOML = [").map_err(|error| {
        EvalError::io("cannot write malformed fixture", malformed.path(), error)
    })?;
    let output = invoke(
        binary,
        malformed.path(),
        &["--json-compact", "--offline"],
        timeout,
    )?;
    if output.status.success() {
        return Err(EvalError::Command(
            "malformed-project smoke unexpectedly succeeded".to_string(),
        ));
    }
    let report = parse_report("malformed project", &output.stdout, schema)?;
    if report.outcome != ReportOutcome::Failed {
        return Err(EvalError::Command(
            "malformed-project smoke did not emit a failed Report V1".to_string(),
        ));
    }

    std::fs::write(fixture.join(".rust-doctor-cache.json"), "corrupt cache")
        .map_err(|error| EvalError::io("cannot write corrupt cache fixture", fixture, error))?;
    let corrupt = invoke(binary, fixture, &["--json-compact", "--offline"], timeout)?;
    require_success("corrupt cache", &corrupt)?;
    parse_report("corrupt cache", &corrupt.stdout, schema)?;

    let invalid_config = fixture.join("rust-doctor.toml");
    std::fs::write(
        &invalid_config,
        "lint = true\ndependencies = false\n[rules.unknown-rule]\nseverity = \"error\"\n",
    )
    .map_err(|error| {
        EvalError::io(
            "cannot write invalid config fixture",
            &invalid_config,
            error,
        )
    })?;
    let invalid = invoke(binary, fixture, &["--json-compact", "--offline"], timeout)?;
    if invalid.status.success() {
        return Err(EvalError::Command(
            "invalid-configuration smoke unexpectedly succeeded".to_string(),
        ));
    }
    parse_report("invalid configuration", &invalid.stdout, schema)?;
    std::fs::write(&invalid_config, "lint = true\ndependencies = false\n")
        .map_err(|error| EvalError::io("cannot restore smoke config", &invalid_config, error))?;

    let destination = fixture.join("unwritable-json-output");
    std::fs::create_dir_all(&destination).map_err(|error| {
        EvalError::io("cannot create output failure fixture", &destination, error)
    })?;
    make_directory_unwritable(&destination)?;
    let report_path = destination.join("report.json");
    let output = invoke_with_path(
        binary,
        fixture,
        &["--json-out"],
        Some(&report_path),
        timeout,
    );
    let restored = make_directory_writable(&destination);
    let output = output?;
    restored?;
    if output.status.success() || !output.stdout.is_empty() {
        return Err(EvalError::Command(
            "unwritable JSON output smoke must fail with empty stdout".to_string(),
        ));
    }
    if String::from_utf8_lossy(&output.stderr).trim().is_empty() {
        return Err(EvalError::Command(
            "unwritable JSON output smoke omitted its recovery error".to_string(),
        ));
    }
    Ok(())
}

fn smoke_no_default(
    default_binary: &Path,
    no_default_binary: &Path,
    fixture: &Path,
    schema: &jsonschema::Validator,
    timeout: Duration,
) -> Result<()> {
    let default_version = version(default_binary, timeout)?;
    let no_default_version = version(no_default_binary, timeout)?;
    if default_version != no_default_version {
        return Err(EvalError::Command(format!(
            "default and no-default binaries report different versions: {default_version:?} vs {no_default_version:?}"
        )));
    }
    let output = invoke(
        no_default_binary,
        fixture,
        &["--json-compact", "--offline"],
        timeout,
    )?;
    require_success("no-default-features CLI", &output)?;
    parse_report("no-default-features CLI", &output.stdout, schema)?;
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "the ordered JSON-RPC handshake is kept as one built-server conversation"
)]
fn smoke_mcp(binary: &Path, fixture: &Path, timeout: Duration) -> Result<()> {
    let mut command = Command::new(binary);
    command
        .arg("--mcp")
        .env("RUST_DOCTOR_MCP_ROOT", fixture)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child =
        KillOnDrop(command.spawn().map_err(|error| {
            EvalError::Command(format!("cannot launch MCP smoke server: {error}"))
        })?);
    let mut stdin = child
        .0
        .stdin
        .take()
        .ok_or_else(|| EvalError::Command("cannot open MCP stdin".to_string()))?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or_else(|| EvalError::Command("cannot open MCP stdout".to_string()))?;
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    send_mcp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "rust-doctor-eval", "version": "1.0"}
            }
        }),
    )?;
    let initialize = receive_mcp(&receiver, 1, timeout)?;
    if initialize.get("result").is_none() {
        return Err(EvalError::Command(format!(
            "MCP initialize failed: {initialize}"
        )));
    }
    send_mcp(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}),
    )?;
    send_mcp(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )?;
    let tools = receive_mcp(&receiver, 2, timeout)?;
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .ok_or_else(|| EvalError::Command("MCP list-tools omitted tools".to_string()))?
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for required in ["scan", "score", "explain_rule", "list_rules"] {
        if !names.contains(&required) {
            return Err(EvalError::Command(format!(
                "MCP list-tools omitted {required}"
            )));
        }
    }
    send_mcp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "scan",
                "arguments": {
                    "directory": fixture.to_string_lossy(),
                    "offline": true,
                    "ignore_project_config": true
                }
            }
        }),
    )?;
    let scan = receive_mcp(&receiver, 3, timeout)?;
    if scan.get("result").is_none() {
        return Err(EvalError::Command(format!("MCP scan failed: {scan}")));
    }
    send_mcp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "scan",
                "arguments": {
                    "directory": fixture.to_string_lossy(),
                    "diff": "HEAD",
                    "offline": true,
                    "ignore_project_config": true
                }
            }
        }),
    )?;
    let baseline = receive_mcp(&receiver, 4, timeout)?;
    if baseline.get("result").is_none() {
        return Err(EvalError::Command(format!(
            "MCP baseline scope failed: {baseline}"
        )));
    }

    send_mcp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "scan",
                "arguments": {
                    "directory": fixture.to_string_lossy(),
                    "offline": true,
                    "ignore_project_config": true
                }
            }
        }),
    )?;
    send_mcp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {"requestId": 5, "reason": "artifact smoke cancellation"}
        }),
    )?;
    let cancellation_deadline = timeout.min(Duration::from_secs(2));
    let cancelled = receive_mcp(&receiver, 5, cancellation_deadline)?;
    if cancelled.get("error").is_none() || cancelled.get("result").is_some() {
        return Err(EvalError::Command(format!(
            "MCP cancellation was not observed before a normal response: {cancelled}"
        )));
    }
    send_mcp(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "id": 6, "method": "tools/list", "params": {}}),
    )?;
    let after_cancellation = receive_mcp(&receiver, 6, timeout)?;
    if after_cancellation.get("result").is_none() {
        return Err(EvalError::Command(format!(
            "MCP server was unusable after cancellation: {after_cancellation}"
        )));
    }
    drop(stdin);
    drop(child);
    let _ = reader.join();
    Ok(())
}

fn smoke_npm(binary: &Path, npm_root: &Path, bun: &Path, timeout: Duration) -> Result<()> {
    let platform = platform_package()?;
    let wrapper_source = npm_root.join("rust-doctor");
    let platform_source = npm_root.join(platform);
    if !wrapper_source.is_dir() || !platform_source.is_dir() {
        return Err(EvalError::Command(format!(
            "npm smoke is missing {} or {}",
            wrapper_source.display(),
            platform_source.display()
        )));
    }
    let root = tempfile::Builder::new()
        .prefix("rust-doctor-npm-smoke-")
        .tempdir()
        .map_err(|error| EvalError::io("cannot create npm smoke directory", ".", error))?;
    let wrapper = root.path().join("wrapper");
    let platform_dir = root.path().join("platform");
    copy_tree(&wrapper_source, &wrapper)?;
    copy_tree(&platform_source, &platform_dir)?;
    let binary_name = if cfg!(windows) {
        "rust-doctor.exe"
    } else {
        "rust-doctor"
    };
    let embedded = platform_dir.join("bin").join(binary_name);
    if let Some(parent) = embedded.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| EvalError::io("cannot create npm binary directory", parent, error))?;
    }
    std::fs::copy(binary, &embedded)
        .map_err(|error| EvalError::io("cannot embed npm smoke binary", &embedded, error))?;
    set_executable(&embedded)?;
    let source_hash = hex_digest(
        &std::fs::read(binary)
            .map_err(|error| EvalError::io("cannot hash source binary", binary, error))?,
    );
    let embedded_hash = hex_digest(
        &std::fs::read(&embedded)
            .map_err(|error| EvalError::io("cannot hash embedded binary", &embedded, error))?,
    );
    if source_hash != embedded_hash {
        return Err(EvalError::Command(
            "npm platform package did not embed the exact built binary".to_string(),
        ));
    }
    validate_package_versions(&wrapper, &platform_dir)?;
    let packs = root.path().join("packs");
    std::fs::create_dir_all(&packs)
        .map_err(|error| EvalError::io("cannot create npm pack directory", &packs, error))?;
    pack(bun, &platform_dir, &packs, "platform.tgz", timeout)?;
    pack(bun, &wrapper, &packs, "wrapper.tgz", timeout)?;
    let install = root.path().join("install");
    std::fs::create_dir_all(&install)
        .map_err(|error| EvalError::io("cannot create npm install directory", &install, error))?;
    std::fs::write(
        install.join("package.json"),
        "{\"name\":\"rust-doctor-smoke\",\"private\":true}",
    )
    .map_err(|error| EvalError::io("cannot write npm smoke package", &install, error))?;
    let mut add = Command::new(bun);
    add.current_dir(&install)
        .args(["add", "--no-save"])
        .arg(packs.join("platform.tgz"))
        .arg(packs.join("wrapper.tgz"));
    require_success(
        "packed npm installation",
        &run_capped(add, timeout, OUTPUT_CAP)?,
    )?;
    let wrapper_bin = install.join("node_modules/rust-doctor/bin/rust-doctor.js");
    let mut launch = Command::new(bun);
    launch
        .current_dir(&install)
        .arg(&wrapper_bin)
        .arg("--version");
    let output = run_capped(launch, timeout, OUTPUT_CAP)?;
    require_success("packed npm wrapper", &output)?;
    let expected = version(binary, timeout)?;
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if actual != expected {
        return Err(EvalError::Command(format!(
            "packed npm wrapper reports {actual:?}, expected {expected:?}"
        )));
    }
    Ok(())
}

fn smoke_archives(binary: &Path, archives: &[PathBuf], timeout: Duration) -> Result<()> {
    let expected = version(binary, timeout)?;
    let expected_bytes = std::fs::read(binary)
        .map_err(|error| EvalError::io("cannot read smoke binary", binary, error))?;
    let expected_hash = hex_digest(&expected_bytes);
    let binary_name = if cfg!(windows) {
        "rust-doctor.exe"
    } else {
        "rust-doctor"
    };
    for archive in archives {
        let archive = archive
            .canonicalize()
            .map_err(|error| EvalError::io("cannot canonicalize native archive", archive, error))?;
        let private_archive = private_archive_copy(&archive)?;
        let entries = validate_archive(private_archive.path(), timeout)?;
        if entries != [binary_name] {
            return Err(EvalError::Command(format!(
                "native archive '{}' must contain only root-level {binary_name}",
                archive.display()
            )));
        }
        let extraction = tempfile::Builder::new()
            .prefix("rust-doctor-archive-smoke-")
            .tempdir()
            .map_err(|error| EvalError::io("cannot create archive extraction", ".", error))?;
        let extracted = extraction.path().join(binary_name);
        let mut extract = Command::new("tar");
        extract
            .args(["-xOf"])
            .arg(private_archive.path())
            .arg(binary_name);
        let output = run_capped(extract, timeout, expected_bytes.len().saturating_add(1))?;
        require_success("native archive extraction", &output)?;
        if output.stdout != expected_bytes || hex_digest(&output.stdout) != expected_hash {
            return Err(EvalError::Command(format!(
                "native archive '{}' does not contain the exact built binary",
                archive.display()
            )));
        }
        std::fs::write(&extracted, &output.stdout).map_err(|error| {
            EvalError::io("cannot materialize archived binary", &extracted, error)
        })?;
        set_executable(&extracted)?;
        let actual = version(&extracted, timeout)?;
        if actual != expected {
            return Err(EvalError::Command(format!(
                "extracted archive '{}' reports {actual:?}, expected {expected:?}",
                archive.display()
            )));
        }
    }
    Ok(())
}

fn smoke_crate(crate_package: &Path, timeout: Duration) -> Result<()> {
    let crate_package = crate_package.canonicalize().map_err(|error| {
        EvalError::io("cannot canonicalize Cargo package", crate_package, error)
    })?;
    let private_package = private_archive_copy(&crate_package)?;
    let extraction = tempfile::Builder::new()
        .prefix("rust-doctor-crate-smoke-")
        .tempdir()
        .map_err(|error| EvalError::io("cannot create crate extraction", ".", error))?;
    validate_archive(private_package.path(), timeout)?;
    let mut extract = Command::new("tar");
    extract
        .args(["-xf"])
        .arg(private_package.path())
        .args(["-C"])
        .arg(extraction.path());
    require_success(
        "Cargo package extraction",
        &run_capped(extract, timeout, OUTPUT_CAP)?,
    )?;
    let package_root = std::fs::read_dir(extraction.path())
        .map_err(|error| EvalError::io("cannot inspect Cargo package", extraction.path(), error))?
        .filter_map(std::result::Result::ok)
        .find(|entry| entry.path().join("Cargo.toml").is_file())
        .map(|entry| entry.path())
        .ok_or_else(|| {
            EvalError::Command("verified .crate contains no package Cargo.toml".to_string())
        })?;
    let manifest_path = package_root.join("Cargo.toml");
    let manifest: toml::Value =
        toml::from_str(&std::fs::read_to_string(&manifest_path).map_err(|error| {
            EvalError::io("cannot read packaged manifest", &manifest_path, error)
        })?)
        .map_err(|error| {
            EvalError::Command(format!(
                "verified .crate contains an invalid Cargo.toml: {error}"
            ))
        })?;
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| {
            EvalError::Command("verified .crate manifest has no package table".to_string())
        })?;
    let packaged_name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    let packaged_version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    if packaged_name != env!("CARGO_PKG_NAME") || packaged_version != env!("CARGO_PKG_VERSION") {
        return Err(EvalError::Command(format!(
            "verified .crate reports {packaged_name}@{packaged_version}, expected {}@{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        )));
    }
    let target = extraction.path().join("target");
    let mut check = Command::new("cargo");
    check
        .current_dir(&package_root)
        .args(["check", "--locked", "--offline", "--no-default-features"])
        .env("CARGO_TARGET_DIR", target);
    require_success(
        "verified Cargo package build",
        &run_capped(check, timeout, OUTPUT_CAP)?,
    )
}

fn private_archive_copy(archive: &Path) -> Result<tempfile::NamedTempFile> {
    let mut source = std::fs::File::open(archive).map_err(|error| {
        EvalError::io("cannot open archive for private validation", archive, error)
    })?;
    let mut private = tempfile::NamedTempFile::new()
        .map_err(|error| EvalError::io("cannot create private archive copy", ".", error))?;
    std::io::copy(&mut source, &mut private).map_err(|error| {
        EvalError::io("cannot copy archive for private validation", archive, error)
    })?;
    private.as_file_mut().sync_all().map_err(|error| {
        EvalError::io("cannot sync private archive copy", private.path(), error)
    })?;
    Ok(private)
}

fn validate_archive(archive: &Path, timeout: Duration) -> Result<Vec<String>> {
    let mut list = Command::new("tar");
    list.args(["-tf"]).arg(archive);
    let listing = run_capped(list, timeout, 1024 * 1024)?;
    require_success("archive listing", &listing)?;
    let entries: Vec<_> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .map(str::trim_end)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect();
    if entries.is_empty() || entries.len() > ARCHIVE_ENTRY_LIMIT {
        return Err(EvalError::Command(format!(
            "archive '{}' has an invalid entry count",
            archive.display()
        )));
    }
    for entry in &entries {
        let normalized = entry.trim_end_matches('/');
        if normalized.is_empty()
            || entry.contains('\\')
            || !Path::new(normalized)
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
        {
            return Err(EvalError::Command(format!(
                "archive '{}' contains an unsafe path",
                archive.display()
            )));
        }
    }
    let mut verbose = Command::new("tar");
    verbose.args(["-tvf"]).arg(archive);
    let types = run_capped(verbose, timeout, 4 * 1024 * 1024)?;
    require_success("archive type listing", &types)?;
    let entry_types: Vec<_> = String::from_utf8_lossy(&types.stdout)
        .lines()
        .filter_map(|line| line.chars().find(|character| !character.is_whitespace()))
        .collect();
    if entry_types.len() != entries.len()
        || entry_types
            .iter()
            .any(|entry_type| !matches!(entry_type, '-' | 'd'))
    {
        return Err(EvalError::Command(format!(
            "archive '{}' contains links or special files",
            archive.display()
        )));
    }
    let mut expanded = Command::new("tar");
    expanded.args(["-xOf"]).arg(archive);
    require_success(
        "archive expanded-size validation",
        &run_capped(expanded, timeout, ARCHIVE_EXPANDED_CAP)?,
    )?;
    Ok(entries)
}

#[cfg(unix)]
fn make_directory_unwritable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| EvalError::io("cannot inspect output fixture", path, error))?
        .permissions();
    permissions.set_mode(0o555);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| EvalError::io("cannot protect output fixture", path, error))?;
    if std::fs::write(path.join("permission-probe"), b"probe").is_ok() {
        make_directory_writable(path)?;
        return Err(EvalError::Unsupported(
            "artifact smoke must run without root write bypass to prove unwritable output handling"
                .to_string(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn make_directory_writable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| EvalError::io("cannot inspect output fixture", path, error))?
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| EvalError::io("cannot restore output fixture", path, error))
}

#[cfg(windows)]
fn make_directory_unwritable(path: &Path) -> Result<()> {
    let mut command = Command::new("icacls");
    command
        .arg(path)
        .args(["/inheritance:r", "/deny", "*S-1-1-0:(OI)(CI)(W)"]);
    require_success(
        "protect Windows output fixture",
        &run_capped(command, Duration::from_secs(15), 1024 * 1024)?,
    )?;
    if std::fs::write(path.join("permission-probe"), b"probe").is_ok() {
        make_directory_writable(path)?;
        return Err(EvalError::Unsupported(
            "Windows ACL did not make the output fixture unwritable".to_string(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn make_directory_writable(path: &Path) -> Result<()> {
    let mut command = Command::new("icacls");
    command.arg(path).arg("/reset").arg("/T").arg("/C");
    require_success(
        "restore Windows output fixture",
        &run_capped(command, Duration::from_secs(15), 1024 * 1024)?,
    )
}

#[cfg(not(any(unix, windows)))]
fn make_directory_unwritable(_path: &Path) -> Result<()> {
    Err(EvalError::Unsupported(
        "genuinely unwritable output smoke requires Unix modes or Windows ACLs".to_string(),
    ))
}

#[cfg(not(any(unix, windows)))]
fn make_directory_writable(_path: &Path) -> Result<()> {
    Ok(())
}

fn invoke(
    binary: &Path,
    fixture: &Path,
    arguments: &[&str],
    timeout: Duration,
) -> Result<ProcessOutput> {
    invoke_with_path(binary, fixture, arguments, None, timeout)
}

fn invoke_with_path(
    binary: &Path,
    fixture: &Path,
    arguments: &[&str],
    path_argument: Option<&Path>,
    timeout: Duration,
) -> Result<ProcessOutput> {
    let mut command = Command::new(binary);
    command.arg(fixture).args(arguments);
    if let Some(path) = path_argument {
        command.arg(path);
    }
    run_capped(command, timeout, OUTPUT_CAP)
}

fn require_success(surface: &str, output: &ProcessOutput) -> Result<()> {
    if output.timed_out {
        return Err(EvalError::Command(format!("{surface} smoke timed out")));
    }
    if output.output_overflow {
        return Err(EvalError::Command(format!(
            "{surface} smoke exceeded the output cap"
        )));
    }
    if !output.status.success() {
        return Err(EvalError::Command(format!(
            "{surface} smoke failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn parse_report(surface: &str, bytes: &[u8], schema: &jsonschema::Validator) -> Result<ReportV1> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| EvalError::Command(format!("{surface} emitted invalid JSON: {error}")))?;
    validate_schema(surface, &value, schema)?;
    serde_json::from_value(value).map_err(|error| {
        EvalError::Command(format!("{surface} is not Report V1 compatible: {error}"))
    })
}

fn load_schema(path: &Path) -> Result<jsonschema::Validator> {
    let bytes = std::fs::read(path)
        .map_err(|error| EvalError::io("cannot read Report V1 schema", path, error))?;
    let schema: Value = serde_json::from_slice(&bytes).map_err(|source| EvalError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    jsonschema::validator_for(&schema)
        .map_err(|error| EvalError::Command(format!("Report V1 schema is invalid: {error}")))
}

fn validate_schema(surface: &str, value: &Value, schema: &jsonschema::Validator) -> Result<()> {
    let errors: Vec<_> = schema
        .iter_errors(value)
        .take(8)
        .map(|error| error.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(EvalError::Command(format!(
            "{surface} does not validate as Report V1: {}",
            errors.join("; ")
        )))
    }
}

fn materialize_fixture() -> Result<tempfile::TempDir> {
    let fixture = tempfile::Builder::new()
        .prefix("rust-doctor-artifact-smoke-")
        .tempdir()
        .map_err(|error| EvalError::io("cannot create artifact fixture", ".", error))?;
    std::fs::create_dir_all(fixture.path().join("src"))
        .map_err(|error| EvalError::io("cannot create artifact source", fixture.path(), error))?;
    std::fs::write(
        fixture.path().join("Cargo.toml"),
        "[package]\nname = \"artifact-smoke\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.85\"\n",
    )
    .map_err(|error| EvalError::io("cannot write artifact manifest", fixture.path(), error))?;
    std::fs::write(
        fixture.path().join("rust-doctor.toml"),
        "lint = true\ndependencies = false\n",
    )
    .map_err(|error| EvalError::io("cannot write artifact config", fixture.path(), error))?;
    std::fs::write(
        fixture.path().join("src/lib.rs"),
        "pub fn value(input: Option<u8>) -> u8 { input.unwrap_or_default() }\n",
    )
    .map_err(|error| EvalError::io("cannot write artifact source", fixture.path(), error))?;
    git(fixture.path(), &["init", "--quiet"])?;
    git(fixture.path(), &["config", "user.email", "smoke@localhost"])?;
    git(fixture.path(), &["config", "user.name", "Smoke"])?;
    git(fixture.path(), &["add", "."])?;
    git(fixture.path(), &["commit", "--quiet", "-m", "base"])?;
    std::fs::write(
        fixture.path().join("src/lib.rs"),
        "pub fn value(input: Option<u8>) -> u8 { input.unwrap() }\n",
    )
    .map_err(|error| EvalError::io("cannot mutate artifact source", fixture.path(), error))?;
    Ok(fixture)
}

fn git(root: &Path, arguments: &[&str]) -> Result<()> {
    let mut command = Command::new("git");
    command.current_dir(root).args(arguments);
    let output = run_capped(command, Duration::from_secs(20), 1024 * 1024)?;
    require_success("fixture Git", &output)
}

fn version(binary: &Path, timeout: Duration) -> Result<String> {
    let mut command = Command::new(binary);
    command.arg("--version");
    let output = run_capped(command, timeout, 64 * 1024)?;
    require_success("version", &output)?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn canonical_binary(path: &Path, label: &str) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .map_err(|error| EvalError::io("cannot canonicalize smoke binary", path, error))?;
    if !canonical.is_file() {
        return Err(EvalError::Command(format!(
            "{label} '{}' is not a file",
            path.display()
        )));
    }
    Ok(canonical)
}

fn send_mcp(stdin: &mut impl Write, message: &Value) -> Result<()> {
    serde_json::to_writer(&mut *stdin, message)
        .map_err(|error| EvalError::Command(format!("cannot encode MCP message: {error}")))?;
    stdin
        .write_all(b"\n")
        .and_then(|()| stdin.flush())
        .map_err(|error| EvalError::Command(format!("cannot send MCP message: {error}")))
}

fn receive_mcp(
    receiver: &mpsc::Receiver<std::io::Result<String>>,
    id: u64,
    timeout: Duration,
) -> Result<Value> {
    loop {
        let line = receiver
            .recv_timeout(timeout)
            .map_err(|error| EvalError::Command(format!("MCP response {id} timed out: {error}")))?
            .map_err(|error| EvalError::Command(format!("cannot read MCP response: {error}")))?;
        let message: Value = serde_json::from_str(&line)
            .map_err(|error| EvalError::Command(format!("invalid MCP response: {error}")))?;
        if message["id"].as_u64() == Some(id) {
            return Ok(message);
        }
    }
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn platform_package() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "x86_64") => Ok("darwin-x64"),
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("windows", "x86_64") => Ok("win32-x64"),
        (os, arch) => Err(EvalError::Unsupported(format!(
            "npm artifact smoke does not support {os}-{arch}"
        ))),
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)
        .map_err(|error| EvalError::io("cannot create npm package copy", destination, error))?;
    for entry in std::fs::read_dir(source)
        .map_err(|error| EvalError::io("cannot read npm package", source, error))?
    {
        let entry =
            entry.map_err(|error| EvalError::io("cannot read npm package entry", source, error))?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| EvalError::io("cannot inspect npm package", &path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(EvalError::Command(format!(
                "npm smoke refuses package symlink '{}'",
                path.display()
            )));
        }
        if metadata.is_dir() {
            copy_tree(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)
                .map_err(|error| EvalError::io("cannot copy npm package file", &path, error))?;
        }
    }
    Ok(())
}

fn validate_package_versions(wrapper: &Path, platform: &Path) -> Result<()> {
    let wrapper_json = package_json(&wrapper.join("package.json"))?;
    let platform_json = package_json(&platform.join("package.json"))?;
    let expected = env!("CARGO_PKG_VERSION");
    if wrapper_json["version"] != expected || platform_json["version"] != expected {
        return Err(EvalError::Command(format!(
            "npm package metadata must match Cargo version {expected}"
        )));
    }
    let package_name = platform_json["name"]
        .as_str()
        .ok_or_else(|| EvalError::Command("platform package has no name".to_string()))?;
    if wrapper_json["optionalDependencies"][package_name] != expected {
        return Err(EvalError::Command(format!(
            "wrapper dependency {package_name} must match Cargo version {expected}"
        )));
    }
    Ok(())
}

fn package_json(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path)
        .map_err(|error| EvalError::io("cannot read npm package.json", path, error))?;
    serde_json::from_slice(&bytes).map_err(|source| EvalError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn pack(
    bun: &Path,
    package: &Path,
    destination: &Path,
    filename: &str,
    timeout: Duration,
) -> Result<()> {
    let mut command = Command::new(bun);
    command
        .current_dir(package)
        .args(["pm", "pack", "--ignore-scripts", "--filename", filename]);
    require_success("npm pack", &run_capped(command, timeout, OUTPUT_CAP)?)?;

    let archive = package.join(filename);
    let target = destination.join(filename);
    std::fs::rename(&archive, &target)
        .map_err(|error| EvalError::io("cannot move npm package archive", &target, error))
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| EvalError::io("cannot inspect embedded binary", path, error))?
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| EvalError::io("cannot mark embedded binary executable", path, error))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_2020_12_validation_rejects_nested_type_errors() {
        let schema = jsonschema::validator_for(&json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {"summary": {"type": "object", "required": ["score"]}},
            "required": ["summary"]
        }))
        .unwrap();
        assert!(validate_schema("fixture", &json!({"summary": []}), &schema).is_err());
    }

    #[test]
    fn platform_package_matches_supported_release_targets() {
        assert!(platform_package().is_ok());
    }
}
