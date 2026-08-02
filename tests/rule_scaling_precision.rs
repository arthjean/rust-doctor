#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Output;

mod support;

use support::rule_scaling::{
    ExpectedSpan, cargo_json_records, compiler_messages, evidence, observe_precision, primary_span,
    run_clippy,
};

fn fixture(path: impl AsRef<Path>) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rule-scaling-kernel/matrix")
        .join(path)
}

fn expected_signature(span: &ExpectedSpan) -> (u64, u64, u64, u64) {
    (
        span.line_start,
        span.column_start,
        span.line_end,
        span.column_end,
    )
}

#[test]
fn malformed_cargo_json_cannot_disappear_from_precision_evidence() {
    let output = Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: b"{\"reason\":\"compiler-artifact\"}\nnot-json\n".to_vec(),
        stderr: Vec::new(),
    };
    let error = cargo_json_records(&output).unwrap_err();
    assert!(error.contains("record 2"), "{error}");
}

#[test]
fn each_rule_passes_its_independent_twenty_positive_forty_negative_matrix() {
    let evidence = evidence();
    let rules = &evidence.precision.rules;
    assert_eq!(rules.len(), 5);
    let target_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/rule-scaling-kernel/precision-observation");
    let observation = observe_precision(&evidence, &target_root);

    let source = fs::read_to_string(fixture("src/lib.rs")).unwrap();
    let mut positive_cases = BTreeSet::new();
    let mut negative_cases = BTreeSet::new();
    for rule in rules {
        let id = rule.id.as_str();
        let positives = &rule.positives;
        let negatives = &rule.negatives;
        let measured = observation.rule(id);
        assert_eq!(positives.len(), 4, "{id}");
        assert_eq!(negatives.len(), 8, "{id}");
        assert_eq!(measured.tp, 4, "{id}");
        assert_eq!(measured.fp, 0, "{id}");
        assert_eq!(measured.tn, 8, "{id}");
        assert_eq!(measured.r#fn, 0, "{id}");
        assert!(measured.passed(), "{id}");

        let expected = positives
            .iter()
            .map(|case| {
                assert!(
                    positive_cases.insert(case.case.as_str()),
                    "duplicate case {}",
                    case.case
                );
                expected_signature(&case.span)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            measured
                .spans
                .iter()
                .map(expected_signature)
                .collect::<Vec<_>>(),
            expected,
            "{id}"
        );

        for case in negatives {
            assert!(negative_cases.insert(case), "duplicate case {case}");
            assert!(
                source.contains(&format!("matrix-case:{case}")),
                "missing source marker for {case}"
            );
        }
    }
    assert_eq!(positive_cases.len(), 20);
    assert_eq!(negative_cases.len(), 40);

    assert_eq!(
        observation.build_output_candidate_diagnostics,
        evidence
            .precision
            .contexts
            .build_output_candidate_diagnostics
    );

    let unicode_line = source.lines().nth(82).unwrap();
    let literal_column = unicode_line.chars().position(|value| value == '"').unwrap() + 1;
    assert_eq!(literal_column, 21);
    let candidate_byte = unicode_line.find("\"-alpha beta\"").unwrap();
    let candidate_column = unicode_line[..candidate_byte].chars().count() + 1;
    assert_eq!(
        evidence
            .precision
            .contexts
            .unicode_primary_span
            .column_start,
        candidate_column as u64,
    );
    let non_unix = &evidence.precision.contexts.non_unix_permissions;
    assert_eq!(
        observation.non_unix_permissions.spans,
        vec![non_unix.primary_span.clone()]
    );
    assert_eq!(
        observation.non_unix_permissions.spans.len(),
        non_unix.candidate_diagnostics
    );
}

#[test]
fn tests_and_non_executable_contexts_add_no_distinct_candidate_span() {
    let evidence = evidence();
    let ids = evidence
        .precision
        .rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<BTreeSet<_>>();
    let expected = evidence
        .precision
        .rules
        .iter()
        .flat_map(|rule| {
            let id = rule.id.as_str();
            rule.positives
                .iter()
                .map(move |case| (id, expected_signature(&case.span)))
        })
        .collect::<BTreeSet<_>>();

    let mut arguments = vec!["--tests", "--no-deps", "--message-format=json", "--"];
    arguments.extend(evidence.catalog.explicit_flags());
    let output = run_clippy(
        &fixture("Cargo.toml"),
        &arguments,
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/rule-scaling-kernel/precision-matrix-tests"),
    );
    assert!(output.status.success());
    let actual = compiler_messages(&output)
        .iter()
        .filter_map(|message| {
            let id = message.code.as_ref()?.code.as_str();
            ids.contains(id)
                .then(|| (id.to_owned(), expected_signature(&primary_span(message))))
        })
        .collect::<BTreeSet<_>>();
    let expected = expected
        .into_iter()
        .map(|(id, span)| (id.to_owned(), span))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}
