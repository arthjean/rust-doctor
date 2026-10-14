//! The registry: one traversal whatever it holds, and each detector asked
//! about the nodes of its own kind and no other.
//!
//! A rule may register several detectors, one per node kind it reads, as the
//! blocking and SQL families do for the call and the method form. What a rule
//! is solicited on is therefore the sum of the kinds its detectors declared,
//! which is what these tests compute rather than assume.

use ra_ap_syntax::{AstNode, Edition, SourceFile, SyntaxKind};

use super::{REGISTERED, SILENT, SILENT_RULE, analyze, solicitations};
use crate::policy::{SOURCE_DISABLED_TLS, SOURCE_DYNAMIC_SHELL};
use crate::source_kernel::Detector;

fn nodes(source: &str, kind: Option<SyntaxKind>) -> usize {
    SourceFile::parse(source, Edition::Edition2024)
        .tree()
        .syntax()
        .descendants()
        .filter(|node| kind.is_none_or(|kind| node.kind() == kind))
        .count()
}

/// How often `rule` is asked about `source` when `detectors` are registered:
/// once per node of each kind one of its detectors declared.
fn expected_solicitations(source: &str, detectors: &[&Detector], rule: &str) -> usize {
    detectors
        .iter()
        .filter(|detector| detector.definition.id == rule)
        .map(|detector| nodes(source, Some(detector.node)))
        .sum()
}

/// The registered detector of a rule that has exactly one.
fn only(rule: &str) -> &'static Detector {
    let mut found = REGISTERED.iter().copied().filter(|detector| detector.definition.id == rule);
    let detector = found.next().expect("the rule registers a detector");
    assert!(found.next().is_none(), "{rule} registers more than one detector");
    detector
}

/// The walk is single and independent of the registry: the number of
/// visited nodes depends neither on the number of detectors nor on their
/// order, and a silent detector does not change the result.
#[test]
fn the_registry_shares_one_traversal_and_stays_order_independent() {
    let source = "use std::process::Command;
fn run(user: &str) {
    let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\"));
    let _ = http_client::Client::builder().danger_accept_invalid_certs(true);
}";
    let all = nodes(source, None);
    let (tls, shell) = (only(SOURCE_DISABLED_TLS.id), only(SOURCE_DYNAMIC_SHELL.id));

    let (expected, counters) = analyze(source, &REGISTERED);
    assert_eq!(expected, [SOURCE_DISABLED_TLS.id, SOURCE_DYNAMIC_SHELL.id]);
    assert_eq!(counters.nodes_visited, all);
    for detector in REGISTERED {
        let rule = detector.definition.id;
        assert_eq!(
            solicitations(&counters, rule),
            expected_solicitations(source, &REGISTERED, rule),
            "{rule}"
        );
    }

    let permuted = analyze(source, &[shell, tls]);
    assert_eq!(permuted.0, expected);
    assert_eq!(permuted.1.nodes_visited, all);

    let single = analyze(source, &[tls]);
    assert_eq!(single.1.nodes_visited, all);
    assert_eq!(solicitations(&single.1, SOURCE_DYNAMIC_SHELL.id), 0);

    // A registered detector that emits nothing leaves the result unchanged.
    let with_silent = analyze(source, &[tls, &SILENT, shell]);
    assert_eq!(with_silent.0, expected);
    assert_eq!(with_silent.1.nodes_visited, all);
    assert_eq!(solicitations(&with_silent.1, SILENT_RULE.id), 1);
}

/// Grouping the registry by node kind may not change what a detector is
/// asked: a detector is solicited on every node of its kind and on no other,
/// whichever kinds its neighbours declared.
#[test]
fn a_detector_is_solicited_on_its_own_kind_and_never_on_another() {
    let source = "fn run(user: &str) { let _ = Command::new(\"sh\").arg(user); }";
    assert_eq!(nodes(source, Some(SyntaxKind::SOURCE_FILE)), 1);
    assert!(nodes(source, Some(SyntaxKind::CALL_EXPR)) > 0);
    assert!(nodes(source, Some(SyntaxKind::METHOD_CALL_EXPR)) > 0);

    let mut registered: Vec<&'static Detector> = vec![&SILENT];
    registered.extend(REGISTERED);
    let (_, counters) = analyze(source, &registered);
    assert_eq!(solicitations(&counters, SILENT_RULE.id), 1);
    for detector in REGISTERED {
        let rule = detector.definition.id;
        let expected = expected_solicitations(source, &registered, rule);
        assert!(expected > 0, "{rule}");
        assert_eq!(solicitations(&counters, rule), expected, "{rule}");
    }
}
