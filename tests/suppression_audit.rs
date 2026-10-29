#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! EP-001: a suppression is visible as a finding.
//!
//! Three rules make silencing the scanner as visible as the code it hides: an
//! inner `#![allow]` covering a whole file or module, several `allow`
//! attributes stacked on one item, and a `[lints]` table that switches a
//! catalogued rule off from the manifest. Each test here scans a fixture
//! through the same `inspect` a user runs, and is the pointer its rule's
//! `tests/rule_evidence.json` entry names.

use std::path::{Path, PathBuf};

use rust_doctor::{InspectRequest, RuleLevel, RuleOverride, Status, inspect};
use serde_json::Value;

const CRATE_LEVEL: &str = "rust_doctor::structure::crate_level_allow";
const STACKED: &str = "rust_doctor::structure::stacked_allow_attribute";
const LINT_TABLE: &str = "rust_doctor::cargo::permissive_lint_table";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/suppression")
        .join(name)
}

fn report(name: &str) -> Value {
    report_of(name, InspectRequest::new(fixture(name)))
}

fn report_of(name: &str, request: InspectRequest) -> Value {
    let report = inspect(request);
    assert_eq!(report.status, Status::Complete, "{name}: {:?}", report.errors);
    serde_json::to_value(&report).expect("a valid report should serialize")
}

/// The packages a rule's diagnostics are reported in.
fn packages_of(report: &Value, code: &str) -> Vec<String> {
    diagnostics_of(report, code)
        .iter()
        .filter_map(|diagnostic| diagnostic["package"].as_str().map(str::to_owned))
        .collect()
}

fn diagnostics_of<'a>(report: &'a Value, code: &str) -> Vec<&'a Value> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter(|diagnostic| diagnostic["code"] == code)
        .collect()
}

/// EP-001 Definition of Done: a workspace whose only defect is a blanket
/// `#![allow(clippy::all)]` in `lib.rs` yields at least one diagnostic and a
/// score below 100.
#[test]
fn a_blanket_crate_allow_costs_the_score_its_perfection() {
    let report = report("blanket-allow");
    let observed = diagnostics_of(&report, CRATE_LEVEL);
    assert_eq!(observed.len(), 1, "{report}");
    let finding = observed[0];
    assert_eq!(finding["path"], "src/lib.rs");
    assert_eq!(finding["span"]["line_start"], 1);
    assert!(
        finding["message"]
            .as_str()
            .is_some_and(|message| message.contains("clippy::all")),
        "the finding does not name the lints it covers: {finding}"
    );

    let score = report["audit"]["score"]["value"]
        .as_u64()
        .expect("a complete scan should publish a score");
    assert!(score < 100, "a blanket suppression left the score at {score}");
}

/// US-001: the inner attribute is reported at file and at inline-module scope,
/// reasoned or not; an outer `#[allow]` on a single item is not this rule's
/// subject; a finding inside a test target carries the mark and does not
/// weigh.
#[test]
fn a_crate_level_allow_is_reported_with_its_scope() {
    let report = report("attribute-scope");
    let observed = diagnostics_of(&report, CRATE_LEVEL);

    let subjects: Vec<&str> = observed
        .iter()
        .filter_map(|diagnostic| diagnostic["message"].as_str())
        .collect();
    assert_eq!(observed.len(), 3, "{subjects:?}");

    let file_wide = observed
        .iter()
        .find(|diagnostic| {
            diagnostic["path"] == "src/lib.rs"
                && diagnostic["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("whole file"))
        })
        .expect("the file-level allow should be reported");
    assert!(
        file_wide["message"]
            .as_str()
            .is_some_and(|message| message.contains("clippy::unwrap_used")),
        "{file_wide}"
    );
    assert!(file_wide["context"].is_null(), "{file_wide}");

    for module in ["imports", "reasoned"] {
        assert!(
            subjects
                .iter()
                .any(|subject| subject.contains(&format!("module \"{module}\""))),
            "module {module} is not reported: {subjects:?}"
        );
    }

    // US-024: the allow inside the test target is test code, which this rule
    // leaves out, so it cannot join a production family either.
    assert!(
        observed
            .iter()
            .all(|diagnostic| diagnostic["path"] != "tests/probe.rs"),
        "the allow inside the test target was reported: {subjects:?}"
    );

    // The outer `#[allow]` items belong to the stacked and unreasoned rules:
    // nothing here points at them.
    assert!(
        observed
            .iter()
            .all(|diagnostic| diagnostic["span"]["line_start"] != 17),
        "an outer allow on a single item was reported as crate-level"
    );
}

/// US-002: two separate attributes and one four-lint attribute are each one
/// finding; a single narrow allow and a `cfg_attr` stay quiet.
#[test]
fn stacked_allow_attributes_are_reported_once_per_item() {
    let report = report("attribute-scope");
    let observed = diagnostics_of(&report, STACKED);
    assert_eq!(observed.len(), 2, "{report}");

    assert!(
        observed.iter().any(|diagnostic| {
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.starts_with("2 allow attributes"))
        }),
        "the two stacked attributes are not reported"
    );
    assert!(
        observed.iter().any(|diagnostic| {
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("4 suppressions"))
        }),
        "the four-lint attribute is not reported"
    );
    for diagnostic in &observed {
        assert_eq!(diagnostic["path"], "src/lib.rs");
        assert_eq!(diagnostic["category"], "maintainability");
        assert_eq!(diagnostic["severity"], "warning");
    }
}

/// US-003: a `[lints.clippy]` entry that sets a catalogued, active rule to
/// `allow` is reported in both spellings, with a span pointing at the key; a
/// `deny`, an uncatalogued lint and a `[lints.rust]` entry stay quiet.
#[test]
fn a_permissive_lint_table_entry_is_reported_with_its_span() {
    let report = report("lint-table");
    let observed = diagnostics_of(&report, LINT_TABLE);

    let messages: Vec<&str> = observed
        .iter()
        .filter_map(|diagnostic| diagnostic["message"].as_str())
        .collect();
    assert_eq!(observed.len(), 2, "{messages:?}");
    for rule in ["clippy::unwrap_used", "clippy::expect_used"] {
        assert!(
            messages.iter().any(|message| message.contains(rule)),
            "{rule} is not reported: {messages:?}"
        );
    }
    for quiet in ["clippy::panic", "needless_range_loop", "unsafe_code"] {
        assert!(
            messages.iter().all(|message| !message.contains(quiet)),
            "{quiet} should stay quiet: {messages:?}"
        );
    }
    for diagnostic in &observed {
        assert_eq!(diagnostic["path"], "Cargo.toml");
        assert_eq!(diagnostic["category"], "reliability");
        assert!(
            diagnostic["span"]["line_start"].as_u64().is_some_and(|line| line > 1),
            "the diagnostic does not point at the offending key: {diagnostic}"
        );
    }
}

/// US-023: the scan honors the manifest's allow, so the site it hides is not
/// reported, and the allow is reported once, as a P3 choice that caps nothing.
#[test]
fn a_manifest_allow_hides_its_sites_and_is_charged_once_at_p3() {
    let report = report("lint-table");
    assert!(
        diagnostics_of(&report, "clippy::unwrap_used").is_empty(),
        "the site the manifest allows is reported: {report}"
    );
    let allows: Vec<&Value> = diagnostics_of(&report, LINT_TABLE)
        .into_iter()
        .filter(|diagnostic| {
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("clippy::unwrap_used"))
        })
        .collect();
    assert_eq!(allows.len(), 1, "{report}");
    let entry = rust_doctor::catalog()
        .into_iter()
        .find(|entry| entry.id == LINT_TABLE)
        .expect("the rule is catalogued");
    assert_eq!(entry.tier, rust_doctor::RuleTier::P3);
    assert!(
        report["audit"]["score"]["ceilings"]
            .as_array()
            .is_some_and(|ceilings| ceilings.iter().all(|ceiling| {
                ceiling["rule_ids"]
                    .as_array()
                    .is_none_or(|ids| !ids.iter().any(|id| id == LINT_TABLE))
            })),
        "the allow caps a dimension: {}",
        report["audit"]["score"]
    );
}

/// US-023: a level the request sets wins over the manifest, so the reader who
/// asks to see the sites sees them.
#[test]
fn a_requested_level_wins_over_a_manifest_allow() {
    let request = InspectRequest::new(fixture("lint-table"))
        .with_rule_override(RuleOverride::new("clippy::unwrap_used", RuleLevel::Warn));
    let report = report_of("lint-table", request);
    assert_eq!(
        diagnostics_of(&report, "clippy::unwrap_used").len(),
        1,
        "the requested level did not bring the site back: {report}"
    );
}

/// US-023: one member's allow is not another's, so the member that does not
/// allow the lint keeps its sites.
#[test]
fn a_member_allow_hides_only_that_members_sites() {
    let report = report("member-lint-table");
    assert_eq!(
        packages_of(&report, "clippy::print_stdout"),
        ["suppression-member-strict"],
        "{report}"
    );
    assert_eq!(packages_of(&report, LINT_TABLE), ["suppression-member-allowing"]);
}

/// US-023: an allow inherited through `lints.workspace = true` hides the
/// inheriting member's sites like its own would.
#[test]
fn an_inherited_workspace_allow_hides_the_members_sites() {
    let report = report("workspace-lint-table");
    assert!(
        diagnostics_of(&report, "clippy::unwrap_used").is_empty(),
        "the inherited allow did not hide the site: {report}"
    );
}

/// US-003: a member inheriting `[lints] workspace = true` points at the root
/// table, which is judged once rather than once per member.
#[test]
fn a_workspace_lint_table_is_judged_once_at_the_root() {
    let report = report("workspace-lint-table");
    let observed = diagnostics_of(&report, LINT_TABLE);
    assert_eq!(observed.len(), 1, "{report}");
    assert_eq!(observed[0]["path"], "Cargo.toml");
    assert!(
        observed[0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("clippy::unwrap_used")),
        "{}",
        observed[0]
    );
    assert_eq!(observed[0]["occurrences"], 1);
}
