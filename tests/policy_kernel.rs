#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::Path;

use rust_doctor::{InspectRequest, Status, inspect};
use serde_json::Value;

#[test]
fn default_policy_preserves_the_versioned_clippy_command_and_representative_ids() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let oracle: Value = serde_json::from_str(include_str!("fixtures/policy-gate/oracle.json"))
        .expect("policy oracle should be valid JSON");
    let report = inspect(InspectRequest::new(
        root.join("tests/fixtures/kernel-contract/precision-matrix"),
    ));

    assert_eq!(report.status, Status::Complete);
    assert_eq!(
        serde_json::to_value(
            report
                .scan
                .command
                .as_ref()
                .expect("complete scan should expose its command")
        )
        .expect("command should serialize"),
        oracle["clippy_command"]
    );

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
