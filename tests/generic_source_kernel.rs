#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Proofs of the generic source kernel: the imported form is detected just like
//! the fully qualified form, no detector compares a literal path, and the reach
//! structure carries no field specific to a crate.

use std::fs;
use std::path::{Path, PathBuf};

use rust_doctor::{Diagnostic, DiagnosticSource, InspectRequest, Status, inspect};
use serde_json::Value;

const DETECTORS: &str = include_str!("../src/source_kernel/detectors.rs");
const KERNEL: &str = include_str!("../src/source_kernel.rs");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/source-kernel")
        .join(name)
}

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/source-kernel/precision/oracle.json")).unwrap()
}

/// Line bounds of the body of `symbol`, the closing being the first line
/// reduced to `}`.
fn symbol_lines(source: &str, symbol: &str) -> (usize, usize) {
    let lines: Vec<_> = source.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.contains(&format!("fn {symbol}")))
        .unwrap_or(lines.len());
    assert!(start < lines.len(), "missing symbol {symbol}");
    let end = lines[start..]
        .iter()
        .position(|line| *line == "}")
        .map_or(lines.len(), |offset| start + offset);
    (start + 1, end + 1)
}

/// Literal strings of a code line, comments excluded.
fn string_literals(line: &str) -> Vec<String> {
    let code = line.split("//").next().unwrap_or_default();
    let mut literals = Vec::new();
    let mut current: Option<String> = None;
    let mut escaped = false;
    for character in code.chars() {
        match &mut current {
            Some(literal) => {
                if escaped {
                    escaped = false;
                    literal.push(character);
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    literals.push(std::mem::take(literal));
                    current = None;
                } else {
                    literal.push(character);
                }
            }
            None if character == '"' => current = Some(String::new()),
            None => {}
        }
    }
    literals
}

/// US-069: the imported, renamed or grouped form is emitted like the fully
/// qualified form.
#[test]
fn imported_forms_are_reported_like_fully_qualified_ones() {
    let report = inspect(InspectRequest::new(fixture("precision")));
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    let oracle = oracle();

    let native: Vec<&Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.source == DiagnosticSource::RustDoctor)
        .collect();

    for (code, cases) in oracle["imported_forms"].as_object().unwrap() {
        for case in cases.as_array().unwrap() {
            let path = case["path"].as_str().unwrap();
            let symbol = case["symbol"].as_str().unwrap();
            let source = fs::read_to_string(fixture("precision").join(path)).unwrap();
            let (start, end) = symbol_lines(&source, symbol);
            assert!(
                native.iter().any(|diagnostic| {
                    diagnostic.code.as_deref() == Some(code.as_str())
                        && diagnostic.path.as_deref() == Some(path)
                        && diagnostic
                            .span
                            .as_ref()
                            .is_some_and(|span| (start..=end).contains(&span.line_start))
                }),
                "imported form {case} produced no finding"
            );
        }
    }

    // The same unit carries both spellings: they are counted separately.
    let positives = fs::read_to_string(fixture("precision").join("app/src/positives.rs")).unwrap();
    let (qualified_start, qualified_end) = symbol_lines(&positives, "shell_sh");
    assert!(native.iter().any(|diagnostic| {
        diagnostic.path.as_deref() == Some("app/src/positives.rs")
            && diagnostic
                .span
                .as_ref()
                .is_some_and(|span| (qualified_start..=qualified_end).contains(&span.line_start))
    }));
}

/// US-069: no literal path comparison survives in the detectors. This test
/// fails if a qualified path string reappears in them.
#[test]
fn no_detector_compares_a_written_qualified_path() {
    let offending: Vec<_> = DETECTORS
        .lines()
        .enumerate()
        .flat_map(|(index, line)| {
            string_literals(line)
                .into_iter()
                .filter(|literal| literal.contains("::"))
                .map(move |literal| format!("line {}: {literal}", index + 1))
        })
        .collect();

    assert!(
        offending.is_empty(),
        "detectors must resolve provenance instead of comparing paths: {offending:?}"
    );
    // The detector is indeed the one describing its target, otherwise the test
    // above would pass on a file empty of any target.
    assert!(DETECTORS.contains("segments: &[\"process\", \"Command\"]"));
    assert!(DETECTORS.contains("krate: \"reqwest\""));
}

/// US-071: reachability carries no field named after a crate or a rule.
#[test]
fn reachability_carries_only_package_and_target_identity() {
    let declaration = KERNEL
        .split_once("struct Reachability {")
        .and_then(|(_, rest)| rest.split_once('}'))
        .map(|(fields, _)| fields.to_owned())
        .expect("the kernel declares the reachability structure");
    let fields: Vec<_> = declaration
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .map(|line| line.trim_end_matches(',').to_owned())
        .collect();

    // `test_gated` is a fact about the traversal that reached the file, the way
    // the four others are facts about the target it was reached from: what this
    // list refuses is a field named after a crate or a rule, which is a
    // detector keeping its state in the reachability of every file.
    assert_eq!(
        fields,
        [
            "package_id: String",
            "package_name: String",
            "target_key: String",
            "target_name: String",
            "test_gated: bool",
        ]
    );
}

/// US-071: the targeted crate is resolved through the manifest, and its absence
/// silences the detector without an error.
#[test]
fn a_renamed_dependency_resolves_and_an_absent_one_stays_silent() {
    let report = inspect(InspectRequest::new(fixture("precision")));
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);

    let renamed: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code.as_deref() == Some("rust_doctor::source::disabled_tls_verification")
        })
        .collect();
    assert!(
        renamed
            .iter()
            .all(|diagnostic| diagnostic.package.as_deref() == Some("source-kernel-app")),
        "{renamed:?}"
    );
    assert!(!renamed.is_empty());
    assert!(
        renamed
            .iter()
            .all(|diagnostic| diagnostic.path.as_deref() != Some("no-dependency/src/lib.rs"))
    );
    assert!(
        report.errors.iter().all(|error| error.stage != "source"),
        "{:?}",
        report.errors
    );
}
