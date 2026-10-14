//! The whole-binary benchmark: the release binary scans this repository and
//! the corpus clone cache cold and warm, and every phase `--timings` publishes
//! is summarized as a median and a median absolute deviation.
//!
//! It never runs as a pull request gate. Run it on a quiet machine with
//! `cargo test --release --test performance -- --ignored --nocapture`, after
//! the clone cache is in place (`RUST_DOCTOR_CORPUS_DIR`, else
//! `~/.cache/rust-doctor/corpus`). `RUST_DOCTOR_PERF_SUBJECTS=a,b` narrows the
//! run to the named subjects, `rust-doctor` being this repository, and
//! `RUST_DOCTOR_PERF_BINARY` measures another build than this one. Results land
//! in `target/performance/results.json`. They are compared with
//! `tests/fixtures/performance/baseline.json` only when the CPU model, the core
//! count and the toolchain match, since a number measured elsewhere says
//! nothing about this host.

#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

/// Runs thrown away before the samples of each cohort.
const WARMUPS: usize = 1;
const SAMPLES: usize = 5;
/// A regression is a median at least this many percent slower...
const REGRESSION_PERCENT: u64 = 10;
/// ...and at least this many milliseconds slower, so noise on a short phase
/// never fails the run.
const REGRESSION_MILLISECONDS: u64 = 250;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cohort {
    /// Every run starts from an empty target directory.
    Cold,
    /// Every run reuses the target directory the warmup filled.
    Warm,
}

impl Cohort {
    const fn name(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
        }
    }
}

struct Subject {
    name: String,
    commit: String,
    workspace: PathBuf,
}

#[test]
#[ignore = "benchmark: run with --release -- --ignored, never a pull request gate"]
fn the_release_binary_scans_every_subject_within_its_recorded_budget() {
    assert!(
        env::var_os("RUST_DOCTOR_PERF_BINARY").is_some()
            || binary().parent().and_then(Path::file_name) == Some("release".as_ref()),
        "the benchmark drives the release binary: run it with `cargo test --release`"
    );
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/performance");
    fs::create_dir_all(&output).unwrap();
    let mut toolchain = None;
    let mut measured = BTreeMap::new();
    for subject in subjects(&output) {
        let mut digest = None;
        let mut cohorts = BTreeMap::new();
        for cohort in [Cohort::Cold, Cohort::Warm] {
            let target = support::scan_target(&subject.workspace).join(cohort.name());
            let mut samples: BTreeMap<String, Vec<u64>> = BTreeMap::new();
            for run in 0..WARMUPS + SAMPLES {
                if cohort == Cohort::Cold || run == 0 {
                    let _ = fs::remove_dir_all(&target);
                }
                let report = scan(&subject.workspace, &target);
                let label = format!("{} {} run {run}", subject.name, cohort.name());
                assert_eq!(report["status"], "complete", "{label}: {}", report["errors"]);
                let found = blake3::hash(&serde_json::to_vec(&report["diagnostics"]).unwrap());
                assert_eq!(
                    *digest.get_or_insert(found),
                    found,
                    "{label}: the diagnostics changed between runs"
                );
                toolchain.get_or_insert_with(|| report["toolchain"]["rustc"].clone());
                if run < WARMUPS {
                    continue;
                }
                for (phase, milliseconds) in report["timings"].as_object().unwrap() {
                    samples
                        .entry(phase.clone())
                        .or_default()
                        .push(milliseconds.as_u64().unwrap());
                }
            }
            let summary: BTreeMap<_, _> = samples
                .into_iter()
                .map(|(phase, values)| (phase, summarize(values)))
                .collect();
            cohorts.insert(cohort.name(), summary);
        }
        measured.insert(
            subject.name.clone(),
            json!({ "commit": subject.commit, "cohorts": cohorts }),
        );
    }
    let results = json!({
        "host": {
            "cpu_model": cpu_model(),
            "cores": std::thread::available_parallelism().map_or(0, usize::from),
            "toolchain": toolchain.unwrap_or(Value::Null),
        },
        "warmups": WARMUPS,
        "samples": SAMPLES,
        "subjects": measured,
    });
    fs::write(
        output.join("results.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(&results).unwrap());

    let baseline_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/performance/baseline.json");
    let Ok(baseline) = fs::read(&baseline_path) else {
        println!("No committed baseline: nothing compared.");
        return;
    };
    let baseline: Value = serde_json::from_slice(&baseline).unwrap();
    if baseline["host"] != results["host"] {
        println!("The committed baseline was measured on another host: nothing compared.");
        return;
    }
    let regressions = regressions(&baseline, &results);
    assert!(regressions.is_empty(), "regressions: {regressions:#?}");
}

/// This repository at its `HEAD`, then every curated corpus repository the
/// clone cache holds, each materialized at its pinned commit so the measured
/// tree never moves under the binary being measured.
fn subjects(output: &Path) -> Vec<Subject> {
    let selected: Option<Vec<String>> = env::var("RUST_DOCTOR_PERF_SUBJECTS")
        .ok()
        .map(|names| names.split(',').map(str::to_owned).collect());
    let wanted = |name: &str| selected.as_ref().is_none_or(|names| names.iter().any(|n| n == name));
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut entries = vec![("rust-doctor".to_owned(), repository.to_path_buf(), head(repository))];
    let cache = env::var_os("RUST_DOCTOR_CORPUS_DIR").map_or_else(
        || Path::new(&env::var_os("HOME").unwrap()).join(".cache/rust-doctor/corpus"),
        PathBuf::from,
    );
    if cache.is_dir() {
        for entry in support::corpus::artifact().manifest.repositories {
            entries.push((entry.name.clone(), cache.join(&entry.name), entry.commit));
        }
    } else {
        println!("No corpus clone cache at {}: this repository only.", cache.display());
    }
    entries
        .into_iter()
        .filter(|(name, source, _)| wanted(name) && source.join(".git").exists())
        .map(|(name, source, commit)| {
            let workspace = output.join("work").join(&name);
            let _ = fs::remove_dir_all(&workspace);
            fs::create_dir_all(&workspace).unwrap();
            support::corpus::materialise(&source, &commit, output, &workspace);
            Subject {
                name,
                commit,
                workspace,
            }
        })
        .collect()
}

fn scan(workspace: &Path, target: &Path) -> Value {
    let output = Command::new(binary())
        .arg(workspace)
        .args(["--json", "--yes", "--timings", "--advisory-db", "/dev/null"])
        .env("CARGO_TARGET_DIR", target)
        // A wall-clock cutoff would make a large subject's findings depend on
        // machine load, and the digest check would read that as a defect.
        .env("RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS", "600")
        .output()
        .expect("the release binary should start");
    serde_json::from_slice(&output.stdout).expect("every scan should print a report")
}

/// The binary under measurement: `RUST_DOCTOR_PERF_BINARY` when a run
/// compares two builds, else the release binary this test was built with.
fn binary() -> PathBuf {
    env::var_os("RUST_DOCTOR_PERF_BINARY")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_rust-doctor")), PathBuf::from)
}

fn head(repository: &Path) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|info| {
            info.lines()
                .find_map(|line| line.strip_prefix("model name"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, model)| model.trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The median and the median absolute deviation of one phase's samples.
fn summarize(mut values: Vec<u64>) -> Value {
    let center = median(&mut values);
    let mut deviations: Vec<u64> = values.iter().map(|value| value.abs_diff(center)).collect();
    json!({ "median": center, "mad": median(&mut deviations) })
}

fn median(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    match values.len() {
        0 => 0,
        length if length % 2 == 1 => values[length / 2],
        length => (values[length / 2 - 1] + values[length / 2]) / 2,
    }
}

/// Every phase whose median grew by at least both bounds against the
/// baseline, named `subject/cohort/phase`.
fn regressions(baseline: &Value, results: &Value) -> Vec<String> {
    let mut found = Vec::new();
    let Some(subjects) = results["subjects"].as_object() else {
        return found;
    };
    for (subject, measured) in subjects {
        let Some(cohorts) = measured["cohorts"].as_object() else {
            continue;
        };
        for (cohort, phases) in cohorts {
            for (phase, summary) in phases.as_object().into_iter().flatten() {
                let before = &baseline["subjects"][subject]["cohorts"][cohort][phase]["median"];
                let (Some(before), Some(after)) = (before.as_u64(), summary["median"].as_u64())
                else {
                    continue;
                };
                if after >= before + REGRESSION_MILLISECONDS
                    && after * 100 >= before * (100 + REGRESSION_PERCENT)
                {
                    found.push(format!("{subject}/{cohort}/{phase}: {before} ms to {after} ms"));
                }
            }
        }
    }
    found
}

#[test]
fn a_regression_needs_both_ten_percent_and_two_hundred_fifty_milliseconds() {
    let baseline = json!({ "subjects": { "s": { "cohorts": { "warm": {
        "total": { "median": 1000 },
        "walk": { "median": 100 },
        "clippy_linting": { "median": 10_000 },
    }}}}});
    let results = json!({ "subjects": { "s": { "cohorts": { "warm": {
        "total": { "median": 1250 },
        "walk": { "median": 400 },
        "clippy_linting": { "median": 10_900 },
    }}}}});
    assert_eq!(
        regressions(&baseline, &results),
        ["s/warm/total: 1000 ms to 1250 ms", "s/warm/walk: 100 ms to 400 ms"]
    );
    let mut samples = vec![5, 1, 9, 3, 7];
    assert_eq!(median(&mut samples), 5);
    assert_eq!(summarize(vec![10, 12, 11, 30, 9]), json!({ "median": 11, "mad": 1 }));
}
