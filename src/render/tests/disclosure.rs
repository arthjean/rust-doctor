use super::*;
use crate::report::{PolicyBlockingReport, PolicyReport, PolicyRuleReport};
use crate::{BlockingLevelSource, DiagnosticContext, NotEvaluated, RuleLevel, RuleLevelSource};

fn rebuilt(mut report: InspectReport) -> InspectReport {
    report.audit = Audit::build(1, 100, report.status, &report.diagnostics);
    report.summary = Summary::from_diagnostics(&report.diagnostics);
    report
}

fn in_context(diagnostic: Diagnostic, id: &str, context: DiagnosticContext) -> Diagnostic {
    Diagnostic {
        id: id.to_owned(),
        context: Some(context),
        ..diagnostic
    }
}

#[test]
fn a_withheld_rule_that_moves_the_score_is_never_called_worthless() {
    let mut report = report();
    report.diagnostics = vec![diagnostic("clippy::exit", "reliability", 1)];
    let report = rebuilt(report);
    let score = report.audit.score.as_ref().unwrap();
    assert_eq!(score.withheld_rule_ids, ["clippy::exit"]);
    assert_eq!(score.applied_ceiling, None);
    let without = Audit::build(1, 100, Status::Complete, &[]);
    assert!(
        without.score.unwrap().dimensions.reliability > score.dimensions.reliability,
        "removing it moves its dimension"
    );

    let output = rendered(&report, 200, false, false);
    assert!(
        output.contains(
            "clippy::exit (86% noise on 5 sites) reports here but is left out of what to fix \
             first: the corpus found it wrong on most of the sites it reviewed."
        ),
        "{output}"
    );
    assert!(!output.contains("nothing worth"), "{output}");
}

#[test]
fn no_withheld_rule_prints_no_withheld_sentence() {
    let report = report();
    assert!(report.audit.score.as_ref().unwrap().withheld_rule_ids.is_empty());
    let output = rendered(&report, 200, false, false);
    assert!(!output.contains("left out"), "{output}");
}

#[test]
fn the_dimensions_are_printed_for_an_authoritative_score_only() {
    let report = report();
    let dimensions = report.audit.score.as_ref().unwrap().dimensions;
    let expected = format!(
        "Security {} · Reliability {} · Maintainability {} · Performance {} · Dependencies {}",
        dimensions.security,
        dimensions.reliability,
        dimensions.maintainability,
        dimensions.performance,
        dimensions.dependencies
    );
    let output = rendered(&report, 200, false, false);
    assert!(output.lines().any(|row| row == expected), "{output}");
    assert!(
        output.find("└─────┘") < output.find(&expected),
        "the line sits under the block: {output}"
    );

    let mut partial = report.clone();
    partial.status = Status::Incomplete;
    partial.complete = false;
    partial.audit = Audit::build(1, 100, Status::Incomplete, &partial.diagnostics);
    let output = rendered(&partial, 200, false, false);
    assert!(!output.contains("Security "), "{output}");
    for reason in &partial.audit.score.as_ref().unwrap().reasons {
        assert!(output.contains(reason.explanation()), "{output}");
    }

    partial.audit.source_files = 0;
    partial.audit.score = None;
    let output = rendered(&partial, 200, false, false);
    assert!(!output.contains("Security "), "{output}");
}

#[test]
fn the_scope_line_states_what_was_scored_and_what_clippy_read() {
    let mut report = report();
    let output = rendered(&report, 200, false, false);
    assert!(
        output.contains(
            "Scored 1 production files (100 lines) in 1.2s; Clippy did not run, native rules \
             also read tests"
        ),
        "{output}"
    );
    report.scan.command = Some(vec!["cargo".to_owned(), "clippy".to_owned()]);
    let output = rendered(&report, 200, false, false);
    assert!(
        output.contains(
            "Scored 1 production files (100 lines) in 1.2s; Clippy linted library and binary \
             targets, native rules also read tests"
        ),
        "{output}"
    );
}

fn rule(id: &str, not_evaluated: Option<NotEvaluated>) -> PolicyRuleReport {
    let definition = crate::policy::find(id).unwrap();
    PolicyRuleReport {
        id: id.to_owned(),
        category: definition.category.to_owned(),
        tier: definition.tier,
        level: RuleLevel::Warn,
        source: RuleLevelSource::Default,
        corpus_noise_basis_points: None,
        corpus_reviewed_sites: None,
        not_evaluated,
    }
}

#[test]
fn unsupported_rules_are_named_and_absent_crates_counted_under_verbose() {
    let mut report = report();
    report.policy = Some(PolicyReport {
        config_file: None,
        blocking: PolicyBlockingReport {
            level: BlockingLevel::Error,
            source: BlockingLevelSource::Default,
        },
        rules: vec![
            rule(
                "rust_doctor::source::disabled_tls_verification",
                Some(NotEvaluated::CrateVersionUnsupported),
            ),
            rule("rust_doctor::source::permissive_cors", Some(NotEvaluated::CrateAbsent)),
            rule("rust_doctor::source::block_on_in_async", Some(NotEvaluated::CrateAbsent)),
            rule("clippy::todo", None),
        ],
        ignore: Vec::new(),
        overrides: Vec::new(),
    });
    let named = "Not evaluated, the crate version is outside the range the rule was validated \
                 against: rust_doctor::source::disabled_tls_verification";
    let counted = "2 rules were not evaluated: no package depends on their crate.";

    let output = rendered(&report, 200, false, false);
    assert!(output.contains(named), "{output}");
    assert!(!output.contains(counted), "{output}");
    let output = rendered(&report, 200, false, true);
    assert!(output.contains(named), "{output}");
    assert!(output.contains(counted), "{output}");
}

#[test]
fn a_category_tally_counts_what_weighs_and_sets_the_rest_apart() {
    let mut report = report();
    let base = diagnostic("clippy::dbg_macro", "maintainability", 1);
    report.diagnostics = vec![
        Diagnostic {
            id: "production".to_owned(),
            ..base.clone()
        },
        Diagnostic {
            occurrences: 4,
            ..in_context(base.clone(), "tests", DiagnosticContext::Tests)
        },
        in_context(base, "build", DiagnosticContext::BuildScript),
    ];
    let report = rebuilt(report);
    let output = rendered(&report, 200, false, false);
    assert!(
        output.lines().any(|row| row
            == "Maintainability: 0 errors, 1 warnings, 0 info, 0 unknown (occurrences) (+5 \
                outside production code)"),
        "{output}"
    );

    let output = rendered(&super::report(), 200, false, false);
    assert!(
        output.lines().any(|row| row == "Bugs: 0 errors, 1 warnings, 0 info, 0 unknown (occurrences)"),
        "{output}"
    );
}

#[test]
fn a_group_heading_counts_its_own_severity_and_names_the_rest() {
    let mut report = report();
    let warning = diagnostic("clippy::print_stderr", "maintainability", 1);
    let info = Diagnostic {
        id: "bin".to_owned(),
        base_severity: Severity::Info,
        severity: Severity::Info,
        occurrences: 31,
        ..warning.clone()
    };
    report.diagnostics = vec![warning, info];
    let output = rendered(&rebuilt(report), 200, false, true);
    assert!(
        output.contains("Warning: Print stderr (1 occurrence, 31 info in binary targets)"),
        "{output}"
    );
}

#[test]
fn the_split_follows_the_delta_and_survives_a_partial_report() {
    let mut report = report();
    let base = diagnostic("clippy::dbg_macro", "maintainability", 1);
    report.diagnostics = vec![
        Diagnostic {
            id: "introduced".to_owned(),
            ..base.clone()
        },
        in_context(base.clone(), "introduced-test", DiagnosticContext::Tests),
        in_context(base, "pre-existing-test", DiagnosticContext::Tests),
    ];
    let introduced = vec!["introduced".to_owned(), "introduced-test".to_owned()];
    report.audit = Audit::build(1, 100, Status::Complete, &report.diagnostics[..2]);
    report.summary = Summary::from_diagnostics(&report.diagnostics);
    report.delta = Some(DeltaReport {
        fingerprint_version: 1,
        base_diagnostics: 1,
        current_diagnostics: 3,
        introduced,
        pre_existing: vec![DeltaMatch {
            current_id: "pre-existing-test".to_owned(),
            baseline_id: "pre-existing-test".to_owned(),
        }],
        fixed: Vec::new(),
        summary: DeltaSummary {
            introduced: 2,
            pre_existing: 1,
            fixed: 0,
            cross_file_matches: 0,
            rule_count_matches: 0,
        },
    });
    let output = rendered(&report, 200, false, false);
    assert!(
        output.lines().any(|row| row
            == "Maintainability: 0 errors, 1 warnings, 0 info, 0 unknown (occurrences) (+1 \
                outside production code)"),
        "{output}"
    );

    let mut partial = report.clone();
    partial.delta = None;
    partial.status = Status::Incomplete;
    partial.complete = false;
    partial.audit = Audit::build(1, 100, Status::Incomplete, &partial.diagnostics);
    let output = rendered(&partial, 200, false, false);
    assert!(
        output.lines().any(|row| row
            == "Maintainability: 0 errors, 1 warnings, 0 info, 0 unknown (occurrences) (+2 \
                outside production code)"),
        "{output}"
    );
}
