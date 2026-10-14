#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Which delta pairing removes refactor noise without hiding new findings.
//!
//! The harness replays three pairing policies on real commit pairs: the last
//! 50 of this repository touching `src/`, and the last 10 touching a `.rs`
//! file of each agent-population repository the corpus clone cache holds.
//!
//! - (a) today's evidence pairing, run through the library itself;
//! - (b) (a), then the leftovers of one rule paired when the Git-renamed path
//!   and the `git diff --unified=0 --find-renames` hunk map make their lines
//!   correspond: the same line moved by the hunks above it, or a current line
//!   inside a modified hunk and a base line inside that hunk's old range;
//! - (c) react-doctor's count per (renamed path, rule).
//!
//! Every finding (a) introduces and (b) or (c) does not is a candidate, and a
//! candidate is classified false (the base already had it: a refactor moved or
//! touched the line) or true (the diff wrote it) by reading the diff. The
//! verdicts live in `tests/fixtures/delta-pairing/verdicts.json`, the result in
//! `docs/delta-pairing-2026-10.md`. It scans hundreds of commits, so it runs
//! only when asked:
//!
//! ```text
//! cargo test --release --test delta_pairing -- --ignored --nocapture
//! ```
//!
//! Reports are read back from `target/delta-pairing/reports/<repo>/<sha>.json`
//! (`RUST_DOCTOR_PAIRING_DIR` moves the whole directory), and a missing one is
//! scanned with `RUST_DOCTOR_PAIRING_BINARY`, else the binary this build made.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rust_doctor::Diagnostic;
use serde_json::{Value, json};

/// The commit of this repository the measurement was taken at.
const OWN_TIP: &str = "1f5d913";

struct Subject {
    name: String,
    source: PathBuf,
    tip: String,
    count: usize,
    pathspec: &'static str,
}

#[derive(Default)]
struct Hunk {
    old_start: usize,
    old_len: usize,
    new_start: usize,
    new_len: usize,
}

/// What the diff did to one current path.
#[derive(Default)]
struct FileDiff {
    old: Option<String>,
    hunks: Vec<Hunk>,
}

/// Every current path the diff touched, keyed by its current name, and the
/// base path every renamed current path came from.
#[derive(Default)]
struct DiffMap {
    files: HashMap<String, FileDiff>,
}

impl DiffMap {
    fn parse(patch: &str) -> Self {
        let mut map = Self::default();
        let mut current: Option<(Option<String>, FileDiff)> = None;
        let flush = |entry: Option<(Option<String>, FileDiff)>, map: &mut Self| {
            if let Some((Some(path), diff)) = entry {
                map.files.insert(path, diff);
            }
        };
        for line in patch.lines() {
            if line.starts_with("diff --git ") {
                flush(current.take(), &mut map);
                current = Some((None, FileDiff::default()));
            } else if let Some((path, diff)) = current.as_mut() {
                if let Some(old) = line.strip_prefix("rename from ") {
                    diff.old = Some(old.to_owned());
                } else if let Some(new) = line.strip_prefix("rename to ") {
                    *path = Some(new.to_owned());
                } else if let Some(new) = line.strip_prefix("+++ b/") {
                    *path = Some(new.to_owned());
                } else if let Some(header) = line.strip_prefix("@@ -") {
                    diff.hunks.push(hunk(header));
                }
            }
        }
        flush(current, &mut map);
        map
    }

    /// The base path a current path came from.
    fn base_path<'a>(&'a self, current: &'a str) -> &'a str {
        self.files
            .get(current)
            .and_then(|diff| diff.old.as_deref())
            .unwrap_or(current)
    }

    /// Whether a base line and a current line of one file correspond.
    fn corresponds(&self, current_path: &str, base_line: usize, current_line: usize) -> bool {
        let Some(diff) = self.files.get(current_path) else {
            return base_line == current_line;
        };
        let inside = diff.hunks.iter().any(|hunk| {
            hunk.old_len > 0
                && hunk.new_len > 0
                && (hunk.new_start..hunk.new_start + hunk.new_len).contains(&current_line)
                && (hunk.old_start..hunk.old_start + hunk.old_len).contains(&base_line)
        });
        inside || mapped(&diff.hunks, base_line) == Some(current_line)
    }
}

/// `-a[,b] +c[,d] @@`, where an omitted length is one.
fn hunk(header: &str) -> Hunk {
    let mut ranges = header.split(' ');
    let range = |text: Option<&str>| -> (usize, usize) {
        let text = text.unwrap_or_default().trim_start_matches(['-', '+']);
        let (start, len) = text.split_once(',').unwrap_or((text, "1"));
        (start.parse().unwrap_or(0), len.parse().unwrap_or(1))
    };
    let (old_start, old_len) = range(ranges.next());
    let (new_start, new_len) = range(ranges.next());
    Hunk {
        old_start,
        old_len,
        new_start,
        new_len,
    }
}

/// Where an unchanged base line lands, `None` when a hunk rewrote it.
fn mapped(hunks: &[Hunk], line: usize) -> Option<usize> {
    let mut offset = 0isize;
    for hunk in hunks {
        let rewritten = hunk.old_len > 0
            && (hunk.old_start..hunk.old_start + hunk.old_len).contains(&line);
        if rewritten {
            return None;
        }
        // A pure insertion follows `old_start`; any other hunk ends before
        // `old_start + old_len`.
        let before = if hunk.old_len == 0 {
            hunk.old_start < line
        } else {
            hunk.old_start + hunk.old_len <= line
        };
        if before {
            offset += hunk.new_len as isize - hunk.old_len as isize;
        }
    }
    line.checked_add_signed(offset)
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "core.quotePath=false"])
        .args(arguments)
        .current_dir(directory)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn subjects() -> Option<Vec<Subject>> {
    let cache = env::var_os("RUST_DOCTOR_CORPUS_DIR").map_or_else(
        || Path::new(&env::var_os("HOME").unwrap()).join(".cache/rust-doctor/corpus"),
        PathBuf::from,
    );
    if !cache.is_dir() {
        println!("No corpus clone cache at {}: the pairing measurement is skipped.", cache.display());
        return None;
    }
    let mut subjects = vec![Subject {
        name: "rust-doctor".to_owned(),
        source: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        tip: OWN_TIP.to_owned(),
        count: 50,
        pathspec: "src/",
    }];
    for (name, tip) in agent_population() {
        subjects.push(Subject {
            source: cache.join(&name),
            name,
            tip,
            count: 10,
            pathspec: "*.rs",
        });
    }
    let missing: Vec<_> = subjects
        .iter()
        .filter(|subject| !subject.source.join(".git").exists())
        .map(|subject| subject.name.as_str())
        .collect();
    if !missing.is_empty() {
        println!("The clone cache lacks {missing:?}: the pairing measurement is skipped.");
        return None;
    }
    Some(subjects)
}

/// The name and pinned commit of every agent-population repository.
fn agent_population() -> Vec<(String, String)> {
    let corpus: Value = serde_json::from_str(include_str!("corpus.json")).unwrap();
    corpus["agent_population"]["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repository| {
            (
                repository["name"].as_str().unwrap().to_owned(),
                repository["commit"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// The report of one commit, scanned when no earlier run left it.
fn report(subject: &Subject, tree: &Path, output: &Path, sha: &str) -> Value {
    let path = output.join("reports").join(&subject.name).join(format!("{sha}.json"));
    if !path.exists() {
        git(tree, &["checkout", "-q", "--force", "--detach", sha]);
        git(tree, &["clean", "-qfdx"]);
        let binary = env::var_os("RUST_DOCTOR_PAIRING_BINARY")
            .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_rust-doctor")), PathBuf::from);
        let scan = Command::new(binary)
            .arg(tree)
            .args(["--json", "--yes", "--advisory-db", "/dev/null"])
            .env("CARGO_TARGET_DIR", tree.with_file_name("target"))
            .output()
            .unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, scan.stdout).unwrap();
    }
    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap()
}

/// The stage and code of every error a report carries: two sides whose
/// passes did not cover the same ground measure that, not their pairing.
fn coverage(report: &Value) -> BTreeSet<(String, String)> {
    report["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|error| {
            (
                error["stage"].as_str().unwrap_or_default().to_owned(),
                error["code"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn diagnostics(report: &Value) -> Vec<Diagnostic> {
    serde_json::from_value(report["diagnostics"].clone()).unwrap()
}

fn extract(tree: &Path, sha: &str, into: &Path) {
    let _ = fs::remove_dir_all(into);
    fs::create_dir_all(into).unwrap();
    let archive = Command::new("git")
        .args(["archive", "--format=tar", sha])
        .current_dir(tree)
        .output()
        .unwrap();
    assert!(archive.status.success());
    let mut tar = Command::new("tar")
        .arg("-x")
        .current_dir(into)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(tar.stdin.as_mut().unwrap(), &archive.stdout).unwrap();
    assert!(tar.wait().unwrap().success());
}

fn line(diagnostic: &Diagnostic) -> Option<usize> {
    diagnostic.span.as_ref().map(|span| span.line_start)
}

fn text(root: &Path, diagnostic: &Diagnostic) -> String {
    let (Some(path), Some(line)) = (diagnostic.path.as_deref(), line(diagnostic)) else {
        return String::new();
    };
    fs::read_to_string(root.join(path))
        .ok()
        .and_then(|source| source.lines().nth(line.saturating_sub(1)).map(str::trim).map(str::to_owned))
        .unwrap_or_default()
}

type Group = (Option<String>, Option<String>);

/// One commit pair measured under the three policies.
fn measure(
    base: &[Diagnostic],
    base_root: &Path,
    current: &[Diagnostic],
    current_root: &Path,
    diff: &DiffMap,
) -> Value {
    let delta = rust_doctor::delta_harness::pair(base, base_root, current, current_root).unwrap();
    // (a): the current findings the library introduces, the base ones it fixes.
    let mut by_id: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, diagnostic) in current.iter().enumerate() {
        by_id.entry(diagnostic.id.as_str()).or_default().push(index);
    }
    let introduced_a: Vec<usize> = delta
        .introduced
        .iter()
        .filter_map(|id| by_id.get_mut(id.as_str()).and_then(Vec::pop))
        .collect();
    let mut leftovers: Vec<Option<&Diagnostic>> = delta.fixed.iter().map(Some).collect();

    // (b): a leftover of the same rule whose line corresponds.
    let mut paired_b = BTreeSet::new();
    for &index in &introduced_a {
        let finding = &current[index];
        let (Some(path), Some(at)) = (finding.path.as_deref(), line(finding)) else {
            continue;
        };
        let from = diff.base_path(path);
        let best = leftovers
            .iter()
            .enumerate()
            .filter_map(|(slot, leftover)| leftover.map(|leftover| (slot, leftover)))
            .filter(|(_, leftover)| {
                leftover.code == finding.code
                    && leftover.path.as_deref() == Some(from)
                    && line(leftover).is_some_and(|base| diff.corresponds(path, base, at))
            })
            .min_by_key(|(_, leftover)| line(leftover).unwrap_or(0).abs_diff(at))
            .map(|(slot, _)| slot);
        if let Some(slot) = best {
            leftovers[slot] = None;
            paired_b.insert(index);
        }
    }

    // (c): counts per (renamed path, rule) over every finding of both sides.
    let mut counts: BTreeMap<Group, (usize, usize)> = BTreeMap::new();
    for finding in base {
        let path = finding.path.as_deref().map(|path| {
            diff.files
                .iter()
                .find(|(_, file)| file.old.as_deref() == Some(path))
                .map_or(path, |(current, _)| current.as_str())
                .to_owned()
        });
        counts.entry((path, finding.code.clone())).or_default().0 += 1;
    }
    for finding in current {
        counts.entry((finding.path.clone(), finding.code.clone())).or_default().1 += 1;
    }
    let mut introduced_a_by_group: BTreeMap<Group, Vec<usize>> = BTreeMap::new();
    for &index in &introduced_a {
        let finding = &current[index];
        introduced_a_by_group
            .entry((finding.path.clone(), finding.code.clone()))
            .or_default()
            .push(index);
    }
    let mut fixed_by_group: BTreeMap<Group, usize> = BTreeMap::new();
    for finding in &delta.fixed {
        let path = finding.path.as_deref().map(|path| {
            diff.files
                .iter()
                .find(|(_, file)| file.old.as_deref() == Some(path))
                .map_or(path, |(current, _)| current.as_str())
                .to_owned()
        });
        *fixed_by_group.entry((path, finding.code.clone())).or_default() += 1;
    }
    let introduced_c: usize = counts.values().map(|(base, current)| current.saturating_sub(*base)).sum();
    // What (c) introduces over (a): a finding (a) paired across files by its
    // stable identity, which a count per file cannot follow without a rename.
    let added_c: Vec<Value> = counts
        .iter()
        .filter_map(|(group, (base_count, current_count))| {
            let a = introduced_a_by_group.get(group).map_or(0, Vec::len);
            let added = current_count.saturating_sub(*base_count).saturating_sub(a);
            (added > 0).then(|| json!({"path": group.0, "rule": group.1, "added": added}))
        })
        .collect();

    let mut groups = Vec::new();
    for (group, members) in &introduced_a_by_group {
        let (base_count, current_count) = counts.get(group).copied().unwrap_or_default();
        let c = current_count.saturating_sub(base_count);
        let removed_c = members.len().saturating_sub(c);
        let removed_b = members.iter().filter(|index| paired_b.contains(index)).count();
        let fixed = fixed_by_group.get(group).copied().unwrap_or(0);
        let removed_d = members.len().min(fixed);
        if removed_b == 0 && removed_c == 0 && removed_d == 0 {
            continue;
        }
        let candidates: Vec<Value> = members
            .iter()
            .map(|&index| {
                let finding = &current[index];
                json!({
                    "line": line(finding),
                    "text": text(current_root, finding),
                    "removed_by_b": paired_b.contains(&index),
                })
            })
            .collect();
        let base_texts: Vec<Value> = delta
            .fixed
            .iter()
            .filter(|finding| {
                finding.code == group.1
                    && finding.path.as_deref()
                        == group.0.as_deref().map(|path| diff.base_path(path))
            })
            .map(|finding| json!({"line": line(finding), "text": text(base_root, finding)}))
            .collect();
        groups.push(json!({
            "path": group.0,
            "rule": group.1,
            "introduced_a": members.len(),
            "introduced_c": c,
            "removed_b": removed_b,
            "removed_c": removed_c,
            "removed_d": removed_d,
            "fixed_a": fixed,
            "current": candidates,
            "base_leftovers": base_texts,
        }));
    }
    json!({
        "introduced": {
            "a": introduced_a.len(),
            "b": introduced_a.len() - paired_b.len(),
            "d": introduced_a.len()
                - groups.iter().map(|group| group["removed_d"].as_u64().unwrap() as usize).sum::<usize>(),
            "c": introduced_c,
        },
        "fixed_a": delta.fixed.len(),
        "cross_file_a": delta.summary.cross_file_matches,
        "added_c": added_c,
        "groups": groups,
    })
}

/// The pairs of one subject, each measured or excluded with its reason.
fn replay(subject: &Subject, output: &Path) -> Vec<Value> {
    let work = output.join("work").join(&subject.name);
    let tree = work.join("tree");
    if !tree.join(".git").exists() {
        fs::create_dir_all(&work).unwrap();
        git(&work, &["clone", "-q", "--shared", "--no-checkout", subject.source.to_str().unwrap(), "tree"]);
    }
    let count = subject.count.to_string();
    let commits = git(&tree, &["rev-list", "--no-merges", "-n", &count, &subject.tip, "--", subject.pathspec]);
    let mut pairs = Vec::new();
    for commit in commits.lines() {
        let parent = git(&tree, &["rev-parse", &format!("{commit}^1")]).trim().to_owned();
        let base_report = report(subject, &tree, output, &parent);
        let current_report = report(subject, &tree, output, commit);
        let (base_errors, current_errors) = (coverage(&base_report), coverage(&current_report));
        if base_errors != current_errors {
            pairs.push(json!({
                "base": parent,
                "commit": commit,
                "excluded": format!("the sides' errors differ: {base_errors:?} against {current_errors:?}"),
            }));
            continue;
        }
        let base_root = output.join("trees").join(&subject.name).join("base");
        let current_root = output.join("trees").join(&subject.name).join("current");
        extract(&tree, &parent, &base_root);
        extract(&tree, commit, &current_root);
        let patch = git(&tree, &["diff", "--no-ext-diff", "--no-color", "--unified=0", "--find-renames", &parent, commit]);
        let mut measured = measure(
            &diagnostics(&base_report),
            &base_root,
            &diagnostics(&current_report),
            &current_root,
            &DiffMap::parse(&patch),
        );
        measured["base"] = json!(parent);
        measured["commit"] = json!(commit);
        measured["complete"] = json!(current_report["complete"]);
        pairs.push(measured);
    }
    let _ = fs::remove_dir_all(output.join("trees").join(&subject.name));
    pairs
}

/// One candidate's verdict key: repository, commit, path, rule, line.
fn key(subject: &str, pair: &Value, group: &Value, candidate: &Value) -> String {
    format!(
        "{subject} {} {} {} {}",
        &pair["commit"].as_str().unwrap()[..12],
        group["path"].as_str().unwrap_or("-"),
        group["rule"].as_str().unwrap_or("-"),
        candidate["line"]
    )
}

#[test]
#[ignore = "scans hundreds of commits; run with --ignored"]
fn the_three_pairing_policies_replay_on_real_history() {
    let Some(subjects) = subjects() else {
        return;
    };
    let output = env::var_os("RUST_DOCTOR_PAIRING_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("target/delta-pairing"),
        PathBuf::from,
    );
    let verdicts: BTreeMap<String, String> = serde_json::from_str(include_str!(
        "fixtures/delta-pairing/verdicts.json"
    ))
    .unwrap();
    let mut results = serde_json::Map::new();
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unclassified = Vec::new();
    let mut hidden_swaps = Vec::new();
    for subject in &subjects {
        let pairs = replay(subject, &output);
        for pair in &pairs {
            if pair.get("excluded").is_some() {
                *tally.entry("pairs excluded").or_default() += 1;
                continue;
            }
            *tally.entry("pairs measured").or_default() += 1;
            for policy in ["a", "b", "c", "d"] {
                let introduced = pair["introduced"][policy].as_u64().unwrap() as usize;
                *tally.entry(match policy {
                    "a" => "introduced (a)",
                    "b" => "introduced (b)",
                    "d" => "introduced (d)",
                    _ => "introduced (c)",
                }).or_default() += introduced;
            }
            for added in pair["added_c"].as_array().unwrap() {
                *tally.entry("false added by (c)").or_default() += added["added"].as_u64().unwrap() as usize;
            }
            for group in pair["groups"].as_array().unwrap() {
                let mut false_in_group = 0;
                let mut true_in_group = 0;
                for candidate in group["current"].as_array().unwrap() {
                    let key = key(&subject.name, pair, group, candidate);
                    let removed_by_b = candidate["removed_by_b"].as_bool().unwrap();
                    match verdicts.get(&key).map(String::as_str) {
                        Some("false") => {
                            false_in_group += 1;
                            if removed_by_b {
                                *tally.entry("false removed by (b)").or_default() += 1;
                            }
                        }
                        Some("true") => {
                            true_in_group += 1;
                            if removed_by_b {
                                *tally.entry("true hidden by (b)").or_default() += 1;
                            }
                        }
                        _ => unclassified.push((key, group.clone())),
                    }
                }
                let removed_d = group["removed_d"].as_u64().unwrap() as usize;
                *tally.entry("false removed by (d)").or_default() += removed_d.min(false_in_group);
                *tally.entry("true hidden by (d)").or_default() += removed_d.saturating_sub(false_in_group);
                let removed_c = group["removed_c"].as_u64().unwrap() as usize;
                *tally.entry("false removed by (c)").or_default() += removed_c.min(false_in_group);
                *tally.entry("true hidden by (c)").or_default() += removed_c.saturating_sub(false_in_group);
                let swap = group["introduced_c"] == 0 && group["fixed_a"].as_u64().unwrap() > 0 && true_in_group > 0;
                if swap {
                    hidden_swaps.push(format!(
                        "{} {} {} {}",
                        subject.name,
                        &pair["commit"].as_str().unwrap()[..12],
                        group["path"].as_str().unwrap_or("-"),
                        group["rule"].as_str().unwrap_or("-")
                    ));
                }
            }
        }
        results.insert(subject.name.clone(), json!(pairs));
    }
    fs::write(
        output.join("results.json"),
        serde_json::to_vec_pretty(&json!({"pairs": results, "tally": tally, "hidden_swaps": hidden_swaps})).unwrap(),
    )
    .unwrap();
    println!("{tally:#?}\nhidden swaps under (c): {hidden_swaps:#?}");
    for (key, group) in &unclassified {
        println!("UNCLASSIFIED {key}\n{}", serde_json::to_string_pretty(group).unwrap());
    }
    assert!(unclassified.is_empty(), "{} candidates carry no verdict", unclassified.len());
    let removed_b = tally.get("false removed by (b)").copied().unwrap_or(0);
    let removed_c = tally.get("false removed by (c)").copied().unwrap_or(0);
    println!(
        "(b) removes {removed_b} of the {removed_c} false introductions (c) removes: {}",
        if removed_b * 5 >= removed_c * 4 { "adopt (b)" } else { "put the choice to the maintainer" }
    );
}

#[test]
fn the_hunk_map_maps_a_moved_line_and_refuses_a_rewritten_one() {
    let patch = "diff --git a/a.rs b/b.rs\nsimilarity index 90%\nrename from a.rs\nrename to b.rs\n\
                 --- a/a.rs\n+++ b/b.rs\n@@ -2,0 +3,2 @@\n+x\n+y\n@@ -10 +12 @@\n-old\n+new\n";
    let map = DiffMap::parse(patch);
    assert_eq!(map.base_path("b.rs"), "a.rs");
    assert!(map.corresponds("b.rs", 1, 1));
    assert!(map.corresponds("b.rs", 5, 7));
    assert!(map.corresponds("b.rs", 10, 12));
    assert!(!map.corresponds("b.rs", 5, 5));
    assert!(map.corresponds("c.rs", 4, 4));
}
