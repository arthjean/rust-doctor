#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/kernel-contract")
        .join(name)
}

fn inspect_json(path: &Path) -> Output {
    binary()
        .args(["inspect", "--json"])
        .arg(path)
        .output()
        .expect("rust-doctor should start")
}

fn parse_report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should contain one JSON report")
}

fn curated(report: &Value) -> Vec<&Value> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter(|diagnostic| !diagnostic["category"].is_null())
        .collect()
}

fn collect_fixture_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .expect("fixture directory should be readable")
        .map(|entry| entry.expect("fixture entry should be readable").path())
        .collect();
    entries.sort();

    for path in entries {
        let relative = path
            .strip_prefix(root)
            .expect("fixture entry should remain under its root");
        if relative
            .components()
            .any(|component| component.as_os_str() == "target")
            || path.file_name().is_some_and(|name| name == "Cargo.lock")
        {
            continue;
        }
        if path.is_dir() {
            collect_fixture_files(root, &path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

fn fixture_hash(root: &Path) -> blake3::Hash {
    let mut files = Vec::new();
    collect_fixture_files(root, root, &mut files);
    files.sort();

    let mut hasher = blake3::Hasher::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .expect("fixture file should remain under its root");
        hasher.update(relative.as_os_str().as_encoded_bytes());
        hasher.update(&[0]);
        hasher.update(&fs::read(path).expect("fixture file should be readable"));
        hasher.update(&[0]);
    }
    hasher.finalize()
}

fn copy_fixture(source: &Path, destination: &Path) {
    if destination.exists() {
        fs::remove_dir_all(destination).expect("stale copied fixture should be removable");
    }
    fs::create_dir_all(destination).expect("copied fixture root should be creatable");

    let mut files = Vec::new();
    collect_fixture_files(source, source, &mut files);
    for source_path in files {
        let relative = source_path
            .strip_prefix(source)
            .expect("fixture file should remain under its root");
        let destination_path = destination.join(relative);
        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).expect("copied fixture directory should be creatable");
        }
        fs::copy(&source_path, destination_path).expect("fixture file should copy");
    }

    // `collect_fixture_files` écarte le fichier de verrouillage de l'empreinte,
    // mais la copie doit le porter: sans lui, le workspace copié produit un
    // binaire sans graphe résolu et le pack dépendances le signale à juste
    // titre, ce que ce test ne mesure pas.
    let lockfile = source.join("Cargo.lock");
    if lockfile.is_file() {
        fs::copy(lockfile, destination.join("Cargo.lock")).expect("lockfile should copy");
    }
}

fn temporary_fixture(label: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("{label}-{}", std::process::id()))
}

fn prepend_crate_allow(path: &Path) {
    let source = fs::read_to_string(path).expect("copied fixture source should be readable");
    let allow = "#![allow(clippy::dbg_macro, clippy::todo, clippy::unimplemented)]\n\n";
    fs::write(path, format!("{allow}{source}")).expect("copied fixture source should be writable");
}

fn replace_in_file(path: &Path, replacements: &[(&str, &str)]) {
    let mut source = fs::read_to_string(path).expect("copied fixture source should be readable");
    for (before, after) in replacements {
        source = source.replace(before, after);
    }
    fs::write(path, source).expect("copied fixture source should be writable");
}

fn diagnostic_ids(report: &Value, curated_only: bool) -> BTreeSet<String> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter(|diagnostic| !curated_only || !diagnostic["category"].is_null())
        .map(|diagnostic| {
            diagnostic["id"]
                .as_str()
                .expect("diagnostic id should be a string")
                .to_owned()
        })
        .collect()
}

#[test]
fn precision_matrix_matches_all_positive_and_negative_oracles_without_mutation() {
    let root = fixture("precision-matrix");
    let before = fixture_hash(&root);
    let oracle: Value = serde_json::from_slice(
        &fs::read(root.join("oracle.json")).expect("precision oracle should be readable"),
    )
    .expect("precision oracle should be valid JSON");
    let output = inspect_json(&root);
    let report = parse_report(&output);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(report["status"], "complete");
    assert_eq!(
        report["project"]["packages"][0]["targets"],
        serde_json::json!(["aliases", "precision-matrix", "precision_matrix"])
    );

    let mut expected: Vec<_> = oracle["positive"]
        .as_array()
        .expect("positive oracle should be an array")
        .iter()
        .map(|case| {
            (
                case["code"].as_str().expect("oracle code").to_owned(),
                case["path"].as_str().expect("oracle path").to_owned(),
                case["line"].as_u64().expect("oracle line"),
                case["occurrences"].as_u64().expect("oracle occurrences"),
            )
        })
        .collect();
    let mut observed: Vec<_> = curated(&report)
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["code"]
                    .as_str()
                    .expect("diagnostic code")
                    .to_owned(),
                diagnostic["path"]
                    .as_str()
                    .expect("diagnostic path")
                    .to_owned(),
                diagnostic["span"]["line_start"]
                    .as_u64()
                    .expect("diagnostic line"),
                diagnostic["occurrences"]
                    .as_u64()
                    .expect("diagnostic occurrences"),
            )
        })
        .collect();
    expected.sort();
    observed.sort();
    assert_eq!(observed, expected);
    assert_eq!(observed.len(), 9);

    let negatives = oracle["negative"]
        .as_array()
        .expect("negative oracle should be an array");
    assert_eq!(negatives.len(), 12);
    let mut negative_kinds: Vec<_> = negatives
        .iter()
        .map(|case| case["kind"].as_str().expect("negative kind"))
        .collect();
    negative_kinds.sort_unstable();
    assert_eq!(
        negative_kinds,
        [
            "allow",
            "allow",
            "allow",
            "comment",
            "comment",
            "comment",
            "neighbor_macro",
            "neighbor_macro",
            "neighbor_macro",
            "string",
            "string",
            "string",
        ]
    );
    let fixture_source = fs::read_to_string(root.join("src/lib.rs"))
        .expect("negative matrix source should be readable");
    assert!(negatives.iter().all(|case| {
        fixture_source.contains(case["marker"].as_str().expect("negative marker"))
    }));
    assert!(oracle["positive"].as_array().is_some_and(|cases| {
        cases
            .iter()
            .all(|case| case.get("message").is_none() && case.get("rendered").is_none())
    }));
    assert_eq!(fixture_hash(&root), before);
}

#[test]
fn one_inspection_drives_json_and_terminal_for_the_same_diagnostics() {
    let workspace = fixture("precision-matrix");
    let report = rust_doctor::inspect(rust_doctor::InspectRequest::new(&workspace));
    let mut json = Vec::new();
    let mut terminal = Vec::new();
    rust_doctor::render::render_json(&report, &mut json).expect("JSON rendering should succeed");
    rust_doctor::render::render_terminal_with_options(
        &report,
        &mut terminal,
        rust_doctor::render::TerminalOptions {
            workspace_root: &workspace,
            elapsed: std::time::Duration::ZERO,
            verbose: true,
            width: 140,
            color: false,
            animate: false,
        },
    )
    .expect("terminal rendering should succeed");

    let rendered: Value = serde_json::from_slice(&json).expect("rendered JSON should parse");
    let rendered_ids = diagnostic_ids(&rendered, false);
    let report_ids: BTreeSet<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.id.clone())
        .collect();
    assert_eq!(rendered_ids, report_ids);

    let terminal = String::from_utf8(terminal).expect("terminal output should be UTF-8");
    for diagnostic in &report.diagnostics {
        let path = diagnostic.path.as_deref().unwrap_or("<unknown>");
        let (line, column) = diagnostic
            .span
            .as_ref()
            .map_or((0, 0), |span| (span.line_start, span.column_start));
        let location = format!("{path}:{line}:{column}");
        assert!(terminal.contains(&location), "missing {location}");
        assert!(terminal.contains(&diagnostic.message));
    }
}

#[test]
fn allow_and_correction_rescans_remove_only_the_targeted_ids() {
    let source = fixture("precision-matrix");
    let allowed = temporary_fixture("precision-matrix-allowed");
    copy_fixture(&source, &allowed);
    for relative in ["src/lib.rs", "src/main.rs", "tests/aliases.rs"] {
        prepend_crate_allow(&allowed.join(relative));
    }
    let allowed_output = inspect_json(&allowed);
    let allowed_report = parse_report(&allowed_output);
    assert_eq!(allowed_output.status.code(), Some(0));
    assert!(curated(&allowed_report).is_empty());
    fs::remove_dir_all(&allowed).expect("allowed fixture copy should be removable");

    let corrected = temporary_fixture("precision-matrix-corrected");
    copy_fixture(&source, &corrected);
    let initial_output = inspect_json(&corrected);
    let initial = parse_report(&initial_output);
    assert_eq!(initial_output.status.code(), Some(0));
    let targeted_ids = diagnostic_ids(&initial, true);
    let untargeted_ids: BTreeSet<_> = diagnostic_ids(&initial, false)
        .difference(&targeted_ids)
        .cloned()
        .collect();
    assert_eq!(targeted_ids.len(), 9);

    replace_in_file(
        &corrected.join("src/précision.rs"),
        &[
            ("dbg!(value)", "value"),
            ("todo!(\"précision\")", "7"),
            ("unimplemented!(\"précision\")", "9"),
        ],
    );
    replace_in_file(
        &corrected.join("src/main.rs"),
        &[
            ("std::dbg!(value)", "value"),
            ("std::todo!(\"qualified\")", "7"),
            ("std::unimplemented!(\"qualified\")", "9"),
        ],
    );
    replace_in_file(
        &corrected.join("tests/aliases.rs"),
        &[
            (
                "use std::{dbg as aliased_dbg, todo as aliased_todo, unimplemented as aliased_unimplemented};",
                "",
            ),
            ("aliased_dbg!(value)", "value"),
            ("aliased_todo!(\"aliased\")", "7"),
            ("aliased_unimplemented!(\"aliased\")", "9"),
        ],
    );

    let rescanned_output = inspect_json(&corrected);
    let rescanned = parse_report(&rescanned_output);
    assert_eq!(rescanned_output.status.code(), Some(0));
    assert!(curated(&rescanned).is_empty());
    let rescanned_ids = diagnostic_ids(&rescanned, false);
    assert!(targeted_ids.is_disjoint(&rescanned_ids));
    assert_eq!(rescanned_ids, untargeted_ids);
    fs::remove_dir_all(corrected).expect("corrected fixture copy should be removable");
}

#[test]
fn deny_and_compile_failure_retain_prior_findings_and_unique_causes() {
    let denied_output = inspect_json(&fixture("denied"));
    let denied = parse_report(&denied_output);
    assert_eq!(denied_output.status.code(), Some(1));
    assert_eq!(denied["status"], "incomplete");
    assert!(
        curated(&denied)
            .iter()
            .any(|diagnostic| diagnostic["severity"] == "error")
    );

    let failed_output = inspect_json(&fixture("partial-failure"));
    let failed = parse_report(&failed_output);
    assert_eq!(failed_output.status.code(), Some(1));
    assert_eq!(failed["status"], "incomplete");
    assert!(
        curated(&failed)
            .iter()
            .any(|diagnostic| diagnostic["code"] == "clippy::todo")
    );
    assert!(failed["diagnostics"].as_array().is_some_and(|diagnostics| {
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "E0308")
    }));

    let errors = failed["errors"]
        .as_array()
        .expect("failure causes should be an array");
    let unique: BTreeSet<_> = errors
        .iter()
        .map(|error| {
            (
                error["stage"].as_str().expect("error stage"),
                error["code"].as_str().expect("error code"),
                error["message"].as_str().expect("error message"),
            )
        })
        .collect();
    assert_eq!(unique.len(), errors.len());
    assert!(
        unique
            .iter()
            .any(|(stage, code, _)| *stage == "execution" && *code == "clippy-exit")
    );
}

#[test]
fn pinned_repository_evaluation_is_complete_trusted_and_offline_only() {
    let artifact: Value = serde_json::from_str(include_str!(
        "../tasks/rust-doctor-curated-rule-kernel-evaluation.json"
    ))
    .expect("real-world evaluation artifact should be valid JSON");
    let repositories = artifact["repositories"]
        .as_array()
        .expect("evaluated repositories should be an array");

    assert_eq!(repositories.len(), 5);
    assert_eq!(artifact["network_in_automated_tests"], false);
    assert_eq!(artifact["verdict"], "pass");
    assert!(repositories.iter().all(|repository| {
        repository["url"]
            .as_str()
            .is_some_and(|url| url.starts_with("https://github.com/"))
            && repository["commit"]
                .as_str()
                .is_some_and(|commit| commit.len() == 40)
            && repository["trusted"] == true
            && repository["build_code_warning_acknowledged"] == true
            && repository["scan"]["status"] == "complete"
            && repository["scan"]["exit_code"] == 0
            && repository["scan"]["counts"]
                .as_object()
                .is_some_and(|counts| counts.len() == 3 && counts.values().all(|count| count == 0))
            && repository["scan"]["curated_findings"]
                .as_array()
                .is_some_and(Vec::is_empty)
            && repository["scan"]["manual_verdicts"]
                .as_array()
                .is_some_and(Vec::is_empty)
            && repository["scan"]["false_positives"] == 0
            && repository["scan"]["ambiguous"] == 0
            && repository["repository_state"]["before"] == "clean"
            && repository["repository_state"]["after"] == "clean"
            && repository["repository_state"]["tracked_files_unchanged"] == true
    }));
    assert!(
        repositories
            .iter()
            .any(|repository| repository["cargo_shape"]["library"] == true)
    );
    assert!(
        repositories
            .iter()
            .any(|repository| repository["cargo_shape"]["binary"] == true)
    );
    assert!(repositories.iter().any(|repository| {
        repository["cargo_shape"]["workspace_members"]
            .as_u64()
            .is_some_and(|members| members >= 2)
    }));
    assert!(
        repositories
            .iter()
            .any(|repository| repository["cargo_shape"]["proc_macro"] == true)
    );

    let serialized = serde_json::to_string(&artifact).expect("artifact should serialize");
    assert!(!serialized.contains("/home/"));
    assert!(!serialized.contains("/tmp/"));
    assert_eq!(artifact["totals"]["repositories"], 5);
    assert_eq!(artifact["totals"]["complete_scans"], 5);
    assert_eq!(artifact["totals"]["tracked_repositories_unchanged"], 5);
    assert_eq!(artifact["totals"]["false_positives"], 0);
    assert_eq!(artifact["totals"]["ambiguous"], 0);
}
