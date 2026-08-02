#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::Path;

use rust_doctor::{InspectRequest, Status, inspect};
use serde_json::Value;

#[test]
fn default_policy_expands_the_clippy_command_and_preserves_representative_ids() {
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
    assert_eq!(
        command,
        &[
            "cargo",
            "clippy",
            "--workspace",
            "--all-targets",
            "--no-deps",
            "--message-format=json",
            "--",
            "-W",
            "clippy::dbg_macro",
            "-W",
            "clippy::mem_forget",
            "-W",
            "clippy::non_send_fields_in_send_ty",
            "-W",
            "clippy::permissions_set_readonly_false",
            "-W",
            "clippy::suspicious_command_arg_space",
            "-W",
            "clippy::todo",
            "-W",
            "clippy::unimplemented",
            "-W",
            "clippy::zombie_processes",
        ]
        .map(str::to_owned)
    );

    let mut remaining_command = command.iter();
    for historical_argument in oracle["clippy_command"]
        .as_array()
        .expect("historical command should be an array")
    {
        let historical_argument = historical_argument
            .as_str()
            .expect("historical command arguments should be strings");
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
