#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! EP-001: the structural pass, end to end.
//!
//! Every proof here starts at `inspect` or at the built binary, because the
//! question this epic answers is not whether a detector fires but whether what
//! it finds reaches the score, the gate, the render and the baseline on the
//! path the native detectors already take.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rust_doctor::{
    CategoryOverride, InspectRequest, RuleLevel, RuleOverride, SCHEMA_VERSION, Status, inspect,
};
use serde_json::Value;

mod support;

const RULE: &str = "rust_doctor::structure::unreasoned_allow_attribute";

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> PathBuf {
    repository().join("tests/fixtures/structure").join(name)
}

/// Scratch workspace of a test, with its own Cargo target directory: without
/// one, Cargo's artifact garbage collection deletes rlibs a running test binary
/// still references.
fn scratch(label: &str) -> PathBuf {
    let root = repository()
        .join("target/structure-kernel")
        .join(format!("{label}-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    root
}

fn copy_fixture(name: &str, destination: &Path) {
    fn visit(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                if entry.file_name() != "target" {
                    visit(&entry.path(), &target);
                }
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    visit(&fixture(name), destination);
}

fn published(request: InspectRequest) -> Value {
    let report = inspect(request);
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    serde_json::to_value(&report).expect("a valid report should serialize")
}

/// Diagnostics the catalog knows about, in published order.
fn curated(report: &Value) -> Vec<&Value> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter(|diagnostic| !diagnostic["category"].is_null())
        .collect()
}

fn structural(report: &Value) -> Vec<&Value> {
    curated(report)
        .into_iter()
        .filter(|diagnostic| diagnostic["code"] == RULE)
        .collect()
}

/// US-001, US-002, US-004: one diagnostic per family, on the published path.
///
/// This is the trigger `tests/rule_evidence.json` names for the rule: the
/// fixture is scanned through the same `inspect` a user runs, and the category,
/// the tier and the help are read back off the shipped policy.
#[test]
fn the_structural_pass_publishes_one_diagnostic_per_family() {
    let report = published(InspectRequest::new(fixture("unreasoned-allow")));
    assert_eq!(report["schema_version"], SCHEMA_VERSION);

    let rule = report["policy"]["rules"]
        .as_array()
        .expect("a scan publishes its rules")
        .iter()
        .find(|published| published["id"] == RULE)
        .expect("the structural rule is in the shipped policy");
    assert_eq!(rule["category"], "maintainability");
    assert_eq!(rule["tier"], "P3");
    assert_eq!(rule["level"], "warn");

    let findings = structural(&report);
    assert_eq!(findings.len(), 3, "{findings:#?}");
    for finding in &findings {
        assert_eq!(finding["source"], "rust-doctor");
        assert_eq!(finding["category"], rule["category"]);
        assert_eq!(finding["severity"], "warning");
        assert_eq!(finding["base_severity"], "warning");
        assert!(
            finding["help"].as_str().is_some_and(|help| !help.is_empty()),
            "the rule publishes no help"
        );
    }

    // A family whose members share a file: the diagnostic points at the first
    // in sorted order and names the rest.
    let within = findings[0];
    assert_eq!(within["path"], "src/lib.rs");
    assert_eq!(within["occurrences"], 2);
    assert_eq!(
        within["related"],
        serde_json::json!([{
            "path": "src/lib.rs",
            "span": {
                "line_start": 24,
                "column_start": 1,
                "line_end": 24,
                "column_end": 20,
            },
        }])
    );

    // A family whose members straddle two files: `related` crosses the file
    // boundary, because the rule reports what the codebase silences.
    let across = findings[1];
    assert_eq!(across["path"], "src/lib.rs");
    assert_eq!(across["occurrences"], 2);
    assert_eq!(across["related"][0]["path"], "src/reached.rs");
    assert_eq!(
        across["related"]
            .as_array()
            .expect("related should be an array")
            .len(),
        1
    );

    // A family of one publishes no `related` key at all, rather than an empty
    // array, so a per-site diagnostic serializes as it did under schema 10.
    let alone = findings[2];
    assert_eq!(alone["occurrences"], 1);
    assert!(
        alone.get("related").is_none(),
        "a single-site finding published an empty related array"
    );

    // The exemption inside `#[cfg(test)]` is marked, so it stays published and
    // counted while it stops weighing on the score.
    assert_eq!(alone["context"], "tests");
    assert!(within.get("context").is_none());
    assert!(across.get("context").is_none());

    // The findings are scored: the dimension the category maps to leaves 100.
    // The marked family is excluded from that arithmetic, the two others are
    // not, which is what makes the mark observable rather than declarative.
    assert_eq!(report["audit"]["score"]["dimensions"]["maintainability"], 99);
    assert_eq!(report["audit"]["categories"][0]["name"], "Maintainability");
    assert_eq!(report["audit"]["categories"][0]["distinct"]["total"], 3);

    // Nothing published names a path outside the workspace.
    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains(repository().to_str().unwrap()));
    for finding in &findings {
        for location in std::iter::once(&finding["path"]).chain(
            finding["related"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|location| &location["path"]),
        ) {
            let path = location.as_str().expect("a location names its file");
            assert!(!Path::new(path).is_absolute(), "{path}");
            assert!(!path.contains(".."), "{path}");
        }
    }
}

/// US-014: an exemption that states its reason is not a finding.
#[test]
fn a_stated_reason_leaves_the_census_with_nothing_to_report() {
    let report = published(InspectRequest::new(fixture("reasoned-allow")));
    assert!(structural(&report).is_empty(), "{report:#?}");
    assert_eq!(report["audit"]["score"]["maintainability"], Value::Null);
    assert_eq!(report["audit"]["score"]["value"], 100);
    assert_eq!(report["audit"]["score"]["authoritative"], true);
}

/// US-001: an empty structural result costs the report nothing.
///
/// The same workspace scanned with the rule active and with it switched off
/// produces the same bytes, which is what "the pass returns an empty vector and
/// the report is unchanged" means once it is observable.
#[test]
fn an_empty_structural_result_changes_no_byte_of_the_report() {
    let active = published(InspectRequest::new(fixture("reasoned-allow")));
    let inactive = published(
        InspectRequest::new(fixture("reasoned-allow"))
            .with_rule_override(RuleOverride::new(RULE, RuleLevel::Off)),
    );

    let mut inactive_without_policy = inactive.clone();
    let mut active_without_policy = active.clone();
    // The published policy names the level of every rule, so it is the one
    // place the two runs legitimately differ.
    inactive_without_policy["policy"] = Value::Null;
    active_without_policy["policy"] = Value::Null;
    assert_eq!(
        serde_json::to_string(&active_without_policy).unwrap(),
        serde_json::to_string(&inactive_without_policy).unwrap()
    );
}

/// US-004: `--rule` and `--category` reach a structural rule like any other.
#[test]
fn the_policy_switches_a_structural_rule_off_and_raises_it_to_error() {
    let off = published(
        InspectRequest::new(fixture("unreasoned-allow"))
            .with_rule_override(RuleOverride::new(RULE, RuleLevel::Off)),
    );
    assert!(structural(&off).is_empty(), "{off:#?}");
    assert_eq!(off["audit"]["score"]["value"], 100);
    assert_eq!(off["audit"]["score"]["dimensions"]["maintainability"], 100);
    assert_eq!(off["gate"]["status"], "passed");

    let raised = inspect(
        InspectRequest::new(fixture("unreasoned-allow"))
            .with_category_override(CategoryOverride::new("maintainability", RuleLevel::Error)),
    );
    assert_eq!(raised.status, Status::Complete, "{:?}", raised.errors);
    let raised = serde_json::to_value(&raised).unwrap();
    let findings = structural(&raised);
    assert_eq!(findings.len(), 3);
    for finding in &findings {
        assert_eq!(finding["severity"], "error");
        assert_eq!(finding["base_severity"], "warning");
    }
    assert_eq!(raised["gate"]["status"], "failed");
    // The marked family stays published and stops blocking, exactly as a
    // `println!` in a build script does.
    assert_eq!(raised["gate"]["blocking_diagnostics"], 2);
}

/// US-002: `--verbose` names every other site of a family.
#[test]
fn the_verbose_render_names_the_related_locations() {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(fixture("unreasoned-allow"))
        .args(["--yes", "--verbose"])
        .env("CARGO_TARGET_DIR", scratch("render"))
        .output()
        .unwrap();
    let rendered = String::from_utf8(output.stdout).unwrap();

    assert!(rendered.contains("Rule ID: {RULE}".replace("{RULE}", RULE).as_str()));
    assert!(
        rendered.contains("Also at: src/lib.rs:24:1"),
        "the render named no related location\n{rendered}"
    );
    assert!(
        rendered.contains("Also at: src/reached.rs:6:1"),
        "the render dropped the cross-file location\n{rendered}"
    );
    assert!(!rendered.contains(repository().to_str().unwrap()));
}

/// US-003: the identity of a structural finding excludes its position.
#[test]
fn a_structural_fingerprint_survives_an_insertion_and_moves_with_its_content() {
    let root = scratch("fingerprint");
    let project = root.join("project");
    copy_fixture("unreasoned-allow", &project);
    let source = project.join("src/lib.rs");

    let identities = |report: &Value| -> Vec<String> {
        structural(report)
            .into_iter()
            .map(|finding| finding["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let before = published(InspectRequest::new(&project));
    let original = fs::read_to_string(&source).unwrap();

    fs::write(&source, format!("{}{original}", "// shifted\n".repeat(50))).unwrap();
    let shifted = published(InspectRequest::new(&project));
    assert_ne!(
        structural(&before)[0]["span"],
        structural(&shifted)[0]["span"],
        "the insertion did not move the finding"
    );
    assert_eq!(
        identities(&before),
        identities(&shifted),
        "a structural fingerprint moved with an unrelated insertion"
    );

    // Rewriting the exemption itself is a change of normalized content, so the
    // family it formed is a different family and carries a different identity.
    let rewritten_source = original.replace("#[allow(dead_code)]", "#[allow(unused_mut)]");
    assert_ne!(rewritten_source, original);
    fs::write(&source, rewritten_source).unwrap();
    let rewritten = identities(&published(InspectRequest::new(&project)));
    let unchanged: Vec<&String> = rewritten
        .iter()
        .filter(|id| identities(&before).contains(id))
        .collect();
    assert_eq!(
        unchanged.len(),
        identities(&before).len() - 1,
        "exactly the rewritten family should have changed identity"
    );
}

/// US-003: the baseline classifies a shifted structural finding as pre-existing.
#[test]
fn a_shifted_structural_finding_is_pre_existing_against_its_baseline() {
    let root = scratch("baseline");
    let project = root.join("project");
    copy_fixture("unreasoned-allow", &project);
    for arguments in [
        &["init", "--initial-branch=main", "--quiet"][..],
        &["config", "user.name", "Rust Doctor"][..],
        &["config", "user.email", "rust-doctor@example.invalid"][..],
        &["add", "."][..],
        &["commit", "--quiet", "--message=baseline"][..],
    ] {
        support::git_output(&project, arguments);
    }

    let source = project.join("src/lib.rs");
    let original = fs::read_to_string(&source).unwrap();
    fs::write(&source, format!("{}{original}", "// shifted\n".repeat(50))).unwrap();

    let report = inspect(InspectRequest::new(&project).with_baseline_scope("HEAD"));
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    let delta = report
        .delta
        .clone()
        .expect("a baseline scan publishes its delta");
    let published = serde_json::to_value(&report).unwrap();
    let structural_ids: Vec<&str> = structural(&published)
        .into_iter()
        .map(|finding| finding["id"].as_str().unwrap())
        .collect();
    assert!(!structural_ids.is_empty());

    for id in structural_ids {
        assert!(
            delta.pre_existing.iter().any(|matched| matched.current_id == id),
            "a structural finding shifted by an insertion was reported as introduced"
        );
        assert!(!delta.introduced.iter().any(|introduced| introduced == id));
    }
}

/// US-001: a unit the parser cannot read is skipped, named under the `structure`
/// stage, and never stops the pass on the rest of the workspace.
///
/// The crate is built here rather than committed as a fixture: a file that
/// fails to parse also fails to compile, so a committed one would turn every
/// scan of the surrounding directory incomplete.
#[test]
fn an_unparseable_unit_is_reported_under_the_structure_stage() {
    let root = scratch("parse-error");
    let project = root.join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        concat!(
            "[package]\n",
            "name = \"structure-parse-error\"\n",
            "version = \"0.1.0\"\n",
            "edition = \"2024\"\n",
            "publish = false\n\n",
            "[lib]\n",
            "path = \"src/lib.rs\"\n",
        ),
    )
    .unwrap();
    fs::write(
        project.join("src/lib.rs"),
        "pub mod broken;\n\n#[allow(dead_code)]\npub struct Readable {\n    unread: u8,\n}\n",
    )
    .unwrap();
    fs::write(project.join("src/broken.rs"), "pub fn broken( {\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(&project)
        .args(["--yes", "--json"])
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();

    let skipped: Vec<&Value> = report["errors"]
        .as_array()
        .expect("errors should be an array")
        .iter()
        .filter(|error| error["stage"] == "structure")
        .collect();
    assert_eq!(skipped.len(), 1, "{:#?}", report["errors"]);
    assert_eq!(skipped[0]["code"], "parse-error");
    let message = skipped[0]["message"]
        .as_str()
        .expect("an error says what it skipped");
    assert!(message.contains("src/broken.rs"), "{message}");
    assert!(!message.contains(repository().to_str().unwrap()), "{message}");

    // The unreadable unit is the only one dropped: the exemption in the file
    // next to it still reaches the report.
    assert_eq!(
        structural(&report)
            .iter()
            .map(|finding| finding["path"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["src/lib.rs"],
        "{:#?}",
        report["diagnostics"]
    );
}
