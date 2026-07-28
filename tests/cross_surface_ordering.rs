//! Cross-surface ordering contract (US-015).
//!
//! Every consumer renders the same immutable report, so every consumer must
//! answer "what should I fix first?" identically. This suite drives the real
//! binary and compares the order JSON, SARIF, and the agent handoff produce. A
//! surface that reorders findings on its own fails release validation
//! (US-017 AC-9).

// Integration test crates are outside Clippy's allow-unwrap-in-tests handling.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rust_doctor::api::{ScanRequest, scan as library_scan};
use rust_doctor::config::AdapterPolicy;
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

/// A fixture with findings in several categories, priorities, and files, laid
/// out so that path order and severity order both disagree with priority order.
fn write_project(root: &Path) {
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='ordering-fixture'\nversion='0.1.0'\nedition='2024'\nrust-version='1.97'\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    // `a_style.rs` sorts first by path but must not rank first.
    std::fs::write(
        root.join("src/a_style.rs"),
        r#"pub fn owned() -> String {
    let value = String::from("literal");
    let copy = value.clone();
    let again = copy.clone();
    again
}
"#,
    )
    .unwrap();
    // `z_security.rs` sorts last by path and must rank first.
    std::fs::write(
        root.join("src/z_security.rs"),
        r#"pub const API_KEY: &str = "sk_live_0123456789abcdefABCDEF";

pub fn query(user: &str) -> String {
    format!("SELECT * FROM users WHERE name = '{user}'")
}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub mod a_style;\npub mod z_security;\n",
    )
    .unwrap();
}

const fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_rust-doctor")
}

fn scan_output(root: &Path, arguments: &[&str]) -> Output {
    Command::new(binary())
        .arg(root)
        .args(arguments)
        .env("RUST_DOCTOR_DISABLE_ANIMATION", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn scan(root: &Path, arguments: &[&str]) -> String {
    let output = scan_output(root, arguments);
    String::from_utf8(output.stdout).unwrap()
}

/// Root-cause keys in the order a surface presented them, first appearance
/// only. Comparing keys rather than raw diagnostics keeps the assertion honest
/// when a surface legitimately groups occurrences.
fn first_appearance(keys: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = Vec::new();
    for key in keys {
        if !seen.contains(&key) {
            seen.push(key);
        }
    }
    seen
}

fn assert_surface_order(
    diagnostics: &[Value],
    rendered: &str,
    surfaces: &[&str],
    marker_field: &str,
    label: &str,
) {
    let markers = first_appearance(
        diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic["visible_on"].as_array().is_some_and(|visible| {
                    visible.iter().any(|surface| {
                        surface
                            .as_str()
                            .is_some_and(|surface| surfaces.contains(&surface))
                    })
                })
            })
            .filter_map(|diagnostic| diagnostic[marker_field].as_str().map(str::to_string)),
    );
    let missing: Vec<_> = markers
        .iter()
        .filter(|marker| !rendered.contains(marker.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "{label} omitted canonical root causes: {missing:?}\n{rendered}"
    );
    let positions: Vec<_> = markers
        .iter()
        .filter_map(|marker| rendered.find(marker))
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{label} reordered canonical diagnostics: {markers:?}\n{rendered}"
    );
}

fn assert_human_surface_order(diagnostics: &[Value], output: Output) {
    let stderr = String::from_utf8(output.stderr).unwrap();
    let (terminal, plan) = stderr
        .split_once("# Remediation Plan")
        .expect("plan output follows terminal output");
    assert_surface_order(diagnostics, terminal, &["terminal"], "title", "terminal");
    assert_surface_order(
        diagnostics,
        plan,
        &["score", "ci-failure", "pr-comment"],
        "rule",
        "plan",
    );
}

#[test]
fn json_sarif_and_handoff_agree_on_priority_order() {
    let project = tempfile::tempdir().unwrap();
    write_project(project.path());

    let report: Value = serde_json::from_str(&scan(project.path(), &["--json"])).unwrap();
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.len() >= 3,
        "fixture produced too few findings to compare orders: {}",
        diagnostics.len()
    );

    let json_order = first_appearance(diagnostics.iter().map(|diagnostic| {
        diagnostic["root_cause_key"]
            .as_str()
            .unwrap_or_else(|| diagnostic["rule"].as_str().unwrap())
            .to_string()
    }));

    // The canonical report leads with the highest priority, not the first path.
    let first = &diagnostics[0];
    assert_eq!(
        first["priority"].as_str(),
        Some("p0"),
        "canonical report did not lead with a P0 finding: {first}"
    );
    assert!(
        json_order
            .iter()
            .position(|key| key.contains("hardcoded-secrets") || key.contains("sql-injection"))
            < json_order.iter().position(|key| key.contains("clone")),
        "security root causes must outrank performance ones: {json_order:?}"
    );

    // Report V1 declares the same groups in the same order.
    let declared: Vec<String> = report["root_causes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| group["key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        declared.first(),
        json_order.first(),
        "declared root causes disagree with diagnostic order"
    );

    let sarif: Value =
        serde_json::from_str(&scan(project.path(), &["--sarif"])).expect("SARIF output must parse");
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    let sarif_order = first_appearance(results.iter().map(|result| {
        result["partialFingerprints"]["rustDoctorRootCause/v1"]
            .as_str()
            .unwrap_or_else(|| result["ruleId"].as_str().unwrap())
            .to_string()
    }));
    assert_eq!(
        sarif_order, json_order,
        "SARIF reordered the canonical report"
    );

    // SARIF carries the decision metadata GitHub consumers need.
    let leading = &results[0];
    assert_eq!(leading["properties"]["priority"].as_str(), Some("p0"));
    assert!(leading["properties"]["rootCauseKey"].is_string());
    assert!(
        leading["partialFingerprints"]["rustDoctorSiteId/v1"].is_string(),
        "stable fingerprint is missing"
    );
    let rules = sarif["runs"][0]["tool"]["driver"]["rules"]
        .as_array()
        .unwrap();
    assert!(
        rules
            .iter()
            .any(|rule| rule["properties"]["precision"].is_string()),
        "SARIF rules carry no precision metadata"
    );

    let handoff_dir = tempfile::tempdir().unwrap();
    scan(
        project.path(),
        &["--output-dir", handoff_dir.path().to_str().unwrap()],
    );
    let dump: Value = serde_json::from_slice(
        &std::fs::read(handoff_dir.path().join("diagnostics.json")).unwrap(),
    )
    .unwrap();
    let handoff_order = first_appearance(dump["diagnostics"].as_array().unwrap().iter().map(
        |diagnostic| {
            diagnostic["root_cause_key"]
                .as_str()
                .unwrap_or_else(|| diagnostic["rule"].as_str().unwrap())
                .to_string()
        },
    ));
    assert_eq!(
        handoff_order, json_order,
        "the agent handoff reordered the canonical report"
    );

    assert_human_surface_order(
        diagnostics,
        scan_output(project.path(), &["--verbose", "--plan"]),
    );
}

#[test]
fn repeated_runs_produce_a_byte_stable_order() {
    let project = tempfile::tempdir().unwrap();
    write_project(project.path());

    let first = scan(project.path(), &["--json"]);
    let second = scan(project.path(), &["--json"]);
    let extract = |output: &str| -> Vec<String> {
        let report: Value = serde_json::from_str(output).unwrap();
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["site_id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(extract(&first), extract(&second));
}

#[test]
fn degraded_cli_library_score_and_sarif_share_one_decision() {
    let project = tempfile::tempdir().unwrap();
    write_project(project.path());
    std::fs::write(
        project.path().join("build.rs"),
        "fn main() { std::thread::sleep(std::time::Duration::from_secs(5)); }\n",
    )
    .unwrap();
    let manifest = std::fs::read_to_string(project.path().join("Cargo.toml")).unwrap();
    std::fs::write(
        project.path().join("Cargo.toml"),
        manifest.replace("[package]", "[package]\nbuild = \"build.rs\""),
    )
    .unwrap();
    let degraded = [
        "--disable-adapter",
        "supply-chain,quality,network-dependent",
        "--offline",
        "--no-project-config",
        "--max-duration",
        "1",
    ];

    let mut json_args = degraded.to_vec();
    json_args.push("--json");
    let cli_report: Value =
        serde_json::from_slice(&scan_output(project.path(), &json_args).stdout).unwrap();

    let mut request = ScanRequest::new(project.path());
    request.options.use_project_config = false;
    request.options.deadline = Some(std::time::Duration::from_secs(1));
    request.options.adapters = AdapterPolicy {
        compiler_lint: true,
        custom_ast: true,
        supply_chain: false,
        quality: false,
        network: false,
    };
    let library_report = serde_json::to_value(library_scan(request).unwrap()).unwrap();

    assert_eq!(cli_report["summary"], library_report["summary"]);
    assert_eq!(cli_report["root_causes"], library_report["root_causes"]);
    assert_eq!(cli_report["dimensions"], library_report["dimensions"]);
    assert_eq!(
        cli_report["summary"]["score_authoritative"].as_bool(),
        Some(false)
    );
    let reasons = cli_report["summary"]["score_reasons"].as_array().unwrap();
    assert!(!reasons.is_empty());

    let mut score_args = degraded.to_vec();
    score_args.push("--score");
    let score = scan_output(project.path(), &score_args);
    assert_eq!(score.status.code(), Some(1));
    assert!(score.stdout.is_empty(), "hidden score leaked to stdout");

    let mut sarif_args = degraded.to_vec();
    sarif_args.push("--sarif");
    let sarif: Value =
        serde_json::from_slice(&scan_output(project.path(), &sarif_args).stdout).unwrap();
    let properties = &sarif["runs"][0]["properties"];
    assert_eq!(
        properties["rustDoctorScore"],
        cli_report["summary"]["score"]
    );
    assert_eq!(
        properties["rustDoctorScoreLabel"],
        cli_report["summary"]["score_label"]
    );
    assert_eq!(
        properties["rustDoctorScoreAuthoritative"],
        cli_report["summary"]["score_authoritative"]
    );
    assert_eq!(
        properties["rustDoctorScoreReasons"],
        cli_report["summary"]["score_reasons"]
    );

    let terminal = scan_output(project.path(), &degraded);
    let rendered = String::from_utf8_lossy(&terminal.stderr);
    let first_reason = reasons[0].as_str().unwrap();
    assert!(
        rendered.contains(first_reason),
        "terminal omitted canonical authority reason {first_reason:?}"
    );
}
