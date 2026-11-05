#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::{Path, PathBuf};

use rust_doctor::{InspectRequest, RuleLevel, RuleOverride, Status, inspect};
use serde_json::Value;

const OVERFLOW: &str = "rust_doctor::cargo::unchecked_release_overflow";
const DEBUG_SYMBOLS: &str = "rust_doctor::cargo::release_debug_symbols";
const RUSTFLAGS: &str = "rust_doctor::cargo::permissive_rustflags";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/profile-hardening")
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

#[test]
fn an_unchecked_release_profile_is_reported_for_a_binary_workspace() {
    let report = report("unchecked");
    let observed = diagnostics_of(&report, OVERFLOW);
    assert_eq!(observed.len(), 1, "{report}");
    let finding = observed[0];
    assert_eq!(finding["path"], "Cargo.toml");
    assert_eq!(finding["span"]["line_start"], 7, "{finding}");
    assert!(
        finding["help"]
            .as_str()
            .is_some_and(|help| help.contains("a few percent")),
        "the help does not name the measured cost: {finding}"
    );
    assert!(diagnostics_of(&report, DEBUG_SYMBOLS).is_empty());
}

#[test]
fn a_hardened_release_profile_and_benign_rustflags_stay_silent() {
    let report = report("checked");
    for code in [OVERFLOW, DEBUG_SYMBOLS, RUSTFLAGS] {
        assert!(
            diagnostics_of(&report, code).is_empty(),
            "{code} fired on the hardened fixture: {report}"
        );
    }
}

#[test]
fn a_missing_release_section_reports_overflow_and_nothing_else() {
    let report = report("default-profile");
    let observed = diagnostics_of(&report, OVERFLOW);
    assert_eq!(observed.len(), 1, "{report}");
    assert_eq!(observed[0]["path"], "Cargo.toml");
    assert!(observed[0]["span"].is_null(), "{}", observed[0]);
    assert!(diagnostics_of(&report, DEBUG_SYMBOLS).is_empty());
}

#[test]
fn a_virtual_workspace_manifest_carries_both_profile_findings() {
    let report = report("virtual");
    let overflow = diagnostics_of(&report, OVERFLOW);
    assert_eq!(overflow.len(), 1, "{report}");
    assert_eq!(overflow[0]["path"], "Cargo.toml");

    let symbols = diagnostics_of(&report, DEBUG_SYMBOLS);
    assert_eq!(symbols.len(), 1, "{report}");
    let finding = symbols[0];
    assert_eq!(finding["path"], "Cargo.toml");
    assert_eq!(finding["span"]["line_start"], 6, "{finding}");
    assert!(
        finding["message"]
            .as_str()
            .is_some_and(|message| message.contains("build paths")),
        "the finding does not state what ships in the binary: {finding}"
    );

    let packages = report["audit"]["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 1, "{report}");
    let member = &packages[0]["name"];
    assert_eq!(&overflow[0]["package"], member, "{report}");
    assert_eq!(&finding["package"], member, "{report}");
    assert_eq!(
        packages[0]["score"]["value"], report["audit"]["score"]["value"],
        "{report}"
    );
}

#[test]
fn neutralizing_rustflags_are_reported_in_both_spellings() {
    let report = report("rustflags");
    assert!(diagnostics_of(&report, OVERFLOW).is_empty(), "{report}");
    let observed = diagnostics_of(&report, RUSTFLAGS);
    assert_eq!(observed.len(), 3, "{report}");
    for finding in &observed {
        assert_eq!(finding["path"], ".cargo/config.toml");
        assert!(
            finding["help"]
                .as_str()
                .is_some_and(|help| help.contains("--cap-lints allow")),
            "the help does not state the closed list: {finding}"
        );
    }
    let messages: Vec<&str> = observed
        .iter()
        .filter_map(|finding| finding["message"].as_str())
        .collect();
    for flag in ["--cap-lints allow", "-A warnings", "-C overflow-checks=off"] {
        assert!(
            messages.iter().any(|message| message.contains(flag)),
            "{flag} is not named: {messages:?}"
        );
    }
}

#[test]
fn the_three_rules_are_switchable_off() {
    for (name, codes) in [
        ("virtual", [OVERFLOW, DEBUG_SYMBOLS].as_slice()),
        ("rustflags", [RUSTFLAGS].as_slice()),
    ] {
        let mut request = InspectRequest::new(fixture(name));
        for code in codes {
            request = request.with_rule_override(RuleOverride::new(*code, RuleLevel::Off));
        }
        let report = inspect(request);
        assert_eq!(report.status, Status::Complete, "{name}: {:?}", report.errors);
        let report = serde_json::to_value(&report).expect("a valid report should serialize");
        for code in codes {
            assert!(
                diagnostics_of(&report, code).is_empty(),
                "{name}: {code} still fired switched off"
            );
        }
    }
}
