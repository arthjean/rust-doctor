//! Tests of the Clippy pass, in a file of their own so that both halves of the
//! module stay under the size bound `oversized_unit` reports at.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::*;
use crate::policy::{CATEGORIES, PolicyInput};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

#[test]
fn arguments_prune_off_rules_but_keep_error_rules_at_warning() {
    let input = PolicyInput::default()
        .with_rule("clippy::dbg_macro", RuleLevel::Off)
        .with_rule("clippy::todo", RuleLevel::Error);
    let plan = PolicyPlan::compile(&input).expect("policy should compile");
    let arguments = arguments_for_plan(&plan);

    assert!(!arguments.contains(&"clippy::dbg_macro"));
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["-W", "clippy::todo"])
    );
    assert!(!arguments.contains(&"-D"));

    // Turning off every category turns off every rule, whatever the catalog
    // volume: the command then carries the silencing alone, so the scan reports
    // nothing rather than everything Clippy warns about.
    let all_off = CATEGORIES
        .iter()
        .fold(PolicyInput::default(), |input, category| {
            input.with_category(*category, RuleLevel::Off)
        });
    let all_off = PolicyPlan::compile(&all_off).expect("policy should compile");
    assert_eq!(
        arguments_for_plan(&all_off),
        [
            "clippy",
            "--workspace",
            "--no-deps",
            "--message-format=json",
            "--",
            "-A",
            "clippy::all",
        ]
    );
}

#[test]
fn the_command_carries_the_shipped_catalog_and_nothing_else() {
    let workspace = fixture("clean").canonicalize().unwrap();
    let plan = PolicyPlan::default();
    let arguments = arguments_for_plan(&plan);
    let built = command(
        Path::new("cargo"),
        &workspace,
        &arguments,
        None,
        &CommandEnvironment::default(),
    );
    let arguments: Vec<_> = built.get_args().collect();

    assert_eq!(built.get_program(), OsStr::new("cargo"));
    // The catalog is the only source of the list: the command must carry every
    // active Clippy rule, in catalog order, under `-W`.
    let expected: Vec<String> = [
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
        crate::policy::CATALOG
            .iter()
            .filter(|definition| definition.producer == Producer::Clippy)
            .flat_map(|definition| ["-W".to_owned(), definition.id.to_owned()]),
    )
    .collect();
    assert_eq!(
        arguments,
        expected
            .iter()
            .map(|argument| OsStr::new(argument.as_str()))
            .collect::<Vec<_>>()
    );
    for forbidden in ["clippy::restriction", "--force-warn", "-D"] {
        assert!(!arguments.contains(&OsStr::new(forbidden)));
    }
    // `clippy::all` appears once, and only to be switched off: raising the
    // whole group would flood the report with rules the catalog cannot explain.
    assert_eq!(
        arguments
            .windows(2)
            .filter(|pair| pair[1] == OsStr::new("clippy::all"))
            .map(|pair| pair[0])
            .collect::<Vec<_>>(),
        [OsStr::new("-A")]
    );
    assert_eq!(
        arguments
            .iter()
            .filter(|argument| **argument == OsStr::new("--"))
            .count(),
        1
    );
    assert_eq!(built.get_current_dir(), Some(workspace.as_path()));
}

/// `Disabled` is a complete outcome and `NotRun` is not: a policy that switches
/// Clippy off must not cost the report its authoritative flag.
#[test]
fn a_disabled_pass_is_complete_and_an_unstarted_one_is_not() {
    assert!(ClippyExecution::Disabled.is_complete());
    assert!(ClippyExecution::Disabled.has_outcome());
    assert!(!ClippyExecution::NotRun.is_complete());
    assert!(!ClippyExecution::NotRun.has_outcome());
}
