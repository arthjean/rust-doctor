#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

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

    assert!(
        observed
            .iter()
            .all(|diagnostic| diagnostic["path"] != "tests/probe.rs"),
        "the allow inside the test target was reported: {subjects:?}"
    );

    assert!(
        observed
            .iter()
            .all(|diagnostic| diagnostic["span"]["line_start"] != 17),
        "an outer allow on a single item was reported as crate-level"
    );
}

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

#[test]
fn an_inherited_workspace_allow_hides_the_members_sites() {
    let report = report("workspace-lint-table");
    assert!(
        diagnostics_of(&report, "clippy::unwrap_used").is_empty(),
        "the inherited allow did not hide the site: {report}"
    );
}

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
