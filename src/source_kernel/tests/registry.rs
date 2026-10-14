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

/// The registered detector of a rule on one node kind, when it has exactly one.
fn only(rule: &str, kind: SyntaxKind) -> &'static Detector {
    let mut found = REGISTERED
        .iter()
        .copied()
        .filter(|detector| detector.definition.id == rule && detector.node == kind);
    let detector = found.next().expect("the rule registers a detector");
    assert!(found.next().is_none(), "{rule} registers more than one detector on {kind:?}");
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
    let (tls, shell) = (
        only(SOURCE_DISABLED_TLS.id, SyntaxKind::METHOD_CALL_EXPR),
        only(SOURCE_DYNAMIC_SHELL.id, SyntaxKind::METHOD_CALL_EXPR),
    );

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

/// A rule whose detector matches a crate outside the sysroot declares a gate
/// on that crate, and a gate names a crate one of its detectors matches: a
/// rule written for one crate's API never reads another version of it
/// unannounced, and no gate closes a rule over a crate it never reads.
#[test]
fn every_crate_a_detector_matches_is_gated_on_its_rule() {
    let normalized = |name: &str| name.replace('-', "_");
    for detector in REGISTERED {
        let gated: Vec<String> = detector
            .definition
            .requires
            .iter()
            .map(|gate| normalized(gate.krate))
            .collect();
        for krate in detector.crates {
            assert!(
                gated.contains(&normalized(krate)),
                "{} matches `{krate}` and declares no gate on it",
                detector.definition.id
            );
        }
    }
    for rule in crate::policy::CATALOG.iter().filter(|rule| !rule.requires.is_empty()) {
        for gate in rule.requires {
            assert!(
                REGISTERED.iter().any(|detector| detector.definition.id == rule.id
                    && detector.crates.iter().any(|krate| normalized(krate) == normalized(gate.krate))),
                "{} gates on `{}`, which none of its detectors matches",
                rule.id,
                gate.krate
            );
        }
    }
    // The crates a detector's own tables name, read from its source, are the
    // crates it declares: a table that gains a crate gains its gate.
    let sources = [
        include_str!("../detectors.rs"),
        include_str!("../detectors/blocking.rs"),
        include_str!("../detectors/shell.rs"),
        include_str!("../detectors/sql.rs"),
        include_str!("../detectors/http.rs"),
        include_str!("../detectors/runtime.rs"),
        include_str!("../detectors/secrets.rs"),
        include_str!("../detectors/web.rs"),
    ];
    let declared: Vec<&str> = REGISTERED.iter().flat_map(|detector| detector.crates.iter().copied()).collect();
    for source in sources {
        for head in source.split("krate: \"").skip(1).chain(source.split("(&[\"").skip(1)).chain(source.split("(\"").skip(1)) {
            let name = head.split('"').next().unwrap_or_default();
            let tabled = head.split('"').nth(1).is_some_and(|rest| rest.starts_with(", "));
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c == '_') || !tabled {
                continue;
            }
            if ["std", "core", "alloc"].contains(&name) || !KNOWN_LIBRARY_CRATES.contains(&name) {
                continue;
            }
            assert!(declared.contains(&name), "a detector table names `{name}`, which no detector declares");
        }
    }
}

/// Library crates a detector table can name. A table entry that is not one
/// of them, a method or a shell name, is not a crate.
const KNOWN_LIBRARY_CRATES: [&str; 13] = [
    "actix_cors",
    "cookie",
    "diesel",
    "futures",
    "jsonwebtoken",
    "postgres",
    "reqwest",
    "rusqlite",
    "sea_orm",
    "sqlx",
    "tokio",
    "tokio_postgres",
    "tower_http",
];
