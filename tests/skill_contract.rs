#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! `skills/rust-doctor/` ships the commands an agent runs and the rule ids it
//! names. A flag the CLI does not accept is a skill that dies on its first
//! step, and a rule id the catalog does not carry is a fix recipe for a finding
//! nobody can trigger. Both are checked against the shipped binary rather than
//! against a copy of what it once accepted.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rust_doctor::catalog;

fn skill_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/rust-doctor")
}

fn skill_document(name: &str) -> String {
    fs::read_to_string(skill_root().join(name)).unwrap()
}

/// Every markdown file of the skill: `SKILL.md` and the references it discloses.
fn skill_documents() -> Vec<(PathBuf, String)> {
    let mut documents = Vec::new();
    let mut pending = vec![skill_root()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "md") {
                let text = fs::read_to_string(&path).unwrap();
                documents.push((path, text));
            }
        }
    }
    assert!(!documents.is_empty(), "the skill has no markdown to check");
    documents
}

/// A token as it sits in prose or in a table cell, stripped of the markdown and
/// punctuation around it. Dashes, underscores and colons are what a flag and a
/// rule id are made of, so they never bound the token.
fn bare(token: &str) -> &str {
    token.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric()
            && character != '-'
            && character != '_'
            && character != ':'
    })
}

/// A long flag, `--json`, and never a markdown rule or a frontmatter fence.
fn is_flag(token: &str) -> bool {
    token
        .strip_prefix("--")
        .is_some_and(|name| name.starts_with(|character: char| character.is_ascii_alphabetic()))
}

fn help(arguments: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .args(arguments)
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success(), "`{arguments:?} --help` failed");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn every_flag_the_skill_documents_is_a_flag_the_cli_accepts() {
    let accepted = format!("{}{}", help(&[]), help(&["rules", "list"]));
    for (path, text) in skill_documents() {
        for flag in text
            .split_whitespace()
            .map(bare)
            .filter(|token| is_flag(token))
        {
            assert!(
                accepted.contains(flag),
                "{} documents `{flag}`, which the CLI does not accept",
                path.display()
            );
        }
    }
}

#[test]
fn every_rule_the_skill_names_is_a_catalogued_rule() {
    let catalogued: Vec<&str> = catalog().iter().map(|entry| entry.id).collect();
    for (path, text) in skill_documents() {
        for id in text
            .split_whitespace()
            .map(bare)
            .filter(|token| token.starts_with("clippy::") || token.starts_with("rust_doctor::"))
        {
            assert!(
                catalogued.contains(&id),
                "{} names `{id}`, which the catalog does not publish",
                path.display()
            );
        }
    }
}

#[test]
fn the_skill_states_the_size_of_the_catalog_it_scans_with() {
    let stated = format!("{} curated rules", catalog().len());
    let text = skill_document("SKILL.md");
    assert!(
        text.contains(&stated),
        "SKILL.md should state `{stated}`, the catalog it ships beside"
    );
}
