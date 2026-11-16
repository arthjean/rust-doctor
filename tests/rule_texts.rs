#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

mod support;

const LINT_TABLE: &str = "rust_doctor::cargo::permissive_lint_table";
const UNUSED: &str = "rust_doctor::cargo::unused_dependency";
const FEATURE: &str = "rust_doctor::structure::unreferenced_feature";
const ORPHAN: &str = "rust_doctor::structure::orphan_module_file";

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn workspace(fixture: &str) -> PathBuf {
    let root = support::fresh_workspace("rule-texts", &NEXT_WORKSPACE);
    support::copy_tree(&support::fixture_dir(fixture), &root);
    root
}

fn scan(root: &Path, arguments: &[&str]) -> Value {
    let output = support::rust_doctor()
        .arg(root)
        .args(["--yes", "--json"])
        .args(support::NO_ADVISORY_DB)
        .args(arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        !output.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn findings<'a>(report: &'a Value, rule: &str) -> Vec<&'a Value> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == rule)
        .collect()
}

fn message_naming<'a>(report: &'a Value, rule: &str, needle: &str) -> &'a str {
    let messages: Vec<&str> = findings(report, rule)
        .into_iter()
        .filter_map(|diagnostic| diagnostic["message"].as_str())
        .collect();
    let named = messages
        .iter()
        .copied()
        .find(|message| message.contains(needle));
    assert!(named.is_some(), "no {rule} finding names {needle}: {messages:?}");
    named.unwrap()
}

fn published_rules() -> Vec<Value> {
    let output = support::rust_doctor()
        .args(["rules", "list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn published(rules: &[Value], id: &str) -> (String, String) {
    let entry = rules.iter().find(|entry| entry["id"] == id);
    assert!(entry.is_some(), "{id} is not published");
    let entry = entry.unwrap();
    (
        entry["title"].as_str().unwrap().to_owned(),
        entry["help"].as_str().unwrap().to_owned(),
    )
}

#[test]
fn each_reworded_help_is_what_the_rule_list_publishes() {
    let rules = published_rules();

    let (title, help) = published(&rules, "clippy::undocumented_unsafe_blocks");
    assert_eq!(title, "Unsafe block or impl without a SAFETY comment");
    assert!(help.contains("`// SAFETY:` comment"), "{help}");
    assert!(
        help.contains("#[expect(clippy::undocumented_unsafe_blocks, reason = \"...\")]"),
        "{help}"
    );

    let (_, help) = published(&rules, ORPHAN);
    assert_eq!(
        help,
        "Declare the file with a mod declaration. Remove it only if nothing should compile it."
    );

    let (_, help) = published(&rules, "rust_doctor::source::blocking_call_in_async");
    assert!(
        help.contains("your runtime's blocking pool: `spawn_blocking`, `unblock`"),
        "{help}"
    );
    for runtime in ["tokio", "smol", "async-std", "async_std", "reqwest"] {
        assert!(!help.contains(runtime), "the help names {runtime}: {help}");
    }
}

#[test]
fn an_orphan_finding_carries_the_help_that_keeps_compiled_files() {
    let report = scan(&workspace("structure/orphan-module"), &[]);
    let orphan = findings(&report, ORPHAN);
    assert!(!orphan.is_empty(), "{report}");
    for finding in orphan {
        assert_eq!(
            finding["help"],
            "Declare the file with a mod declaration. Remove it only if nothing should compile it."
        );
    }
}

#[test]
fn an_unread_feature_says_no_cfg_and_no_build_script_reads_it() {
    let report = scan(&workspace("structure/feature-inventory"), &[]);
    let declared: Vec<&str> = findings(&report, FEATURE)
        .into_iter()
        .filter_map(|diagnostic| diagnostic["message"].as_str())
        .filter(|message| message.contains(" declares feature "))
        .collect();
    assert!(!declared.is_empty(), "{report}");
    for message in declared {
        assert!(
            message.contains("which no cfg and no build script reads"),
            "{message}"
        );
    }
}

#[test]
fn a_lint_table_entry_states_how_many_findings_it_hid() {
    let report = scan(&workspace("suppression/lint-table"), &[]);
    assert_eq!(
        message_naming(&report, LINT_TABLE, "clippy::unwrap_used"),
        "Manifest lint table sets catalogued rule \"clippy::unwrap_used\" to \"allow\", which hides 1 finding in this package."
    );
    assert_eq!(
        message_naming(&report, LINT_TABLE, "clippy::expect_used"),
        "Manifest lint table sets catalogued rule \"clippy::expect_used\" to \"allow\", which hides no finding in this package."
    );

    let member = scan(&workspace("suppression/member-lint-table"), &[]);
    assert_eq!(
        message_naming(&member, LINT_TABLE, "clippy::print_stdout"),
        "Manifest lint table sets catalogued rule \"clippy::print_stdout\" to \"allow\", which hides 1 finding in this package."
    );

    let inherited = scan(&workspace("suppression/workspace-lint-table"), &[]);
    assert_eq!(
        message_naming(&inherited, LINT_TABLE, "clippy::unwrap_used"),
        "Manifest lint table sets catalogued rule \"clippy::unwrap_used\" to \"allow\", which hides 1 finding in the packages that inherit it."
    );
}

#[test]
fn a_lint_table_entry_claims_no_count_for_a_package_the_scan_did_not_lint() {
    let report = scan(
        &workspace("suppression/workspace-lint-table"),
        &["--package", "suppression-workspace-beta"],
    );
    assert_eq!(
        message_naming(&report, LINT_TABLE, "clippy::unwrap_used"),
        "Manifest lint table sets catalogued rule \"clippy::unwrap_used\" to \"allow\"."
    );
}

#[test]
fn an_inherited_dependency_points_at_the_members_own_line_and_keeps_its_id() {
    let first = workspace("rule-texts/inherited-dependency");
    let report = scan(&first, &[]);
    let unused = findings(&report, UNUSED);
    assert_eq!(unused.len(), 1, "{report}");
    assert_eq!(unused[0]["path"], "app/Cargo.toml");
    assert_eq!(unused[0]["span"]["line_start"], 11, "{}", unused[0]);

    let second = workspace("rule-texts/inherited-dependency");
    let manifest = second.join("app/Cargo.toml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(&manifest, text.replace("[dependencies]", "\n[dependencies]")).unwrap();
    let moved = scan(&second, &[]);
    let moved = findings(&moved, UNUSED);
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(moved[0]["span"]["line_start"], 12, "{}", moved[0]);
    assert_eq!(moved[0]["id"], unused[0]["id"]);
}

#[test]
fn a_lint_table_entry_claims_no_count_when_a_member_it_covers_did_not_compile() {
    let root = workspace("suppression/workspace-lint-table");
    let source = root.join("beta/src/lib.rs");
    let text = fs::read_to_string(&source).unwrap();
    fs::write(&source, format!("{text}\npub fn broken() -> u32 {{\n    \"x\"\n}}\n")).unwrap();
    let report = scan(&root, &[]);
    assert_eq!(
        message_naming(&report, LINT_TABLE, "clippy::unwrap_used"),
        "Manifest lint table sets catalogued rule \"clippy::unwrap_used\" to \"allow\".",
        "{report}"
    );
}
