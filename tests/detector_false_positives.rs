#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

mod support;

const ORPHAN: &str = "rust_doctor::structure::orphan_module_file";
const COMPLEX: &str = "rust_doctor::structure::complex_function";
const UNUSED: &str = "rust_doctor::cargo::unused_dependency";

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

fn workspace(fixture: &str) -> PathBuf {
    let root = support::fresh_workspace("detector-false-positives", &NEXT_WORKSPACE);
    support::copy_tree(&support::fixture_dir(fixture), &root);
    commit(&root);
    root
}

fn commit(root: &Path) {
    for arguments in [
        &["init", "--quiet"][..],
        &["add", "-A"],
        &[
            "-c",
            "user.name=Rust Doctor",
            "-c",
            "user.email=rust-doctor@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    ] {
        let output = support::git_output(root, arguments);
        assert!(output.status.success(), "git {arguments:?}: {output:?}");
    }
}

fn run(root: &Path, with_clippy: bool, json: bool) -> Vec<u8> {
    let mut arguments: Vec<String> = vec![root.to_str().unwrap().to_owned(), "--yes".to_owned()];
    if json {
        arguments.push("--json".to_owned());
    }
    arguments.extend(support::NO_ADVISORY_DB.iter().map(|value| (*value).to_owned()));
    if !with_clippy {
        for entry in rust_doctor::catalog() {
            if entry.id.starts_with("clippy::") {
                arguments.push("--rule".to_owned());
                arguments.push(format!("{}=off", entry.id));
            }
        }
    }
    let output = support::rust_doctor()
        .args(&arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(!output.stdout.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
    output.stdout
}

fn scan(root: &Path, with_clippy: bool) -> Value {
    serde_json::from_slice(&run(root, with_clippy, true)).unwrap()
}

fn linear(root: &Path, with_clippy: bool) -> String {
    let printed = String::from_utf8_lossy(&run(root, with_clippy, false)).into_owned();
    printed.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn paths_of(report: &Value, rule: &str) -> Vec<String> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == rule)
        .map(|diagnostic| diagnostic["path"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn a_module_a_local_macro_declares_is_walked_like_any_other() {
    let root = workspace("structure/macro-declared-modules");
    let report = scan(&root, false);

    assert_eq!(report["status"], "complete", "{report:#}");
    assert!(report["errors"].as_array().is_some_and(Vec::is_empty), "{:#}", report["errors"]);
    assert!(paths_of(&report, ORPHAN).is_empty(), "{:?}", paths_of(&report, ORPHAN));
    assert_eq!(
        report["audit"]["source_files"], 5,
        "src/lib.rs, the three production modules the macro declares and the probe crate's root"
    );
    assert_eq!(paths_of(&report, COMPLEX), ["src/gamma.rs"]);
    assert!(paths_of(&report, UNUSED).is_empty(), "{:?}", paths_of(&report, UNUSED));
}

#[test]
fn a_file_no_invocation_names_is_the_only_orphan() {
    let root = workspace("structure/macro-declared-modules");
    fs::write(root.join("src/stray.rs"), "pub fn stray() -> u32 {\n    0\n}\n").unwrap();
    commit(&root);
    let report = scan(&root, false);

    assert_eq!(paths_of(&report, ORPHAN), ["src/stray.rs"]);
}

const FEATURE: &str = "rust_doctor::structure::unreferenced_feature";

#[test]
fn a_feature_the_build_script_reads_is_referenced() {
    let root = workspace("structure/build-script-features");
    let report = scan(&root, false);
    let messages: Vec<&str> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == FEATURE)
        .map(|diagnostic| diagnostic["message"].as_str().unwrap_or_default())
        .collect();

    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(messages[0].contains("declares feature \"unused\""), "{messages:?}");
}

const BUILD_STEP: &str = "rust_doctor::repo::secret_in_build_step";

fn build_steps(report: &Value) -> Vec<(u64, String, String, String)> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == BUILD_STEP)
        .map(|diagnostic| {
            (
                diagnostic["span"]["line_start"].as_u64().unwrap_or_default(),
                diagnostic["severity"].as_str().unwrap_or_default().to_owned(),
                diagnostic["message"].as_str().unwrap_or_default().to_owned(),
                diagnostic["help"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_secret_the_build_embeds_is_reported_at_info_for_what_it_is() {
    let root = workspace("repo-hygiene/embedded-secret");
    let steps = build_steps(&scan(&root, false));
    assert_eq!(steps.len(), 3, "{steps:#?}");

    let (line, severity, message, help) = &steps[0];
    assert_eq!((*line, severity.as_str()), (11, "info"));
    assert_eq!(
        message,
        "`cargo build` compiles with `ANALYTICS_KEY` in scope, and `option_env!` at src/lib.rs:1 embeds it in the binary: whatever it holds ships to every user."
    );
    assert_eq!(
        help,
        "Keep only a value meant to be public in a variable the build embeds. If this one is secret, read it at run time instead."
    );

    let (line, severity, message, help) = &steps[1];
    assert_eq!((*line, severity.as_str()), (18, "warning"));
    assert!(message.contains("the step env holds the secret DEPLOY_TOKEN"), "{message}");
    assert_eq!(help, "Move the secret to the one step that needs it and compile in a step without it.");

    let (line, severity, message, help) = &steps[2];
    assert_eq!((*line, severity.as_str()), (27, "warning"), "a variable only test code embeds stays a secret");
    assert!(message.contains("TEST_ONLY_TOKEN"), "{message}");
    assert!(help.contains("OIDC") && help.contains("cargo publish"), "{help}");
}

#[test]
fn without_the_source_kernel_no_secret_is_demoted() {
    let root = workspace("repo-hygiene/embedded-secret");
    let mut arguments = vec![root.to_str().unwrap().to_owned(), "--yes".to_owned(), "--json".to_owned()];
    arguments.extend(support::NO_ADVISORY_DB.iter().map(|value| (*value).to_owned()));
    for entry in rust_doctor::catalog() {
        if entry.id.starts_with("clippy::") || entry.id.starts_with("rust_doctor::source::") {
            arguments.push("--rule".to_owned());
            arguments.push(format!("{}=off", entry.id));
        }
    }
    let output = support::rust_doctor()
        .args(&arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(&root))
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let steps = build_steps(&report);
    assert_eq!(steps.len(), 3, "{steps:#?}");
    assert!(steps.iter().all(|(_, severity, _, _)| severity == "warning"), "{steps:#?}");
    assert!(steps[0].2.contains("the step env holds the secret ANALYTICS_KEY"), "{steps:#?}");
}

const UNREASONED_ALLOW: &str = "rust_doctor::structure::unreasoned_allow_attribute";

#[test]
fn a_file_only_a_build_script_and_tests_reach_is_reported_as_build_script_code() {
    let root = workspace("structure/build-and-test-route");
    let report = scan(&root, false);
    let findings: Vec<&Value> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == UNREASONED_ALLOW)
        .collect();

    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert_eq!(findings[0]["path"], "src/shared.rs");
    assert_eq!(findings[0]["context"], "build-script", "{:#}", findings[0]);
    let printed = linear(&root, false);
    assert!(printed.contains("build script, not scored"), "{printed}");
}

const UNWALKED_LINE: &str = "2 compiled files were not read by the native rules: they are declared in a way the module walk cannot follow.";

#[test]
fn a_file_cargo_compiled_is_never_an_orphan_and_its_package_keeps_its_dependencies() {
    let root = workspace("structure/foreign-macro-modules");
    let report = scan(&root, true);

    assert_eq!(report["status"], "complete", "{:#}", report["errors"]);
    assert!(paths_of(&report, ORPHAN).is_empty(), "{:?}", paths_of(&report, ORPHAN));
    assert_eq!(report["scan"]["unwalked_compiled_files"], 2, "{:#}", report["scan"]);
    assert!(paths_of(&report, UNUSED).is_empty(), "{:?}", paths_of(&report, UNUSED));
    let printed = linear(&root, true);
    assert!(printed.contains(UNWALKED_LINE), "{printed}");
}

#[test]
fn without_a_clean_compilation_nothing_is_vetoed_and_nothing_is_counted() {
    let root = workspace("structure/foreign-macro-modules");
    let report = scan(&root, false);
    assert_eq!(paths_of(&report, ORPHAN), ["app/src/alpha.rs", "app/src/beta.rs"]);
    assert_eq!(paths_of(&report, UNUSED), ["app/Cargo.toml"]);
    assert!(report["scan"].get("unwalked_compiled_files").is_none(), "{:#}", report["scan"]);
    assert!(!linear(&root, false).contains("compiled files were not read"));

    let broken = workspace("structure/foreign-macro-modules");
    fs::write(broken.join("app/src/beta.rs"), "pub fn value() -> u32 {\n    missing()\n}\n").unwrap();
    commit(&broken);
    let report = scan(&broken, true);
    assert_eq!(paths_of(&report, ORPHAN), ["app/src/alpha.rs", "app/src/beta.rs"]);
    assert!(report["scan"].get("unwalked_compiled_files").is_none(), "{:#}", report["scan"]);
}
