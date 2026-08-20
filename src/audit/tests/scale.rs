//! What the score does when the workspace changes size.
//!
//! core-v3 reads a rate, so every claim here is a pair: the same findings over a different number
//! of lines, or a different number of findings over the same lines. The tests that do not depend
//! on the denominator stay in the parent file.

use super::*;

/// More sites of one rule never raise a dimension, and no count is dense enough to panic.
///
/// core-v2 saturated at four occurrences, so a rule firing a thousand times cost exactly what it
/// cost at twenty. core-v3 has no such step: the density keeps growing and the dimension keeps
/// falling, until the exponential has nowhere left to fall to.
#[test]
fn more_sites_of_one_rule_never_raise_its_dimension() {
    let mut previous = 101;
    for sites in [1, 5, 20, 50, 1_000] {
        let rules = [(
            "clippy::indexing_slicing",
            "reliability",
            Severity::Warning,
            sites,
        )];
        let audit = Audit::build(100, 10_000, Status::Complete, &diagnostics_for(&rules))
            .score
            .expect("ten kilolines is a scorable workspace");
        assert!(
            audit.dimensions.reliability < previous,
            "{sites} sites scored {} against {previous}",
            audit.dimensions.reliability
        );
        previous = audit.dimensions.reliability;
    }
    assert_eq!(previous, 0, "a hundred sites per kiloline is a zero");
}

/// Duplicating a workspace, sites and lines together, moves the score by zero points.
///
/// This is the defect core-v3 exists to repair: the same code copied ten times is the same code,
/// and core-v2 scored the copy strictly worse because it counted findings and never divided.
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

/// Growing the workspace without growing the findings raises the score, which is the other half
/// of the same statement: the score reads a rate, not a total.
#[test]
fn the_same_findings_over_more_lines_score_better() {
    let profile = [(
        "clippy::indexing_slicing",
        "reliability",
        Severity::Warning,
        12,
    )];
    let dense = Audit::build(4, 5_000, Status::Complete, &diagnostics_for(&profile));
    let doubled = Audit::build(8, 10_000, Status::Complete, &diagnostics_for(&profile));
    let sparse = Audit::build(40, 50_000, Status::Complete, &diagnostics_for(&profile));

    let dense = dense.score.expect("five kilolines is scorable");
    let doubled = doubled.score.expect("ten kilolines is scorable");
    let sparse = sparse.score.expect("fifty kilolines is scorable");
    // Doubling the lines alone never lowers the score, which is the weak form; ten times over is
    // the same statement with room for the rounding to show.
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

/// Repairing nine sites in ten raises the score wherever there was anything to repair.
///
/// The ladder below walks the whole density range one step at a time; this is the single claim
/// the model is sold on, stated once and at the size a reader would actually repair.
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

/// Every tier publishes the ceiling it published before core-v3, and nothing else caps.
///
/// The penalty changed; the ceilings did not. This walks the four tiers through a real audit
/// rather than reading the two tables back, so a ceiling that stopped being applied fails here
/// even though both tables still hold the right numbers.
#[test]
fn every_tier_caps_at_the_value_it_capped_at_before() {
    /// One catalogued rule and the two ceilings its tier is expected to still impose.
    struct Case {
        code: &'static str,
        category: &'static str,
        tier: RuleTier,
        dimension_ceiling: Option<u8>,
        overall_ceiling: Option<u8>,
    }

    // One catalogued rule per tier, each in a dimension of its own, fired densely enough that the
    // curve alone would leave the dimension well above every ceiling.
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

/// A report at the ceiling the baseline comparison refuses past still scores, in range.
///
/// `DIAGNOSTIC_LIMIT` is the largest population any pass of this crate will carry, so it is the
/// size at which the numerator has to stay finite and the conversion out of the curve total. Both
/// dimensions here sit at fifty error-weighted sites per kiloline, and the two lambdas separate
/// them: maintainability decays to a flat zero, reliability to `100 · exp(-5)`, which rounds to
/// one. What is being proved is that the curve arrives somewhere rather than at a panic.
#[test]
fn a_report_at_the_diagnostic_limit_still_scores_in_range() {
    let sites = crate::delta::DIAGNOSTIC_LIMIT / 2;
    let rules = [
        ("clippy::indexing_slicing", "reliability", Severity::Error, sites),
        ("clippy::dbg_macro", "maintainability", Severity::Error, sites),
    ];
    let audit = Audit::build(10_000, 1_000_000, Status::Complete, &diagnostics_for(&rules));
    let score = audit.score.expect("a million lines is a scorable workspace");

    assert_eq!(score.dimensions.reliability, 1);
    assert_eq!(score.dimensions.maintainability, 0);
    assert!(score.value <= 100);
    assert_eq!(score.value, 62, "the three untouched dimensions carry it");
    assert_eq!(score.label, ScoreLabel::NeedsWork);
}

/// Repairing sites raises the score at every density the corpus spans.
#[test]
fn repairing_sites_raises_the_score_at_every_density() {
    // One `P3` rule and nothing else: a tier ceiling pins a dimension flat, so a repair under one
    // is worth nothing by design and would refute a monotonicity this test is not claiming.
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

/// A rule that no source file can dilute is charged per workspace, not per kiloline.
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

/// A rule's numerator recomputes from the published fields alone: one severity weight per site.
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
    let aggregation = aggregate_rules(10_000, &diagnostics);

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

/// An uncatalogued severity weighs nothing and costs the report its authoritative flag.
#[test]
fn a_finding_of_unknown_severity_scores_nothing() {
    let mut diagnostics = diagnostics_for(&[("clippy::todo", "correctness", Severity::Warning, 1)]);
    if let Some(unknown) = diagnostics.first_mut() {
        unknown.severity = Severity::Unknown;
    }
    let aggregation = aggregate_rules(10_000, &diagnostics);
    let scorable: Vec<_> = aggregation
        .rules
        .iter()
        .filter(|rule| rule.is_scorable())
        .collect();
    assert!(scorable.is_empty(), "an unknown severity weighs nothing");
}

/// A hundred and twenty lines are scored against the floor, not against their own twelve
/// hundredths of a kiloline.
///
/// This is the case the floor exists for. Three sites divided by the workspace's real size is a
/// density of twenty-five, which the curve reads as a flat zero and which would tell the author of
/// a small crate that three findings are a catastrophe. Divided by the floor it is 1.5, and the
/// dimension lands where the model puts it. The absolute value is provisional: `US-014` freezes the
/// lambda table against the measured corpus spread, and what is asserted here is the contrast.
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
