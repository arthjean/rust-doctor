#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
#[cfg(any(feature = "mcp", feature = "lsp"))]
use std::process::Stdio;
use std::process::{Command, Output};
use std::time::Duration;
#[cfg(any(feature = "mcp", feature = "lsp"))]
use std::time::Instant;

const MANIFEST: &str =
    "[package]\nname='product-loop'\nversion='0.1.0'\nedition='2024'\nrust-version='1.97'\n";

#[test]
fn explicit_telemetry_uses_the_aggregate_allowlist_and_overrides_make_zero_requests() {
    let project = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    write_project(project.path());

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/events", listener.local_addr().unwrap());
    let enable = command(config.path())
        .args(["telemetry", "enable", "--endpoint", &endpoint, "--yes"])
        .output()
        .unwrap();
    assert_success(&enable);

    let capture = std::thread::spawn(move || capture_request(&listener));
    let scan = scan_command(config.path(), project.path())
        .output()
        .unwrap();
    assert_success(&scan);
    let request = capture.join().unwrap();
    let body = request.split_once("\r\n\r\n").unwrap().1;
    let event: Value = serde_json::from_str(body).unwrap();
    assert_eq!(event["schema_version"], "1.1");
    assert_eq!(event["event_kind"], "scan");
    assert!(matches!(
        event["completeness"].as_str(),
        Some("complete" | "partial" | "incomplete")
    ));
    assert_eq!(
        event["suppression_counts"],
        serde_json::json!({
            "inline": 0,
            "rule": 0,
            "category": 0,
            "tag": 0,
            "path": 0,
            "security_policy": 0
        })
    );
    for prohibited in [
        "product-loop",
        "src/lib.rs",
        "unwrap-in-production",
        "repository",
        "source_text",
        "diagnostic_message",
        "git_remote",
        "environment",
        "command_argument",
    ] {
        assert!(
            !body.contains(prohibited),
            "telemetry request leaked {prohibited}: {body}"
        );
    }

    let failed = TcpListener::bind("127.0.0.1:0").unwrap();
    let failed_endpoint = format!("http://{}/events", failed.local_addr().unwrap());
    let enable = command(config.path())
        .args([
            "telemetry",
            "enable",
            "--endpoint",
            &failed_endpoint,
            "--yes",
        ])
        .output()
        .unwrap();
    assert_success(&enable);
    let capture = std::thread::spawn(move || capture_request_with_status(&failed, "500 Error"));
    let scan = scan_command(config.path(), project.path())
        .output()
        .unwrap();
    assert_success(&scan);
    capture.join().unwrap();
}

#[test]
fn revoked_consent_and_overrides_make_zero_requests_across_every_surface() {
    let project = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    write_project(project.path());

    let denied = TcpListener::bind("127.0.0.1:0").unwrap();
    denied.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/events", denied.local_addr().unwrap());
    let enable = command(config.path())
        .args(["telemetry", "enable", "--endpoint", &endpoint, "--yes"])
        .output()
        .unwrap();
    assert_success(&enable);

    for policy in [
        TelemetryDenial::Flag,
        TelemetryDenial::Environment,
        TelemetryDenial::Offline,
    ] {
        assert_all_surfaces_make_zero_requests(&denied, config.path(), project.path(), policy);
    }

    let disable = command(config.path())
        .args(["telemetry", "disable"])
        .output()
        .unwrap();
    assert_success(&disable);
    assert_all_surfaces_make_zero_requests(
        &denied,
        config.path(),
        project.path(),
        TelemetryDenial::RevokedConsent,
    );
}

#[derive(Clone, Copy)]
enum TelemetryDenial {
    Flag,
    Environment,
    Offline,
    RevokedConsent,
}

impl TelemetryDenial {
    const fn label(self) -> &'static str {
        match self {
            Self::Flag => "--no-telemetry",
            Self::Environment => "RUST_DOCTOR_TELEMETRY=0",
            Self::Offline => "--offline",
            Self::RevokedConsent => "revoked consent",
        }
    }

    fn apply(self, command: &mut Command) {
        match self {
            Self::Flag => {
                command.arg("--no-telemetry");
            }
            Self::Environment => {
                command.env("RUST_DOCTOR_TELEMETRY", "0");
            }
            Self::Offline => {
                command.arg("--offline");
            }
            Self::RevokedConsent => {}
        }
    }
}

fn assert_all_surfaces_make_zero_requests(
    listener: &TcpListener,
    config_root: &Path,
    project: &Path,
    policy: TelemetryDenial,
) {
    let mut cli = scan_command(config_root, project);
    policy.apply(&mut cli);
    assert_scan_makes_zero_requests(listener, cli, &format!("CLI {}", policy.label()));

    let mut action = scan_command(config_root, project);
    action.env("GITHUB_ACTIONS", "true");
    policy.apply(&mut action);
    assert_scan_makes_zero_requests(listener, action, &format!("Action {}", policy.label()));

    #[cfg(feature = "mcp")]
    {
        let mut mcp = server_command(config_root, "--mcp");
        policy.apply(&mut mcp);
        assert_server_makes_zero_requests(listener, mcp, &format!("MCP {}", policy.label()));
    }

    #[cfg(feature = "lsp")]
    {
        let mut lsp = server_command(config_root, "--lsp");
        policy.apply(&mut lsp);
        assert_server_makes_zero_requests(listener, lsp, &format!("LSP {}", policy.label()));
    }
}

fn assert_scan_makes_zero_requests(listener: &TcpListener, mut command: Command, label: &str) {
    let output = command.output().unwrap();
    assert_success(&output);
    assert!(
        listener.accept().is_err(),
        "{label} must make zero telemetry requests"
    );
}

#[cfg(any(feature = "mcp", feature = "lsp"))]
fn server_command(config_root: &Path, flag: &str) -> Command {
    let mut command = command(config_root);
    command.arg(flag).env("RUST_DOCTOR_TELEMETRY", "1");
    command
}

#[cfg(any(feature = "mcp", feature = "lsp"))]
fn assert_server_makes_zero_requests(listener: &TcpListener, mut command: Command, label: &str) {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut request_observed = false;
    while Instant::now() < deadline {
        if listener.accept().is_ok() {
            request_observed = true;
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !request_observed {
        request_observed = listener.accept().is_ok();
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        !request_observed,
        "{label} must make zero telemetry requests"
    );
}

#[test]
fn explicit_share_is_stateless_percent_encoded_and_source_free() {
    let project = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    write_project(project.path());

    let output = scan_command(config.path(), project.path())
        .arg("--share")
        .output()
        .unwrap();
    assert_success(&output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let url = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Share: "))
        .expect("share URL");
    assert!(url.len() <= 8 * 1024);
    assert!(
        url.contains("category=error-handling%3A1"),
        "category tuple must be percent encoded: {url}"
    );
    for prohibited in [
        "product-loop",
        "src%2Flib.rs",
        "unwrap-in-production",
        "private-source-marker",
    ] {
        assert!(!url.contains(prohibited), "share URL leaked {prohibited}");
    }

    let parsed = reqwest::Url::parse(url).unwrap();
    let pairs: std::collections::BTreeMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(pairs.get("v").map(String::as_str), Some("1"));
    assert_eq!(
        pairs.get("completeness").map(String::as_str),
        Some("complete")
    );
    assert_eq!(pairs.get("authoritative").map(String::as_str), Some("true"));
}

#[test]
fn real_cli_share_covers_maximum_cardinalities_and_percent_encoding() {
    let project = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let fixture = tempfile::NamedTempFile::new().unwrap();
    write_project(project.path());

    let (category_count, unavailable_check_count) =
        write_cardinality_fixture(config.path(), project.path(), fixture.path());

    let output = scan_command(config.path(), project.path())
        .arg("--share")
        .env("RUST_DOCTOR_INTERNAL_SHARE_REPORT_FIXTURE", fixture.path())
        .output()
        .unwrap();
    assert_success(&output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let url = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Share: "))
        .expect("share URL");
    assert!(url.len() <= 8 * 1024);
    assert!(url.contains("category=error-handling%3A1"));
    assert!(url.contains("check=cancelled%3Abaseline"));

    let parsed = reqwest::Url::parse(url).unwrap();
    let pairs: Vec<_> = parsed.query_pairs().into_owned().collect();
    assert_eq!(
        pairs.iter().filter(|(key, _)| key == "category").count(),
        category_count
    );
    assert_eq!(pairs.iter().filter(|(key, _)| key == "check").count(), 32);
    assert!(pairs.iter().any(|(key, value)| {
        key == "checks_omitted" && value == &(unavailable_check_count - 32).to_string()
    }));
    for prohibited in [
        "product-loop",
        "src%2Flib.rs",
        "unwrap-in-production",
        "private-source-marker",
        "bounded+public+fixture",
    ] {
        assert!(!url.contains(prohibited), "share URL leaked {prohibited}");
    }
}

#[test]
fn real_cli_oversized_share_preserves_local_output_without_partial_url() {
    let project = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let fixture = tempfile::NamedTempFile::new().unwrap();
    write_project(project.path());

    let mut report = cli_report(config.path(), project.path());
    report["tool_version"] = Value::String("x".repeat(8 * 1024));
    write_json(fixture.path(), &report);

    let output = scan_command(config.path(), project.path())
        .arg("--share")
        .env("RUST_DOCTOR_INTERNAL_SHARE_REPORT_FIXTURE", fixture.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stdout.contains("rust-doctor"),
        "local scan result disappeared: {stdout}"
    );
    assert!(!stdout.contains("Share: "));
    assert!(!stdout.contains("https://rust-doctor.vercel.app/share/"));
    assert!(!stderr.contains("https://rust-doctor.vercel.app/share/"));
    assert!(stderr.contains("share URL was not created"));
    assert!(stderr.contains("the maximum is 8192"));
}

fn write_cardinality_fixture(config_root: &Path, project: &Path, fixture: &Path) -> (usize, usize) {
    let mut report = cli_report(config_root, project);
    let template = report["diagnostics"][0].clone();
    let categories = [
        "error-handling",
        "performance",
        "security",
        "correctness",
        "architecture",
        "dependencies",
        "async",
        "framework",
        "cargo",
        "style",
    ];
    report["diagnostics"] = Value::Array(
        categories
            .iter()
            .map(|category| {
                let mut diagnostic = template.clone();
                diagnostic["category"] = Value::String((*category).to_string());
                diagnostic["severity"] = Value::String("warning".to_string());
                diagnostic
            })
            .collect(),
    );
    report["summary"]["error_count"] = serde_json::json!(0);
    report["summary"]["warning_count"] = serde_json::json!(categories.len());
    report["summary"]["info_count"] = serde_json::json!(0);
    report["summary"]["diagnostic_count"] = serde_json::json!(categories.len());
    report["error_count"] = serde_json::json!(0);
    report["warning_count"] = serde_json::json!(categories.len());
    report["info_count"] = serde_json::json!(0);

    let check_names = [
        "clippy",
        "custom rules",
        "dependencies (cargo-audit)",
        "dependencies (cargo-deny)",
        "dependencies (cargo-machete)",
        "unsafe audit (cargo-geiger)",
        "coverage",
        "msrv",
        "semver (cargo-semver-checks)",
        "baseline",
        "staged snapshot",
        "package scan",
        "scope:lines",
    ];
    let statuses = [
        "planned",
        "running",
        "skipped",
        "failed",
        "timed_out",
        "cancelled",
    ];
    report["projects"][0]["checks"] = Value::Array(
        statuses
            .iter()
            .flat_map(|status| {
                check_names.iter().map(move |name| {
                    serde_json::json!({
                        "name": name,
                        "required": false,
                        "status": status,
                        "reason": "bounded public fixture"
                    })
                })
            })
            .collect(),
    );
    write_json(fixture, &report);
    (categories.len(), statuses.len() * check_names.len())
}

fn cli_report(config_root: &Path, project: &Path) -> Value {
    let output = scan_command(config_root, project)
        .arg("--json")
        .output()
        .unwrap();
    assert_success(&output);
    serde_json::from_slice(&output.stdout).unwrap()
}

fn write_json(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn write_project(root: &Path) {
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), MANIFEST).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn private_source_marker() -> u8 { Some(1_u8).unwrap() }\n",
    )
    .unwrap();
}

fn command(config_root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rust-doctor"));
    command.env("XDG_CONFIG_HOME", config_root);
    command
}

fn scan_command(config_root: &Path, project: &Path) -> Command {
    let mut command = command(config_root);
    command
        .arg(project)
        .args([
            "--no-project-config",
            "--disable-adapter",
            "compiler-lint,supply-chain,quality",
            "--no-color",
        ])
        .env("RUST_DOCTOR_TELEMETRY", "1");
    command
}

fn capture_request(listener: &TcpListener) -> String {
    capture_request_with_status(listener, "204 No Content")
}

fn capture_request_with_status(listener: &TcpListener, status: &str) -> String {
    let (mut stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let expected = loop {
        let read = stream.read(&mut buffer).unwrap();
        request.extend_from_slice(&buffer[..read]);
        if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap();
            break header_end + 4 + content_length;
        }
    };
    while request.len() < expected {
        let read = stream.read(&mut buffer).unwrap();
        request.extend_from_slice(&buffer[..read]);
    }
    stream
        .write_all(
            format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
    String::from_utf8(request).unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
