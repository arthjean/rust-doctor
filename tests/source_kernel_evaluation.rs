#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;

use serde_json::Value;

#[test]
fn pinned_source_evaluation_is_complete_private_and_static() {
    let artifact: Value = serde_json::from_str(include_str!(
        "../tasks/rust-doctor-native-source-kernel-evaluation.json"
    ))
    .unwrap();
    let repositories = artifact["repositories"].as_array().unwrap();
    let expected = BTreeMap::from([
        (
            "anyhow",
            ("18c2598afa0f996f56217ef128aa3a20ea1e9512", 29, 211_279),
        ),
        (
            "hexyl",
            ("abc20a380c8c2d9d76c1976222725d3211cef809", 7, 112_569),
        ),
        (
            "log",
            ("6e1735597bb21c5d979a077395df85e1d633e077", 12, 185_462),
        ),
        (
            "serde_json",
            ("efa66e3a1d61459ab2d325f92ebe3acbd6ca18b1", 53, 687_591),
        ),
        (
            "thiserror",
            ("72ae716e6d6a7f7fdabdc394018c745b4d39ca45", 31, 140_879),
        ),
    ]);
    let actual: BTreeMap<_, _> = repositories
        .iter()
        .map(|repository| {
            (
                repository["name"].as_str().unwrap(),
                (
                    repository["commit"].as_str().unwrap(),
                    repository["scan"]["source_pairs"].as_u64().unwrap(),
                    repository["scan"]["bytes_parsed"].as_u64().unwrap(),
                ),
            )
        })
        .collect();
    assert_eq!(actual, expected);

    assert_eq!(artifact["epic"], "EP-008");
    assert_eq!(artifact["network_in_automated_tests"], false);
    assert_eq!(artifact["scan_network_mode"], "offline");
    assert_eq!(artifact["verdict"], "pass");
    assert_eq!(
        artifact["clippy_catalog"]["version"],
        "clippy 0.1.97 (8bab26f4f6 2026-07-14)"
    );
    for code in [
        "rust_doctor::source::disabled_tls_verification",
        "rust_doctor::source::dynamic_shell_command",
    ] {
        assert!(artifact["clippy_catalog"][code]["exact_equivalent"].is_null());
        assert!(artifact["clippy_catalog"][code]["candidate_lints"].is_array());
        assert!(
            artifact["clippy_catalog"][code]["conclusion"]
                .as_str()
                .is_some_and(|conclusion| !conclusion.is_empty())
        );
    }
    assert!(repositories.iter().all(|repository| {
        let scan = &repository["scan"];
        repository["trusted"] == true
            && repository["build_code_warning_acknowledged"] == true
            && repository["cargo_shape"].is_object()
            && scan["toolchain"]["cargo"] == "cargo 1.97.1 (c980f4866 2026-06-30)"
            && scan["toolchain"]["rustc"] == "rustc 1.97.1 (8bab26f4f 2026-07-14)"
            && scan["toolchain"]["clippy"] == "clippy 0.1.97 (8bab26f4f6 2026-07-14)"
            && scan["status"] == "complete"
            && scan["exit_code"] == 0
            && scan["source_pairs"] == scan["files_read"]
            && scan["source_pairs"].as_u64().is_some_and(|count| count > 0)
            && scan["bytes_parsed"].as_u64().is_some_and(|count| count > 0)
            && scan["source_errors"]
                .as_object()
                .is_some_and(|errors| errors.is_empty())
            && scan["counts"]["rust_doctor::source::disabled_tls_verification"] == 0
            && scan["counts"]["rust_doctor::source::dynamic_shell_command"] == 0
            && scan["source_findings"]
                .as_array()
                .is_some_and(|findings| findings.is_empty())
            && scan["manual_verdicts"].as_array().is_some_and(|verdicts| {
                verdicts.len() == scan["source_findings"].as_array().unwrap().len()
            })
            && scan["false_positives"] == 0
            && scan["ambiguous"] == 0
            && repository["repository_state"]["before"] == "clean"
            && repository["repository_state"]["after"] == "clean"
            && repository["repository_state"]["tracked_files_unchanged"] == true
            && repository["repository_state"]["source_files_unchanged"] == true
    }));

    let serialized = serde_json::to_string(&artifact).unwrap();
    for forbidden in [
        "http://",
        "https://",
        "file://",
        "/home/",
        "/tmp/",
        "credential=",
        "RD_SOURCE_",
        "\u{1b}",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "artifact leaked {forbidden:?}"
        );
    }

    assert_eq!(artifact["totals"]["repositories"], 5);
    assert_eq!(artifact["totals"]["complete_scans"], 5);
    assert_eq!(artifact["totals"]["source_pairs"], 132);
    assert_eq!(artifact["totals"]["files_read"], 132);
    assert_eq!(artifact["totals"]["bytes_parsed"], 1_337_780);
    assert_eq!(artifact["totals"]["source_errors"], 0);
    assert_eq!(artifact["totals"]["source_findings"], 0);
    assert_eq!(artifact["totals"]["false_positives"], 0);
    assert_eq!(artifact["totals"]["ambiguous"], 0);
    assert_eq!(artifact["totals"]["source_trees_unchanged"], 5);
    assert_eq!(
        artifact["rule_decision"]["rust_doctor::source::disabled_tls_verification"],
        "enabled"
    );
    assert_eq!(
        artifact["rule_decision"]["rust_doctor::source::dynamic_shell_command"],
        "enabled"
    );
}
