#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

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

#[test]
fn a_workspace_scan_prints_the_workspace_dimensions_and_its_scope() {
    let root = members();
    let report: Value = serde_json::from_slice(&run(&root, &["--json"])).unwrap();
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

#[test]
fn the_clippy_dependencies_phase_ends_at_the_last_dependency_artifact() {
    let (dependencies, linting) = clippy_phases("todo");
    assert_eq!(dependencies, 0, "no dependency, yet {dependencies} ms on them");
    assert!(linting > 0);
    let (dependencies, _) = clippy_phases("dependency-app");
    assert!(dependencies > 0, "a path dependency was compiled in no time");
}
