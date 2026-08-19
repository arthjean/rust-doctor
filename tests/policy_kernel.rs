#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::env;
use std::path::Path;

use rust_doctor::{InspectRequest, RuleLevel, Status, inspect};
use serde_json::Value;

/// Cargo's artifact cache is what this test measures against, so it must not
/// share one. `inspect` shells out to `cargo clippy`, and a fixture already
/// compiled under an inherited `CARGO_TARGET_DIR` replays with no warning at
/// all: the representative diagnostics simply are not there, and the failure
/// reads as a regression in the report rather than as a cache hit. Every other
/// test that runs Cargo pins the directory the same way.
///
/// This binary carries one test, so the variable is set for the process rather
/// than guarded per call.
fn isolate_target_directory() {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/policy-kernel-target");
    // Single test, single thread: nothing else reads the environment here.
    unsafe { env::set_var("CARGO_TARGET_DIR", &target) };
}

#[test]
fn default_policy_expands_the_clippy_command_and_preserves_representative_ids() {
    isolate_target_directory();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let oracle: Value = serde_json::from_str(include_str!("fixtures/policy-gate/oracle.json"))
        .expect("policy oracle should be valid JSON");
    let report = inspect(InspectRequest::new(
        root.join("tests/fixtures/kernel-contract/precision-matrix"),
    ));

    assert_eq!(report.status, Status::Complete);
    let command = report
        .scan
        .command
        .as_ref()
        .expect("complete scan should expose its command");
    // The catalog is the only source of the published list: the command must
    // carry every active Clippy rule of the policy, in published order.
    let policy = report
        .policy
        .as_ref()
        .expect("a scan should publish its policy");
    let expected: Vec<String> = [
        "cargo",
        "clippy",
        "--workspace",
        "--no-deps",
        "--message-format=json",
        "--",
        "-A",
        "clippy::all",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain(
        policy
            .rules
            .iter()
            .filter(|rule| rule.id.starts_with("clippy::") && rule.level != RuleLevel::Off)
            .flat_map(|rule| ["-W".to_owned(), rule.id.clone()]),
    )
    .collect();
    // Six base arguments since the scope is Cargo's default targets, then the
    // two that silence everything Clippy warns about by default, then one `-W`
    // per active Clippy rule of the policy. The count is derived from the
    // published policy, never frozen, so that widening the catalog does not
    // require editing this test.
    assert_eq!(
        expected.len(),
        8 + 2 * policy
            .rules
            .iter()
            .filter(|rule| rule.id.starts_with("clippy::") && rule.level != RuleLevel::Off)
            .count()
    );
    assert_eq!(command, &expected);

    // A single historical argument was removed, and it is named here rather
    // than erased from the oracle: `--all-targets` compiled tests, benches,
    // examples and build scripts, from which 69.9% of the pack findings came.
    // The scope is now Cargo's default targets. The test therefore proves two
    // things: all the rest of the historical command survives in order, and
    // that removal is deliberate rather than a drift.
    const WITHDRAWN: &str = "--all-targets";
    assert!(!command.iter().any(|argument| argument == WITHDRAWN));

    let mut remaining_command = command.iter();
    for historical_argument in oracle["clippy_command"]
        .as_array()
        .expect("historical command should be an array")
    {
        let historical_argument = historical_argument
            .as_str()
            .expect("historical command arguments should be strings");
        if historical_argument == WITHDRAWN {
            continue;
        }
        assert!(
            remaining_command.any(|argument| argument == historical_argument),
            "historical argument {historical_argument} should remain in order"
        );
    }

    let expected = oracle["representative_diagnostic_ids"]
        .as_object()
        .expect("representative IDs should be an object");
    for (code, id) in expected {
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code.as_deref() == Some(code)
                    && diagnostic.path.as_deref() == Some("src/main.rs")
            })
            .expect("each representative diagnostic should remain present");
        assert_eq!(diagnostic.id, id.as_str().expect("ID should be a string"));
    }
}
