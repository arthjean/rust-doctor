#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rust_doctor::{
    CategoryOverride, DiagnosticSource, GateStatus, InspectRequest, RuleLevel, RuleOverride,
    Severity, Status, inspect,
};
use serde_json::Value;

mod support;

use support::rule_scaling::{ExpectedSpan, oracle};

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rule-scaling-kernel")
        .join(path)
}

fn target(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/rule-scaling-kernel")
        .join(name)
}

fn clippy(name: &str, manifest: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO"))
        .arg("clippy")
        .arg("--manifest-path")
        .arg(manifest)
        .args(arguments)
        .current_dir(
            manifest
                .parent()
                .expect("fixture manifest should have a parent"),
        )
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", target(name))
        .output()
        .expect("Clippy oracle process should start")
}

fn records(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn diagnostics<'a>(records: &'a [Value], codes: &BTreeSet<&str>) -> Vec<&'a Value> {
    records
        .iter()
        .filter(|record| {
            record["reason"] == "compiler-message"
                && record["message"]["code"]["code"]
                    .as_str()
                    .is_some_and(|code| codes.contains(code))
        })
        .collect()
}

fn primary_span(diagnostic: &Value) -> &Value {
    diagnostic["message"]["spans"]
        .as_array()
        .and_then(|spans| spans.iter().find(|span| span["is_primary"] == true))
        .expect("candidate diagnostic should have a primary span")
}

fn assert_primary_span(diagnostic: &Value, expected: &ExpectedSpan, id: &str) {
    let span = primary_span(diagnostic);
    assert_eq!(span["file_name"], "src/lib.rs", "{id}");
    assert_eq!(span["line_start"], expected.line_start, "{id}");
    assert_eq!(span["column_start"], expected.column_start, "{id}");
    assert_eq!(span["line_end"], expected.line_end, "{id}");
    assert_eq!(span["column_end"], expected.column_end, "{id}");
}

fn diagnostic_signature(diagnostic: &Value) -> (String, String, u64, u64, u64, u64) {
    let span = primary_span(diagnostic);
    (
        diagnostic["message"]["code"]["code"]
            .as_str()
            .expect("candidate code should be structured")
            .to_owned(),
        span["file_name"]
            .as_str()
            .expect("primary span should have a path")
            .to_owned(),
        span["line_start"].as_u64().unwrap(),
        span["column_start"].as_u64().unwrap(),
        span["line_end"].as_u64().unwrap(),
        span["column_end"].as_u64().unwrap(),
    )
}

#[test]
fn target_toolchain_help_and_direct_candidates_match_the_captured_oracle() {
    let oracle = oracle();
    for (program, argument, expected) in [
        ("rustc", "--version", oracle.toolchain.rustc.as_str()),
        (env!("CARGO"), "--version", oracle.toolchain.cargo.as_str()),
        (
            "clippy-driver",
            "--version",
            oracle.toolchain.clippy.as_str(),
        ),
    ] {
        let output = Command::new(program).arg(argument).output().unwrap();
        assert!(output.status.success(), "{program}");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim_end(), expected);
    }

    let help = Command::new("clippy-driver")
        .args(["-W", "help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    let advertised: BTreeMap<_, _> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let level = fields.next()?;
            name.starts_with("clippy::").then(|| {
                (
                    format!("clippy::{}", name["clippy::".len()..].replace('-', "_")),
                    level.to_owned(),
                )
            })
        })
        .collect();
    for rule in &oracle.rules {
        assert_eq!(
            advertised.get(&rule.id).map(String::as_str),
            Some(rule.clippy_default.as_str()),
            "{}",
            rule.id
        );
    }

    let manifest = fixture("oracle/Cargo.toml");
    let mut arguments = vec!["--message-format=json", "--"];
    arguments.extend(oracle.explicit_flags());
    let output = clippy("direct", &manifest, &arguments);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records = records(&output);
    let codes = oracle.candidate_ids();
    let findings = diagnostics(&records, &codes);
    assert_eq!(findings.len(), oracle.rules.len());
    for rule in &oracle.rules {
        let finding = findings
            .iter()
            .find(|finding| finding["message"]["code"]["code"] == rule.id)
            .unwrap();
        assert_eq!(finding["message"]["level"], "warning", "{}", rule.id);
        assert_eq!(finding["message"]["message"], rule.message, "{}", rule.id);
        assert_primary_span(finding, &rule.integration_span, &rule.id);
    }
    assert!(records.iter().any(|record| {
        record["reason"] == "build-script-executed"
            && record["package_id"]
                .as_str()
                .is_some_and(|id| id.contains("rule-scaling-oracle"))
    }));
    let build_output_codes: BTreeSet<_> = findings
        .iter()
        .filter(|finding| primary_span(finding)["file_name"] == "build.rs")
        .map(|finding| finding["message"]["code"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        build_output_codes,
        oracle
            .observed_contexts
            .build_output_candidate_codes
            .iter()
            .map(String::as_str)
            .collect()
    );
}

#[test]
fn each_candidate_has_an_isolated_positive_with_exact_baseline_behavior() {
    let oracle = oracle();
    let codes = oracle.candidate_ids();

    for rule in &oracle.rules {
        let manifest = fixture(&rule.positive_fixture);
        let name = rule.id.replace("clippy::", "").replace('_', "-");
        let explicit = clippy(
            &format!("positive-{name}-explicit"),
            &manifest,
            &["--message-format=json", "--", "-W", rule.id.as_str()],
        );
        assert!(
            explicit.status.success(),
            "{}: {}",
            rule.id,
            String::from_utf8_lossy(&explicit.stderr)
        );
        let explicit_records = records(&explicit);
        let explicit_findings = diagnostics(&explicit_records, &codes);
        assert_eq!(explicit_findings.len(), 1, "{}", rule.id);
        let finding = explicit_findings[0];
        assert_eq!(finding["message"]["code"]["code"], rule.id, "{}", rule.id);
        assert_eq!(finding["message"]["level"], "warning", "{}", rule.id);
        assert_eq!(finding["message"]["message"], rule.message, "{}", rule.id);
        assert_primary_span(finding, &rule.positive_span, &rule.id);

        let baseline = clippy(
            &format!("positive-{name}-baseline"),
            &manifest,
            &["--message-format=json"],
        );
        assert!(baseline.status.success(), "{}", rule.id);
        let baseline_records = records(&baseline);
        let baseline_findings = diagnostics(&baseline_records, &codes);
        if rule.clippy_default.as_str() == "allow" {
            assert!(baseline_findings.is_empty(), "{}", rule.id);
        } else {
            assert_eq!(rule.clippy_default.as_str(), "warn", "{}", rule.id);
            assert_eq!(baseline_findings.len(), 1, "{}", rule.id);
            assert_eq!(
                diagnostic_signature(baseline_findings[0]),
                diagnostic_signature(finding),
                "{}",
                rule.id
            );
        }
    }
}

#[test]
fn baseline_levels_suppression_denial_and_macro_boundaries_are_observed() {
    let oracle = oracle();
    let manifest = fixture("oracle/Cargo.toml");
    let codes = oracle.candidate_ids();

    let baseline = clippy("baseline", &manifest, &["--message-format=json"]);
    assert!(baseline.status.success());
    let baseline_records = records(&baseline);
    let baseline_findings = diagnostics(&baseline_records, &codes);
    let baseline_codes: BTreeSet<_> = baseline_findings
        .iter()
        .map(|finding| finding["message"]["code"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        baseline_codes,
        oracle
            .rules
            .iter()
            .filter(|rule| rule.clippy_default.as_str() == "warn")
            .map(|rule| rule.id.as_str())
            .collect()
    );
    for finding in baseline_findings {
        let id = finding["message"]["code"]["code"].as_str().unwrap();
        let expected_line = oracle
            .rules
            .iter()
            .find(|rule| rule.id == id)
            .unwrap()
            .integration_span
            .line_start;
        assert_eq!(primary_span(finding)["line_start"], expected_line, "{id}");
    }

    let mut allowed_arguments = vec!["--features", "allowed", "--message-format=json", "--"];
    allowed_arguments.extend(oracle.explicit_flags());
    let allowed = clippy("allowed", &manifest, &allowed_arguments);
    assert!(allowed.status.success());
    let allowed_records = records(&allowed);
    let allowed_codes: BTreeSet<_> = diagnostics(&allowed_records, &codes)
        .iter()
        .map(|finding| finding["message"]["code"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        allowed_codes,
        oracle
            .observed_contexts
            .source_allow
            .iter()
            .map(String::as_str)
            .collect()
    );

    let mut denied_arguments = vec!["--features", "denied", "--message-format=json", "--"];
    denied_arguments.extend(oracle.explicit_flags());
    let denied = clippy("denied", &manifest, &denied_arguments);
    assert!(!denied.status.success());
    let denied_records = records(&denied);
    let denied_findings = diagnostics(&denied_records, &codes);
    let denied_codes: BTreeSet<_> = denied_findings
        .iter()
        .map(|finding| finding["message"]["code"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        denied_codes,
        oracle
            .observed_contexts
            .source_deny
            .iter()
            .map(String::as_str)
            .collect()
    );
    let completion = denied_records
        .iter()
        .position(|record| record["reason"] == "build-finished" && record["success"] == false)
        .unwrap();
    for rule in &oracle.rules {
        let position = denied_records
            .iter()
            .position(|record| {
                record["reason"] == "compiler-message"
                    && record["message"]["code"]["code"] == rule.id
                    && record["message"]["level"] == "error"
            })
            .unwrap();
        assert!(position < completion, "{}", rule.id);
    }

    let mut macro_arguments = vec!["--features", "local-macro", "--message-format=json", "--"];
    macro_arguments.extend(oracle.explicit_flags());
    let local_macro = clippy("local-macro", &manifest, &macro_arguments);
    assert!(local_macro.status.success());
    let macro_records = records(&local_macro);
    let macro_findings = diagnostics(&macro_records, &codes);
    let macro_spans: BTreeMap<_, _> = macro_findings
        .iter()
        .map(|finding| {
            (
                finding["message"]["code"]["code"].as_str().unwrap(),
                primary_span(finding)["line_start"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        macro_spans,
        oracle
            .observed_contexts
            .local_macro
            .iter()
            .map(|span| (span.id.as_str(), span.primary_line))
            .collect()
    );
    assert!(macro_findings.iter().all(|finding| {
        let span = primary_span(finding);
        span["file_name"] == "src/lib.rs" && span["line_start"].as_u64().is_some()
    }));

    let dependency_manifest = fixture("dependency-app/Cargo.toml");
    let mut dependency_arguments = vec!["--no-deps", "--message-format=json", "--"];
    dependency_arguments.extend(oracle.explicit_flags());
    let dependency = clippy("dependency", &dependency_manifest, &dependency_arguments);
    assert!(dependency.status.success());
    let dependency_records = records(&dependency);
    let dependency_findings = diagnostics(&dependency_records, &codes);
    let dependency_spans: BTreeMap<_, _> = dependency_findings
        .iter()
        .map(|finding| {
            let span = primary_span(finding);
            assert!(
                span["file_name"]
                    .as_str()
                    .unwrap()
                    .ends_with("rule-scaling-kernel/dependency/src/lib.rs")
            );
            (
                finding["message"]["code"]["code"].as_str().unwrap(),
                span["line_start"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        dependency_spans,
        oracle
            .observed_contexts
            .external_macro_under_no_deps
            .iter()
            .map(|span| (span.id.as_str(), span.primary_line))
            .collect()
    );
    let dependency_direct_codes: BTreeSet<_> = dependency_findings
        .iter()
        .filter(|finding| primary_span(finding)["line_start"] == 10)
        .map(|finding| finding["message"]["code"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        dependency_direct_codes,
        oracle
            .observed_contexts
            .dependency_direct_candidate_codes
            .iter()
            .map(String::as_str)
            .collect()
    );
}

#[test]
fn default_report_and_policy_overrides_cover_all_five_rules_without_schema_change() {
    let oracle = oracle();
    let path = fixture("oracle");
    let baseline = inspect(InspectRequest::new(&path));
    assert_eq!(baseline.schema_version, 13);
    assert_eq!(baseline.status, Status::Complete);
    assert!(baseline.complete);
    let published = serde_json::to_value(&baseline).expect("a valid report should serialize");
    assert_eq!(
        baseline.scan.command.as_deref().unwrap(),
        support::expected_clippy_command(&published["policy"])
    );
    // The frozen command of EP-018 stays an ordered subsequence of the current
    // command, minus one argument: no widening of the catalog removed or moved
    // a rule already admitted, and the single withdrawal is named here rather
    // than erased from the oracle. `--all-targets` compiled tests, benches,
    // examples and build scripts, and the scope is now Cargo's default targets.
    const WITHDRAWN: &str = "--all-targets";
    let mut current = baseline.scan.command.as_deref().unwrap().iter();
    for historical in &oracle.clippy_command {
        if historical == WITHDRAWN {
            continue;
        }
        assert!(
            current.any(|argument| argument == historical),
            "{historical} left the command"
        );
    }
    assert!(
        oracle
            .clippy_command
            .iter()
            .any(|argument| argument == WITHDRAWN)
    );
    assert!(
        !baseline
            .scan
            .command
            .as_deref()
            .unwrap()
            .iter()
            .any(|argument| argument == WITHDRAWN)
    );
    assert_eq!(
        baseline
            .scan
            .command
            .as_ref()
            .unwrap()
            .iter()
            .filter(|argument| argument.as_str() == "--")
            .count(),
        1
    );

    assert_eq!(oracle.historical_rules.len(), 7);
    // Rules admitted after EP-018 that find something on this fixture, with
    // the number of findings each contributes. The `expect()` of the
    // `zombie_processes` case is one of them since EP-024. Of the two families
    // `duplicate_function_body` reported since EP-002, one remains: the
    // fixture repeats the same pair of cases three times over, once per
    // written form of the same call, and `spaced_argument` is the only member
    // whose body carries the three top-level statements the admission floor
    // requires since the EP-005 design revision. The single-statement chains
    // fell under `MINIMUM_STATEMENTS`, which is that revision working as
    // measured.
    // The two suppression-audit rules of EP-001 both fire on this fixture by
    // design: its crate-level `#![allow(dead_code, reason = ...)]` is exactly
    // the file-wide scope `crate_level_allow` reports whatever the reason, and
    // its quiet counterpart carries one attribute naming four lints, which is
    // the accumulation `stacked_allow_attribute` reports.
    const ADMITTED_AFTER_EP018: [(&str, usize); 4] = [
        ("clippy::expect_used", 1),
        ("rust_doctor::structure::crate_level_allow", 1),
        ("rust_doctor::structure::duplicate_function_body", 1),
        ("rust_doctor::structure::stacked_allow_attribute", 1),
    ];
    let admitted_after_ep018: usize = ADMITTED_AFTER_EP018.iter().map(|(_, count)| count).sum();
    assert_eq!(
        baseline.diagnostics.len(),
        oracle.rules.len() + oracle.historical_rules.len() + admitted_after_ep018
    );
    for (code, expected) in ADMITTED_AFTER_EP018 {
        assert_eq!(
            baseline
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code.as_deref() == Some(code))
                .count(),
            expected,
            "{code}"
        );
    }
    let historical_codes: BTreeSet<_> = oracle
        .historical_rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect();
    for expected in &oracle.historical_rules {
        let findings: Vec<_> = baseline
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_deref() == Some(expected.id.as_str()))
            .collect();
        assert_eq!(findings.len(), 1, "{}", expected.id);
        let finding = findings[0];
        assert_eq!(finding.message, expected.message, "{}", expected.id);
        assert_eq!(
            finding.path.as_deref(),
            Some(expected.path.as_str()),
            "{}",
            expected.id
        );
        assert_eq!(
            serde_json::to_value(finding.span.as_ref()).unwrap(),
            serde_json::to_value(expected.span.as_ref()).unwrap(),
            "{}",
            expected.id
        );
    }
    for candidate in baseline.diagnostics.iter().filter(|diagnostic| {
        diagnostic
            .code
            .as_deref()
            .is_some_and(|code| oracle.rules.iter().any(|rule| rule.id == code))
    }) {
        for historical in baseline.diagnostics.iter().filter(|diagnostic| {
            diagnostic
                .code
                .as_deref()
                .is_some_and(|code| historical_codes.contains(code))
        }) {
            assert_ne!(
                (candidate.path.as_ref(), candidate.span.as_ref()),
                (historical.path.as_ref(), historical.span.as_ref()),
                "candidate and historical rules must not duplicate one cause"
            );
        }
    }

    for rule in &oracle.rules {
        let id = rule.id.as_str();
        let finding = baseline
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code.as_deref() == Some(id))
            .unwrap();
        assert_eq!(finding.source, DiagnosticSource::Clippy, "{id}");
        assert_eq!(
            finding.category.as_deref(),
            Some(rule.category.as_str()),
            "{id}"
        );
        assert_eq!(finding.base_severity, Severity::Warning, "{id}");
        assert_eq!(finding.severity, Severity::Warning, "{id}");
        assert_eq!(finding.help.as_deref(), Some(rule.help.as_str()), "{id}");
        assert_eq!(finding.message, rule.message, "{id}");
        assert_eq!(finding.path.as_deref(), Some("src/lib.rs"), "{id}");
        assert_eq!(
            serde_json::to_value(finding.span.as_ref()).unwrap(),
            serde_json::to_value(Some(&rule.integration_span)).unwrap(),
            "{id}"
        );
        let baseline_id = finding.id.clone();

        let off = inspect(
            InspectRequest::new(&path).with_rule_override(RuleOverride::new(id, RuleLevel::Off)),
        );
        assert_eq!(off.status, Status::Complete, "{id}");
        assert!(
            off.diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code.as_deref() != Some(id))
        );
        assert!(
            !off.scan
                .command
                .as_ref()
                .unwrap()
                .windows(2)
                .any(|pair| { pair[0] == "-W" && pair[1] == id })
        );

        let warn = inspect(
            InspectRequest::new(&path).with_rule_override(RuleOverride::new(id, RuleLevel::Warn)),
        );
        let warning = warn
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code.as_deref() == Some(id))
            .unwrap();
        assert_eq!(warning.severity, Severity::Warning, "{id}");
        assert_eq!(warning.id, baseline_id, "{id}");

        let error = inspect(
            InspectRequest::new(&path).with_rule_override(RuleOverride::new(id, RuleLevel::Error)),
        );
        let error_finding = error
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code.as_deref() == Some(id))
            .unwrap();
        assert_eq!(error_finding.base_severity, Severity::Warning, "{id}");
        assert_eq!(error_finding.severity, Severity::Error, "{id}");
        assert_eq!(error_finding.id, baseline_id, "{id}");
        assert!(
            error
                .scan
                .command
                .as_ref()
                .unwrap()
                .windows(2)
                .any(|pair| { pair[0] == "-W" && pair[1] == id })
        );
        assert!(
            !error
                .scan
                .command
                .as_ref()
                .unwrap()
                .contains(&"-D".to_owned())
        );
    }

    let precedence = inspect(
        InspectRequest::new(&path)
            .with_category_override(CategoryOverride::new("reliability", RuleLevel::Off))
            .with_rule_override(RuleOverride::new("clippy::mem_forget", RuleLevel::Error)),
    );
    assert_eq!(
        precedence
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code.as_deref() == Some("clippy::mem_forget"))
            .map(|diagnostic| diagnostic.severity),
        Some(Severity::Error)
    );
    assert!(
        precedence
            .diagnostics
            .iter()
            .all(|diagnostic| { diagnostic.code.as_deref() != Some("clippy::zombie_processes") })
    );
}

#[test]
fn denied_candidate_is_retained_in_an_incomplete_report_and_hostile_policy_stops_early() {
    let oracle = oracle();
    let denied = inspect(InspectRequest::new(fixture("denied")));
    assert_eq!(denied.schema_version, 13);
    assert_eq!(denied.status, Status::Incomplete);
    assert!(!denied.complete);
    assert_eq!(denied.gate.status, GateStatus::NotEvaluated);
    for rule in &oracle.rules {
        let id = rule.id.as_str();
        assert!(
            denied.diagnostics.iter().any(|diagnostic| {
                diagnostic.code.as_deref() == Some(id)
                    && diagnostic.base_severity == Severity::Error
            }),
            "{id}"
        );
    }

    let hostile = "clippy::mem_forget/private\u{1b}[31m";
    let rejected = inspect(
        InspectRequest::new("/path/that/must/not/be/inspected")
            .with_rule_override(RuleOverride::new(hostile, RuleLevel::Warn)),
    );
    assert_eq!(rejected.status, Status::Failed);
    assert!(rejected.scan.command.is_none());
    let rendered = serde_json::to_string(&rejected).unwrap();
    assert!(!rendered.contains(hostile));
    assert!(!rendered.contains("/path/that/must/not/be/inspected"));
    assert!(!rendered.contains('\u{1b}'));
}
