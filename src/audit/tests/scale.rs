use super::*;

#[test]
fn more_sites_of_one_rule_never_raise_its_dimension() {
    let mut previous = 101;
    for sites in [1, 5, 20, 50, 1_000] {
        let rules = [("clippy::type_complexity", "maintainability", Severity::Warning, sites)];
        let audit = Audit::build(100, 10_000, Status::Complete, &diagnostics_for(&rules))
            .score
            .expect("ten kilolines is a scorable workspace");
        assert!(
            audit.dimensions.maintainability < previous,
            "{sites} sites scored {} against {previous}",
            audit.dimensions.maintainability
        );
        previous = audit.dimensions.maintainability;
    }
    assert_eq!(previous, 0, "a hundred sites per kiloline is a zero");
}

#[test]
fn the_score_is_invariant_to_duplicating_the_workspace() {
    let profile = [
        ("clippy::todo", "correctness", Severity::Warning, 12),
        ("clippy::dbg_macro", "maintainability", Severity::Warning, 3),
    ];
    let tenfold: Vec<_> = profile
        .iter()
        .map(|(code, category, severity, sites)| (*code, *category, *severity, sites * 10))
        .collect();

    let once = Audit::build(4, 5_000, Status::Complete, &diagnostics_for(&profile));
    let ten_times = Audit::build(40, 50_000, Status::Complete, &diagnostics_for(&tenfold));

    assert_eq!(
        once.score.map(|score| score.value),
        ten_times.score.map(|score| score.value),
    );
}

#[test]
fn the_same_findings_over_more_lines_score_better() {
    let profile = [("clippy::todo", "correctness", Severity::Warning, 12)];
    let dense = Audit::build(4, 5_000, Status::Complete, &diagnostics_for(&profile));
    let doubled = Audit::build(8, 10_000, Status::Complete, &diagnostics_for(&profile));
    let sparse = Audit::build(40, 50_000, Status::Complete, &diagnostics_for(&profile));

    let dense = dense.score.expect("five kilolines is scorable");
    let doubled = doubled.score.expect("ten kilolines is scorable");
    let sparse = sparse.score.expect("fifty kilolines is scorable");
    assert!(
        doubled.value >= dense.value,
        "{} should not fall below {}",
        doubled.value,
        dense.value
    );
    assert!(
        sparse.value > dense.value,
        "{} should beat {}",
        sparse.value,
        dense.value
    );
}

#[test]
fn repairing_nine_sites_in_ten_raises_the_score() {
    for lines in [2_000, 20_000, 200_000] {
        let files = lines / 100;
        let sites = lines / 200;
        let profile = |count: usize| {
            [
                ("clippy::dbg_macro", "maintainability", Severity::Warning, count),
                ("clippy::indexing_slicing", "reliability", Severity::Warning, count),
            ]
        };
        let before = Audit::build(files, lines, Status::Complete, &diagnostics_for(&profile(sites)));
        let after = Audit::build(
            files,
            lines,
            Status::Complete,
            &diagnostics_for(&profile(sites / 10)),
        );

        let before = before.score.expect("a scorable workspace");
        let after = after.score.expect("a scorable workspace");
        assert!(before.value < 100, "{lines} lines starts with findings");
        assert!(
            after.value > before.value,
            "{lines} lines: repairing {sites} sites down to {} scored {} against {}",
            sites / 10,
            after.value,
            before.value
        );
    }
}

#[test]
fn every_tier_caps_at_the_value_it_capped_at_before() {
    struct Case {
        code: &'static str,
        category: &'static str,
        tier: RuleTier,
        dimension_ceiling: Option<u8>,
        overall_ceiling: Option<u8>,
    }

    const CASES: [Case; 4] = [
        Case {
            code: "rust_doctor::source::dynamic_shell_command",
            category: "security",
            tier: RuleTier::P0,
            dimension_ceiling: Some(20),
            overall_ceiling: Some(40),
        },
        Case {
            code: "rust_doctor::cargo::path_dependency_outside_workspace",
            category: "dependencies",
            tier: RuleTier::P1,
            dimension_ceiling: Some(50),
            overall_ceiling: Some(65),
        },
        Case {
            code: "clippy::todo",
            category: "correctness",
            tier: RuleTier::P2,
            dimension_ceiling: Some(75),
            overall_ceiling: None,
        },
        Case {
            code: "clippy::dbg_macro",
            category: "maintainability",
            tier: RuleTier::P3,
            dimension_ceiling: None,
            overall_ceiling: None,
        },
    ];

    for Case {
        code,
        category,
        tier,
        dimension_ceiling,
        overall_ceiling,
    } in CASES
    {
        assert_eq!(
            tier_dimension_ceiling(tier),
            dimension_ceiling,
            "{tier:?} dimension ceiling"
        );
        assert_eq!(
            tier_overall_ceiling(tier),
            overall_ceiling,
            "{tier:?} overall ceiling"
        );

        let rules = [(code, category, Severity::Warning, 1)];
        let audit = Audit::build(40, 50_000, Status::Complete, &diagnostics_for(&rules));
        let score = audit.score.expect("fifty kilolines is scorable");
        assert_eq!(score.worst_tier, Some(tier), "{code}");
        assert_eq!(score.applied_ceiling, overall_ceiling, "{code}");
        if let Some(ceiling) = dimension_ceiling {
            let published = score.dimensions.values().into_iter().max().unwrap_or(0);
            assert!(published <= 100, "{code}");
            assert!(
                score.dimensions.values().into_iter().any(|value| value == ceiling),
                "{code} should pin one dimension at {ceiling}"
            );
        }
        if let Some(ceiling) = overall_ceiling {
            assert!(score.value <= ceiling, "{code} scored {}", score.value);
        }
    }
}

#[test]
fn a_report_at_the_diagnostic_limit_still_scores_in_range() {
    let sites = crate::delta::DIAGNOSTIC_LIMIT / 2;
    let rules = [
        ("clippy::todo", "correctness", Severity::Error, sites),
        ("clippy::dbg_macro", "maintainability", Severity::Error, sites),
    ];
    let audit = Audit::build(10_000, 1_000_000, Status::Complete, &diagnostics_for(&rules));
    let score = audit.score.expect("a million lines is a scorable workspace");

    assert_eq!(score.dimensions.reliability, 0);
    assert_eq!(score.dimensions.maintainability, 0);
    assert!(score.value <= 100);
    assert_eq!(score.value, 62, "the three untouched dimensions carry it");
    assert_eq!(score.label, ScoreLabel::NeedsWork);
}

#[test]
fn repairing_sites_raises_the_score_at_every_density() {
    for lines in [2_000, 20_000, 200_000] {
        let files = lines / 100;
        let mut previous = None;
        for divisor in [100, 200, 400, 1_000, 2_000] {
            let sites = lines / divisor;
            let rules = [(
                "clippy::dbg_macro",
                "maintainability",
                Severity::Warning,
                sites,
            )];
            let audit = Audit::build(files, lines, Status::Complete, &diagnostics_for(&rules));
            let score = audit.score.expect("a scorable workspace");
            if let Some(previous) = previous {
                assert!(
                    score.value > previous,
                    "{lines} lines, {sites} sites scored {} against {previous}",
                    score.value
                );
            }
            previous = Some(score.value);
        }
    }
}

#[test]
fn a_manifest_finding_is_charged_whatever_the_workspace_measures() {
    let rules = [(
        "rust_doctor::cargo::missing_lockfile",
        "dependencies",
        Severity::Error,
        1,
    )];
    let small = Audit::build(4, 5_000, Status::Complete, &diagnostics_for(&rules));
    let large = Audit::build(400, 500_000, Status::Complete, &diagnostics_for(&rules));

    let small = small.score.expect("five kilolines is scorable");
    let large = large.score.expect("five hundred kilolines is scorable");
    assert_eq!(
        small.dimensions.dependencies, large.dimensions.dependencies,
        "one missing lockfile is one missing lockfile",
    );
}

#[test]
fn a_rule_weighs_one_severity_per_site() {
    let rules = [
        ("clippy::todo", "correctness", Severity::Warning, 7),
        (
            "rust_doctor::source::dynamic_shell_command",
            "security",
            Severity::Error,
            2,
        ),
    ];
    let diagnostics = diagnostics_for(&rules);
    let aggregation = aggregate_rules(10_000, 1, &diagnostics);

    for (code, _, severity, sites) in rules {
        let aggregate = aggregation
            .rules
            .iter()
            .find(|rule| rule.id == code)
            .expect("the rule should be aggregated");
        let weight = density::severity_weight(severity).expect("both severities weigh");
        assert_eq!(aggregate.numerator, weight * sites as u64, "{code}");
        assert!(aggregate.is_scorable(), "{code}");
        assert!(aggregate.contribution() > 0, "{code} costs the score points");
    }
}

#[test]
fn a_finding_of_unknown_severity_scores_nothing() {
    let mut diagnostics = diagnostics_for(&[("clippy::todo", "correctness", Severity::Warning, 1)]);
    if let Some(unknown) = diagnostics.first_mut() {
        unknown.severity = Severity::Unknown;
    }
    let aggregation = aggregate_rules(10_000, 1, &diagnostics);
    let scorable: Vec<_> = aggregation
        .rules
        .iter()
        .filter(|rule| rule.is_scorable())
        .collect();
    assert!(scorable.is_empty(), "an unknown severity weighs nothing");
}

#[test]
fn a_hundred_and_twenty_line_crate_is_scored_against_the_floor() {
    let rules = [("clippy::dbg_macro", "maintainability", Severity::Warning, 3)];
    let audit = Audit::build(2, 120, Status::Complete, &diagnostics_for(&rules));
    let score = audit
        .score
        .expect("a hundred and twenty lines is a scorable workspace");

    assert_eq!(
        score.dimensions.maintainability,
        density::dimension_score(3.0 / density::KILOLINE_FLOOR, ScoreDimension::Maintainability),
        "the floor is the denominator a small workspace is charged against",
    );
    assert_eq!(score.dimensions.maintainability, 61);
    assert_eq!(
        density::dimension_score(3.0 / 0.12, ScoreDimension::Maintainability),
        0,
        "the same three sites against their own kilolines",
    );
}
