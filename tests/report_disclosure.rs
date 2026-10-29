#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Proofs of EP-002 of the self-scan remediation, from the binary a user runs:
//! the linear report names the workspace's five dimensions, says what it
//! scored and what Clippy read, and names the members a `--package` scan
//! selected.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

mod support;

fn members() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/precision-controls/members")
        .canonicalize()
        .unwrap()
}

fn run(root: &Path, arguments: &[&str]) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg(root)
        .arg("--yes")
        .args(arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .output()
        .unwrap();
    output.stdout
}

fn dimensions_line(dimensions: &Value) -> String {
    format!(
        "Security {} · Reliability {} · Maintainability {} · Performance {} · Dependencies {}",
        dimensions["security"],
        dimensions["reliability"],
        dimensions["maintainability"],
        dimensions["performance"],
        dimensions["dependencies"]
    )
}

/// US-008 and US-009 on a two-member workspace: the dimension line is the
/// workspace score's, not a member's, and the scope line counts what was scored
/// and says Clippy linted the library and binary targets.
#[test]
fn a_workspace_scan_prints_the_workspace_dimensions_and_its_scope() {
    let root = members();
    let report: Value = serde_json::from_slice(&run(&root, &["--json"])).unwrap();
    // The linear report wraps at 80 columns on a captured stdout, and wraps on
    // spaces, so the rows are joined back before a sentence is looked for.
    let text = String::from_utf8(run(&root, &[])).unwrap().replace('\n', " ");

    let workspace = dimensions_line(&report["audit"]["score"]["dimensions"]);
    assert!(text.contains(&format!("└─────┘ {workspace} ")), "{text}");
    for member in report["audit"]["packages"].as_array().unwrap() {
        let own = dimensions_line(&member["score"]["dimensions"]);
        if own != workspace {
            assert!(!text.contains(&own), "a member's dimensions were printed: {text}");
        }
    }
    let scope = format!(
        "Scored {} production files ({} lines) in ",
        report["audit"]["source_files"], report["audit"]["production_lines"]
    );
    let rest = text.split_once(&scope).map(|(_, rest)| rest);
    assert!(rest.is_some(), "{text}");
    assert!(
        rest.unwrap()
            .split_once("s; ")
            .is_some_and(|(_, after)| after
                .starts_with("Clippy linted library and binary targets, native rules also read tests")),
        "{text}"
    );
}

/// US-009: a `--package` scan names the member it selected in the scope line.
#[test]
fn a_package_scan_names_the_selected_member() {
    let text = String::from_utf8(run(&members(), &["--package", "alpha"]))
        .unwrap()
        .replace('\n', " ");
    assert!(text.contains(" lines) of member alpha in "), "{text}");
    assert!(!text.contains("beta in "), "{text}");
}

fn clippy_phases(fixture: &str) -> (u64, u64) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/kernel-contract")
        .join(fixture)
        .canonicalize()
        .unwrap();
    let report: Value = serde_json::from_slice(&run(&root, &["--json", "--timings"])).unwrap();
    let timings = &report["timings"];
    (
        timings["clippy_dependencies"].as_u64().unwrap(),
        timings["clippy_linting"].as_u64().unwrap(),
    )
}

/// US-012: `clippy_dependencies` ends at the last artifact of a package
/// outside the members, so a workspace with no dependency spends none there,
/// and one with a path dependency spends Cargo's start and that crate there.
#[test]
fn the_clippy_dependencies_phase_ends_at_the_last_dependency_artifact() {
    let (dependencies, linting) = clippy_phases("todo");
    assert_eq!(dependencies, 0, "no dependency, yet {dependencies} ms on them");
    assert!(linting > 0);
    let (dependencies, _) = clippy_phases("dependency-app");
    assert!(dependencies > 0, "a path dependency was compiled in no time");
}
