#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

struct ExpectedCapture {
    name: &'static str,
    exit_code: i32,
    build_finished: bool,
    minimum_diagnostics: usize,
}

fn protocol_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/protocol")
}

const CRATE_POLICY: &str = "#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]";
const CLIPPY_POLICY: &str = "allow-unwrap-in-tests = true\nallow-expect-in-tests = true\n";
const LINT_POLICY: [&str; 7] = [
    "panic = \"deny\"",
    "unimplemented = \"deny\"",
    "dbg_macro = \"deny\"",
    "todo = \"warn\"",
    "unwrap_used = \"warn\"",
    "expect_used = \"warn\"",
    "unwrap_in_result = \"warn\"",
];

#[test]
fn real_protocol_captures_record_status_completion_and_diagnostics() {
    let expectations = [
        ExpectedCapture {
            name: "clean",
            exit_code: 0,
            build_finished: true,
            minimum_diagnostics: 0,
        },
        ExpectedCapture {
            name: "clippy-warning",
            exit_code: 0,
            build_finished: true,
            minimum_diagnostics: 1,
        },
        ExpectedCapture {
            name: "compile-error",
            exit_code: 101,
            build_finished: false,
            minimum_diagnostics: 1,
        },
    ];

    for expected in expectations {
        verify_capture(&expected);
    }
}

fn verify_capture(expected: &ExpectedCapture) {
    let root = protocol_root();
    let corpus = fs::read_to_string(root.join(format!("{}.jsonl", expected.name)))
        .expect("protocol corpus should be readable");
    let absolute_fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(expected.name);
    assert!(!corpus.contains(&absolute_fixture_root.display().to_string()));
    assert!(corpus.contains("<workspace>"));

    let messages: Vec<Value> = corpus
        .lines()
        .map(|line| serde_json::from_str(line).expect("captured line should be valid JSON"))
        .collect();
    let diagnostics = messages
        .iter()
        .filter(|message| message["reason"] == "compiler-message")
        .count();
    if expected.minimum_diagnostics == 0 {
        assert_eq!(diagnostics, 0);
    } else {
        assert!(diagnostics >= expected.minimum_diagnostics);
    }

    let finished = messages
        .iter()
        .rfind(|message| message["reason"] == "build-finished")
        .expect("capture should contain build-finished");
    assert_eq!(finished["success"], expected.build_finished);

    if expected.name == "compile-error" {
        let first_diagnostic = messages
            .iter()
            .position(|message| message["reason"] == "compiler-message")
            .expect("compile-error should contain a diagnostic");
        let completion = messages
            .iter()
            .position(|message| message["reason"] == "build-finished")
            .expect("compile-error should contain build-finished");
        assert!(first_diagnostic < completion);
    }

    let exit_code = fs::read_to_string(root.join(format!("{}.exit-code", expected.name)))
        .expect("exit code should be readable");
    assert_eq!(
        exit_code
            .trim()
            .parse::<i32>()
            .expect("exit code should be numeric"),
        expected.exit_code
    );
}

#[test]
fn synthetic_non_json_line_is_documented_as_noise() {
    let corpus = fs::read_to_string(protocol_root().join("synthetic-noise.jsonl"))
        .expect("synthetic corpus should be readable");
    let (json_lines, noise_lines) =
        corpus
            .lines()
            .fold((0, 0), |(json, noise), line| match line.starts_with('{') {
                true => (json + 1, noise),
                false => (json, noise + 1),
            });

    assert_eq!(json_lines, 1);
    assert_eq!(noise_lines, 1);
}

#[test]
fn project_fixtures_share_the_required_lint_policy() {
    let projects = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/projects");
    for name in [
        "clean",
        "clippy-warning",
        "compile-error",
        "same_name_targets",
    ] {
        let root = projects.join(name);
        let manifest = fs::read_to_string(root.join("Cargo.toml"))
            .expect("standalone fixture manifest should be readable");
        assert!(manifest.contains("[lints.clippy]"), "{name}");
        for policy in LINT_POLICY {
            assert!(manifest.contains(policy), "{name}: {policy}");
        }
        assert_eq!(
            fs::read_to_string(root.join("clippy.toml"))
                .expect("standalone fixture Clippy policy should be readable"),
            CLIPPY_POLICY,
            "{name}"
        );
        assert!(
            fs::read_to_string(root.join("src/lib.rs"))
                .expect("standalone fixture crate root should be readable")
                .starts_with(CRATE_POLICY),
            "{name}"
        );
        if root.join("src/main.rs").is_file() {
            assert!(
                fs::read_to_string(root.join("src/main.rs"))
                    .expect("standalone fixture binary root should be readable")
                    .starts_with(CRATE_POLICY),
                "{name}"
            );
        }
    }

    let workspace = projects.join("virtual-workspace");
    let workspace_manifest = fs::read_to_string(workspace.join("Cargo.toml"))
        .expect("virtual workspace manifest should be readable");
    assert!(workspace_manifest.contains("[workspace.lints.clippy]"));
    for policy in LINT_POLICY {
        assert!(workspace_manifest.contains(policy), "{policy}");
    }
    assert_eq!(
        fs::read_to_string(workspace.join("clippy.toml"))
            .expect("workspace Clippy policy should be readable"),
        CLIPPY_POLICY
    );

    for member in [workspace.join("member"), projects.join("shared")] {
        assert!(
            fs::read_to_string(member.join("Cargo.toml"))
                .expect("workspace member manifest should be readable")
                .contains("[lints]\nworkspace = true")
        );
        assert!(
            fs::read_to_string(member.join("src/lib.rs"))
                .expect("workspace member crate root should be readable")
                .starts_with(CRATE_POLICY)
        );
    }
}
