#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::undocumented_unsafe_blocks))]

use std::env;
use std::path::Path;

use rust_doctor::{InspectRequest, RuleLevel, Status, inspect};
use serde_json::Value;

mod support;

fn isolate_target_directory() {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/policy-kernel-target");
    unsafe { env::set_var("CARGO_TARGET_DIR", &target) };
}

#[test]
fn default_policy_expands_the_clippy_command_and_preserves_representative_ids() {
    isolate_target_directory();
    let oracle: Value = serde_json::from_str(include_str!("fixtures/policy-gate/oracle.json"))
        .expect("policy oracle should be valid JSON");
    let report = inspect(InspectRequest::new(support::fixture_dir(
        "kernel-contract/precision-matrix",
    )));

    assert_eq!(report.status, Status::Complete);
    let command = report
        .scan
        .command
        .as_ref()
        .expect("complete scan should expose its command");
    let policy = report
        .policy
        .as_ref()
        .expect("a scan should publish its policy");
    let expected: Vec<String> = [
        "cargo",
        "clippy",
        "--workspace",
        "--no-deps",
        "--keep-going",
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
    .chain(["-W".to_owned(), "warnings".to_owned()])
    .collect();
    assert_eq!(
        expected.len(),
        11 + 2 * policy
            .rules
            .iter()
            .filter(|rule| rule.id.starts_with("clippy::") && rule.level != RuleLevel::Off)
            .count()
    );
    assert_eq!(command, &expected);

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
