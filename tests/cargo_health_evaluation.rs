#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;

use serde_json::Value;

#[test]
fn pinned_cargo_health_evaluation_is_trusted_private_and_not_executed_by_tests() {
    let artifact: Value = serde_json::from_str(include_str!(
        "../tasks/rust-doctor-cargo-health-kernel-evaluation.json"
    ))
    .expect("Cargo Health evaluation artifact should be valid JSON");
    let repositories = artifact["repositories"]
        .as_array()
        .expect("evaluated repositories should be an array");
    let expected = BTreeMap::from([
        ("anyhow", "18c2598afa0f996f56217ef128aa3a20ea1e9512"),
        ("hexyl", "abc20a380c8c2d9d76c1976222725d3211cef809"),
        ("log", "6e1735597bb21c5d979a077395df85e1d633e077"),
        ("serde_json", "efa66e3a1d61459ab2d325f92ebe3acbd6ca18b1"),
        ("thiserror", "72ae716e6d6a7f7fdabdc394018c745b4d39ca45"),
    ]);
    let actual: BTreeMap<_, _> = repositories
        .iter()
        .map(|repository| {
            (
                repository["name"]
                    .as_str()
                    .expect("repository name should be a string"),
                repository["commit"]
                    .as_str()
                    .expect("repository commit should be a string"),
            )
        })
        .collect();

    assert_eq!(actual, expected);
    assert_eq!(artifact["network_in_automated_tests"], false);
    assert_eq!(artifact["scan_network_mode"], "offline");
    assert_eq!(artifact["verdict"], "pass");
    assert!(repositories.iter().all(|repository| {
        repository["url"]
            .as_str()
            .is_some_and(|url| url.starts_with("https://github.com/"))
            && repository["trusted"] == true
            && repository["trust_rationale"]
                .as_str()
                .is_some_and(|rationale| !rationale.is_empty())
            && repository["build_code_warning_acknowledged"] == true
            && repository["cargo_shape"].is_object()
            && repository["scan"]["toolchain"]["cargo"] == "cargo 1.97.1 (c980f4866 2026-06-30)"
            && repository["scan"]["toolchain"]["rustc"] == "rustc 1.97.1 (8bab26f4f 2026-07-14)"
            && repository["scan"]["toolchain"]["clippy"] == "clippy 0.1.97 (8bab26f4f6 2026-07-14)"
            && repository["scan"]["command"]
                == serde_json::json!([
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
                    "clippy::todo",
                    "-W",
                    "clippy::unimplemented"
                ])
            && repository["scan"]["status"] == "complete"
            && repository["scan"]["exit_code"] == 0
            && repository["scan"]["counts"]
                .as_object()
                .is_some_and(|counts| {
                    counts.len() == 2
                        && counts["rust_doctor::cargo::unbounded_registry_dependency"] == 0
                        && counts["rust_doctor::cargo::unpinned_git_dependency"] == 0
                })
            && repository["scan"]["cargo_health_findings"]
                .as_array()
                .is_some_and(|findings| {
                    repository["scan"]["manual_verdicts"]
                        .as_array()
                        .is_some_and(|verdicts| verdicts.len() == findings.len())
                })
            && repository["scan"]["false_positives"] == 0
            && repository["scan"]["ambiguous"] == 0
            && repository["repository_state"]["before"] == "clean"
            && repository["repository_state"]["after"] == "clean"
            && repository["repository_state"]["tracked_files_unchanged"] == true
    }));

    for repository in repositories {
        let scan = serde_json::to_string(&repository["scan"]).expect("scan should serialize");
        for forbidden in ["/home/", "/tmp/", "file://", "git+", "registry+", "\u{1b}"] {
            assert!(!scan.contains(forbidden), "scan leaked {forbidden:?}");
        }
    }

    assert_eq!(artifact["totals"]["repositories"], 5);
    assert_eq!(artifact["totals"]["complete_scans"], 5);
    assert_eq!(artifact["totals"]["cargo_health_findings"], 0);
    assert_eq!(artifact["totals"]["false_positives"], 0);
    assert_eq!(artifact["totals"]["ambiguous"], 0);
    assert_eq!(artifact["totals"]["tracked_repositories_unchanged"], 5);
}
