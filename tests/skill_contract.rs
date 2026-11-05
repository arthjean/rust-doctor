#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rust_doctor::{catalog, score_block};

fn skill_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/rust-doctor")
}

fn skill_document(name: &str) -> String {
    fs::read_to_string(skill_root().join(name)).unwrap()
}

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

fn bare(token: &str) -> &str {
    token.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric()
            && character != '-'
            && character != '_'
            && character != ':'
    })
}

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
    let accepted: String = [
        &[][..],
        &["rules", "list"],
        &["rules", "explain"],
        &["why"],
        &["skill", "install"],
        &["setup"],
        &["hook", "install", "git"],
        &["hook", "install", "claude"],
        &["hook", "install", "cursor"],
        &["hook", "run"],
        &["ci", "install"],
        &["report", "markdown"],
    ]
    .iter()
    .map(|command| help(command))
    .collect();
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
fn the_skill_names_the_subcommands_that_explain_a_rule_and_a_site() {
    let skill = skill_document("SKILL.md");
    for command in ["rust-doctor rules explain", "rust-doctor why", "rust-doctor setup"] {
        assert!(skill.contains(command), "SKILL.md omits `{command}`");
    }
    assert!(help(&[]).contains("why"), "`--help` omits `why`");
    assert!(help(&["rules"]).contains("explain"), "`rules --help` omits `explain`");
}

#[test]
fn the_skill_carries_the_exit_status_table_help_prints() {
    let skill = skill_document("SKILL.md");
    let help = help(&[]);
    let table = help.split("Exit status:\n").nth(1).unwrap_or_default();
    let mut rows = 0;
    for line in table.lines().take_while(|line| line.starts_with("  ")) {
        let (code, meaning) = line.trim().split_once(char::is_whitespace).unwrap_or_default();
        let row = format!("| `{code}` | {} |", meaning.trim());
        assert!(skill.contains(&row), "SKILL.md lacks the row {row}");
        rows += 1;
    }
    assert_eq!(rows, 5, "{help}");
    assert!(skill.contains("The precedence is 2, 1, the warning code, then 0."));
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

fn bands_sentence() -> String {
    let mut bands: Vec<(u8, u8, &'static str)> = Vec::new();
    for value in 0..=100u8 {
        let label = score_block::label_for(value).as_str();
        match bands.last_mut() {
            Some(band) if band.2 == label => band.1 = value,
            _ => bands.push((value, value, label)),
        }
    }
    bands.reverse();
    let last = bands.len().saturating_sub(1);
    let mut previous: Option<u8> = None;
    let mut parts = Vec::new();
    for (index, (low, high, label)) in bands.iter().enumerate() {
        parts.push(if index == 0 {
            format!("{low} and above reads `{label}`")
        } else if index == last {
            format!("below {} `{label}`", previous.unwrap_or(*low))
        } else {
            format!("{low} to {high} `{label}`")
        });
        previous = Some(*low);
    }
    parts.join(", ")
}

#[test]
fn the_skill_names_the_score_model_the_binary_computes() {
    let shipped = rust_doctor::SCORE_MODEL;
    assert!(
        skill_document("SKILL.md").contains(shipped),
        "SKILL.md describes the score without naming `{shipped}`, the model it is computed under"
    );
    for (path, text) in skill_documents() {
        for named in text
            .split_whitespace()
            .map(bare)
            .filter(|token| token.starts_with("core-v"))
        {
            assert_eq!(
                named,
                shipped,
                "{} names `{named}`, a model this binary no longer computes",
                path.display()
            );
        }
    }
}

#[test]
fn the_bands_the_skill_states_are_the_bands_the_reports_label_by() {
    let stated = bands_sentence();
    let text = skill_document("SKILL.md")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.contains(&stated),
        "SKILL.md should state the bands as `{stated}`"
    );
}

#[test]
fn every_command_that_may_compile_carries_the_trust_warning() {
    let warning = "Cargo may execute build.rs files and procedural macros. Inspect trusted local repositories only.";
    for command in [&[][..], &["inspect"], &["why"], &["hook", "run"]] {
        assert!(help(command).contains(warning), "`{command:?} --help` omits the trust warning");
    }
    for command in [&["hook", "run"][..], &["hook", "install", "claude"], &["hook", "install", "cursor"]] {
        assert!(help(command).contains("--no-clippy"), "`{command:?} --help` omits --no-clippy");
    }
}
