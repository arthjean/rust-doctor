use std::fs;
use std::path::PathBuf;

use super::super::advisory_check::{AdvisoryCheck, not_evaluated};
use super::super::sanitize::{HomePaths, shown_path};
use super::super::{NotEvaluated, PolicyReport, ReportError, Status};
use super::report_with_diagnostics;
use crate::cargo_health::{
    ADVISORY_RULES, AdvisoryDbReport, AdvisoryOutcome, AdvisorySource, UNREADABLE,
    UnreadableDatabase,
};
use crate::policy::PolicyPlan;
use crate::score_block::{UncheckedAdvisories, unchecked_advisories_line};
use crate::{Audit, GateReport, GateStatus, InspectReport};

fn scratch(label: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/report-advisory-check")
        .join(format!("{}-{label}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    crate::path_form::canonical(root).unwrap()
}

fn unreadable(path: PathBuf) -> AdvisoryOutcome {
    AdvisoryOutcome::Unreadable(UnreadableDatabase {
        path,
        files: 1489,
        skipped: 1489,
        message: crate::cargo_health::unreadable_sentence(None, 1489, 1489),
    })
}

fn with_advisory_rules(reason: Option<NotEvaluated>) -> InspectReport {
    let reasons: Vec<(&str, NotEvaluated)> = reason
        .into_iter()
        .flat_map(|reason| ADVISORY_RULES.iter().map(move |rule| (rule.id, reason)))
        .collect();
    let mut report = report_with_diagnostics(Vec::new());
    report.policy = Some(PolicyReport::from_plan(&PolicyPlan::default(), &reasons));
    report
}

fn linear(report: &InspectReport) -> String {
    let mut written = Vec::new();
    crate::render::render_terminal(report, &mut written).unwrap();
    String::from_utf8(written)
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn markdown(report: &InspectReport) -> String {
    let wire = serde_json::to_value(report).unwrap();
    crate::render::markdown::render_markdown(&wire, 20).unwrap()
}

#[test]
fn a_database_is_shown_from_the_workspace_then_from_home_and_never_whole() {
    let root = scratch("shown");
    let workspace = root.join("workspace");
    let home = root.join("home");
    for directory in [
        workspace.join("vendored-db"),
        home.join(".cargo/advisory-dbs/advisory-db-3157b0e258782691"),
        root.join("elsewhere/dbs/rustsec"),
    ] {
        fs::create_dir_all(directory).unwrap();
    }
    let homes = HomePaths::from_path(Some(home.clone()));

    assert_eq!(shown_path(&workspace.join("vendored-db"), Some(&workspace), &homes), "vendored-db");
    assert_eq!(shown_path(&workspace, Some(&workspace), &homes), ".");
    assert_eq!(
        shown_path(
            &home.join(".cargo/advisory-dbs/advisory-db-3157b0e258782691"),
            Some(&workspace),
            &homes
        ),
        "~/.cargo/advisory-dbs/advisory-db-3157b0e258782691"
    );
    let external = shown_path(&root.join("elsewhere/dbs/rustsec"), Some(&workspace), &homes);
    assert_eq!(external, "<external>/dbs/rustsec");
    let no_home = shown_path(&home.join(".cargo"), None, &HomePaths::default());
    assert_eq!(no_home, "<external>/home/.cargo");
    for shown in [external, no_home] {
        assert!(!shown.contains(root.to_str().unwrap()), "{shown}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn only_an_outcome_the_wire_cannot_tell_apart_is_kept_beside_it() {
    let home = HomePaths::default();
    assert_eq!(
        AdvisoryCheck::of(Some(&AdvisoryOutcome::TurnedOff), None, &home),
        AdvisoryCheck::TurnedOff
    );
    let consulted = AdvisoryOutcome::Consulted(AdvisoryDbReport {
        source: AdvisorySource::Explicit,
        date: None,
        advisories: 1,
        skipped: 0,
    });
    for published in [None, Some(&AdvisoryOutcome::Missing), Some(&consulted)] {
        assert_eq!(AdvisoryCheck::of(published, None, &home), AdvisoryCheck::Published);
    }
    let root = scratch("kept");
    assert_eq!(
        AdvisoryCheck::of(Some(&unreadable(root.join("db"))), Some(&root), &home),
        AdvisoryCheck::Unreadable {
            database: "db".to_owned(),
            skipped: 1489,
            files: 1489,
        }
    );

    assert_eq!(not_evaluated(&AdvisoryOutcome::Missing), Some(NotEvaluated::AdvisoryDbMissing));
    assert_eq!(not_evaluated(&AdvisoryOutcome::TurnedOff), Some(NotEvaluated::AdvisoryDbMissing));
    assert_eq!(
        not_evaluated(&unreadable(root.join("db"))),
        Some(NotEvaluated::AdvisoryDbMissing),
        "schema 24 publishes no new reason: the error code carries the unreadable database"
    );
    assert_eq!(not_evaluated(&consulted), None);
    assert_eq!(not_evaluated(&AdvisoryOutcome::NotRequested), None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_renderer_says_under_the_dimensions_that_no_advisory_was_checked() {
    let missing = with_advisory_rules(Some(NotEvaluated::AdvisoryDbMissing));
    assert_eq!(missing.unchecked_advisories(), Some(UncheckedAdvisories::NoDatabase));
    let line = unchecked_advisories_line(UncheckedAdvisories::NoDatabase);
    let dimensions = " · Dependencies 100 ";
    assert!(linear(&missing).contains(&format!("{dimensions}{line}")), "{}", linear(&missing));
    assert!(
        linear(&missing).contains("Known-vulnerability check skipped: no RustSec database under ~/.cargo."),
        "{}",
        linear(&missing)
    );
    assert!(markdown(&missing).contains(&format!("Dependencies 100\n\n{line}\n")), "{}", markdown(&missing));

    let mut off = with_advisory_rules(Some(NotEvaluated::AdvisoryDbMissing));
    off.scan.advisory_check = AdvisoryCheck::TurnedOff;
    assert_eq!(off.unchecked_advisories(), Some(UncheckedAdvisories::TurnedOff));
    let flagged = unchecked_advisories_line(UncheckedAdvisories::TurnedOff);
    assert!(linear(&off).contains(&format!("{dimensions}{flagged}")), "{}", linear(&off));
    assert!(!linear(&off).contains("no RustSec database"), "{}", linear(&off));
    assert_eq!(
        serde_json::to_value(&off).unwrap(),
        serde_json::to_value(&missing).unwrap(),
        "the flag is said on the terminal and adds no JSON field"
    );

    let mut consulted = with_advisory_rules(None);
    consulted.scan.advisory_db = Some(AdvisoryDbReport {
        source: AdvisorySource::CargoDeny,
        date: Some("2999-01-01".to_owned()),
        advisories: 1489,
        skipped: 2,
    });
    assert_eq!(consulted.unchecked_advisories(), None);
    assert!(!linear(&consulted).contains("known-vulnerability"), "{}", linear(&consulted));
    assert!(linear(&consulted).contains("2 advisory files could not be read."), "{}", linear(&consulted));
    assert!(!markdown(&consulted).contains("known-vulnerability"), "{}", markdown(&consulted));
    assert_eq!(report_with_diagnostics(Vec::new()).unchecked_advisories(), None);
}

#[test]
fn the_linear_report_names_the_unreadable_database_and_the_wire_does_not() {
    let mut report = with_advisory_rules(Some(NotEvaluated::AdvisoryDbMissing));
    report.status = Status::Incomplete;
    report.complete = false;
    report.audit = Audit::build(1, 100, Status::Incomplete, &[]);
    report.gate = GateReport {
        blocking: report.gate.blocking,
        status: GateStatus::NotEvaluated,
        blocking_diagnostics: None,
    };
    report.errors = vec![ReportError {
        stage: "dependencies".to_owned(),
        code: UNREADABLE.to_owned(),
        message: crate::cargo_health::unreadable_sentence(None, 1489, 1489),
    }];
    report.scan.advisory_check = AdvisoryCheck::Unreadable {
        database: "~/.cargo/advisory-dbs/advisory-db-3157b0e258782691".to_owned(),
        skipped: 1489,
        files: 1489,
    };
    assert_eq!(report.unchecked_advisories(), None);

    let said = linear(&report);
    assert!(
        said.contains("Scan incomplete: The RustSec database at ~/.cargo/advisory-dbs/advisory-db-3157b0e258782691 could not be read: 1489 of 1489 files were skipped. The known-vulnerability check did not run. (dependencies/advisory-db-unreadable)"),
        "{said}"
    );
    assert!(!said.contains("Known-vulnerability check skipped"), "{said}");
    let wire = serde_json::to_string(&report).unwrap();
    assert!(!wire.contains("advisory-dbs"), "{wire}");
    assert!(
        wire.contains("The RustSec database could not be read: 1489 of 1489 files were skipped."),
        "{wire}"
    );
}
