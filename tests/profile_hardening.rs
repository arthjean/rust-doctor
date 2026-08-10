#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! EP-003: the settings under which the code is compiled are part of the scan.
//!
//! Three rules read what no other pass reads: a release profile compiled
//! without overflow checks, a release profile shipping full debug info
//! unstripped, and a `.cargo/config.toml` whose rustflags switch a check off
//! for every build. Each test here scans a fixture through the same `inspect`
//! a user runs, and is the pointer its rule's `tests/rule_evidence.json` entry
//! names.

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

/// US-009: a binary workspace whose `[profile.release]` does not enable
/// overflow checks is reported once, at the section that omits the setting,
/// and the help names the measured cost so the reader can decline the advice.
#[test]
fn an_unchecked_release_profile_is_reported_for_a_binary_workspace() {
    let report = report("unchecked");
    let observed = diagnostics_of(&report, OVERFLOW);
    assert_eq!(observed.len(), 1, "{report}");
    let finding = observed[0];
    assert_eq!(finding["path"], "Cargo.toml");
    // The fixture writes `[profile.release]` on line 7 of its manifest.
    assert_eq!(finding["span"]["line_start"], 7, "{finding}");
    assert!(
        finding["help"]
            .as_str()
            .is_some_and(|help| help.contains("a few percent")),
        "the help does not name the measured cost: {finding}"
    );
    assert!(diagnostics_of(&report, DEBUG_SYMBOLS).is_empty());
}

/// US-009: `overflow-checks = true` silences the overflow rule, `strip`
/// silences the debug rule even with `debug = true`, a profile inheriting
/// from release is not judged, and benign rustflags are not this family's
/// subject.
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

/// US-009: with no `[profile.release]` at all, the overflow rule still fires,
/// because Cargo's release default is `overflow-checks = false`, and the
/// debug rule does not, because Cargo's release default ships no debug info.
/// The finding carries no span: no manifest line is the defect.
#[test]
fn a_missing_release_section_reports_overflow_and_nothing_else() {
    let report = report("default-profile");
    let observed = diagnostics_of(&report, OVERFLOW);
    assert_eq!(observed.len(), 1, "{report}");
    assert_eq!(observed[0]["path"], "Cargo.toml");
    assert!(observed[0]["span"].is_null(), "{}", observed[0]);
    assert!(diagnostics_of(&report, DEBUG_SYMBOLS).is_empty());
}

/// US-008 and US-009: a virtual workspace manifest with no `[package]` is
/// still the manifest the profiles are read from. Its `debug = true` with no
/// `strip` ships symbols, and its binary member makes the overflow rule fire;
/// both findings name the root manifest, and the debug finding points at the
/// line that carries the setting.
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
    // The fixture writes `debug = true` on line 6 of the root manifest.
    assert_eq!(finding["span"]["line_start"], 6, "{finding}");
    assert!(
        finding["message"]
            .as_str()
            .is_some_and(|message| message.contains("build paths")),
        "the finding does not state what ships in the binary: {finding}"
    );
}

/// US-010: each flag of the closed list is reported under its canonical
/// spelling, wherever it was written: the separated and joined forms in
/// `[build]`, and the joined string spelling under a `[target.*]` table. The
/// findings name the configuration file, never the workspace path.
#[test]
fn neutralizing_rustflags_are_reported_in_both_spellings() {
    let report = report("rustflags");
    // The fixture ships no binary target: the overflow rule judges only a
    // workspace that produces one (US-009).
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

/// FR-15: all three rules stay switchable off through the same override
/// surface as every other rule.
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
