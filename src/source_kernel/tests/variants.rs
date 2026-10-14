//! US-033: every native source detector keeps its verdict under the rewrites
//! that do not change what the code does.
//!
//! A detector is written against the form its author had in mind, and the
//! first user writes another: the import renamed, the path spelled out, the
//! builder stored in a `let`, the call moved into a module. Each rule declares
//! a canonical positive and a canonical negative next to its detector, and the
//! gate renders both under every rewrite below. The positive must still fire,
//! or the rewrite is listed in the rule's `known_misses` with its reason; the
//! negative must stay silent under all of them. A rule without a table fails.
//!
//! Everything here parses text and compiles nothing, so the gate touches
//! neither the disk nor the network.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ra_ap_syntax::Edition;

use super::synthetic_unit;
use crate::source_kernel::detectors::{
    Case, CrateAliases, DETECTORS, Detector, VARIANTS, Variants,
};
use crate::source_kernel::{Enumeration, Findings, Identity, LIMITS, Registry, analyze_unit};

/// The rewrites, by the name a `known_misses` entry cites.
const REWRITES: [&str; 8] = [
    "canonical",
    "rename-bindings",
    "alias-import",
    "fully-qualified",
    "wrap-in-block",
    "inline-module",
    "extract-receiver",
    "reformat",
];

/// The crates the canonical cases import from, each under its own name.
const CRATES: [&str; 2] = ["reqwest", "rusqlite"];

/// One case under one rewrite, as the text of a whole unit.
fn render(case: &Case, rewrite: &str) -> String {
    let short = case.import.rsplit("::").next().unwrap_or(case.import);
    let (import, item) = match rewrite {
        "alias-import" => (format!("use {} as Alias;", case.import), "Alias"),
        "fully-qualified" => (String::new(), case.import),
        _ => (format!("use {};", case.import), short),
    };
    let receiver = case.receiver.replace("{Item}", item);
    let (setup, chain) = (case.setup, case.chain);
    let statements = if rewrite == "extract-receiver" {
        format!("{setup}\nlet mut receiver = {receiver};\nlet _ = receiver{chain};")
    } else {
        format!("{setup}\nlet _ = {receiver}{chain};")
    };
    let body = if rewrite == "wrap-in-block" {
        format!("{{\n{statements}\n}}")
    } else {
        statements
    };
    let asynchronous = if case.asynchronous { "async " } else { "" };
    let function = format!(
        "{import}\n{asynchronous}fn case({}) {{\n{body}\n}}\n",
        case.parameters
    );
    let unit = if rewrite == "inline-module" {
        format!("mod inner {{\n{function}}}\n")
    } else {
        function
    };
    let unit = rename(&unit, if rewrite == "rename-bindings" { "renamed_" } else { "" });
    if rewrite == "reformat" {
        reformat(&unit)
    } else {
        unit
    }
}

/// Every `$name` becomes `{prefix}name`, in code and inside format strings
/// alike, which is what a rename does to a captured identifier.
fn rename(text: &str, prefix: &str) -> String {
    text.replace('$', prefix)
}

/// Breaks every argument list and every method chain over lines, with a
/// comment at each break, and leaves string literals alone.
fn reformat(text: &str) -> String {
    let mut out = String::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut previous = ' ';
    for character in text.chars() {
        if in_string {
            out.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            previous = character;
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                out.push(character);
            }
            '(' | ',' => {
                out.push(character);
                out.push_str(" /* variant */\n        ");
            }
            '.' if previous == ')' => out.push_str("\n    // variant\n    ."),
            _ => out.push(character),
        }
        if !character.is_whitespace() {
            previous = character;
        }
    }
    out
}

/// Does `rule` report anything in this unit? Only the rule's own detectors
/// are solicited, and the unit's dependencies are the crates the cases name.
fn fires(detectors: &[&'static Detector], rule: &str, source: &str) -> bool {
    let enumeration = Enumeration {
        units: BTreeMap::from([(
            Identity {
                path: PathBuf::from("/synthetic/lib.rs"),
                edition: Edition::Edition2024,
            },
            synthetic_unit(source),
        )]),
        tables: BTreeMap::from([(
            "synthetic".to_owned(),
            CRATES
                .iter()
                .map(|krate| ((*krate).to_owned(), (*krate).to_owned()))
                .collect::<CrateAliases>(),
        )]),
        ..Enumeration::default()
    };
    let registry = Registry::new(
        detectors
            .iter()
            .copied()
            .filter(|detector| detector.definition.id == rule),
    );
    let mut findings = Findings::default();
    for unit in enumeration.units.values() {
        analyze_unit(unit, &enumeration, &registry, LIMITS, &mut findings);
    }
    !findings.candidates.is_empty()
}

/// Every verdict the tables break, one line each.
fn gate(tables: &[&Variants], detectors: &[&'static Detector]) -> Vec<String> {
    let mut failures = Vec::new();
    let rules: BTreeSet<&str> = detectors.iter().map(|detector| detector.definition.id).collect();
    for rule in &rules {
        if !tables.iter().any(|table| table.rule == *rule) {
            failures.push(format!("{rule} declares no variant table"));
        }
    }
    for table in tables {
        for (rewrite, reason) in table.known_misses {
            if !REWRITES.contains(rewrite) || reason.trim().is_empty() {
                failures.push(format!("{}: known miss {rewrite:?} names no rewrite or no reason", table.rule));
            }
        }
        for rewrite in REWRITES {
            let known = table.known_misses.iter().any(|(missed, _)| *missed == rewrite);
            let positive = render(&table.positive, rewrite);
            match (fires(detectors, table.rule, &positive), known) {
                (false, false) => failures.push(format!("{}: the positive goes silent under {rewrite}:\n{positive}", table.rule)),
                (true, true) => failures.push(format!("{}: {rewrite} is listed as a miss and fires", table.rule)),
                _ => {}
            }
            let negative = render(&table.negative, rewrite);
            if fires(detectors, table.rule, &negative) {
                failures.push(format!("{}: the negative fires under {rewrite}:\n{negative}", table.rule));
            }
        }
    }
    failures
}

#[test]
fn every_native_detector_keeps_its_verdict_under_every_rewrite() {
    let started = Instant::now();
    let failures = gate(&VARIANTS, &DETECTORS);
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the gate took {:?}",
        started.elapsed()
    );
}

#[test]
fn the_gate_refuses_a_missing_table_a_silent_positive_and_a_stale_miss() {
    // A registry whose rule declares nothing.
    let missing = gate(&VARIANTS[1..], &DETECTORS);
    assert!(
        missing.iter().any(|failure| failure.ends_with("declares no variant table")),
        "{missing:?}"
    );

    // A positive that cannot fire, with no miss declared for it.
    let silent = Variants {
        rule: VARIANTS[1].rule,
        positive: Case {
            chain: ".danger_accept_invalid_certs(false)",
            ..copy(&VARIANTS[1].negative)
        },
        negative: copy(&VARIANTS[1].negative),
        known_misses: &[],
    };
    assert!(gate(&[&silent], &DETECTORS[2..3]).iter().any(|failure| failure.contains("goes silent")));

    // A miss declared for a rewrite that fires.
    let stale = Variants {
        rule: VARIANTS[1].rule,
        positive: copy(&VARIANTS[1].positive),
        negative: copy(&VARIANTS[1].negative),
        known_misses: &[("reformat", "said to be missed")],
    };
    assert!(gate(&[&stale], &DETECTORS[2..3]).iter().any(|failure| failure.contains("listed as a miss")));
}

#[test]
fn every_rewrite_renders_a_unit_that_parses() {
    for table in VARIANTS {
        for rewrite in REWRITES {
            for case in [&table.positive, &table.negative] {
                let source = render(case, rewrite);
                let parse = ra_ap_syntax::SourceFile::parse(&source, Edition::Edition2024);
                assert!(parse.errors().is_empty(), "{rewrite} broke the syntax:\n{source}");
            }
        }
    }
}

fn copy(case: &Case) -> Case {
    Case {
        asynchronous: case.asynchronous,
        import: case.import,
        parameters: case.parameters,
        setup: case.setup,
        receiver: case.receiver,
        chain: case.chain,
    }
}
