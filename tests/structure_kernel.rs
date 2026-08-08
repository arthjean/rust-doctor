#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! EP-001 to EP-004: the structural pass, end to end.
//!
//! Every proof here starts at `inspect` or at the built binary, because the
//! question these epics answer is not whether a detector fires but whether what
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
const DUPLICATE: &str = "rust_doctor::structure::duplicate_function_body";
const NEAR_DUPLICATE: &str = "rust_doctor::structure::near_duplicate_function_body";
const ORPHAN: &str = "rust_doctor::structure::orphan_module_file";
const FEATURE: &str = "rust_doctor::structure::unreferenced_feature";

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
    rule_findings(report, RULE)
}

fn rule_findings<'a>(report: &'a Value, rule: &str) -> Vec<&'a Value> {
    curated(report)
        .into_iter()
        .filter(|diagnostic| diagnostic["code"] == rule)
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
    assert_eq!(findings.len(), 4, "{findings:#?}");
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

    // US-014: the crate-level form is counted like the item-level one, and the
    // integration test crate that carries it is named by a Cargo target kind,
    // so the finding is marked without any path being read as a context.
    let crate_level = findings[3];
    assert_eq!(crate_level["path"], "tests/crate_level.rs");
    assert_eq!(crate_level["context"], "tests");
    assert_eq!(crate_level["occurrences"], 1);
    assert_eq!(
        crate_level["message"],
        "#![allow(dead_code)] switches a lint off without a stated reason."
    );

    // The findings are scored: the dimension the category maps to leaves 100.
    // The marked families are excluded from that arithmetic, the two others are
    // not, which is what makes the mark observable rather than declarative.
    assert_eq!(report["audit"]["score"]["dimensions"]["maintainability"], 99);
    assert_eq!(report["audit"]["categories"][0]["name"], "Maintainability");
    assert_eq!(report["audit"]["categories"][0]["distinct"]["total"], 4);

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
    assert_eq!(findings.len(), 4);
    for finding in &findings {
        assert_eq!(finding["severity"], "error");
        assert_eq!(finding["base_severity"], "warning");
    }
    assert_eq!(raised["gate"]["status"], "failed");
    // The marked families stay published and stop blocking, exactly as a
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

    // FR-10, FR-11: what the pass could not cover takes the authoritative flag
    // off the score rather than being reported as a complete answer. The time
    // budget stops the pass through this same error path.
    assert_eq!(report["status"], "incomplete");
    assert_eq!(report["audit"]["score"]["authoritative"], false);

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

// ---------------------------------------------------------------------------
// EP-002: duplicate function detection
// ---------------------------------------------------------------------------

/// US-006, US-007, US-008: the two duplication rules, end to end.
///
/// This is the trigger `tests/rule_evidence.json` names for both of them. The
/// fixture is scanned through the same `inspect` a user runs, so the category,
/// the tier and the help are read back off the shipped policy, and the forms
/// the rules must leave alone are in the same crate as the forms they report.
#[test]
fn the_duplication_pass_publishes_one_family_per_shape() {
    let report = published(InspectRequest::new(fixture("duplicate-function")));

    for rule in [DUPLICATE, NEAR_DUPLICATE] {
        let published = report["policy"]["rules"]
            .as_array()
            .expect("a scan publishes its rules")
            .iter()
            .find(|entry| entry["id"] == rule)
            .expect("the rule is in the shipped policy");
        assert_eq!(published["category"], "maintainability");
        assert_eq!(published["tier"], "P3");
        assert_eq!(published["level"], "warn");
    }

    // US-006: the two exact copies are one finding of two occurrences, pointing
    // at the first in sorted order and naming the second. The pair of one-line
    // predicates below the node floor and the function of comparable size doing
    // something else are both left alone, which is what proves the rule was
    // checked for over-reach.
    let exact = rule_findings(&report, DUPLICATE);
    let shipped = exact
        .iter()
        .find(|finding| finding["path"] == "src/lib.rs" && finding["context"].is_null())
        .expect("the shipped clone family is missing");
    assert_eq!(shipped["occurrences"], 2);
    assert_eq!(shipped["source"], "rust-doctor");
    assert_eq!(shipped["severity"], "warning");
    assert_eq!(shipped["span"]["line_start"], 19);
    assert_eq!(
        shipped["related"],
        serde_json::json!([{
            "path": "src/lib.rs",
            "span": {
                "line_start": 31,
                "column_start": 1,
                "line_end": 41,
                "column_end": 2,
            },
        }])
    );
    assert!(
        shipped.get("similarity_basis_points").is_none(),
        "an exact family published a similarity it did not measure"
    );

    // US-007: the copy edited after the fact is a near family, scored, and the
    // pair already published as an exact family is not published again here.
    let near = rule_findings(&report, NEAR_DUPLICATE);
    assert_eq!(near.len(), 1, "{near:#?}");
    assert_eq!(near[0]["occurrences"], 2);
    let similarity = near[0]["similarity_basis_points"]
        .as_u64()
        .expect("a near family publishes its score in --json");
    assert!((8_000..10_000).contains(&similarity), "{similarity}");
    assert_eq!(near[0]["path"], "src/edited.rs");
    assert_eq!(near[0]["related"][0]["path"], "src/lib.rs");

    // US-008: the family inside `#[cfg(test)]` and the one inside the build
    // script are marked, stay published, and stay counted. Only the two shipped
    // families weigh, which is what keeps the score at 100 with four findings.
    let marks: Vec<&str> = exact
        .iter()
        .map(|finding| finding["context"].as_str().unwrap_or("shipped"))
        .collect();
    assert_eq!(marks, ["build-script", "shipped", "tests"], "{exact:#?}");
    assert_eq!(report["audit"]["categories"][0]["distinct"]["total"], 4);
    assert_eq!(report["audit"]["categories"][0]["occurrences"]["total"], 8);
    assert_eq!(report["audit"]["score"]["value"], 100);
    assert_eq!(report["audit"]["score"]["dimensions"]["maintainability"], 97);

    // Nothing published names a path outside the workspace.
    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains(repository().to_str().unwrap()));
}

/// US-006, US-007: switching a duplication rule off removes its findings and
/// leaves the other one untouched.
#[test]
fn each_duplication_rule_is_switched_off_on_its_own() {
    let without_exact = published(
        InspectRequest::new(fixture("duplicate-function"))
            .with_rule_override(RuleOverride::new(DUPLICATE, RuleLevel::Off)),
    );
    assert!(rule_findings(&without_exact, DUPLICATE).is_empty());
    assert_eq!(rule_findings(&without_exact, NEAR_DUPLICATE).len(), 1);

    let without_near = published(
        InspectRequest::new(fixture("duplicate-function"))
            .with_rule_override(RuleOverride::new(NEAR_DUPLICATE, RuleLevel::Off)),
    );
    assert_eq!(rule_findings(&without_near, DUPLICATE).len(), 3);
    assert!(rule_findings(&without_near, NEAR_DUPLICATE).is_empty());

    // With both off the score is computed without them, and the dimension they
    // were the only occupants of returns to its untouched value.
    let neither = published(
        InspectRequest::new(fixture("duplicate-function"))
            .with_rule_override(RuleOverride::new(DUPLICATE, RuleLevel::Off))
            .with_rule_override(RuleOverride::new(NEAR_DUPLICATE, RuleLevel::Off)),
    );
    assert!(rule_findings(&neither, DUPLICATE).is_empty());
    assert!(rule_findings(&neither, NEAR_DUPLICATE).is_empty());
    assert_eq!(neither["audit"]["score"]["dimensions"]["maintainability"], 100);
}

/// US-003, US-006: a clone family keeps its identity when unrelated code moves
/// above it, and loses it when the shape itself is edited.
#[test]
fn a_clone_family_keeps_its_identity_across_an_insertion() {
    let root = scratch("duplication-fingerprint");
    let project = root.join("project");
    copy_fixture("duplicate-function", &project);
    let source = project.join("src/lib.rs");

    let identities = |report: &Value| -> Vec<String> {
        rule_findings(report, DUPLICATE)
            .into_iter()
            .map(|finding| finding["id"].as_str().unwrap_or_default().to_owned())
            .collect()
    };
    let before = published(InspectRequest::new(&project));
    let original = fs::read_to_string(&source).unwrap();

    fs::write(&source, format!("{}{original}", "// shifted\n".repeat(50))).unwrap();
    let shifted = published(InspectRequest::new(&project));
    assert_ne!(
        rule_findings(&before, DUPLICATE)[1]["span"],
        rule_findings(&shifted, DUPLICATE)[1]["span"],
        "the insertion did not move the family"
    );
    assert_eq!(
        identities(&before),
        identities(&shifted),
        "a clone family was renamed by an unrelated insertion"
    );

    // Editing one member out of the shape dissolves the family it belonged to.
    fs::write(&source, original.replace("sum -= bound;", "sum -= bound + 1;")).unwrap();
    let edited = published(InspectRequest::new(&project));
    assert_eq!(
        rule_findings(&edited, DUPLICATE)
            .iter()
            .filter(|finding| finding["context"].is_null())
            .count(),
        0,
        "the family survived an edit to one of its members"
    );
}

// ---------------------------------------------------------------------------
// EP-003: complexity and size hotspots
// ---------------------------------------------------------------------------

const COMPLEX: &str = "rust_doctor::structure::complex_function";
const OVERSIZED: &str = "rust_doctor::structure::oversized_unit";

/// US-010: the complexity rule, end to end.
///
/// This is the trigger `tests/rule_evidence.json` names for the rule. The
/// fixture is scanned through the same `inspect` a user runs, and the quiet
/// function next to the dense one is the recorded silence that proves the rule
/// was checked for over-reach.
#[test]
fn the_complexity_pass_names_the_functions_too_tangled_to_review() {
    let report = published(InspectRequest::new(fixture("complex-function")));
    assert_eq!(report["schema_version"], SCHEMA_VERSION);

    let rule = report["policy"]["rules"]
        .as_array()
        .expect("a scan publishes its rules")
        .iter()
        .find(|published| published["id"] == COMPLEX)
        .expect("the complexity rule is in the shipped policy");
    assert_eq!(rule["category"], "maintainability");
    assert_eq!(rule["tier"], "P3");
    assert_eq!(rule["level"], "warn");

    let findings = rule_findings(&report, COMPLEX);
    assert_eq!(findings.len(), 1, "{findings:#?}");
    let hotspot = findings[0];
    assert_eq!(hotspot["source"], "rust-doctor");
    assert_eq!(hotspot["severity"], "warning");
    assert_eq!(hotspot["occurrences"], 1);
    assert!(hotspot.get("related").is_none());

    // US-010: both figures appear in --json, on the diagnostic, and the
    // message states them for the terminal reader.
    assert_eq!(
        hotspot["complexity"],
        serde_json::json!({ "cyclomatic": 11, "cognitive": 30 })
    );
    let message = hotspot["message"].as_str().expect("a hotspot says what it measured");
    assert!(
        message.contains("schedule")
            && message.contains("cyclomatic complexity 11")
            && message.contains("cognitive complexity 30"),
        "{message}"
    );

    // A diagnostic that is not a hotspot carries no complexity key at all.
    for diagnostic in curated(&report) {
        if diagnostic["code"] != COMPLEX {
            assert!(diagnostic.get("complexity").is_none(), "{diagnostic:#?}");
        }
    }
}

/// US-010: the thresholds are read from `rust-doctor.toml`, in both
/// directions, and `--rule` switches the rule off entirely.
#[test]
fn the_complexity_thresholds_are_read_from_the_workspace_configuration() {
    let root = scratch("complexity-thresholds");
    let project = root.join("project");
    copy_fixture("complex-function", &project);

    // Raised above what the fixture measures: nothing is reported.
    fs::write(
        project.join("rust-doctor.toml"),
        "[structure]\ncyclomatic-threshold = 12\ncognitive-threshold = 31\n",
    )
    .unwrap();
    let raised = published(InspectRequest::new(&project));
    assert!(rule_findings(&raised, COMPLEX).is_empty(), "{raised:#?}");

    // Lowered under the calm function: both functions are reported.
    fs::write(
        project.join("rust-doctor.toml"),
        "[structure]\ncognitive-threshold = 2\n",
    )
    .unwrap();
    let lowered = published(InspectRequest::new(&project));
    assert_eq!(rule_findings(&lowered, COMPLEX).len(), 2, "{lowered:#?}");

    // The existing --rule mechanism reaches the rule like any other.
    let off = published(
        InspectRequest::new(fixture("complex-function"))
            .with_rule_override(RuleOverride::new(COMPLEX, RuleLevel::Off)),
    );
    assert!(rule_findings(&off, COMPLEX).is_empty(), "{off:#?}");
}

/// US-011: the oversized units, end to end, one diagnostic each.
///
/// This is the trigger `tests/rule_evidence.json` names for the rule. The
/// workspace is generated rather than committed: a thousand committed filler
/// lines would say nothing to a reader of this repository.
#[test]
fn the_oversized_units_reach_the_report_and_the_generated_file_does_not() {
    let root = scratch("oversized");
    let project = root.join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        concat!(
            "[package]\n",
            "name = \"structure-oversized\"\n",
            "version = \"0.1.0\"\n",
            "edition = \"2024\"\n",
            "publish = false\n\n",
            "[lib]\n",
            "path = \"src/lib.rs\"\n",
        ),
    )
    .unwrap();

    // One long function, one long impl block, one long module, in a file long
    // enough to be a unit of its own.
    let mut source = String::from("pub mod produced;\n\npub struct Wide;\n\n");
    source.push_str("pub fn stretched() -> u64 {\n    let mut total: u64 = 0;\n");
    for _ in 0..150 {
        source.push_str("    total += 1;\n");
    }
    source.push_str("    total\n}\n\nimpl Wide {\n");
    for index in 0..500 {
        source.push_str(&format!("    pub fn slot_{index}(&self) {{}}\n"));
    }
    source.push_str("}\n\npub mod carried {\n");
    for index in 0..500 {
        source.push_str(&format!("    pub const SLOT_{index}: u16 = {index};\n"));
    }
    source.push_str("}\n");
    fs::write(project.join("src/lib.rs"), &source).unwrap();

    // The same size under a recognized generator header: excluded whole.
    let mut produced = String::from("// @generated by a code generator. DO NOT EDIT.\n");
    produced.push_str(&source.replace("pub mod produced;\n\n", "").replace("Wide", "Tall"));
    fs::write(project.join("src/produced.rs"), produced).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(&project)
        .args(["--yes", "--json"])
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "complete", "{:#?}", report["errors"]);

    let findings = rule_findings(&report, OVERSIZED);
    let named: Vec<(&str, &str)> = findings
        .iter()
        .map(|finding| {
            (
                finding["path"].as_str().unwrap_or_default(),
                finding["message"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    // One diagnostic per unit: the file, the function, the impl block and the
    // module, and nothing from the generated file next to them.
    assert_eq!(findings.len(), 4, "{named:#?}");
    assert!(named.iter().all(|(path, _)| *path == "src/lib.rs"), "{named:#?}");
    assert!(
        named.iter().any(|(_, message)| message.starts_with("src/lib.rs is ")
            && message.ends_with(" lines long.")),
        "{named:#?}"
    );
    assert!(
        named
            .iter()
            .any(|(_, message)| message.starts_with("Function stretched spans ")),
        "{named:#?}"
    );
    assert!(
        named.iter().any(|(_, message)| message.starts_with("Impl block Wide spans ")),
        "{named:#?}"
    );
    assert!(
        named.iter().any(|(_, message)| message.starts_with("Module carried spans ")),
        "{named:#?}"
    );
}

// ---------------------------------------------------------------------------
// EP-004: Rust-native slop signals
// ---------------------------------------------------------------------------

/// US-013: the orphan module rule, end to end.
///
/// This is the trigger `tests/rule_evidence.json` names for the rule. The
/// fixture is scanned through the same `inspect` a user runs, and every other
/// way of reaching a file sits in the same crate as the unreached one, which is
/// what proves the rule was checked for over-reach.
#[test]
fn the_orphan_rule_names_the_file_cargo_never_compiles() {
    let report = published(InspectRequest::new(fixture("orphan-module")));

    let rule = report["policy"]["rules"]
        .as_array()
        .expect("a scan publishes its rules")
        .iter()
        .find(|published| published["id"] == ORPHAN)
        .expect("the orphan rule is in the shipped policy");
    assert_eq!(rule["category"], "maintainability");
    assert_eq!(rule["tier"], "P3");
    assert_eq!(rule["level"], "warn");

    // One finding, and only one: the ordinary `mod`, the `#[path]` attribute,
    // the `mod` gated on another platform, the `include!`, the build script,
    // the file that build script names, and the integration test root are all
    // ways of reaching a file, and the generated file next to the orphan is
    // read by no structural detector.
    let findings = rule_findings(&report, ORPHAN);
    assert_eq!(
        findings
            .iter()
            .map(|finding| finding["path"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["src/orphan.rs"],
        "{findings:#?}"
    );
    let orphan = findings[0];
    assert_eq!(orphan["source"], "rust-doctor");
    assert_eq!(orphan["severity"], "warning");
    assert_eq!(orphan["occurrences"], 1);
    assert!(orphan.get("related").is_none());
    assert!(orphan.get("context").is_none());
    assert_eq!(orphan["span"]["line_start"], 1);
    let message = orphan["message"].as_str().expect("an orphan says what it is");
    assert!(
        message.contains("src/orphan.rs") && message.contains("structure-orphan-module"),
        "{message}"
    );

    // The rule is reached by the policy like any other, and the score is
    // computed without it once it is off.
    let off = published(
        InspectRequest::new(fixture("orphan-module"))
            .with_rule_override(RuleOverride::new(ORPHAN, RuleLevel::Off)),
    );
    assert!(rule_findings(&off, ORPHAN).is_empty(), "{off:#?}");
    assert_eq!(off["audit"]["score"]["dimensions"]["maintainability"], 100);

    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains(repository().to_str().unwrap()));
}

/// US-013: a file the module tree stops reaching becomes an orphan, and the
/// finding keeps its identity while the file keeps its name.
#[test]
fn a_file_becomes_an_orphan_when_its_declaration_goes() {
    let root = scratch("orphan-declaration");
    let project = root.join("project");
    copy_fixture("orphan-module", &project);
    let source = project.join("src/lib.rs");

    let orphans = |report: &Value| -> Vec<(String, String)> {
        rule_findings(report, ORPHAN)
            .into_iter()
            .map(|finding| {
                (
                    finding["path"].as_str().unwrap_or_default().to_owned(),
                    finding["id"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    };
    let before = published(InspectRequest::new(&project));
    let original = fs::read_to_string(&source).unwrap();

    // Dropping the declaration of a reached module makes its file an orphan.
    fs::write(&source, original.replace("pub mod reached;", "")).unwrap();
    let after = published(InspectRequest::new(&project));
    let paths: Vec<String> = orphans(&after).into_iter().map(|(path, _)| path).collect();
    assert_eq!(paths, ["src/orphan.rs", "src/reached.rs"], "{paths:?}");

    // US-003: the identity of the file that was already an orphan is unchanged
    // by an edit elsewhere, because a structural fingerprint carries no
    // position.
    let unchanged = orphans(&before)[0].clone();
    assert!(
        orphans(&after).contains(&unchanged),
        "the untouched orphan was renamed by an edit above it"
    );
}

/// US-013: reachability is answered per package, and being compiled is not.
///
/// A member reaching into its neighbour with `#[path]` compiles a file the
/// module tree of the package holding it never names. The walk stays per
/// package, so the finding names the package whose directory holds the file,
/// but what exempts a candidate is the reach of the whole workspace: a file
/// Cargo compiles must never be published as compiled by nothing.
#[test]
fn a_file_another_package_compiles_is_not_an_orphan() {
    let report = published(InspectRequest::new(fixture("orphan-across-packages")));

    let findings = rule_findings(&report, ORPHAN);
    assert_eq!(
        findings
            .iter()
            .map(|finding| finding["path"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["engine/src/stray.rs"],
        "{findings:#?}"
    );
    assert!(
        findings[0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("orphan-across-engine"),
        "{findings:#?}"
    );
}

/// US-015: the feature inventory, end to end.
///
/// This is the trigger `tests/rule_evidence.json` names for the rule. The
/// fixture is a workspace of two packages, because the question the rule
/// answers is per package: the same feature name is declared by one member and
/// read by the other, and only the reader that does not declare it is a
/// finding.
#[test]
fn the_feature_inventory_reports_both_sides_of_the_disagreement() {
    let report = published(InspectRequest::new(fixture("feature-inventory")));

    let rule = report["policy"]["rules"]
        .as_array()
        .expect("a scan publishes its rules")
        .iter()
        .find(|published| published["id"] == FEATURE)
        .expect("the feature rule is in the shipped policy");
    assert_eq!(rule["category"], "maintainability");
    assert_eq!(rule["tier"], "P3");
    assert_eq!(rule["level"], "warn");

    let findings = rule_findings(&report, FEATURE);
    let named: Vec<(&str, u64, &str)> = findings
        .iter()
        .map(|finding| {
            (
                finding["path"].as_str().unwrap_or_default(),
                finding["span"]["line_start"].as_u64().unwrap_or_default(),
                finding["message"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(findings.len(), 3, "{named:#?}");

    // A feature nothing reads and that activates nothing, pointed at the line
    // of the manifest that declares it. `default`, the feature a gate reads
    // through an `all(...)`, and the one that activates another are silent.
    assert_eq!(named[0].0, "app/Cargo.toml");
    assert_eq!(named[0].1, 18);
    assert!(
        named[0].2.contains("declares feature \"unused\""),
        "{:?}",
        named[0]
    );

    // A gate naming a feature no manifest declares, pointed at the gate.
    assert_eq!(named[1].0, "app/src/lib.rs");
    assert!(
        named[1].2.contains("cfg(feature = \"absent\")"),
        "{:?}",
        named[1]
    );

    // A gate naming a feature the neighbouring package declares and this one
    // does not: resolution is per package, not over the workspace.
    assert_eq!(named[2].0, "app/src/lib.rs");
    assert!(
        named[2].2.contains("cfg(feature = \"engine-only\")"),
        "{:?}",
        named[2]
    );

    let off = published(
        InspectRequest::new(fixture("feature-inventory"))
            .with_rule_override(RuleOverride::new(FEATURE, RuleLevel::Off)),
    );
    assert!(rule_findings(&off, FEATURE).is_empty(), "{off:#?}");
    assert_eq!(off["audit"]["score"]["dimensions"]["maintainability"], 100);

    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains(repository().to_str().unwrap()));
}
