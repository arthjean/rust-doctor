#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! The standing documents state numbers an agent acts on without checking: how
//! large the catalog is, which schema the report publishes, the bounds
//! `oversized_unit` reports at, how many tests hold the modules under them, and
//! how many workflows pin which toolchain. Each of those is recomputed here
//! from the binary that produces it, the way `tests/skill_contract.rs` does for
//! the shipped skill.
//!
//! `AGENTS.md` stated a schema version the binary had left behind, and a
//! rewrite that touched every line of the file carried the stale number through
//! untouched while a second document repeated it. Prose does not notice a
//! number going out of date; recomputing it does.
//!
//! The documents under contract are the `budgets` of `docs/doc-budgets.json`,
//! the list `scripts/verify-doc-budgets.sh` reads, so a document added to one
//! gate is added to both. `docs/doc-gates.md` holds the reasoning.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rust_doctor::{OVERSIZED_UNIT_FILE_LINES, SCHEMA_VERSION, catalog};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(relative: &str) -> String {
    fs::read_to_string(repo().join(relative))
        .unwrap_or_else(|_| unreachable!("{relative} should be readable"))
}

/// A document with its wrapping collapsed: where markdown breaks a sentence is
/// not something the binary decides.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A token as it sits in prose, stripped of the markdown around it. Underscores
/// and colons are what a test name and a rule id are made of, so they never
/// bound the token.
fn bare(token: &str) -> &str {
    token.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric() && character != '_' && character != ':'
    })
}

/// The standing documents, flattened, read from the manifest the budgets gate
/// reads.
fn standing_documents() -> Vec<(String, String)> {
    let manifest: serde_json::Value =
        serde_json::from_str(&read("docs/doc-budgets.json")).expect("the manifest should be JSON");
    let budgets = manifest
        .get("budgets")
        .and_then(serde_json::Value::as_object)
        .expect("the manifest should carry a `budgets` object");
    let documents: Vec<(String, String)> = budgets
        .keys()
        .map(|path| (path.clone(), flat(&read(path))))
        .collect();
    assert!(
        documents.len() > 1,
        "the manifest should budget more than one document"
    );
    documents
}

/// The standing documents and the README, which states the catalog size too.
fn documents_under_contract() -> Vec<(String, String)> {
    let mut documents = standing_documents();
    documents.push(("README.md".to_owned(), flat(&read("README.md"))));
    documents
}

/// Every integer written immediately before `phrase`.
fn numbers_before(text: &str, phrase: &str) -> Vec<u64> {
    text.match_indices(phrase)
        .filter_map(|(index, _)| {
            let reversed: String = text[..index]
                .chars()
                .rev()
                .take_while(char::is_ascii_digit)
                .collect();
            reversed.chars().rev().collect::<String>().parse().ok()
        })
        .collect()
}

/// Every integer written immediately after `phrase`.
fn numbers_after(text: &str, phrase: &str) -> Vec<u64> {
    text.match_indices(phrase)
        .filter_map(|(index, _)| {
            text[index + phrase.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .collect()
}

/// Every double-quoted value written immediately after `marker`.
fn quoted_after(text: &str, marker: &str) -> Vec<String> {
    text.match_indices(marker)
        .map(|(index, _)| {
            text[index + marker.len()..]
                .chars()
                .take_while(|character| *character != '"')
                .collect()
        })
        .collect()
}

/// Asserts that every number written before `phrase` is `expected`, and returns
/// how many documents said so, since a claim nobody makes is a gate that passes
/// on silence.
fn every_number_before(phrase: &str, expected: u64) -> usize {
    let mut stated = 0;
    for (path, text) in documents_under_contract() {
        for found in numbers_before(&text, phrase) {
            assert_eq!(
                found, expected,
                "{path} states `{found}{phrase}`, and the binary says {expected}"
            );
            stated += 1;
        }
    }
    stated
}

/// The number as the documents spell it, since the house style writes a small
/// count as a word.
fn spelled(value: usize) -> &'static str {
    const WORDS: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    WORDS
        .get(value)
        .copied()
        .expect("a count the documents spell out should be under twenty-one")
}

fn capitalized(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

/// The value of a `usize` constant as the source declares it, for a bound the
/// library does not publish.
fn declared_constant(relative: &str, name: &str) -> u64 {
    let source = read(relative);
    let marker = format!("{name}: usize = ");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| unreachable!("{relative} should declare `{name}`"))
        + marker.len();
    source[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '_')
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .expect("a line bound should be an integer")
}

fn rust_sources(directory: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries = fs::read_dir(&current).expect("a source directory should be readable");
        for entry in entries {
            let path = entry.expect("a directory entry should be readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }
    sources
}

/// Every test that holds a module under the bound its own rule reports at. The
/// name is part of the contract: the documents name the family as
/// `the_X_holds_the_size_bound`, and a test that spells it otherwise is one the
/// reader cannot find from the document that promised it.
fn size_bound_tests() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for path in rust_sources(&repo().join("src")) {
        let source = fs::read_to_string(&path).expect("a source file should be readable");
        for line in source.lines() {
            let Some(rest) = line.trim_start().strip_prefix("fn ") else {
                continue;
            };
            let name = rest.split('(').next().unwrap_or_default();
            if !name.contains("size_bound") {
                continue;
            }
            assert!(
                name.starts_with("the_") && name.contains("_holds_the_size_bound"),
                "{} declares `{name}`, and the documents name the family `the_X_holds_the_size_bound`",
                path.display()
            );
            names.insert(name.to_owned());
        }
    }
    names
}

#[test]
fn the_catalog_size_the_documents_state_is_the_catalog_the_binary_ships() {
    let rules = catalog().len() as u64;
    let stated: usize = [
        " rules across",
        " rules are declared",
        " catalogued rules",
        " curated rules",
        " declarations",
    ]
    .iter()
    .map(|phrase| every_number_before(phrase, rules))
    .sum();
    assert!(
        stated >= 3,
        "the documents should state the size of the catalog, and {stated} of them do"
    );
}

#[test]
fn the_producer_counts_the_map_states_are_the_prefixes_the_catalog_carries() {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in catalog() {
        let segments: Vec<&str> = entry.id.split("::").collect();
        let prefix = if let ["rust_doctor", producer, _] = segments.as_slice() {
            format!("rust_doctor::{producer}")
        } else {
            segments.first().copied().unwrap_or_default().to_owned()
        };
        *counts.entry(prefix).or_default() += 1;
    }

    let map = flat(&read("AGENTS.md"));
    let producers = format!("across {} producers", spelled(counts.len()));
    assert!(
        map.contains(&producers),
        "AGENTS.md should state the catalog spreads `{producers}`"
    );
    for (prefix, count) in &counts {
        let claim = format!("`{prefix}::*` ({count}");
        assert!(
            map.contains(&claim),
            "AGENTS.md should state `{claim}`, the rules the catalog carries under that prefix"
        );
    }
    let clippy = counts
        .get("clippy")
        .copied()
        .expect("the catalog should carry Clippy lints");
    every_number_before(" curated lints", clippy as u64);
}

#[test]
fn the_schema_version_the_documents_state_is_the_one_the_binary_publishes() {
    let published = u64::from(SCHEMA_VERSION);
    let mut stated = 0;
    for (path, text) in standing_documents() {
        for found in numbers_after(&text, "currently ") {
            assert_eq!(
                found, published,
                "{path} states the report schema as {found}, and the binary publishes {published}"
            );
            stated += 1;
        }
    }
    assert_eq!(
        stated, 1,
        "the schema version belongs in exactly one standing document, written `currently <n>`"
    );

    // The ledger of what each version added is what makes the frozen archive
    // projectable, so a bump that does not reach it is a bump nobody recorded.
    let home = "docs/subsystems/reporting.md";
    let ledger = flat(&read(home));
    assert!(
        ledger.contains(&format!("v{published} ")),
        "{home} should name what v{published} added to the report"
    );
    let next = published + 1;
    assert!(
        !ledger.contains(&format!("v{next} ")),
        "{home} names a v{next} the binary does not publish"
    );
}

#[test]
fn the_bounds_the_documents_state_are_the_bounds_the_structure_pass_reports_at() {
    let file_lines = OVERSIZED_UNIT_FILE_LINES as u64;
    let stated = every_number_before(" lines `oversized_unit` reports", file_lines);
    assert!(
        stated >= 2,
        "the documents should state the file bound `oversized_unit` reports at, and {stated} do"
    );

    let impl_lines = declared_constant("src/structure/hotspots.rs", "IMPL_LINES");
    let claim = format!("under the {impl_lines} it reports at");
    assert!(
        flat(&read("AGENTS.md")).contains(&claim),
        "AGENTS.md should hold every `impl` block `{claim}`"
    );
}

#[test]
fn the_size_bound_tests_the_documents_count_are_the_tests_the_source_carries() {
    let tests = size_bound_tests();
    let counted = spelled(tests.len());
    let family = "`the_X_holds_the_size_bound` tests";

    let map = format!("{} {family}", capitalized(counted));
    assert!(
        flat(&read("AGENTS.md")).contains(&map),
        "AGENTS.md should count `{map}`, one per module"
    );
    let home = format!("The {counted} {family}");
    assert!(
        flat(&read("docs/testing.md")).contains(&home),
        "docs/testing.md should count `{home}`"
    );

    for (path, text) in standing_documents() {
        for token in text.split_whitespace().map(bare) {
            if token.contains("size_bound") && token != "the_X_holds_the_size_bound" {
                assert!(
                    tests.contains(token),
                    "{path} names `{token}`, which no module of `src/` declares"
                );
            }
        }
    }
}

#[test]
fn the_workflows_the_map_counts_all_pin_the_toolchain_it_names() {
    let directory = repo().join(".github/workflows");
    let mut workflows: Vec<PathBuf> = fs::read_dir(&directory)
        .expect("the workflow directory should be readable")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "yml"))
        .collect();
    workflows.sort();

    let map = flat(&read("AGENTS.md"));
    let counted = format!("{} workflows settle", capitalized(spelled(workflows.len())));
    assert!(
        map.contains(&counted),
        "AGENTS.md should count `{counted}`, the workflows on disk"
    );

    let marker = "pins toolchain ";
    let start = map
        .find(marker)
        .expect("AGENTS.md should name the toolchain its workflows pin")
        + marker.len();
    let toolchain: String = map[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let toolchain = toolchain.trim_end_matches('.').to_owned();
    assert!(
        !toolchain.is_empty(),
        "AGENTS.md should name a toolchain version after `{marker}`"
    );
    assert!(
        flat(&read("docs/ci-workflows.md")).contains(&toolchain),
        "docs/ci-workflows.md should name the same toolchain {toolchain} the map does"
    );

    // The MSRV leg is the one pin that is deliberately not the toolchain: it is
    // the version the crate declares it still compiles on.
    let manifest = read("Cargo.toml");
    let msrv = quoted_after(&manifest, "rust-version = \"")
        .first()
        .cloned()
        .expect("Cargo.toml should declare a rust-version");
    assert!(
        map.contains(&format!("rustc {msrv} or later")),
        "AGENTS.md should state the crate needs `rustc {msrv} or later`"
    );

    for path in &workflows {
        let text = fs::read_to_string(path).expect("a workflow should be readable");
        let pinned = quoted_after(&text, "toolchain: \"");
        assert!(
            pinned.contains(&toolchain),
            "{} pins no toolchain {toolchain}, which the map says every workflow does",
            path.display()
        );
        for version in &pinned {
            assert!(
                *version == toolchain || *version == msrv,
                "{} pins toolchain {version}, which is neither {toolchain} nor the declared MSRV {msrv}",
                path.display()
            );
        }
    }
}
