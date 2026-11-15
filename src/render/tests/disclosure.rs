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
            "\n\nLeft out of what to fix first: clippy::exit (86% noise on 5 sites). The corpus \
             found it wrong on most of the sites it reviewed.\n"
        ),
        "the distrusted rules are a paragraph of their own: {output}"
    );
    assert!(
        output.contains("Reported, but found wrong on most reviewed sites\nP2 warning  Exit  x1"),
        "{output}"
    );
    assert!(!output.contains("nothing worth"), "{output}");
}

#[test]
fn no_withheld_rule_prints_no_withheld_sentence() {
    let report = report();
    assert!(report.audit.score.as_ref().unwrap().withheld_rule_ids.is_empty());
    let output = rendered(&report, 200, false, false);
    assert!(!output.contains("Left out of what to fix first"), "{output}");
    assert!(!output.contains(crate::presentation::WITHHELD_HEADING), "{output}");
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
            "Scored 1 production files (100 lines) in 1.2s. Clippy did not run. Native rules \
             also read tests."
        ),
        "{output}"
    );
    report.scan.command = Some(vec!["cargo".to_owned(), "clippy".to_owned()]);
    let output = rendered(&report, 200, false, false);
    assert!(
        output.contains(
            "Scored 1 production files (100 lines) in 1.2s. Clippy linted library and binary \
             targets; code gated to other platforms and test code were not linted. Native rules \
             also read tests."
        ),
        "a probe without a host line omits the triple and keeps the rest: {output}"
    );
    report.toolchain.host = Some("x86_64-unknown-linux-gnu".to_owned());
    let output = rendered(&report, 200, false, false).replace('\n', " ");
    assert!(
        output.contains(
            "Clippy linted library and binary targets for x86_64-unknown-linux-gnu; code gated \
             to other platforms and test code were not linted. Native rules also read tests."
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
            == "Maintainability: 1 warning (1 from Dbg macro; +4 in tests, +1 in build scripts)"),
        "{output}"
    );

    let output = rendered(&super::report(), 200, false, false);
    assert!(
        output.lines().any(|row| row == "Reliability: 1 warning (1 from Todo)"),
        "{output}"
    );
    assert!(!output.contains("Bugs"), "{output}");
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
    assert!(output.contains("P3 warning  Print stderr  x1 (+31 info)"), "{output}");
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
        output
            .lines()
            .any(|row| row == "Maintainability: 1 warning (1 from Dbg macro; +1 in tests)"),
        "{output}"
    );

    let mut partial = report.clone();
    partial.delta = None;
    partial.status = Status::Incomplete;
    partial.complete = false;
    partial.audit = Audit::build(1, 100, Status::Incomplete, &partial.diagnostics);
    let output = rendered(&partial, 200, false, false);
    assert!(
        output
            .lines()
            .any(|row| row == "Maintainability: 1 warning (1 from Dbg macro; +2 in tests)"),
        "{output}"
    );
}

#[test]
fn a_dimension_ceiling_is_named_only_where_it_binds_and_the_driver_otherwise() {
    let credential = |count: usize| -> InspectReport {
        let mut report = report();
        report.diagnostics = (0..count)
            .map(|index| Diagnostic {
                id: format!("credential-{index}"),
                ..diagnostic("rust_doctor::repo::hardcoded_credential", "security", 1)
            })
            .collect();
        rebuilt(report)
    };
    for count in [1, 3] {
        let report = credential(count);
        let score = report.audit.score.as_ref().unwrap();
        let security = score
            .ceilings
            .iter()
            .find(|ceiling| ceiling.dimension == "security")
            .unwrap();
        assert_eq!(security.ceiling, 50);
        let output = rendered(&report, 300, false, false);
        let value = score.dimensions.security;
        let findings = if count == 1 { "finding" } else { "findings" };
        if value == security.ceiling {
            assert!(
                output.contains(
                    "Security is capped at 50 by a P1 finding: rust_doctor::repo::hardcoded_credential"
                ),
                "{output}"
            );
        } else {
            assert!(value < security.ceiling);
            assert!(
                output.contains(&format!(
                    "Security {value}: driven by rust_doctor::repo::hardcoded_credential ({count} \
                     {findings})"
                )),
                "{output}"
            );
            assert!(!output.contains("Security is capped"), "{output}");
        }
        assert!(!output.contains("ceiling:"), "{output}");
    }
    let below = credential(3);
    assert!(
        below.audit.score.as_ref().unwrap().dimensions.security < 50,
        "three findings hold Security under its ceiling, so the driver line is exercised"
    );
}

#[test]
fn a_rule_spread_across_many_files_is_planned_as_one_change() {
    let mut report = report();
    report.diagnostics = (0..40)
        .map(|index| Diagnostic {
            id: format!("sweep-{index}"),
            path: Some(format!("src/module_{index}.rs")),
            span: None,
            ..diagnostic("clippy::todo", "correctness", 2)
        })
        .collect();
    let output = rendered(&rebuilt(report), 300, false, false);
    assert!(
        output.contains(
            "Spread across 40 files: clippy::todo (80 findings). Plan it as one change, not 80 fixes."
        ),
        "{output}"
    );
    assert!(!output.contains("Migration advisory"), "{output}");
}

#[test]
fn a_workspace_scored_below_every_member_says_why() {
    use crate::audit::SourceFileInventory;
    let inventory = SourceFileInventory {
        files: 1,
        production_lines: 50,
        complete: true,
    };
    let scored = |diagnostics: Vec<Diagnostic>| -> String {
        let mut report = report();
        report.audit = Audit::build_from_inventory(
            inventory,
            vec![("alpha".to_owned(), inventory), ("beta".to_owned(), inventory)],
            &std::collections::BTreeSet::new(),
            Status::Complete,
            std::collections::BTreeSet::new(),
            &diagnostics,
        );
        report.summary = Summary::from_diagnostics(&diagnostics);
        report.diagnostics = diagnostics;
        rendered(&report, 300, false, false)
    };
    let unclaimed = Diagnostic {
        package: None,
        ..diagnostic("rust_doctor::repo::unignored_build_output", "maintainability", 1)
    };
    let claimed = Diagnostic {
        package: Some("alpha".to_owned()),
        ..diagnostic("clippy::todo", "correctness", 1)
    };
    let line = "Findings in CI files and the lockfile weigh on the workspace score only.";
    assert!(scored(vec![unclaimed]).contains(line));
    assert!(!scored(vec![claimed]).contains(line));
}
