//! US-033: every native source detector keeps its verdict under the rewrites
//! that do not change what the code does, and on the real forms of its defect.
//!
//! A detector is written against the form its author had in mind, and the
//! first user writes another: the import renamed, the path spelled out, the
//! builder stored in a `let`, the call moved into a helper. Each rule declares
//! a canonical positive and a canonical negative next to its detector, and the
//! gate renders both under every rewrite below. The positive must still fire,
//! or the rewrite is listed in the rule's `known_misses` with its reason; the
//! negative must stay silent under all of them. `numeric-parameter` turns the
//! positive into a negative, for the rules a parameter's text injects into.
//! A rule without a table fails.
//!
//! The rewrites only move one form around. The semantic cases are the other
//! forms of the defect, written whole: each must fire, or stay silent with the
//! carve-out that says why, and a carved case that fires is a stale carve-out.
//! The advisory rewrites are forms a rule may legitimately not chase, a macro's
//! token tree or an import through the crate's own modules: what they do is
//! printed, never failed on.
//!
//! Everything here parses text and compiles nothing, so the gate touches
//! neither the disk nor the network.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ra_ap_syntax::Edition;

use super::synthetic_unit;
use crate::source_kernel::detectors::{
    Case, CrateAliases, DETECTORS, Detector, Semantic, VARIANTS, Variants,
};
use crate::source_kernel::{Enumeration, Findings, Identity, LIMITS, Registry, analyze_unit};

/// The rewrites held to a verdict, by the name a `known_misses` entry cites.
/// Each keeps the positive's verdict except `numeric-parameter`, which is
/// negative-only.
const REWRITES: [&str; 12] = [
    "canonical",
    "rename-bindings",
    "alias-import",
    "fully-qualified",
    "fully-qualified-under-glob",
    "wrap-in-block",
    "inline-module",
    "extract-receiver",
    "through-helper",
    "api-equivalent",
    "reformat",
    NUMERIC_PARAMETER,
];

/// Every parameter typed `i64`: the positive of a rule whose sink a
/// parameter's text reaches has to go silent.
const NUMERIC_PARAMETER: &str = "numeric-parameter";

/// Forms a rule may legitimately not chase. Their positive is reported, never
/// failed on; their negative is still held silent.
const ADVISORY: [&str; 2] = ["inside-macro", "crate-relative-import"];

/// The crates the cases import from, each under its own name.
const CRATES: [&str; 7] = ["diesel", "futures", "reqwest", "rusqlite", "sea_orm", "sqlx", "tokio"];

/// One case under one rewrite, as the text of a whole unit.
fn render(case: &Case, rewrite: &str) -> String {
    let short = case.import.rsplit("::").next().unwrap_or(case.import);
    let (import, item) = match rewrite {
        "alias-import" => (format!("use {} as Alias;", case.import), "Alias"),
        "fully-qualified" => (String::new(), case.import),
        "fully-qualified-under-glob" => ("use futures::prelude::*;".to_owned(), case.import),
        "crate-relative-import" => (
            format!("mod deps {{ pub use {}; }}\nuse crate::deps::{short};", case.import),
            short,
        ),
        _ => (format!("use {};", case.import), short),
    };
    let receiver = case.receiver.replace("{Item}", item);
    let chain = if rewrite == "api-equivalent" {
        merged_arguments(case.chain)
    } else {
        case.chain.to_owned()
    };
    let setup = case.setup;
    let statements = match rewrite {
        "extract-receiver" => format!("{setup}\nlet mut receiver = {receiver};\nlet _ = receiver{chain};"),
        "inside-macro" => format!("{setup}\nlet _ = vec![{receiver}{chain}];"),
        _ => format!("{setup}\nlet _ = {receiver}{chain};"),
    };
    let body = if rewrite == "wrap-in-block" {
        format!("{{\n{statements}\n}}")
    } else {
        statements
    };
    let asynchronous = if case.asynchronous { "async " } else { "" };
    let parameters = if rewrite == NUMERIC_PARAMETER {
        numeric(case.parameters)
    } else {
        case.parameters.to_owned()
    };
    let function = if rewrite == "through-helper" {
        let arguments = names(&parameters);
        format!(
            "{import}\nfn helper({parameters}) {{\n{body}\n}}\n{asynchronous}fn case({parameters}) {{\nhelper({arguments});\n}}\n"
        )
    } else {
        format!("{import}\n{asynchronous}fn case({parameters}) {{\n{body}\n}}\n")
    };
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

/// The chain with every run of `.arg(a).arg(b)` written `.args([a, b])`, the
/// other spelling `Command` offers for the same argv.
fn merged_arguments(chain: &str) -> String {
    let mut calls: Vec<(String, String)> = Vec::new();
    let mut rest = chain;
    while let Some(after) = rest.strip_prefix('.') {
        let Some(open) = after.find('(') else {
            return chain.to_owned();
        };
        let (name, tail) = after.split_at(open);
        let mut depth = 0;
        let mut in_string = false;
        let mut close = None;
        for (index, character) in tail.char_indices() {
            match character {
                '"' if !tail[..index].ends_with('\\') => in_string = !in_string,
                '(' if !in_string => depth += 1,
                ')' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            return chain.to_owned();
        };
        calls.push((name.to_owned(), tail[1..close].to_owned()));
        rest = &tail[close + 1..];
    }
    let mut out = String::new();
    let mut run: Vec<String> = Vec::new();
    for (name, argument) in calls {
        if name == "arg" {
            run.push(argument);
            continue;
        }
        if !run.is_empty() {
            out.push_str(&format!(".args([{}])", run.join(", ")));
            run.clear();
        }
        out.push_str(&format!(".{name}({argument})"));
    }
    if !run.is_empty() {
        out.push_str(&format!(".args([{}])", run.join(", ")));
    }
    out + rest
}

/// The parameter list with every type written `i64`.
fn numeric(parameters: &str) -> String {
    parameters
        .split(',')
        .filter_map(|parameter| parameter.split(':').next())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| format!("{name}: i64"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The names a parameter list binds, as the arguments of a call.
fn names(parameters: &str) -> String {
    parameters
        .split(',')
        .filter_map(|parameter| parameter.split(':').next())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
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

/// What the tables say: every verdict they break, one line each, and what the
/// advisory forms did.
#[derive(Default)]
struct Gate {
    failures: Vec<String>,
    advisories: Vec<String>,
}

fn gate(tables: &[&Variants], detectors: &[&'static Detector]) -> Gate {
    let mut failures = Vec::new();
    let mut advisories = Vec::new();
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
        rewrites(table, detectors, &mut failures);
        for rewrite in ADVISORY {
            let fired = fires(detectors, table.rule, &render(&table.positive, rewrite));
            advisories.push(format!(
                "{}: the positive {} under {rewrite}",
                table.rule,
                if fired { "fires" } else { "is silent" }
            ));
            negative_holds(table, detectors, rewrite, &mut failures);
        }
        semantic(table, detectors, &mut failures);
    }
    Gate { failures, advisories }
}

/// Holds the positive and the negative of `table` under every rewrite that
/// must keep the verdict.
fn rewrites(table: &Variants, detectors: &[&'static Detector], failures: &mut Vec<String>) {
    for rewrite in REWRITES {
        let known = table.known_misses.iter().any(|(missed, _)| *missed == rewrite);
        let positive = render(&table.positive, rewrite);
        let fired = fires(detectors, table.rule, &positive);
        let broken = if rewrite == NUMERIC_PARAMETER {
            match (fired && table.interpolates_parameters, known) {
                (true, false) => Some(format!("the positive still fires under {rewrite}:\n{positive}")),
                (false, true) => Some(format!("{rewrite} is listed as a miss and stays silent")),
                _ => None,
            }
        } else {
            match (fired, known) {
                (false, false) => Some(format!("the positive goes silent under {rewrite}:\n{positive}")),
                (true, true) => Some(format!("{rewrite} is listed as a miss and fires")),
                _ => None,
            }
        };
        failures.extend(broken.map(|broken| format!("{}: {broken}", table.rule)));
        negative_holds(table, detectors, rewrite, failures);
    }
}

/// The negative of `table` stays silent under `rewrite`.
fn negative_holds(table: &Variants, detectors: &[&'static Detector], rewrite: &str, failures: &mut Vec<String>) {
    let negative = render(&table.negative, rewrite);
    if fires(detectors, table.rule, &negative) {
        failures.push(format!("{}: the negative fires under {rewrite}:\n{negative}", table.rule));
    }
}

/// Holds every semantic case of `table` to the verdict it declares.
fn semantic(table: &Variants, detectors: &[&'static Detector], failures: &mut Vec<String>) {
    for case in table.semantic {
        let fired = fires(detectors, table.rule, case.source);
        let carved = !case.carve_out.trim().is_empty();
        let description = case.description;
        match (case.fires, carved, fired) {
            (true, true, _) => failures.push(format!("{}: {description:?} fires and carries a carve-out", table.rule)),
            (true, false, false) => failures.push(format!("{}: {description:?} should fire and is silent:\n{}", table.rule, case.source)),
            (false, false, _) => failures.push(format!("{}: {description:?} stays silent without a carve-out", table.rule)),
            (false, true, true) => failures.push(format!(
                "{}: {description:?} fires, so its carve-out is stale: {}",
                table.rule, case.carve_out
            )),
            _ => {}
        }
    }
}

#[test]
fn every_native_detector_keeps_its_verdict_under_every_rewrite() {
    let started = Instant::now();
    let outcome = gate(&VARIANTS, &DETECTORS);
    for advisory in &outcome.advisories {
        eprintln!("advisory: {advisory}");
    }
    assert!(outcome.failures.is_empty(), "{}", outcome.failures.join("\n\n"));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the gate took {:?}",
        started.elapsed()
    );
}

#[test]
fn the_gate_refuses_a_missing_table_a_silent_positive_and_a_stale_miss() {
    // A registry whose rule declares nothing.
    let missing = gate(&VARIANTS[1..], &DETECTORS).failures;
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
        interpolates_parameters: false,
        known_misses: VARIANTS[1].known_misses,
        semantic: &[],
    };
    assert!(gate(&[&silent], &DETECTORS[2..3]).failures.iter().any(|failure| failure.contains("goes silent")));

    // A miss declared for a rewrite that fires.
    let stale = Variants {
        rule: VARIANTS[1].rule,
        positive: copy(&VARIANTS[1].positive),
        negative: copy(&VARIANTS[1].negative),
        interpolates_parameters: false,
        known_misses: &[("reformat", "said to be missed")],
        semantic: &[],
    };
    assert!(gate(&[&stale], &DETECTORS[2..3]).failures.iter().any(|failure| failure.contains("listed as a miss")));
}

#[test]
fn the_gate_refuses_a_stale_carve_out_a_silent_case_and_a_carve_out_without_a_reason() {
    const FIRING: &str = "fn client() { let _ = reqwest::Client::builder().danger_accept_invalid_certs(true); }";
    const QUIET: &str = "fn client() { let _ = reqwest::Client::builder().danger_accept_invalid_certs(false); }";
    static CASES: [Semantic; 3] = [
        Semantic { description: "stale", source: FIRING, fires: false, carve_out: "said to be missed" },
        Semantic { description: "silent", source: QUIET, fires: true, carve_out: "" },
        Semantic { description: "unexplained", source: QUIET, fires: false, carve_out: " " },
    ];
    let table = Variants {
        rule: VARIANTS[1].rule,
        positive: copy(&VARIANTS[1].positive),
        negative: copy(&VARIANTS[1].negative),
        interpolates_parameters: false,
        known_misses: VARIANTS[1].known_misses,
        semantic: &CASES,
    };
    let failures = gate(&[&table], &DETECTORS[2..3]).failures;
    for expected in ["carve-out is stale", "should fire and is silent", "without a carve-out"] {
        assert!(failures.iter().any(|failure| failure.contains(expected)), "{expected}: {failures:?}");
    }
}

#[test]
fn the_api_equivalent_rewrite_merges_each_run_of_arg_calls() {
    assert_eq!(
        merged_arguments(".arg(\"-c\").arg(format!(\"echo {user}\"))"),
        ".args([\"-c\", format!(\"echo {user}\")])"
    );
    assert_eq!(
        merged_arguments(".arg(\"-c\").arg(\"echo (\").env(\"A\", \"B\").arg(user).status()"),
        ".args([\"-c\", \"echo (\"]).env(\"A\", \"B\").args([user]).status()"
    );
    assert_eq!(merged_arguments(".execute(&sql, ())"), ".execute(&sql, ())");
}

#[test]
fn every_rewrite_renders_a_unit_that_parses() {
    for table in VARIANTS {
        for rewrite in REWRITES.iter().chain(&ADVISORY) {
            for case in [&table.positive, &table.negative] {
                let source = render(case, rewrite);
                let parse = ra_ap_syntax::SourceFile::parse(&source, Edition::Edition2024);
                assert!(parse.errors().is_empty(), "{rewrite} broke the syntax:\n{source}");
            }
        }
        for case in table.semantic {
            let parse = ra_ap_syntax::SourceFile::parse(case.source, Edition::Edition2024);
            assert!(parse.errors().is_empty(), "{:?} does not parse:\n{}", case.description, case.source);
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
