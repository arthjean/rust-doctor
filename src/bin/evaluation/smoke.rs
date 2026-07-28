use super::manifest::hex_digest;
use super::process::{ProcessOutput, run_capped};
use super::{EvalError, Result};
use crate::SmokeArgs;
use rust_doctor::api::{ScanRequest, scan as library_scan};
use rust_doctor::config::AdapterPolicy;
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecisionProjection {
    score: Option<u64>,
    label: Option<String>,
    authoritative: bool,
    reason_codes: Vec<String>,
    error_count: u64,
    warning_count: u64,
    info_count: u64,
    top_root_causes: Vec<String>,
    top_rules: Vec<String>,
    scored_groups: usize,
    advisory_groups: usize,
    audit_groups: usize,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the command handler owns its parsed Clap subcommand"
)]
pub(crate) fn run(args: SmokeArgs) -> Result<()> {
    let binary = canonical_binary(&args.binary, "default-feature binary")?;
    let no_default = canonical_binary(&args.no_default_binary, "no-default-features binary")?;
    let lsp_binary = canonical_binary(&args.lsp_binary, "LSP-feature binary")?;
    let schema = load_schema(&args.schema)?;
    let fixture = materialize_fixture()?;
    let timeout = Duration::from_secs(args.timeout_secs.max(1));

    let decision = smoke_json(&binary, fixture.path(), &schema, timeout)?;
    smoke_library(fixture.path(), &decision)?;
    smoke_terminal(&binary, fixture.path(), &decision, timeout)?;
    smoke_score(&binary, fixture.path(), &decision, timeout)?;
    smoke_sarif(&binary, fixture.path(), &decision, timeout)?;
    smoke_baseline(&binary, fixture.path(), &schema, timeout)?;
    smoke_failures(&binary, fixture.path(), &schema, timeout)?;
    smoke_no_default(
        &binary,
        &no_default,
        fixture.path(),
        &schema,
        &decision,
        timeout,
    )?;
    smoke_mcp(&binary, fixture.path(), &decision, timeout)?;
    smoke_lsp(&lsp_binary, fixture.path(), &decision, timeout)?;
    smoke_npm(
        &binary,
        &args.npm_platform_package,
        &args.npm_wrapper_package,
        &args.bun,
        fixture.path(),
        &schema,
        &decision,
        timeout,
    )?;
    smoke_archives(
        &binary,
        &args.archives,
        fixture.path(),
        &schema,
        &decision,
        timeout,
    )?;
    smoke_crate(&args.crate_package, timeout)?;
    smoke_action(&args.action, &binary, fixture.path(), &decision, timeout)?;
    Ok(())
}

fn smoke_library(fixture: &Path, expected: &DecisionProjection) -> Result<()> {
    let mut request = ScanRequest::new(fixture);
    request.options.adapters = AdapterPolicy {
        compiler_lint: true,
        custom_ast: true,
        supply_chain: false,
        quality: false,
        network: false,
    };
    let report = library_scan(request)
        .map_err(|error| EvalError::Command(format!("library API smoke failed: {error}")))?;
    require_projection(
        "library API",
        &serde_json::to_value(report).map_err(|error| {
            EvalError::Command(format!("cannot project library report: {error}"))
        })?,
        expected,
    )
}

fn smoke_terminal(
    binary: &Path,
    fixture: &Path,
    expected: &DecisionProjection,
    timeout: Duration,
) -> Result<()> {
    let output = invoke(binary, fixture, &["--offline"], timeout)?;
    require_success("terminal", &output)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains("rust-doctor") {
        return Err(EvalError::Command(
            "terminal smoke did not render the score surface".to_string(),
        ));
    }
    let rendered = format!("{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
    require_rendered_decision("terminal", &rendered, expected)?;
    Ok(())
}

fn smoke_score(
    binary: &Path,
    fixture: &Path,
    expected: &DecisionProjection,
    timeout: Duration,
) -> Result<()> {
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
    if Some(u64::from(score)) != expected.score {
        return Err(EvalError::Command(
            "score surface disagrees with canonical Report V1".to_string(),
        ));
    }
    Ok(())
}

fn smoke_json(
    binary: &Path,
    fixture: &Path,
    schema: &jsonschema::Validator,
    timeout: Duration,
) -> Result<DecisionProjection> {
    let output = invoke(
        binary,
        fixture,
        &["--json", "--json-compact", "--offline"],
        timeout,
    )?;
    require_success("JSON", &output)?;
    let report = parse_report("JSON", &output.stdout, schema)?;
    if report.diagnostics.is_empty() {
        return Err(EvalError::Command(
            "JSON smoke fixture produced no canonical diagnostic".to_string(),
        ));
    }
    decision_projection(&serde_json::to_value(report).map_err(|error| {
        EvalError::Command(format!("cannot project JSON smoke report: {error}"))
    })?)
}

fn smoke_sarif(
    binary: &Path,
    fixture: &Path,
    expected: &DecisionProjection,
    timeout: Duration,
) -> Result<()> {
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
    let properties = &sarif["runs"][0]["properties"];
    if properties["rustDoctorScore"].as_u64() != expected.score
        || properties["rustDoctorScoreLabel"].as_str() != expected.label.as_deref()
        || properties["rustDoctorScoreAuthoritative"].as_bool() != Some(expected.authoritative)
        || properties["rustDoctorScoreReasons"]
            != serde_json::to_value(&expected.reason_codes).unwrap_or_default()
    {
        return Err(EvalError::Command(
            "SARIF decision metadata disagrees with canonical Report V1".to_string(),
        ));
    }
    let top = first_unique(
        results
            .iter()
            .filter_map(|result| {
                result["properties"]["rootCauseKey"]
                    .as_str()
                    .or_else(|| result["ruleId"].as_str())
            })
            .map(str::to_string),
        3,
    );
    if top != expected.top_root_causes {
        return Err(EvalError::Command(format!(
            "SARIF top remediation order {top:?} disagrees with canonical Report V1 {:?}",
            expected.top_root_causes
        )));
    }
    let rules = sarif["runs"][0]["tool"]["driver"]["rules"]
        .as_array()
        .ok_or_else(|| EvalError::Command("SARIF omitted its rule inventory".to_string()))?;
    let mut scored_groups = 0usize;
    let mut advisory_groups = 0usize;
    let mut audit_groups = 0usize;
    for result in results {
        let rule_id = result["ruleId"].as_str().unwrap_or_default();
        let audit = rules.iter().any(|rule| {
            rule["id"].as_str() == Some(rule_id) && rule["properties"]["trustTier"] == "audit-only"
        });
        if audit {
            audit_groups += 1;
        } else if result["properties"]["scoreImpact"] == "scored" {
            scored_groups += 1;
        } else {
            advisory_groups += 1;
        }
    }
    if (scored_groups, advisory_groups, audit_groups)
        != (
            expected.scored_groups,
            expected.advisory_groups,
            expected.audit_groups,
        )
    {
        return Err(EvalError::Command(
            "SARIF inventory totals disagree with canonical Report V1".to_string(),
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
            "--json",
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
        &["--json", "--json-compact", "--offline"],
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
    let corrupt = invoke(
        binary,
        fixture,
        &["--json", "--json-compact", "--offline"],
        timeout,
    )?;
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
    let invalid = invoke(
        binary,
        fixture,
        &["--json", "--json-compact", "--offline"],
        timeout,
    )?;
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
        &["--json", "--json-out"],
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
    expected: &DecisionProjection,
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
        &["--json", "--json-compact", "--offline"],
        timeout,
    )?;
    require_success("no-default-features CLI", &output)?;
    let report = parse_report("no-default-features CLI", &output.stdout, schema)?;
    require_projection(
        "no-default-features CLI",
        &serde_json::to_value(report).map_err(|error| {
            EvalError::Command(format!("cannot project no-default report: {error}"))
        })?,
        expected,
    )?;
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "the complete LSP handshake and decision command stay in one bounded server conversation"
)]
fn smoke_lsp(
    binary: &Path,
    fixture: &Path,
    expected: &DecisionProjection,
    timeout: Duration,
) -> Result<()> {
    let mut command = Command::new(binary);
    command
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child =
        KillOnDrop(command.spawn().map_err(|error| {
            EvalError::Command(format!("cannot launch LSP smoke server: {error}"))
        })?);
    let mut stdin = child
        .0
        .stdin
        .take()
        .ok_or_else(|| EvalError::Command("cannot open LSP stdin".to_string()))?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or_else(|| EvalError::Command("cannot open LSP stdout".to_string()))?;
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(message)) = read_lsp_message(&mut reader) {
            if sender.send(message).is_err() {
                break;
            }
        }
    });
    send_lsp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": format!("file://{}", fixture.to_string_lossy()),
                "capabilities": {},
                "clientInfo": {"name": "rust-doctor-eval", "version": "1.0"},
                "initializationOptions": {"protocolMajor": 1, "projectBudgetMs": 60000}
            }
        }),
    )?;
    let initialize = receive_lsp(&receiver, 1, timeout)?;
    if initialize["result"]["capabilities"].as_object().is_none() {
        return Err(EvalError::Command(format!(
            "LSP initialize omitted server capabilities: {initialize}"
        )));
    }
    send_lsp(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
    )?;
    send_lsp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "workspace/executeCommand",
            "params": {"command": "rust-doctor.projectDecision", "arguments": []}
        }),
    )?;
    let decision = receive_lsp(&receiver, 2, timeout)?;
    if let Some(error) = decision.get("error") {
        return Err(EvalError::Command(format!(
            "LSP project decision failed: {error}"
        )));
    }
    require_projection("LSP", &decision["result"], expected)?;
    send_lsp(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "id": 3, "method": "shutdown", "params": null}),
    )?;
    let shutdown = receive_lsp(&receiver, 3, timeout)?;
    if shutdown.get("error").is_some() {
        return Err(EvalError::Command(format!(
            "LSP shutdown failed: {shutdown}"
        )));
    }
    send_lsp(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "exit", "params": null}),
    )?;
    drop(stdin);
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if child
            .0
            .try_wait()
            .map_err(|error| EvalError::Command(format!("cannot wait for LSP server: {error}")))?
            .is_some()
        {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(EvalError::Command(
                "LSP server did not exit after shutdown".to_string(),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
    drop(child);
    let _ = reader.join();
    Ok(())
}

fn smoke_action(
    action: &Path,
    binary: &Path,
    fixture: &Path,
    expected: &DecisionProjection,
    timeout: Duration,
) -> Result<()> {
    let action = action
        .canonicalize()
        .map_err(|error| EvalError::io("cannot canonicalize Action entrypoint", action, error))?;
    let source = std::fs::read_to_string(&action)
        .map_err(|error| EvalError::io("cannot read Action entrypoint", &action, error))?;
    let repository_root = action.parent().ok_or_else(|| {
        EvalError::InvalidManifest("Action entrypoint has no repository root".to_string())
    })?;
    for script in [
        "scripts/action/prepare.sh",
        "scripts/action/report.sh",
        "scripts/action/sarif.sh",
    ] {
        if !source.contains(script) {
            return Err(EvalError::Command(format!(
                "Action entrypoint omits {script}"
            )));
        }
        let script_path = repository_root.join(script);
        let mut command = Command::new("bash");
        command.args(["-n"]).arg(&script_path);
        require_success(
            "Action entrypoint",
            &run_capped(command, timeout, 1024 * 1024)?,
        )?;
    }
    let report = tempfile::NamedTempFile::new()
        .map_err(|error| EvalError::io("cannot create Action report", ".", error))?;
    let output = invoke_with_path(
        binary,
        fixture,
        &["--json", "--json-compact", "--offline", "--json-out"],
        Some(report.path()),
        timeout,
    )?;
    require_success("Action report source", &output)?;
    let summary = tempfile::NamedTempFile::new()
        .map_err(|error| EvalError::io("cannot create Action summary", ".", error))?;
    let report_script = repository_root.join("scripts/action/report.sh");
    let mut command = Command::new("bash");
    command
        .arg(report_script)
        .env("SERVER_URL", "https://example.invalid")
        .env("REPOSITORY", "owner/repository")
        .env("RUN_ID", "1")
        .env("RUN_ATTEMPT", "1")
        .env("SKIP_SCAN", "false")
        .env("DEGRADED_REASON", "")
        .env("EXIT_CODE", "0")
        .env("REPORT_FILE", report.path())
        .env("COMMIT_STATUS_ENABLED", "false")
        .env("REVIEW_COMMENTS_ENABLED", "false")
        .env("COMMENT_ENABLED", "false")
        .env("EVENT_NAME", "push")
        .env("PR_NUMBER", "")
        .env("GIT_ROOT", fixture)
        .env("SCAN_ROOT", fixture)
        .env("GITHUB_STEP_SUMMARY", summary.path());
    require_success(
        "Action decision rendering",
        &run_capped(command, timeout, OUTPUT_CAP)?,
    )?;
    let rendered = std::fs::read_to_string(summary.path())
        .map_err(|error| EvalError::io("cannot read Action summary", summary.path(), error))?;
    require_rendered_decision("Action", &rendered, expected)?;
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "the ordered JSON-RPC handshake is kept as one built-server conversation"
)]
fn smoke_mcp(
    binary: &Path,
    fixture: &Path,
    expected: &DecisionProjection,
    timeout: Duration,
) -> Result<()> {
    let mut command = Command::new(binary);
    command
        .arg("--mcp")
        .env("RUST_DOCTOR_MCP_ROOT", fixture)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
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
    let scope_cases = [
        ("full", json!({"scope": "full"})),
        ("files", json!({"scope": "files", "files": ["src/lib.rs"]})),
        ("changed", json!({"scope": "changed", "base": "HEAD"})),
        ("lines", json!({"scope": "lines", "base": "HEAD"})),
        ("staged", json!({"scope": "staged"})),
        ("baseline", json!({"scope": "baseline", "base": "HEAD"})),
    ];
    for (offset, (expected_scope, scope_arguments)) in scope_cases.into_iter().enumerate() {
        let id = u64::try_from(offset).unwrap_or(0) + 3;
        let mut arguments = scope_arguments
            .as_object()
            .cloned()
            .ok_or_else(|| EvalError::Command("invalid MCP scope fixture".to_string()))?;
        arguments.insert(
            "directory".to_string(),
            Value::String(fixture.to_string_lossy().into_owned()),
        );
        arguments.insert("offline".to_string(), Value::Bool(true));
        arguments.insert("ignore_project_config".to_string(), Value::Bool(false));
        send_mcp(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": "scan", "arguments": arguments}
            }),
        )?;
        let response = receive_mcp(&receiver, id, timeout)?;
        require_mcp_scope(&response, expected_scope)?;
        if expected_scope == "full" {
            let text = response["result"]["content"][0]["text"]
                .as_str()
                .ok_or_else(|| {
                    EvalError::Command("MCP full scope omitted its report text".to_string())
                })?;
            require_rendered_decision("MCP", text, expected)?;
        }
    }

    let cancellation_fixture = materialize_cancellation_fixture(fixture)?;
    let cancellation_id = 9;
    send_mcp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": cancellation_id,
            "method": "tools/call",
            "params": {
                "name": "scan",
                "arguments": {
                    "directory": cancellation_fixture.to_string_lossy(),
                    "offline": true,
                    "ignore_project_config": false
                }
            }
        }),
    )?;
    send_mcp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {
                "requestId": cancellation_id,
                "reason": "artifact smoke cancellation"
            }
        }),
    )?;
    let cancellation_deadline = timeout.min(Duration::from_secs(2));
    let cancelled = receive_mcp(&receiver, cancellation_id, cancellation_deadline)?;
    if cancelled["error"]["code"].as_i64() != Some(-32603)
        || cancelled["error"]["message"] != "scan cancelled by client"
        || cancelled.get("result").is_some()
    {
        return Err(EvalError::Command(format!(
            "MCP cancellation did not return the stable error contract: {cancelled}"
        )));
    }
    send_mcp(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "id": 10, "method": "tools/list", "params": {}}),
    )?;
    let after_cancellation = receive_mcp(&receiver, 10, timeout)?;
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

fn require_mcp_scope(response: &Value, expected_scope: &str) -> Result<()> {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .ok_or_else(|| {
            EvalError::Command(format!("MCP {expected_scope} scope failed: {response}"))
        })?;
    if !text.contains(&format!("Scope: {expected_scope}")) {
        return Err(EvalError::Command(format!(
            "MCP {expected_scope} scope returned the wrong report scope"
        )));
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the npm smoke binds both final packages, the exact binary, and its decision contract"
)]
fn smoke_npm(
    binary: &Path,
    platform_archive: &Path,
    wrapper_package: &Path,
    bun: &Path,
    fixture: &Path,
    schema: &jsonschema::Validator,
    expected: &DecisionProjection,
    timeout: Duration,
) -> Result<()> {
    let platform = platform_package()?;
    let platform_package = platform_archive.canonicalize().map_err(|error| {
        EvalError::io(
            "cannot canonicalize npm platform package",
            platform_archive,
            error,
        )
    })?;
    let wrapper_package = wrapper_package.canonicalize().map_err(|error| {
        EvalError::io(
            "cannot canonicalize npm wrapper package",
            wrapper_package,
            error,
        )
    })?;
    let root = tempfile::Builder::new()
        .prefix("rust-doctor-npm-smoke-")
        .tempdir()
        .map_err(|error| EvalError::io("cannot create npm smoke directory", ".", error))?;
    let install = root.path().join("install");
    std::fs::create_dir_all(&install)
        .map_err(|error| EvalError::io("cannot create npm install directory", &install, error))?;
    let platform_spec = stage_npm_package(&platform_package, &install, "rust-doctor-platform.tgz")?;
    let wrapper_spec = stage_npm_package(&wrapper_package, &install, "rust-doctor-wrapper.tgz")?;
    std::fs::write(
        install.join("package.json"),
        "{\"name\":\"rust-doctor-smoke\",\"private\":true}",
    )
    .map_err(|error| EvalError::io("cannot write npm smoke package", &install, error))?;
    let mut add = Command::new(bun);
    add.current_dir(&install)
        .args(["add", "--no-save"])
        .arg(&platform_spec)
        .arg(&wrapper_spec);
    require_success(
        "final npm installation",
        &run_capped(add, timeout, OUTPUT_CAP)?,
    )?;
    let platform_dir = install.join("node_modules/@rust-doctor").join(platform);
    let wrapper = install.join("node_modules/rust-doctor");
    validate_package_versions(&wrapper, &platform_dir)?;
    let binary_name = if cfg!(windows) {
        "rust-doctor.exe"
    } else {
        "rust-doctor"
    };
    let embedded = platform_dir.join("bin").join(binary_name);
    let source_hash = hex_digest(
        &std::fs::read(binary)
            .map_err(|error| EvalError::io("cannot hash source binary", binary, error))?,
    );
    let embedded_hash =
        hex_digest(&std::fs::read(&embedded).map_err(|error| {
            EvalError::io("cannot hash installed npm binary", &embedded, error)
        })?);
    if source_hash != embedded_hash {
        return Err(EvalError::Command(
            "final npm platform package does not contain the exact built binary".to_string(),
        ));
    }
    // Package managers may install the wrapper's already-published optional
    // dependency below the wrapper even when the candidate platform tarball is
    // also installed at the root. Remove that temporary nested resolution so
    // the wrapper smoke necessarily executes the exact hash checked above.
    let nested_platform = wrapper.join("node_modules/@rust-doctor").join(platform);
    if nested_platform.exists() {
        std::fs::remove_dir_all(&nested_platform).map_err(|error| {
            EvalError::io(
                "cannot remove stale nested npm platform package",
                &nested_platform,
                error,
            )
        })?;
    }
    let wrapper_bin = install.join("node_modules/rust-doctor/bin/rust-doctor.js");
    let mut launch = Command::new(&wrapper_bin);
    launch.current_dir(&install).arg("--version");
    let output = run_capped(launch, timeout, OUTPUT_CAP)?;
    require_success("final npm wrapper", &output)?;
    let expected_version = version(binary, timeout)?;
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if actual != expected_version {
        return Err(EvalError::Command(format!(
            "final npm wrapper reports {actual:?}, expected {expected_version:?}"
        )));
    }
    let mut decision = Command::new(&wrapper_bin);
    decision
        .current_dir(&install)
        .args(["--json", "--json-compact", "--offline"])
        .arg(fixture);
    let output = run_capped(decision, timeout, OUTPUT_CAP)?;
    require_success("final npm decision", &output)?;
    let report = parse_report("final npm decision", &output.stdout, schema)?;
    require_projection(
        "npm",
        &serde_json::to_value(report)
            .map_err(|error| EvalError::Command(format!("cannot project npm report: {error}")))?,
        expected,
    )?;
    Ok(())
}

fn stage_npm_package(package: &Path, install: &Path, filename: &str) -> Result<String> {
    let destination = install.join(filename);
    std::fs::copy(package, &destination)
        .map_err(|error| EvalError::io("cannot stage npm package", &destination, error))?;
    Ok(format!("./{filename}"))
}

fn smoke_archives(
    binary: &Path,
    archives: &[PathBuf],
    fixture: &Path,
    schema: &jsonschema::Validator,
    expected: &DecisionProjection,
    timeout: Duration,
) -> Result<()> {
    let expected_version = version(binary, timeout)?;
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
        if actual != expected_version {
            return Err(EvalError::Command(format!(
                "extracted archive '{}' reports {actual:?}, expected {expected_version:?}",
                archive.display()
            )));
        }
        let output = invoke(
            &extracted,
            fixture,
            &["--json", "--json-compact", "--offline"],
            timeout,
        )?;
        require_success("archived binary decision", &output)?;
        let report = parse_report("archived binary decision", &output.stdout, schema)?;
        require_projection(
            "native archive",
            &serde_json::to_value(report).map_err(|error| {
                EvalError::Command(format!("cannot project archived report: {error}"))
            })?,
            expected,
        )?;
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
    let mut test = Command::new("cargo");
    test.current_dir(&package_root)
        .args([
            "test",
            "--locked",
            "--no-default-features",
            "--lib",
            "--bins",
        ])
        .env("CARGO_TARGET_DIR", target);
    require_success(
        "verified Cargo package build and test",
        &run_capped(test, timeout, OUTPUT_CAP)?,
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
    command
        .arg(fixture)
        .args(arguments)
        .env("NO_COLOR", "1")
        .env("RUST_DOCTOR_DISABLE_ANIMATION", "1");
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
            "{surface} smoke failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout).trim(),
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

fn decision_projection(report: &Value) -> Result<DecisionProjection> {
    let summary = report
        .get("summary")
        .ok_or_else(|| EvalError::Command("decision report omitted its summary".to_string()))?;
    let root_causes = report["root_causes"].as_array().ok_or_else(|| {
        EvalError::Command("decision report omitted its root-cause inventory".to_string())
    })?;
    let diagnostics = report["diagnostics"].as_array().ok_or_else(|| {
        EvalError::Command("decision report omitted its diagnostic inventory".to_string())
    })?;
    let top_root_causes = root_causes
        .iter()
        .take(3)
        .map(|group| {
            group["key"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| EvalError::Command("root-cause group omitted its key".to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    let top_rules = root_causes
        .iter()
        .take(3)
        .map(|group| {
            group["title"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| EvalError::Command("root-cause group omitted its title".to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(DecisionProjection {
        score: summary["score"].as_u64(),
        label: summary["score_label"].as_str().map(str::to_string),
        authoritative: summary["score_authoritative"].as_bool().unwrap_or(false),
        reason_codes: summary["score_reasons"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        error_count: summary["error_count"].as_u64().unwrap_or(0),
        warning_count: summary["warning_count"].as_u64().unwrap_or(0),
        info_count: summary["info_count"].as_u64().unwrap_or(0),
        top_root_causes,
        top_rules,
        scored_groups: diagnostics
            .iter()
            .filter(|diagnostic| diagnostic["score_impact"] == "scored")
            .count(),
        advisory_groups: diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic["score_impact"] != "scored" && diagnostic["trust_tier"] != "audit-only"
            })
            .count(),
        audit_groups: diagnostics
            .iter()
            .filter(|diagnostic| diagnostic["trust_tier"] == "audit-only")
            .count(),
    })
}

fn require_projection(surface: &str, report: &Value, expected: &DecisionProjection) -> Result<()> {
    let actual = decision_projection(report)?;
    if &actual != expected {
        return Err(EvalError::Command(format!(
            "{surface} decision differs from canonical Report V1: {actual:?} != {expected:?}"
        )));
    }
    Ok(())
}

fn require_rendered_decision(
    surface: &str,
    rendered: &str,
    expected: &DecisionProjection,
) -> Result<()> {
    let score = expected
        .score
        .map_or_else(|| "n/a".to_string(), |score| score.to_string());
    let required = [
        score,
        expected
            .label
            .clone()
            .unwrap_or_else(|| "unavailable".to_string()),
        format!("{} errors", expected.error_count),
        format!("{} warnings", expected.warning_count),
        format!("{} info", expected.info_count),
        format!("{} scored", expected.scored_groups),
        format!("{} advisory", expected.advisory_groups),
        format!("{} audit", expected.audit_groups),
    ];
    let missing: Vec<_> = required
        .iter()
        .chain(&expected.reason_codes)
        .filter(|value| !rendered.contains(value.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(EvalError::Command(format!(
            "{surface} omitted canonical score, authority, or finding counts: {missing:?}"
        )));
    }
    let positions: Vec<_> = expected
        .top_rules
        .iter()
        .filter_map(|rule| rendered.find(rule))
        .collect();
    if positions.len() != expected.top_rules.len()
        || !positions.windows(2).all(|pair| pair[0] < pair[1])
    {
        return Err(EvalError::Command(format!(
            "{surface} omitted or reordered canonical top remediations {:?}; positions {positions:?}",
            expected.top_rules
        )));
    }
    Ok(())
}

fn first_unique(values: impl IntoIterator<Item = String>, limit: usize) -> Vec<String> {
    let mut unique = Vec::new();
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
            if unique.len() == limit {
                break;
            }
        }
    }
    unique
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
        "[package]\nname = \"artifact-smoke\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.97\"\n",
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
    git(fixture.path(), &["add", "src/lib.rs"])?;
    Ok(fixture)
}

fn materialize_cancellation_fixture(root: &Path) -> Result<PathBuf> {
    let fixture = root.join("cancellation-fixture");
    std::fs::create_dir_all(fixture.join("src"))
        .map_err(|error| EvalError::io("cannot create cancellation fixture", &fixture, error))?;
    std::fs::write(
        fixture.join("Cargo.toml"),
        "[package]\nname = \"cancellation-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\nbuild = \"build.rs\"\n",
    )
    .map_err(|error| EvalError::io("cannot write cancellation manifest", &fixture, error))?;
    std::fs::write(
        fixture.join("rust-doctor.toml"),
        "lint = true\ndependencies = false\n",
    )
    .map_err(|error| EvalError::io("cannot write cancellation config", &fixture, error))?;
    std::fs::write(fixture.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n")
        .map_err(|error| EvalError::io("cannot write cancellation source", &fixture, error))?;
    std::fs::write(
        fixture.join("build.rs"),
        "fn main() { std::thread::sleep(std::time::Duration::from_secs(30)); }\n",
    )
    .map_err(|error| EvalError::io("cannot write cancellation build script", &fixture, error))?;
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

fn send_lsp(stdin: &mut impl Write, message: &Value) -> Result<()> {
    let body = serde_json::to_vec(message)
        .map_err(|error| EvalError::Command(format!("cannot encode LSP message: {error}")))?;
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())
        .and_then(|()| stdin.write_all(&body))
        .and_then(|()| stdin.flush())
        .map_err(|error| EvalError::Command(format!("cannot send LSP message: {error}")))
}

fn read_lsp_message(reader: &mut impl BufRead) -> std::io::Result<Option<Value>> {
    let mut content_length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Ok(None);
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some(value) = header.trim().strip_prefix("Content-Length:").map(str::trim) {
            content_length = value.parse::<usize>().ok();
        }
    }
    let length = content_length
        .ok_or_else(|| std::io::Error::other("LSP response omitted Content-Length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(std::io::Error::other)
}

fn receive_lsp(receiver: &mpsc::Receiver<Value>, id: u64, timeout: Duration) -> Result<Value> {
    loop {
        let message = receiver
            .recv_timeout(timeout)
            .map_err(|error| EvalError::Command(format!("LSP response {id} timed out: {error}")))?;
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

    #[test]
    fn npm_package_specs_are_relative_and_stage_exact_bytes() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("source.tgz");
        let install = root.path().join("install");
        std::fs::write(&package, b"exact package bytes").unwrap();
        std::fs::create_dir(&install).unwrap();

        let spec = stage_npm_package(&package, &install, "platform.tgz").unwrap();

        assert_eq!(spec, "./platform.tgz");
        assert_eq!(
            std::fs::read(install.join(&spec)).unwrap(),
            b"exact package bytes"
        );
    }
}
