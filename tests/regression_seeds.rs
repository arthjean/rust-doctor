#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! US-021: every confirmed false positive and every measured miss of a native
//! rule, pinned as a hand-minimized seed this test replays on every
//! `cargo test`, so a fixed verdict cannot quietly come back.
//!
//! A seed is a directory `tests/fixtures/regressions/<rule-tail>--<slug>/`
//! holding a single `seed.rs`, or a `Cargo.toml` with the files of a minimal
//! crate. Its first comment lines are its headers: `rule`, `weakness`,
//! `source` and `verdict`, then optionally `known-false-positive`, for a seed
//! that still fires although it should not, and `split`, a token removed from
//! every file of the seed before the scan. The credential rule's seeds spell
//! their secret split by it, so no file of this repository carries a value the
//! rule matches, which `tests/repo_hygiene.rs` promises.
//!
//! A source seed runs in process through the detectors' registry. A crate seed
//! runs through the binary with every Clippy rule off, so nothing is compiled,
//! inside a fresh git repository the repository pass can enumerate.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

use rust_doctor::seed_harness::{CaseUnit, run_detectors};
use serde_json::Value;

mod support;

/// The ways a native rule has been measured to be wrong, closed so that a
/// seed's weakness is a class two seeds can share and a count can be taken of.
const WEAKNESSES: [&str; 12] = [
    "allow-by-default-lint",
    "api-variant",
    "conditional-module",
    "doc-comment",
    "glob-import",
    "helper-call",
    "library-name",
    "path-attribute",
    "placeholder-credential",
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
    Crate(PathBuf),
}

#[derive(Debug)]
struct Seed {
    name: String,
    rule: String,
    fires: bool,
    split: Option<String>,
    form: Form,
}

/// The headers of a seed: the comment lines that open its head file, each
/// `key: value`, read until the first line that is not one.
fn headers(text: &str, marker: &str) -> BTreeMap<String, String> {
    text.lines()
        .map_while(|line| line.strip_prefix(marker))
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

/// A seed read from its headers, or the reason it is refused, naming it.
fn parse(name: &str, text: &str, marker: &str, rules: &BTreeSet<String>) -> Result<Seed, String> {
    let refuse = |reason: String| format!("regressions/{name}: {reason}");
    let headers = headers(text, marker);
    for key in REQUIRED {
        if !headers.contains_key(key) {
            return Err(refuse(format!("missing header `{key}`")));
        }
    }
    if let Some(key) = headers
        .keys()
        .find(|key| !REQUIRED.contains(&key.as_str()) && !OPTIONAL.contains(&key.as_str()))
    {
        return Err(refuse(format!("unknown header `{key}`")));
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
        return Err(refuse("empty header `source`".to_owned()));
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

/// Every seed of the directory, and every refusal.
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
        let read = if manifest.is_file() {
            let text = fs::read_to_string(&manifest).unwrap();
            if let Err(error) = text.parse::<toml::Table>() {
                refusals.push(format!("regressions/{name}: Cargo.toml does not parse: {error}"));
                continue;
            }
            parse(&name, &text, "# ", &rules).map(|seed| Seed {
                form: Form::Crate(directory.clone()),
                ..seed
            })
        } else if source.is_file() && fs::read_dir(&directory).unwrap().count() == 1 {
            let text = fs::read_to_string(&source).unwrap();
            parse(&name, &text, "// ", &rules).map(|seed| Seed {
                form: Form::Source(text),
                ..seed
            })
        } else {
            Err(format!(
                "regressions/{name}: neither a single seed.rs nor a crate with a Cargo.toml"
            ))
        };
        match read {
            Ok(seed) => seeds.push(seed),
            Err(refusal) => refusals.push(refusal),
        }
    }
    (seeds, refusals)
}

/// Every seed, once the directory holds no refused one.
fn seeds() -> Vec<Seed> {
    let (seeds, refusals) = load();
    assert!(refusals.is_empty(), "{}", refusals.join("\n"));
    seeds
}

/// Does the seed's rule report anything on it?
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
        Form::Crate(directory) => {
            let root = support::fresh_workspace("regression-seeds", &NEXT_WORKSPACE);
            support::copy_tree(directory, &root);
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

/// Removes `token` from every file under `root`.
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

/// The binary's report on `root`, with every Clippy rule off.
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

/// Every rule the native fixes of the gap closure touched carries a seed for
/// the fixed shape and a counterexample, so a fix cannot ship without both.
#[test]
fn every_rule_a_native_fix_touched_holds_both_verdicts() {
    let seeds = seeds();
    for rule in [
        "rust_doctor::source::blocking_call_in_async",
        "rust_doctor::source::sql_built_from_format",
        "rust_doctor::source::dynamic_shell_command",
        "rust_doctor::repo::hardcoded_credential",
        "rust_doctor::structure::orphan_module_file",
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
    let seed = |headers: &str| {
        let text = format!("{headers}\n\nfn main() {{}}\n");
        parse("dynamic_shell_command--probe", &text, "// ", &rules)
    };
    let valid = "// rule: rust_doctor::source::dynamic_shell_command\n// weakness: api-variant\n// source: probe\n// verdict: quiet";
    assert!(seed(valid).is_ok());

    for (headers, expected) in [
        (
            valid.replace("verdict: quiet", "verdict: safe"),
            "regressions/dynamic_shell_command--probe: unknown verdict `safe` (expected quiet or fires)",
        ),
        (
            valid.replace("dynamic_shell_command\n", "no_such_rule\n"),
            "regressions/dynamic_shell_command--probe: unknown rule `rust_doctor::source::no_such_rule`",
        ),
        (
            valid.replace("weakness: api-variant", "weakness: vibes"),
            "regressions/dynamic_shell_command--probe: unknown weakness `vibes`",
        ),
        (
            valid.replace("// source: probe\n", ""),
            "regressions/dynamic_shell_command--probe: missing header `source`",
        ),
        (
            format!("{valid}\n// known-false-positive: still fires"),
            "regressions/dynamic_shell_command--probe: `known-false-positive` needs a reason and the verdict `fires`",
        ),
        (
            format!("{valid}\n// owner: someone"),
            "regressions/dynamic_shell_command--probe: unknown header `owner`",
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
