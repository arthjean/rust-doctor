#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

struct Oracle {
    fixture: &'static str,
    code: &'static str,
    primary_lines: &'static [u64],
}

const ORACLES: [Oracle; 3] = [
    Oracle {
        fixture: "dbg-macro",
        code: "clippy::dbg_macro",
        primary_lines: &[4, 8, 12],
    },
    Oracle {
        fixture: "todo",
        code: "clippy::todo",
        primary_lines: &[4, 8, 12],
    },
    Oracle {
        fixture: "unimplemented",
        code: "clippy::unimplemented",
        primary_lines: &[4, 8, 12],
    },
];

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kernel-contract")
}

fn target_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/kernel-contract")
}

fn clippy(fixture: &str, arguments: &[&str]) -> Output {
    Command::new("cargo")
        .current_dir(fixture_root().join(fixture))
        .env("CARGO_TARGET_DIR", target_root())
        .args(arguments)
        .output()
        .expect("Clippy contract process should start")
}

fn records(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn diagnostics<'a>(records: &'a [Value], code: &str) -> Vec<&'a Value> {
    records
        .iter()
        .filter(|record| {
            record["reason"] == "compiler-message" && record["message"]["code"]["code"] == code
        })
        .collect()
}

#[test]
fn target_clippy_contract_matches_the_curated_oracle_matrix() {
    let version = Command::new("cargo")
        .args(["clippy", "--version"])
        .output()
        .expect("Clippy version process should start");
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).starts_with("clippy 0.1.97 "),
        "unexpected target toolchain: {}",
        String::from_utf8_lossy(&version.stdout)
    );

    for oracle in ORACLES {
        let output = clippy(
            oracle.fixture,
            &["clippy", "--message-format=json", "--", "-W", oracle.code],
        );
        assert!(
            output.status.success(),
            "{}: {}",
            oracle.fixture,
            String::from_utf8_lossy(&output.stderr)
        );
        let records = records(&output);
        let findings = diagnostics(&records, oracle.code);
        let mut primary_lines: Vec<_> = findings
            .iter()
            .map(|finding| {
                finding["message"]["spans"]
                    .as_array()
                    .and_then(|spans| spans.iter().find(|span| span["is_primary"] == true))
                    .and_then(|span| span["line_start"].as_u64())
                    .expect("curated diagnostic should carry a primary span")
            })
            .collect();
        primary_lines.sort_unstable();

        assert_eq!(findings.len(), 3, "{}", oracle.fixture);
        assert!(
            findings
                .iter()
                .all(|finding| finding["message"]["level"] == "warning"),
            "{}",
            oracle.fixture
        );
        assert_eq!(primary_lines, oracle.primary_lines, "{}", oracle.fixture);
    }

    let allowed = clippy(
        "allowed",
        &[
            "clippy",
            "--message-format=json",
            "--",
            "-W",
            "clippy::dbg_macro",
            "-W",
            "clippy::todo",
            "-W",
            "clippy::unimplemented",
        ],
    );
    assert!(
        allowed.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    let allowed_records = records(&allowed);
    assert!(
        ORACLES
            .iter()
            .all(|oracle| { diagnostics(&allowed_records, oracle.code).is_empty() })
    );

    let denied = clippy(
        "denied",
        &[
            "clippy",
            "--message-format=json",
            "--",
            "-W",
            "clippy::todo",
        ],
    );
    assert!(!denied.status.success());
    let denied_records = records(&denied);
    let finding_position = denied_records
        .iter()
        .position(|record| {
            record["reason"] == "compiler-message"
                && record["message"]["code"]["code"] == "clippy::todo"
                && record["message"]["level"] == "error"
        })
        .expect("denied lint should emit a structured error");
    let completion_position = denied_records
        .iter()
        .position(|record| record["reason"] == "build-finished" && record["success"] == false)
        .expect("denied lint should emit failed build completion");
    assert!(finding_position < completion_position);

    let dependency = clippy(
        "dependency-app",
        &[
            "clippy",
            "--no-deps",
            "--message-format=json",
            "--",
            "-W",
            "clippy::todo",
        ],
    );
    assert!(
        dependency.status.success(),
        "{}",
        String::from_utf8_lossy(&dependency.stderr)
    );
    assert!(diagnostics(&records(&dependency), "clippy::todo").is_empty());
}
