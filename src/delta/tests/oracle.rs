//! The frozen adversarial oracle: thirty-two cases replayed twenty times.
//!
//! It sits in a file of its own for the reason the rest of the crate splits its
//! tests, `the_delta_holds_the_size_bound_it_matches_for`: the module that
//! decides what a branch introduced has to pass the size rule it publishes.

use super::*;
use serde_json::{Value, json};

fn line_span(line: usize, source: &str) -> DiagnosticSpan {
    let content = source.trim_end_matches('\n').trim_end_matches('\r');
    span(line, 1, content.chars().count() + 1)
}

/// One adversarial case: the two trees it writes, the diagnostics it names and
/// the feature it stands for.
///
/// The thirty-two cases sit in six families rather than in one `match`. A single
/// dispatcher over all of them reached 181 lines and cyclomatic complexity 23,
/// over both bounds the structural pass of this very crate reports at.
fn oracle_case(
    index: usize,
    baseline_parent: &Path,
    current_parent: &Path,
) -> (String, DeltaReport) {
    let mut scenario = Scenario::under(baseline_parent, current_parent, &format!("{index:02}"));
    scenario.write_both("src/original.rs", ORIGINAL);
    scenario.baseline = vec![diagnostic(
        &format!("base-{index:02}"),
        Some("src/original.rs"),
        Some(span(1, 1, 8)),
    )];
    scenario.current = vec![diagnostic(
        &format!("current-{index:02}"),
        Some("src/original.rs"),
        Some(span(1, 1, 8)),
    )];

    let name = match index {
        0..=3 => whitespace_case(index, &mut scenario),
        4..=7 => identity_case(index, &mut scenario),
        8..=15 => moved_case(index, &mut scenario),
        16..=19 => unproven_case(index, &mut scenario),
        20..=27 => changed_case(index, &mut scenario),
        _ => one_sided_case(index, &mut scenario),
    };
    let delta = scenario.delta();
    (name, delta)
}

/// The two sides written from two spellings of one line, each span covering its
/// own.
fn opposed_lines(scenario: &mut Scenario, baseline_source: &str, current_source: &str) {
    scenario.write_baseline("src/original.rs", baseline_source);
    scenario.write_current("src/original.rs", current_source);
    scenario.baseline[0].span = Some(line_span(1, baseline_source));
    scenario.current[0].span = Some(line_span(1, current_source));
}

/// Whitespace, encoding and multi-line spans: what the normalized excerpt has to
/// see through to call two spellings one finding.
fn whitespace_case(index: usize, scenario: &mut Scenario) -> String {
    match index {
        0 => {
            let evidence = "let café = todo!();\n";
            scenario.write_baseline("src/original.rs", evidence);
            scenario.write_current("src/original.rs", &format!("// shifted\n{evidence}"));
            scenario.baseline[0].span = Some(line_span(1, evidence));
            scenario.current[0].span = Some(line_span(2, evidence));
            "unicode".to_owned()
        }
        1 => {
            opposed_lines(
                scenario,
                "\t todo!(  \"échec\" ) ;\r\n",
                "todo!( \"échec\" ) ;\n",
            );
            "crlf-lf".to_owned()
        }
        2 => {
            opposed_lines(scenario, "todo!(\t\"tabs\" );\n", "todo!( \"tabs\" );\n");
            "tabs".to_owned()
        }
        _ => {
            scenario.write_baseline("src/original.rs", "call(\n\tvalue,\nother)\n");
            scenario.write_current("src/original.rs", "call( \n value,\t\nother)\n");
            let span = DiagnosticSpan {
                line_start: 1,
                column_start: 1,
                line_end: 3,
                column_end: 7,
            };
            scenario.baseline[0].span = Some(span.clone());
            scenario.current[0].span = Some(span);
            "multiline-span".to_owned()
        }
    }
}

/// What the identity is made of, and what it deliberately leaves out.
fn identity_case(index: usize, scenario: &mut Scenario) -> String {
    match index {
        4 => {
            scenario.write_both("src/original.rs", "todo!();\ntodo!();\n");
            scenario.baseline.push(diagnostic(
                "base-duplicate",
                Some("src/original.rs"),
                Some(span(2, 1, 8)),
            ));
            scenario.current.push(diagnostic(
                "current-duplicate",
                Some("src/original.rs"),
                Some(span(2, 1, 8)),
            ));
            "duplicates".to_owned()
        }
        5 => {
            scenario.write_current("src/copy.rs", ORIGINAL);
            scenario.current.insert(
                0,
                diagnostic("current-copy", Some("src/copy.rs"), Some(span(1, 1, 8))),
            );
            "copies".to_owned()
        }
        6 => {
            scenario.baseline[0].code = None;
            scenario.current[0].code = None;
            "nullable-code".to_owned()
        }
        _ => {
            scenario.current[0].severity = Severity::Error;
            "severity-excluded".to_owned()
        }
    }
}

/// The same proof in another file, eight times over, which is what the moved
/// pass exists for.
fn moved_case(index: usize, scenario: &mut Scenario) -> String {
    let moved_path = format!("src/moved-{:02}.rs", index - 8);
    scenario.write_current(&moved_path, " todo!();\n");
    scenario.current[0].path = Some(moved_path);
    scenario.current[0].span = Some(span(1, 2, 9));
    format!("cross-file-stable-{:02}", index - 8)
}

/// No proof on one side or the other, so the message is all that is left.
fn unproven_case(index: usize, scenario: &mut Scenario) -> String {
    match index {
        16 => {
            scenario.baseline[0].path = None;
            scenario.baseline[0].span = None;
            scenario.current[0].path = None;
            scenario.current[0].span = None;
            "pathless-fallback".to_owned()
        }
        17 => {
            scenario.baseline[0].path = Some("src/missing.rs".to_owned());
            scenario.current[0].path = Some("src/missing.rs".to_owned());
            "missing-source-fallback".to_owned()
        }
        18 => {
            let path = "src/current-only.rs";
            scenario.write_current(path, ORIGINAL);
            scenario.baseline[0].path = Some(path.to_owned());
            scenario.current[0].path = Some(path.to_owned());
            "baseline-unavailable-fallback".to_owned()
        }
        _ => {
            let path = "src/baseline-only.rs";
            scenario.write_baseline(path, ORIGINAL);
            scenario.baseline[0].path = Some(path.to_owned());
            scenario.current[0].path = Some(path.to_owned());
            "current-unavailable-fallback".to_owned()
        }
    }
}

/// One member of the identity changed, which has to read as a different
/// finding. Case 23 shares a long prefix on purpose, since a prefix is what a
/// cheap comparison would stop at.
fn changed_case(index: usize, scenario: &mut Scenario) -> String {
    match index {
        20..=23 => {
            let prefix = if index == 23 {
                "shared-prefix-shared-prefix-shared-prefix"
            } else {
                "evidence"
            };
            opposed_lines(
                scenario,
                &format!("{prefix}-old\n"),
                &format!("{prefix}-new\n"),
            );
            format!("changed-evidence-{:02}", index - 20)
        }
        24 => {
            scenario.baseline[0].message = "old message".to_owned();
            scenario.current[0].message = "new message".to_owned();
            "changed-message-00".to_owned()
        }
        25 => {
            scenario.current[0].code = Some("clippy::dbg_macro".to_owned());
            "changed-code-00".to_owned()
        }
        26 => {
            scenario.current[0].source = DiagnosticSource::Rustc;
            "changed-source-00".to_owned()
        }
        _ => {
            scenario.baseline[0].message = "shared prefix old".to_owned();
            scenario.current[0].message = "shared prefix new".to_owned();
            "changed-message-prefix-00".to_owned()
        }
    }
}

/// One side empty, which is what a first scan and a wholly repaired branch look
/// like.
fn one_sided_case(index: usize, scenario: &mut Scenario) -> String {
    if index <= 29 {
        scenario.baseline.clear();
        format!("introduced-only-{:02}", index - 28)
    } else {
        scenario.current.clear();
        format!("fixed-only-{:02}", index - 30)
    }
}

fn oracle_document(baseline_root: &Path, current_root: &Path) -> Value {
    let cases = (0..32)
        .map(|index| {
            let (name, delta) = oracle_case(index, baseline_root, current_root);
            json!({
                "name": name,
                "introduced": delta.summary.introduced,
                "pre_existing": delta.summary.pre_existing,
                "fixed": delta.summary.fixed,
                "cross_file_matches": delta.summary.cross_file_matches,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "fingerprint_version": FINGERPRINT_VERSION,
        "runs": 20,
        "features": [
            "unicode",
            "crlf-lf",
            "tabs",
            "multiline-span",
            "pathless",
            "duplicates",
            "copies",
            "prefix-collisions",
        ],
        "cases": cases,
    })
}

#[test]
fn adversarial_oracle_runs_the_complete_kernel_for_32_cases_and_20_identical_runs() {
    let baseline_root = Scratch(root("oracle-base"));
    let current_root = Scratch(root("oracle-current"));
    let actual = oracle_document(&baseline_root.0, &current_root.0);
    for _ in 1..20 {
        assert_eq!(
            oracle_document(&baseline_root.0, &current_root.0),
            actual,
            "the complete evidence and matching kernel must be deterministic",
        );
    }

    let fixture_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/baseline/delta-oracle.json");
    if std::env::var_os("RUST_DOCTOR_UPDATE_DELTA_ORACLE").is_some() {
        fs::write(
            &fixture_path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
    }
    let expected: Value = serde_json::from_str(&fs::read_to_string(fixture_path).unwrap()).unwrap();
    assert_eq!(actual, expected);
}
