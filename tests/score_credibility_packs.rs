#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Admission proofs of the EP-024 packs.
//!
//! Every pack owns a single fixture carrying its positives, its idiomatic
//! negatives and its negatives neutralized by `#[allow]`. The oracle freezes
//! the verdict observed on the normative toolchain: identifier, category, tier,
//! path, line and occurrence count.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rust_doctor::{InspectRequest, RuleLevel, RuleOverride, Status, inspect};
use serde_json::Value;

mod support;

const PACKS: [(&str, &str); 3] = [
    ("panic", "US-073"),
    ("performance", "US-074"),
    ("concurrency", "US-075"),
];

fn pack_root(pack: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/score-credibility/packs")
        .join(pack)
}

fn oracle(pack: &str) -> Value {
    serde_json::from_str(
        &fs::read_to_string(pack_root(pack).join("oracle.json")).expect("pack oracle should exist"),
    )
    .expect("pack oracle should be valid JSON")
}

fn report(request: InspectRequest) -> Value {
    let report = inspect(request);
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    serde_json::to_value(&report).expect("a valid report should serialize")
}

/// Catalogued diagnostics of the report, in published order.
fn curated(report: &Value) -> Vec<&Value> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter(|diagnostic| !diagnostic["category"].is_null())
        .collect()
}

fn observed(report: &Value) -> Vec<Value> {
    curated(report)
        .into_iter()
        .map(|diagnostic| {
            serde_json::json!({
                "code": diagnostic["code"],
                "category": diagnostic["category"],
                "path": diagnostic["path"],
                "line": diagnostic["span"]["line_start"],
                "occurrences": diagnostic["occurrences"],
            })
        })
        .collect()
}

fn expected(cases: &Value) -> Vec<Value> {
    cases
        .as_array()
        .expect("oracle cases should be an array")
        .iter()
        .map(|case| {
            serde_json::json!({
                "code": case["code"],
                "category": case["category"],
                "path": case["path"],
                "line": case["line"],
                "occurrences": case["occurrences"],
            })
        })
        .collect()
}

fn tiers(report: &Value) -> BTreeMap<String, String> {
    report["policy"]["rules"]
        .as_array()
        .expect("policy rules should be an array")
        .iter()
        .map(|rule| {
            (
                rule["id"].as_str().unwrap().to_owned(),
                rule["tier"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// US-073, US-074, US-075 AC-1: every lint of the pack produces exactly one
/// catalogued diagnostic, with its identifier, its category, its tier and its
/// help.
#[test]
fn every_pack_lint_produces_exactly_one_catalogued_diagnostic() {
    for (pack, story) in PACKS {
        let oracle = oracle(pack);
        assert_eq!(oracle["story"], story, "{pack}");
        let report = report(InspectRequest::new(pack_root(pack)));
        let tiers = tiers(&report);

        let production: Vec<_> = curated(&report)
            .into_iter()
            .filter(|diagnostic| {
                !diagnostic["path"]
                    .as_str()
                    .is_some_and(|path| path.starts_with("tests/"))
            })
            .collect();
        let production_cases: Vec<_> = production
            .iter()
            .map(|diagnostic| {
                serde_json::json!({
                    "code": diagnostic["code"],
                    "category": diagnostic["category"],
                    "path": diagnostic["path"],
                    "line": diagnostic["span"]["line_start"],
                    "occurrences": diagnostic["occurrences"],
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(production_cases, expected(&oracle["positive"]), "{pack}");

        let codes: BTreeSet<_> = production
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap())
            .collect();
        assert_eq!(codes.len(), production.len(), "{pack} repeats a lint");

        for (diagnostic, case) in production
            .iter()
            .zip(oracle["positive"].as_array().unwrap())
        {
            let code = diagnostic["code"].as_str().unwrap();
            assert_eq!(tiers[code], case["tier"].as_str().unwrap(), "{code}");
            assert!(
                diagnostic["help"]
                    .as_str()
                    .is_some_and(|help| !help.is_empty()),
                "{code} publishes no help"
            );
            assert_eq!(diagnostic["severity"], "warning", "{code}");
            assert_eq!(diagnostic["base_severity"], "warning", "{code}");
        }
    }
}

/// US-073, US-074, US-075 AC-2 and AC-4: no negative form, idiomatic or
/// neutralized by `#[allow]`, produces a diagnostic.
#[test]
fn no_negative_form_of_any_pack_produces_a_diagnostic() {
    for (pack, _) in PACKS {
        let oracle = oracle(pack);
        let source = fs::read_to_string(pack_root(pack).join("src/negatives.rs"))
            .expect("pack negatives should exist");
        let negatives = oracle["negative"].as_array().expect("negative cases");
        assert!(
            negatives.len() >= 2 * oracle["positive"].as_array().unwrap().len(),
            "{pack}"
        );

        let mut kinds = BTreeMap::new();
        for case in negatives {
            let marker = case["marker"].as_str().expect("negative marker");
            assert!(source.contains(marker), "{pack} lost {marker}");
            *kinds
                .entry(case["kind"].as_str().expect("negative kind"))
                .or_insert(0_usize) += 1;
        }
        assert_eq!(kinds["idiomatic"], kinds["allow"], "{pack}");

        // The positives all live in `src/lib.rs`: no diagnostic can therefore
        // come from the negatives file.
        let report = report(InspectRequest::new(pack_root(pack)));
        assert!(
            curated(&report)
                .iter()
                .all(|diagnostic| diagnostic["path"] != "src/negatives.rs"),
            "{pack} flagged a negative form"
        );
    }
}

/// Isolated copy of a pack fixture.
///
/// Every policy changes the Clippy arguments, so every scan invalidates the
/// fingerprint of the build directory. Running them all in the shared fixture
/// would put them in competition with the other tests of the binary, which scan
/// the same fixture at the same moment.
fn isolated_pack(pack: &str) -> PathBuf {
    fn copy(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        let mut entries: Vec<_> = fs::read_dir(source)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().unwrap().to_owned();
            if name == "target" {
                continue;
            }
            if path.is_dir() {
                copy(&path, &destination.join(name));
            } else {
                fs::copy(&path, destination.join(name)).unwrap();
            }
        }
    }

    let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/score-credibility-packs")
        .join(format!("{pack}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&destination);
    copy(&pack_root(pack), &destination);
    destination
}

/// US-073 AC-5, US-074 and US-075: a rule set to `off` disappears from the
/// Clippy command and from every diagnostic, without moving the others.
#[test]
fn a_rule_switched_off_leaves_the_command_and_the_findings() {
    for (pack, _) in PACKS {
        let oracle = oracle(pack);
        let root = isolated_pack(pack);
        let baseline = report(InspectRequest::new(&root));
        let baseline_codes: BTreeSet<_> = observed(&baseline)
            .iter()
            .map(|case| case["code"].as_str().unwrap().to_owned())
            .collect();

        for case in oracle["positive"].as_array().unwrap() {
            let code = case["code"].as_str().unwrap();
            let report = report(
                InspectRequest::new(&root)
                    .with_rule_override(RuleOverride::new(code, RuleLevel::Off)),
            );
            let command: Vec<String> = report["scan"]["command"]
                .as_array()
                .expect("a complete scan should publish its command")
                .iter()
                .map(|argument| argument.as_str().unwrap().to_owned())
                .collect();

            assert!(
                !command.contains(&code.to_owned()),
                "{code} stayed in the command"
            );
            assert_eq!(command, support::expected_clippy_command(&report["policy"]));
            let codes: BTreeSet<_> = observed(&report)
                .iter()
                .map(|case| case["code"].as_str().unwrap().to_owned())
                .collect();
            assert!(!codes.contains(code), "{code} still produced a finding");
            let mut expected = baseline_codes.clone();
            expected.remove(code);
            assert_eq!(codes, expected, "{code} moved another finding");
        }
        fs::remove_dir_all(&root).unwrap();
    }
}

/// US-073 AC-3, settled one step earlier than it used to be: a test file in the
/// Cargo sense reaches the report through no lint at all.
///
/// The exemption was the interesting question while the scan compiled test
/// targets, and it was not a property of the lint: it was governed by the
/// `clippy.toml` of the scanned workspace, which Clippy looks for by walking up
/// the parent directories. The fixture still pins the four options and
/// `tests/exemptions.rs` still carries one usage per lint, two of them exempt
/// and two of them not. The scope decides before any of that: the scan compiles
/// Cargo's default targets, so the whole file stays out of the report whatever
/// the options say, and the oracle records the empty verdict.
#[test]
fn no_cargo_test_target_reaches_the_report() {
    let oracle = oracle("panic");
    let configuration = pack_root("panic").join(
        oracle["test_context_configuration"]
            .as_str()
            .expect("the panic pack should name its Clippy configuration"),
    );
    let configuration =
        fs::read_to_string(&configuration).expect("the panic pack should carry that configuration");
    let usages = fs::read_to_string(pack_root("panic").join("tests/exemptions.rs"))
        .expect("the panic pack should carry its test file");

    let documented = oracle["test_context_exemptions"]
        .as_array()
        .expect("the panic pack should document its exemptions");
    assert!(!documented.is_empty());
    let mut verdicts = BTreeSet::new();
    for case in documented {
        let code = case["code"].as_str().unwrap();
        let option = case["clippy_option"]
            .as_str()
            .expect("every exemption should name the option that governs it");
        let exempt = case["exempt_in_cargo_tests"].as_bool().unwrap();
        assert!(
            configuration.contains(&format!("{option} = {exempt}")),
            "{option} is not pinned by the fixture"
        );
        let marker = format!("exemption_{}", code.trim_start_matches("clippy::"));
        assert!(usages.contains(&marker), "the fixture lost {marker}");
        verdicts.insert(exempt);
    }
    // Both verdicts are represented, so the emptiness below cannot be read as
    // the fixture only holding lints Clippy would have exempted anyway.
    assert_eq!(verdicts, BTreeSet::from([false, true]));

    let report = report(InspectRequest::new(pack_root("panic")));
    assert_eq!(
        observed(&report)
            .into_iter()
            .filter(|case| case["path"]
                .as_str()
                .is_some_and(|path| path.starts_with("tests/")))
            .collect::<Vec<_>>(),
        expected(&oracle["test_context"])
    );
}

/// US-074 AC-3 and EP-024: the five dimensions vary, and the dimension of a
/// triggered pack drops strictly below 100.
#[test]
fn each_pack_moves_its_own_dimension_below_one_hundred() {
    for (pack, _) in PACKS {
        let oracle = oracle(pack);
        let report = report(InspectRequest::new(pack_root(pack)));
        let dimensions = &report["audit"]["score"]["dimensions"];
        assert_eq!(dimensions, &oracle["score"]["dimensions"], "{pack}");
        assert_eq!(
            report["audit"]["score"]["value"], oracle["score"]["value"],
            "{pack}"
        );

        let touched: BTreeSet<_> = oracle["positive"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| match case["category"].as_str().unwrap() {
                "security" => "security",
                "correctness" | "reliability" => "reliability",
                "maintainability" => "maintainability",
                "performance" => "performance",
                _ => "dependencies",
            })
            .collect();
        for dimension in touched {
            assert!(
                dimensions[dimension].as_u64().unwrap() < 100,
                "{pack} left {dimension} at 100"
            );
        }
    }
    // The performance pack is the one that proves the Performance dimension,
    // which stayed frozen at 100 while `CATEGORIES` did not admit it.
    let performance = report(InspectRequest::new(pack_root("performance")));
    assert!(
        performance["audit"]["score"]["dimensions"]["performance"]
            .as_u64()
            .unwrap()
            < 100
    );
}

/// US-075 AC-3 and AC-5: a workspace with no asynchronous runtime stays silent
/// on the lints that do not apply to it, and the repository itself produces no
/// diagnostic of the concurrency pack.
#[test]
fn the_concurrency_pack_stays_silent_where_it_does_not_apply() {
    let pack_codes: BTreeSet<_> = oracle("concurrency")["positive"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["code"].as_str().unwrap().to_owned())
        .collect();

    // The panic pack fixture contains neither `async`, nor a lock, nor I/O: no
    // lint of the concurrency pack applies to it.
    let synchronous = report(InspectRequest::new(pack_root("panic")));
    assert!(
        curated(&synchronous).iter().all(|diagnostic| !pack_codes
            .contains(diagnostic["code"].as_str().unwrap_or_default())),
        "the concurrency pack fired where it does not apply"
    );

    let repository = report(InspectRequest::new(Path::new(env!("CARGO_MANIFEST_DIR"))));
    let observed: BTreeSet<_> = curated(&repository)
        .into_iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str())
        .filter(|code| pack_codes.contains(*code))
        .collect();
    assert!(observed.is_empty(), "self-scan produced {observed:?}");
}

/// Admission contract: no Clippy rule of the catalog is `deny` by default.
///
/// A rule that is `deny` by default cannot be switched off: removing its `-W`
/// restores Clippy's refusal and turns the scan into a compilation failure.
/// `clippy::async_yields_async` and `clippy::unused_io_amount` were set aside
/// from the packs for that reason, a verdict measured on the normative
/// toolchain.
#[test]
fn no_catalogued_clippy_rule_is_denied_by_default() {
    let help = std::process::Command::new("clippy-driver")
        .args(["-W", "help"])
        .output()
        .expect("clippy-driver should start");
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).expect("the lint table should be UTF-8");

    let defaults: BTreeMap<String, String> = help
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?.strip_prefix("clippy::")?;
            let level = fields.next()?;
            matches!(level, "allow" | "warn" | "deny").then(|| {
                (
                    format!("clippy::{}", name.replace('-', "_")),
                    level.to_owned(),
                )
            })
        })
        .collect();
    assert!(defaults.len() > 500, "{} lints listed", defaults.len());

    let report = report(InspectRequest::new(pack_root("panic")));
    let catalogued: Vec<_> = report["policy"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|rule| rule["id"].as_str())
        .filter(|id| id.starts_with("clippy::"))
        .collect();
    assert_eq!(catalogued.len(), 33);
    for id in catalogued {
        let level = defaults.get(id).map(String::as_str).unwrap_or_default();
        assert!(
            !level.is_empty(),
            "{id} is not a lint of the normative toolchain"
        );
        assert_ne!(level, "deny", "{id} cannot be switched off");
    }
    for rejected in ["clippy::async_yields_async", "clippy::unused_io_amount"] {
        assert_eq!(defaults[rejected], "deny", "{rejected}");
    }
}

/// US-076: the local dependency health pack, end to end.
///
/// The scans run on a copy: `cargo clippy` creates or rewrites `Cargo.lock`, so
/// scanning the fixture in place would destroy the resolved graph it freezes.
#[test]
fn the_dependency_pack_reaches_the_report_and_its_own_dimension() {
    fn resolution(case: &str) -> PathBuf {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/cargo-health/resolution")
            .join(case);
        let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/score-credibility-packs")
            .join(format!("resolution-{case}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&destination);
        fs::create_dir_all(destination.join("src")).unwrap();
        for entry in fs::read_dir(&source).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                fs::copy(&path, destination.join(path.file_name().unwrap())).unwrap();
            }
        }
        for entry in fs::read_dir(source.join("src")).unwrap() {
            let path = entry.unwrap().path();
            fs::copy(
                &path,
                destination.join("src").join(path.file_name().unwrap()),
            )
            .unwrap();
        }
        destination
    }

    let cases = [
        ("duplicate", "rust_doctor::cargo::duplicate_major_versions"),
        ("absent", "rust_doctor::cargo::missing_lockfile"),
    ];
    for (case, code) in cases {
        let root = resolution(case);
        let report = report(InspectRequest::new(&root));
        let findings = curated(&report);
        let finding = findings
            .iter()
            .find(|diagnostic| diagnostic["code"] == code);
        assert!(finding.is_some(), "{case} produced no {code}");
        let finding = finding.expect("the finding was just asserted");

        assert_eq!(finding["category"], "dependencies", "{case}");
        assert_eq!(finding["severity"], "warning", "{case}");
        assert!(
            finding["help"]
                .as_str()
                .is_some_and(|help| !help.is_empty()),
            "{case}"
        );
        assert!(
            !finding["message"]
                .as_str()
                .unwrap()
                .contains(env!("CARGO_MANIFEST_DIR")),
            "{case} leaked an absolute path"
        );
        assert!(
            report["audit"]["score"]["dimensions"]["dependencies"]
                .as_u64()
                .unwrap()
                < 100,
            "{case} left Dependencies at 100"
        );
        assert!(
            report["audit"]["categories"]
                .as_array()
                .unwrap()
                .iter()
                .any(|category| category["name"] == "Dependencies"),
            "{case}"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    // A workspace clean on these criteria produces no diagnostic of the pack,
    // and an unusable resolution makes the pack abstain without breaking the
    // scan.
    let clean = resolution("clean");
    let report = report(InspectRequest::new(&clean));
    assert!(
        curated(&report)
            .iter()
            .all(|diagnostic| diagnostic["category"] != "dependencies"),
        "the clean workspace produced a dependency finding"
    );
    fs::remove_dir_all(&clean).unwrap();

    let unusable = resolution("unusable");
    let report = inspect(InspectRequest::new(&unusable));
    let published = serde_json::to_value(&report).expect("a valid report should serialize");
    assert!(
        published["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["category"] != "dependencies")
    );
    let errors: Vec<_> = published["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|error| error["stage"] == "dependencies")
        .collect();
    assert_eq!(errors.len(), 1, "{:?}", published["errors"]);
    assert_eq!(errors[0]["code"], "lockfile-resolution-absent");
    assert!(!errors[0]["message"].as_str().unwrap().contains('/'));
    assert!(report.audit.score.is_some());
    fs::remove_dir_all(&unusable).unwrap();
}
