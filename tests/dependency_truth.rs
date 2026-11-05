#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::{Path, PathBuf};

use rust_doctor::{InspectRequest, Status, inspect};
use serde_json::Value;

const UNUSED: &str = "rust_doctor::cargo::unused_dependency";
const TEST_ONLY: &str = "rust_doctor::cargo::test_only_dependency";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dependency-truth")
        .join(name)
}

fn report(name: &str) -> Value {
    let report = inspect(InspectRequest::new(fixture(name)));
    assert_eq!(report.status, Status::Complete, "{name}: {:?}", report.errors);
    serde_json::to_value(&report).expect("a valid report should serialize")
}

fn diagnostics_of<'a>(report: &'a Value, code: &str) -> Vec<&'a Value> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter(|diagnostic| diagnostic["code"] == code)
        .collect()
}

fn messages_of(report: &Value, code: &str) -> Vec<String> {
    diagnostics_of(report, code)
        .iter()
        .filter_map(|diagnostic| diagnostic["message"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn only_the_never_referenced_dependency_is_reported_as_unused() {
    let report = report("unused");
    let observed = diagnostics_of(&report, UNUSED);
    assert_eq!(observed.len(), 2, "{observed:?}");

    let root = observed
        .iter()
        .find(|diagnostic| diagnostic["package"] == "dependency-truth-unused")
        .expect("the root member should report its dead entry");
    assert_eq!(root["path"], "Cargo.toml");
    assert!(
        root["message"]
            .as_str()
            .is_some_and(|message| message.contains("\"never-used\"")),
        "{root}"
    );
    assert!(
        root["help"]
            .as_str()
            .is_some_and(|help| help.contains("rust-doctor.toml")),
        "the escape hatch is not named: {root}"
    );

    let beta = observed
        .iter()
        .find(|diagnostic| diagnostic["package"] == "dependency-truth-beta")
        .expect("the sibling member should report its dead entry");
    assert_eq!(beta["path"], "beta/Cargo.toml");
    assert!(
        beta["message"]
            .as_str()
            .is_some_and(|message| message.contains("\"shared-helper\"")),
        "{beta}"
    );

    let score = report["audit"]["score"]["value"]
        .as_u64()
        .expect("a complete scan should publish a score");
    assert!(score < 100, "two dead entries left the score at {score}");
}

#[test]
fn test_only_dependencies_are_reported_with_their_reference_site() {
    let report = report("test-only");
    let messages = messages_of(&report, TEST_ONLY);
    assert_eq!(messages.len(), 2, "{messages:?}");

    assert!(
        messages.iter().any(|message| {
            message.contains("\"probe-integration\"")
                && message.contains("test, bench or example")
        }),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|message| {
            message.contains("\"probe-inline\"") && message.contains("inline #[cfg(test)] module")
        }),
        "{messages:?}"
    );

    let unused = messages_of(&report, UNUSED);
    assert_eq!(unused.len(), 1, "{unused:?}");
    assert!(unused[0].contains("\"orphan\""), "{unused:?}");
    assert!(
        !messages.iter().any(|message| message.contains("\"orphan\"")),
        "one entry produced two findings: {messages:?}"
    );
}

#[test]
fn both_rules_are_switchable_off() {
    let request = InspectRequest::new(fixture("test-only"))
        .with_rule_override(rust_doctor::RuleOverride::new(UNUSED, rust_doctor::RuleLevel::Off))
        .with_rule_override(rust_doctor::RuleOverride::new(
            TEST_ONLY,
            rust_doctor::RuleLevel::Off,
        ));
    let report = inspect(request);
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    let report = serde_json::to_value(&report).expect("a valid report should serialize");
    assert!(diagnostics_of(&report, UNUSED).is_empty());
    assert!(diagnostics_of(&report, TEST_ONLY).is_empty());
}
