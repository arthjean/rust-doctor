#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::Path;

use rust_doctor::{InspectRequest, RuleLevel, Status, inspect};
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
    // Six arguments de base depuis que le périmètre est celui des cibles par
    // défaut de Cargo, puis un `-W` par règle Clippy active de la policy. Le
    // compte est dérivé de la policy publiée, jamais figé, pour qu'un
    // élargissement du catalogue ne demande pas d'éditer ce test.
    assert_eq!(
        expected.len(),
        6 + 2 * policy
            .rules
            .iter()
            .filter(|rule| rule.id.starts_with("clippy::") && rule.level != RuleLevel::Off)
            .count()
    );
    assert_eq!(command, &expected);

    // Un seul argument historique a été retiré, et il est nommé ici plutôt que
    // gommé de l'oracle: `--all-targets` compilait tests, benchs, exemples et
    // scripts de construction, dont 69,9 % des findings du pack provenaient.
    // Le périmètre est désormais celui des cibles par défaut de Cargo. Le test
    // prouve donc deux choses: tout le reste de la commande historique survit
    // dans l'ordre, et ce retrait-là est délibéré et non une dérive.
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
