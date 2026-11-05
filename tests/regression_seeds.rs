#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

use rust_doctor::seed_harness::{CaseUnit, run_detectors};
use serde_json::Value;

mod support;

const WEAKNESSES: [&str; 15] = [
    "allow-by-default-lint",
    "api-variant",
    "conditional-module",
    "doc-comment",
    "glob-import",
    "helper-call",
    "library-name",
    "macro-argument",
    "nested-item",
    "path-attribute",
    "placeholder-credential",
    "test-case",
    "test-support",
    "toolchain-compatibility",
    "typed-number",
];

const VERDICTS: [&str; 2] = ["fires", "quiet"];

const REQUIRED: [&str; 4] = ["rule", "weakness", "source", "verdict"];
const OPTIONAL: [&str; 2] = ["known-false-positive", "split"];

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn seeds_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/regressions")
}

#[derive(Debug)]
enum Form {
    Source(String),
    Crate,
}

#[derive(Debug)]
struct Seed {
    name: String,
    rule: String,
    fires: bool,
    split: Option<String>,
    form: Form,
}

fn headers(text: &str) -> Result<BTreeMap<String, String>, String> {
    let table: toml::Table = text
        .parse()
        .map_err(|error| format!("seed.toml does not parse: {error}"))?;
    table
        .into_iter()
        .map(|(key, value)| match value {
            toml::Value::String(text) => Ok((key, text)),
            _ => Err(format!("key `{key}` is not a string")),
        })
        .collect()
}

fn parse(name: &str, text: &str, rules: &BTreeSet<String>) -> Result<Seed, String> {
    let refuse = |reason: String| format!("regressions/{name}: {reason}");
    let headers = headers(text).map_err(refuse)?;
    for key in REQUIRED {
        if !headers.contains_key(key) {
            return Err(refuse(format!("missing key `{key}`")));
        }
    }
    if let Some(key) = headers
        .keys()
        .find(|key| !REQUIRED.contains(&key.as_str()) && !OPTIONAL.contains(&key.as_str()))
    {
        return Err(refuse(format!("unknown key `{key}`")));
    }
    let rule = &headers["rule"];
    if !rules.contains(rule) {
        return Err(refuse(format!("unknown rule `{rule}`")));
    }
    let tail = rule.rsplit("::").next().unwrap_or_default();
    if name.split_once("--").map(|(prefix, _)| prefix) != Some(tail) {
        return Err(refuse(format!("the directory does not start with `{tail}--`")));
    }
    let weakness = &headers["weakness"];
    if !WEAKNESSES.contains(&weakness.as_str()) {
        return Err(refuse(format!(
            "unknown weakness `{weakness}` (expected one of {})",
            WEAKNESSES.join(", ")
        )));
    }
    let verdict = &headers["verdict"];
    if !VERDICTS.contains(&verdict.as_str()) {
        return Err(refuse(format!("unknown verdict `{verdict}` (expected quiet or fires)")));
    }
    if headers["source"].is_empty() {
        return Err(refuse("empty key `source`".to_owned()));
    }
    let known = headers.get("known-false-positive");
    if known.is_some_and(|reason| reason.is_empty() || verdict != "fires") {
        return Err(refuse(
            "`known-false-positive` needs a reason and the verdict `fires`".to_owned(),
        ));
    }
    Ok(Seed {
        name: name.to_owned(),
        rule: rule.clone(),
        fires: verdict == "fires",
        split: headers.get("split").cloned(),
        form: Form::Source(String::new()),
    })
}

fn catalogued() -> BTreeSet<String> {
    rust_doctor::catalog()
        .into_iter()
        .map(|entry| entry.id.to_string())
        .collect()
}

fn load() -> (Vec<Seed>, Vec<String>) {
    let rules = catalogued();
    let mut seeds = Vec::new();
    let mut refusals = Vec::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(seeds_directory())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for directory in entries {
        let name = directory.file_name().unwrap().to_string_lossy().into_owned();
        let source = directory.join("seed.rs");
        let manifest = directory.join("Cargo.toml");
        let Ok(metadata) = fs::read_to_string(directory.join("seed.toml")) else {
            refusals.push(format!("regressions/{name}: no seed.toml"));
            continue;
        };
        let read = if manifest.is_file() {
            let text = fs::read_to_string(&manifest).unwrap();
            if let Err(error) = text.parse::<toml::Table>() {
                refusals.push(format!("regressions/{name}: Cargo.toml does not parse: {error}"));
                continue;
            }
            parse(&name, &metadata, &rules).map(|seed| Seed {
                form: Form::Crate,
                ..seed
            })
        } else if source.is_file() && fs::read_dir(&directory).unwrap().count() == 2 {
            let text = fs::read_to_string(&source).unwrap();
            parse(&name, &metadata, &rules).map(|seed| Seed {
                form: Form::Source(text),
                ..seed
            })
        } else {
            Err(format!(
                "regressions/{name}: neither a seed.rs beside its seed.toml nor a crate with a Cargo.toml"
            ))
        };
        match read {
            Ok(seed) => seeds.push(seed),
            Err(refusal) => refusals.push(refusal),
        }
    }
    (seeds, refusals)
}

fn seeds() -> Vec<Seed> {
    let (seeds, refusals) = load();
    assert!(refusals.is_empty(), "{}", refusals.join("\n"));
    seeds
}

fn fires(seed: &Seed) -> Result<bool, String> {
    match &seed.form {
        Form::Source(text) => {
            let text = match &seed.split {
                Some(token) => text.replace(token.as_str(), ""),
                None => text.clone(),
            };
            let detected = run_detectors(&CaseUnit { source: text })
                .map_err(|error| format!("regressions/{}/seed.rs {error}", seed.name))?;
            Ok(detected.iter().any(|finding| finding.rule == seed.rule))
        }
        Form::Crate => {
            let root = support::fresh_workspace("regression-seeds", &NEXT_WORKSPACE);
            support::copy_tree(&support::fixture_dir(&format!("regressions/{}", seed.name)), &root);
            fs::remove_file(root.join("seed.toml"))
                .map_err(|error| format!("regressions/{}/seed.toml {error}", seed.name))?;
            if let Some(token) = &seed.split {
                splice(&root, token);
            }
            commit(&root);
            let report = scan(&root);
            Ok(report["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic["code"] == seed.rule.as_str()))
        }
    }
}

fn splice(root: &Path, token: &str) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            splice(&path, token);
        } else {
            let text = fs::read_to_string(&path).unwrap();
            fs::write(&path, text.replace(token, "")).unwrap();
        }
    }
}

fn commit(root: &Path) {
    for arguments in [
        &["init", "--quiet"][..],
        &["add", "-A"],
        &[
            "-c",
            "user.name=Rust Doctor",
            "-c",
            "user.email=rust-doctor@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "seed",
        ],
    ] {
        let output = support::git_output(root, arguments);
        assert!(output.status.success(), "git {arguments:?}: {output:?}");
    }
}

fn scan(root: &Path) -> Value {
    let mut arguments: Vec<String> = vec![
        root.to_str().unwrap().to_owned(),
        "--json".to_owned(),
        "--yes".to_owned(),
    ];
    arguments.extend(support::NO_ADVISORY_DB.iter().map(|value| (*value).to_owned()));
    for entry in rust_doctor::catalog() {
        if entry.id.starts_with("clippy::") {
            arguments.push("--rule".to_owned());
            arguments.push(format!("{}=off", entry.id));
        }
    }
    let output = support::rust_doctor()
        .args(&arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout);
    assert!(report.is_ok(), "{}", String::from_utf8_lossy(&output.stderr));
    report.unwrap()
}

#[test]
fn every_regression_seed_keeps_its_declared_verdict() {
    let seeds = seeds();
    let mut broken = Vec::new();
    for seed in &seeds {
        match fires(seed) {
            Ok(fired) if fired == seed.fires => {}
            Ok(fired) => broken.push(format!(
                "regressions/{}: declares `{}` and the rule {}",
                seed.name,
                if seed.fires { "fires" } else { "quiet" },
                if fired { "fires" } else { "stays quiet" }
            )),
            Err(error) => broken.push(error),
        }
    }
    assert!(broken.is_empty(), "{}", broken.join("\n"));
}

#[test]
fn every_rule_a_native_fix_touched_holds_both_verdicts() {
    let seeds = seeds();
    for rule in [
        "rust_doctor::source::blocking_call_in_async",
        "rust_doctor::source::sql_built_from_format",
        "rust_doctor::source::dynamic_shell_command",
        "rust_doctor::repo::hardcoded_credential",
        "rust_doctor::structure::orphan_module_file",
        "rust_doctor::structure::duplicate_function_body",
    ] {
        let verdicts: BTreeSet<bool> = seeds
            .iter()
            .filter(|seed| seed.rule == rule)
            .map(|seed| seed.fires)
            .collect();
        assert_eq!(verdicts.len(), 2, "{rule} lacks a quiet seed or a firing one");
    }
}

#[test]
fn the_loader_refuses_a_malformed_seed_naming_it() {
    let rules = catalogued();
    let seed = |headers: &str| parse("dynamic_shell_command--probe", headers, &rules);
    let valid = "rule = \"rust_doctor::source::dynamic_shell_command\"\nweakness = \"api-variant\"\nsource = \"probe\"\nverdict = \"quiet\"\n";
    assert!(seed(valid).is_ok());

    for (headers, expected) in [
        (
            valid.replace("\"quiet\"", "\"safe\""),
            "regressions/dynamic_shell_command--probe: unknown verdict `safe` (expected quiet or fires)",
        ),
        (
            valid.replace("dynamic_shell_command\"", "no_such_rule\""),
            "regressions/dynamic_shell_command--probe: unknown rule `rust_doctor::source::no_such_rule`",
        ),
        (
            valid.replace("\"api-variant\"", "\"vibes\""),
            "regressions/dynamic_shell_command--probe: unknown weakness `vibes`",
        ),
        (
            valid.replace("source = \"probe\"\n", ""),
            "regressions/dynamic_shell_command--probe: missing key `source`",
        ),
        (
            format!("{valid}known-false-positive = \"still fires\"\n"),
            "regressions/dynamic_shell_command--probe: `known-false-positive` needs a reason and the verdict `fires`",
        ),
        (
            format!("{valid}owner = \"someone\"\n"),
            "regressions/dynamic_shell_command--probe: unknown key `owner`",
        ),
        (
            format!("{valid}split = 3\n"),
            "regressions/dynamic_shell_command--probe: key `split` is not a string",
        ),
        (
            "rule = ".to_owned(),
            "regressions/dynamic_shell_command--probe: seed.toml does not parse",
        ),
    ] {
        let refusal = seed(&headers).unwrap_err();
        assert!(refusal.starts_with(expected), "{refusal}");
    }

    let unparsed = run_detectors(&CaseUnit {
        source: "fn broken( {".to_owned(),
    });
    assert!(unparsed.is_err_and(|error| error.contains("does not parse")));
}
