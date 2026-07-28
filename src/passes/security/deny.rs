#![expect(
    clippy::redundant_pub_crate,
    reason = "adapter parsers and contracts are consumed by the sibling conformance module through this private crate module"
)]

use super::deny_codes::{self, DenyCode};
use crate::diagnostics::{Diagnostic, Severity};
use crate::passes::adapter::{self, AdapterContract, EvidenceSource};
use crate::passes::conformance::VersionSupport;
use crate::process;
use crate::scanner::AnalysisPass;
use serde::Deserialize;
use std::path::Path;
use std::process::{Command, Stdio};

const DENY_TIMEOUT_SECS: u64 = 60;
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024;
// cargo-deny uses one bit per failed check: advisories, bans, licenses, sources.
// Any combination is a completed findings run only when structured parsing
// subsequently proves that a summary was emitted.
const QUALIFIED_EXIT_CODES: &[i32] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

pub(crate) const CONTRACT: AdapterContract = AdapterContract {
    pass: "dependencies (cargo-deny)",
    subcommand: "deny",
    parser_contract_version: "deny-jsonlines-v2",
    evidence_source: EvidenceSource::StructuredJsonLines,
};

/// cargo-deny checks advisories, licenses, bans, and sources.
pub struct DenyPass {
    pub offline: bool,
}

impl AnalysisPass for DenyPass {
    fn name(&self) -> &'static str {
        CONTRACT.pass
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        if !is_cargo_deny_available() {
            return Err(CONTRACT.skipped(
                "supply-chain checking disabled. Install with: cargo install cargo-deny",
            ));
        }
        run_deny(project_root, self.offline)
    }

    fn completion_reason(&self) -> Option<String> {
        let version = CONTRACT.tool_version();
        Some(CONTRACT.provenance(&version))
    }
}

/// Check if `cargo deny` is available. Result is cached for the process lifetime.
pub fn is_cargo_deny_available() -> bool {
    process::is_cargo_subcommand_available("deny")
}

pub(crate) fn command_args(offline: bool) -> Vec<&'static str> {
    let mut args = vec!["deny", "--format", "json", "check"];
    if offline {
        args.push("--disable-fetch");
    }
    args
}

fn run_deny(
    project_root: &Path,
    offline: bool,
) -> Result<Vec<Diagnostic>, crate::error::PassError> {
    let (result, tool_version) = run_deny_process(project_root, offline)?;
    CONTRACT.require_complete_run(&result, QUALIFIED_EXIT_CODES)?;
    let exit_code = result
        .exit_code
        .ok_or_else(|| CONTRACT.failed("cargo-deny terminated without an exit status"))?;
    parse_deny_streams(
        &result.stdout,
        &result.stderr,
        &tool_version,
        &CONTRACT.provenance(&tool_version),
        exit_code,
    )
    .map_err(|error| CONTRACT.failed(error))
}

fn run_deny_process(
    project_root: &Path,
    offline: bool,
) -> Result<(process::ProcessOutput, String), crate::error::PassError> {
    let tool_version = CONTRACT.tool_version();
    if CONTRACT.version_support(&tool_version) != VersionSupport::Supported {
        return Err(CONTRACT.failed(format!(
            "cargo-deny version `{tool_version}` is outside the qualified 0.18/0.19 matrix"
        )));
    }
    let child = process::spawn_in_group(
        Command::new("cargo")
            .args(command_args(offline))
            .current_dir(project_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .map_err(|error| CONTRACT.failed(format!("failed to spawn cargo deny: {error}")))?;
    let result = process::run_with_timeout(child, DENY_TIMEOUT_SECS, MAX_OUTPUT_BYTES)
        .map_err(|error| CONTRACT.failed(error))?;
    Ok((result, tool_version))
}

pub(crate) fn parse_deny_streams(
    stdout: &str,
    stderr: &str,
    tool_version: &str,
    provenance: &str,
    exit_code: i32,
) -> Result<Vec<Diagnostic>, String> {
    let mut diagnostics = Vec::new();
    let mut summary = None;
    let mut observed_error_mask = 0;
    let mut structured_lines = 0_usize;

    for line in stdout.lines().chain(stderr.lines()) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: DenyLine = serde_json::from_str(line).map_err(|error| {
            format!("cargo-deny emitted malformed JSON Lines evidence: {error}")
        })?;
        structured_lines += 1;
        match entry.kind.as_str() {
            "summary" => {
                if summary.is_some() {
                    return Err("cargo-deny emitted more than one completion summary".to_string());
                }
                summary = Some(serde_json::from_value::<DenySummary>(entry.fields).map_err(
                    |error| format!("cargo-deny summary has an unsupported shape: {error}"),
                )?);
            }
            "diagnostic" => {
                let fields: DenyFields = serde_json::from_value(entry.fields).map_err(|error| {
                    format!("cargo-deny diagnostic has an unsupported shape: {error}")
                })?;
                let code = fields
                    .code
                    .as_deref()
                    .ok_or_else(|| "cargo-deny diagnostic omitted its semantic code".to_string())?;
                let DenyCode::Check(check) = deny_codes::classify(tool_version, code)? else {
                    return Err(format!(
                        "cargo-deny semantic code `{code}` reports incomplete analyzer evidence"
                    ));
                };
                if fields.severity == DenySeverity::Error {
                    observed_error_mask |= check.exit_bit();
                }
                if let Some(severity) = fields.severity.actionable() {
                    let message = fields
                        .notes
                        .iter()
                        .find_map(|note| adapter::advisory_identity(note))
                        .map_or_else(
                            || fields.message.clone(),
                            |identity| format!("{identity}: {}", fields.message),
                        );
                    let help = fields
                        .labels
                        .iter()
                        .filter_map(|label| label.message.as_deref())
                        .find(|message| !message.is_empty())
                        .map_or_else(
                            || format!("via {provenance}"),
                            |label| format!("{label}\n  via {provenance}"),
                        );
                    diagnostics.push(adapter::project_diagnostic(
                        check.rule(),
                        check.category(),
                        severity,
                        &message,
                        Some(&help),
                        "Cargo.toml",
                    ));
                }
            }
            kind => {
                return Err(format!(
                    "cargo-deny emitted unsupported structured evidence type `{kind}`"
                ));
            }
        }
    }

    if structured_lines == 0 {
        return Err("cargo-deny emitted no structured evidence on stdout or stderr".to_string());
    }
    let summary =
        summary.ok_or_else(|| "cargo-deny output omitted its completion summary".to_string())?;
    let summary_mask = summary.error_mask();
    if summary_mask != exit_code {
        return Err(format!(
            "cargo-deny exit status {exit_code} disagrees with summary error mask {summary_mask}"
        ));
    }
    if observed_error_mask != summary_mask {
        return Err(format!(
            "cargo-deny diagnostics error mask {observed_error_mask} disagrees with summary error mask {summary_mask}"
        ));
    }
    Ok(diagnostics)
}

#[cfg(test)]
pub(crate) fn parse_deny_output(
    output: &str,
    tool_version: &str,
    provenance: &str,
    exit_code: i32,
) -> Result<Vec<Diagnostic>, String> {
    parse_deny_streams(output, "", tool_version, provenance, exit_code)
}

#[derive(Deserialize)]
struct DenyLine {
    #[serde(rename = "type")]
    kind: String,
    fields: serde_json::Value,
}

#[derive(Deserialize)]
struct DenyFields {
    severity: DenySeverity,
    message: String,
    code: Option<String>,
    #[serde(default)]
    labels: Vec<DenyLabel>,
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum DenySeverity {
    Error,
    Warning,
    Note,
    Help,
}

impl DenySeverity {
    const fn actionable(self) -> Option<Severity> {
        match self {
            Self::Error => Some(Severity::Error),
            Self::Warning => Some(Severity::Warning),
            Self::Note | Self::Help => None,
        }
    }
}

#[derive(Deserialize)]
struct DenyLabel {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct DenySummary {
    advisories: DenyCounts,
    bans: DenyCounts,
    licenses: DenyCounts,
    sources: DenyCounts,
}

impl DenySummary {
    fn error_mask(&self) -> i32 {
        i32::from(self.advisories.errors > 0)
            | (i32::from(self.bans.errors > 0) << 1)
            | (i32::from(self.licenses.errors > 0) << 2)
            | (i32::from(self.sources.errors > 0) << 3)
    }
}

#[derive(Deserialize)]
struct DenyCounts {
    errors: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;

    const ADVISORY_DB_DIR: &str = "advisory-db-3157b0e258782691";
    const ADVISORY_SUPPORT: &str = include_str!(
        "../../../evaluation/conformance/cargo-deny/advisory-db-snapshot/support.toml"
    );
    const RSA_ADVISORY: &str = include_str!(
        "../../../evaluation/conformance/cargo-deny/advisory-db-snapshot/crates/rsa/RUSTSEC-2023-0071.md"
    );

    const V018_FINDINGS: &str =
        include_str!("../../../evaluation/conformance/cargo-deny/v0.18-findings.jsonl");
    const V018_CLEAN: &str =
        include_str!("../../../evaluation/conformance/cargo-deny/v0.18-clean.jsonl");
    const V019_FINDINGS: &str =
        include_str!("../../../evaluation/conformance/cargo-deny/v0.19-findings.jsonl");
    const V019_CLEAN: &str =
        include_str!("../../../evaluation/conformance/cargo-deny/v0.19-clean.jsonl");

    #[test]
    fn global_options_precede_check_and_offline_is_explicit() {
        assert_eq!(command_args(false), ["deny", "--format", "json", "check"]);
        assert_eq!(
            command_args(true),
            ["deny", "--format", "json", "check", "--disable-fetch"]
        );
        assert_eq!(
            QUALIFIED_EXIT_CODES,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        );
    }

    #[test]
    fn qualified_fixtures_cover_both_supported_minor_versions() {
        for (fixture, version) in [
            (V018_FINDINGS, "cargo-deny 0.18.3"),
            (V019_FINDINGS, "cargo-deny 0.19.8"),
        ] {
            let diagnostics =
                parse_deny_output(fixture, version, version, 15).expect("qualified fixture parses");
            assert!(diagnostics.iter().any(|item| item.rule == "deny-advisory"));
            assert!(diagnostics.iter().any(|item| item.rule == "deny-license"));
            assert!(diagnostics.iter().any(|item| item.rule == "deny-ban"));
            assert!(diagnostics.iter().any(|item| item.rule == "deny-source"));
            assert!(diagnostics.iter().all(|item| {
                item.help
                    .as_deref()
                    .is_some_and(|help| help.contains(version))
            }));
            assert!(diagnostics.iter().any(|item| {
                adapter::advisory_identity(&item.message).as_deref() == Some("RUSTSEC-2023-0071")
            }));
        }
    }

    #[test]
    fn supported_clean_requires_a_completion_summary() {
        assert!(
            parse_deny_output(V018_CLEAN, "cargo-deny 0.18.3", "cargo-deny 0.18.3", 0,)
                .expect("0.18 clean fixture")
                .is_empty()
        );
        assert!(
            parse_deny_output(V019_CLEAN, "cargo-deny 0.19.8", "cargo-deny 0.19.8", 0,)
                .expect("0.19 clean fixture")
                .is_empty()
        );
        assert!(parse_deny_output("", "cargo-deny 0.19.8", "provenance", 0).is_err());
        assert!(
            parse_deny_output(
                r#"{"type":"diagnostic","fields":{"severity":"warning","code":"duplicate","message":"duplicate","labels":[]}}"#,
                "cargo-deny 0.19.8",
                "provenance",
                0,
            )
            .is_err()
        );
        let config_status_two = concat!(
            r#"{"fields":{"code":"deprecated","labels":[],"message":"removed key","severity":"error"},"type":"diagnostic"}"#,
            "\n",
            r#"{"fields":{"level":"ERROR","message":"failed to validate configuration"},"type":"log"}"#
        );
        assert!(
            parse_deny_output(config_status_two, "cargo-deny 0.18.3", "provenance", 2,).is_err()
        );
        assert!(
            parse_deny_output(
                r#"{"type":"summary","fields":{}}"#,
                "cargo-deny 0.19.8",
                "provenance",
                2,
            )
            .is_err()
        );
    }

    #[test]
    fn every_findings_exit_mask_must_match_typed_summary_and_diagnostics() {
        for mask in 0..=15 {
            let document = document_for_mask(mask);
            let diagnostics = parse_deny_output(&document, "cargo-deny 0.19.8", "provenance", mask)
                .expect("matching bitmask is complete evidence");
            assert_eq!(diagnostics.len(), mask.count_ones() as usize);
            assert!(
                parse_deny_output(&document, "cargo-deny 0.19.8", "provenance", mask ^ 1,).is_err()
            );
        }
    }

    #[test]
    fn evidence_may_arrive_on_either_channel() {
        let expected = parse_deny_streams("", V019_FINDINGS, "cargo-deny 0.19.8", "provenance", 15)
            .expect("stderr fixture parses");
        let observed = parse_deny_streams(V019_FINDINGS, "", "cargo-deny 0.19.8", "provenance", 15)
            .expect("stdout fixture parses");
        assert_eq!(expected.len(), observed.len());
    }

    #[test]
    fn malformed_unknown_and_incomplete_evidence_fail_closed() {
        assert!(parse_deny_output("{broken", "cargo-deny 0.19.8", "provenance", 0).is_err());
        assert!(
            parse_deny_output(
                concat!(
                    r#"{"type":"diagnostic","fields":{"severity":"warning","code":"future-code","message":"future","labels":[]}}"#,
                    "\n",
                    r#"{"type":"summary","fields":{}}"#
                ),
                "cargo-deny 0.19.8",
                "provenance",
                0,
            )
            .is_err()
        );
        assert!(
            parse_deny_output(
                concat!(
                    r#"{"type":"future","fields":{}}"#,
                    "\n",
                    r#"{"type":"summary","fields":{}}"#
                ),
                "cargo-deny 0.19.8",
                "provenance",
                0,
            )
            .is_err()
        );
    }

    #[test]
    #[ignore = "depends on a qualified cargo-deny 0.18.x or 0.19.x binary"]
    fn real_binary_smoke_uses_qualified_contract() {
        let database = prepare_advisory_database();
        let version = CONTRACT.tool_version();
        assert_eq!(
            CONTRACT.version_support(&version),
            VersionSupport::Supported
        );

        for (fixture, expected_exit, expect_findings) in
            [("clean", 0, false), ("findings", 15, true)]
        {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("evaluation/conformance/cargo-deny/fixtures")
                .join(fixture);
            let child = process::spawn_in_group(
                Command::new("cargo")
                    .args(command_args(true))
                    .current_dir(root)
                    .env("RUST_DOCTOR_DENY_DB_ROOT", database.path())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped()),
            )
            .expect("qualified cargo-deny process starts");
            let output = process::run_with_timeout(child, DENY_TIMEOUT_SECS, MAX_OUTPUT_BYTES)
                .expect("qualified cargo-deny process completes");
            CONTRACT
                .require_complete_run(&output, QUALIFIED_EXIT_CODES)
                .expect("cargo-deny reaches a supported terminal status");
            assert_eq!(output.exit_code, Some(expected_exit));
            assert!(output.stdout.is_empty());

            let diagnostics = parse_deny_streams(
                &output.stdout,
                &output.stderr,
                &version,
                &CONTRACT.provenance(&version),
                expected_exit,
            )
            .expect("real structured output parses");
            assert_eq!(!diagnostics.is_empty(), expect_findings);
            let recorded = match (version.contains("0.18."), fixture) {
                (true, "clean") => V018_CLEAN,
                (true, "findings") => V018_FINDINGS,
                (false, "clean") => V019_CLEAN,
                (false, "findings") => V019_FINDINGS,
                _ => panic!("unsupported conformance fixture"),
            };
            assert_eq!(
                normalize_capture(&output.stdout, &output.stderr),
                fixture_values(recorded),
                "{version} raw evidence drifted from the recorded normalized fixture"
            );
        }
    }

    fn document_for_mask(mask: i32) -> String {
        let mut lines = Vec::new();
        for (bit, code) in [
            (1, "vulnerability"),
            (2, "banned"),
            (4, "rejected"),
            (8, "source-not-allowed"),
        ] {
            if mask & bit != 0 {
                lines.push(
                    serde_json::json!({
                        "type": "diagnostic",
                        "fields": {
                            "severity": "error",
                            "code": code,
                            "message": code,
                            "labels": []
                        }
                    })
                    .to_string(),
                );
            }
        }
        let counts = |bit| {
            serde_json::json!({
                "errors": i32::from(mask & bit != 0),
                "helps": 0,
                "notes": 0,
                "warnings": 0
            })
        };
        lines.push(
            serde_json::json!({
                "type": "summary",
                "fields": {
                    "advisories": counts(1),
                    "bans": counts(2),
                    "licenses": counts(4),
                    "sources": counts(8)
                }
            })
            .to_string(),
        );
        lines.join("\n")
    }

    fn normalize_capture(stdout: &str, stderr: &str) -> Vec<serde_json::Value> {
        let mut normalized = Vec::new();
        let mut seen = BTreeSet::new();
        for line in stdout.lines().chain(stderr.lines()) {
            let value: serde_json::Value =
                serde_json::from_str(line).expect("real binary emits JSON Lines");
            match value["type"].as_str().expect("evidence type") {
                "summary" => normalized.push(value),
                "diagnostic" => {
                    let code = value["fields"]["code"]
                        .as_str()
                        .expect("semantic code")
                        .to_string();
                    if !seen.insert(code.clone()) {
                        continue;
                    }
                    let labels: Vec<_> = value["fields"]["labels"]
                        .as_array()
                        .expect("labels")
                        .iter()
                        .map(|label| serde_json::json!({"message": label["message"]}))
                        .collect();
                    let notes: Vec<_> = value["fields"]["notes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|note| note.as_str())
                        .filter(|note| note.starts_with("ID: RUSTSEC-"))
                        .map(serde_json::Value::from)
                        .collect();
                    let mut fields = serde_json::json!({
                        "severity": value["fields"]["severity"],
                        "code": code,
                        "message": value["fields"]["message"],
                        "labels": labels
                    });
                    if !notes.is_empty() {
                        fields["notes"] = serde_json::Value::Array(notes);
                    }
                    normalized.push(serde_json::json!({
                        "type": "diagnostic",
                        "fields": fields
                    }));
                }
                kind => panic!("unexpected real-binary evidence type: {kind}"),
            }
        }
        sort_capture(&mut normalized);
        normalized
    }

    fn fixture_values(fixture: &str) -> Vec<serde_json::Value> {
        let mut values: Vec<_> = fixture
            .lines()
            .map(|line| serde_json::from_str(line).expect("recorded fixture is JSON Lines"))
            .collect();
        sort_capture(&mut values);
        values
    }

    fn sort_capture(values: &mut [serde_json::Value]) {
        values.sort_by_key(|value| {
            value["fields"]["code"]
                .as_str()
                .map_or_else(|| "~summary".to_string(), str::to_string)
        });
    }

    fn prepare_advisory_database() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("temporary advisory root");
        let repository = root.path().join(ADVISORY_DB_DIR);
        let advisory = repository
            .join("crates")
            .join("rsa")
            .join("RUSTSEC-2023-0071.md");
        fs::create_dir_all(advisory.parent().expect("advisory parent"))
            .expect("advisory directory");
        fs::write(repository.join("support.toml"), ADVISORY_SUPPORT)
            .expect("advisory support manifest");
        fs::write(advisory, RSA_ADVISORY).expect("advisory fixture");

        run_git(&repository, &["init", "--quiet"]);
        run_git(&repository, &["add", "."]);
        run_git(
            &repository,
            &[
                "-c",
                "user.name=Rust Doctor",
                "-c",
                "user.email=rust-doctor@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "cargo-deny conformance database",
            ],
        );
        root
    }

    fn run_git(repository: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repository)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git starts");
        assert!(status.success(), "git command failed: {args:?}");
    }
}
