#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use cargo_metadata::MetadataCommand;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum RuleLevel {
    Off,
    Warn,
    Error,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum BlockingLevel {
    None,
    Error,
    Warning,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigurationDocument {
    blocking: Option<BlockingLevel>,
    #[serde(default)]
    categories: BTreeMap<String, RuleLevel>,
    #[serde(default)]
    rules: BTreeMap<String, RuleLevel>,
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/configuration-kernel/workspace")
}

fn version(program: impl AsRef<OsStr>, arguments: &[&str]) -> String {
    let output = Command::new(program).args(arguments).output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn versioned_oracle_matches_the_observed_toolchain_and_manifest() {
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/configuration-kernel/oracle.json")).unwrap();
    let manifest: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let dependencies = manifest["dependencies"].as_table().unwrap();
    let toml_dependency = dependencies["toml"].as_table().unwrap();
    let rustc = version(
        std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()),
        &["--version"],
    );
    let cargo = version(env!("CARGO"), &["--version"]);
    let clippy = version("clippy-driver", &["--version"]);

    assert_eq!(oracle["epic"], "EP-011");
    assert_eq!(oracle["story"], "US-030");
    assert_eq!(oracle["verdict"], "pass");
    assert_eq!(manifest["package"]["rust-version"].as_str(), Some("1.95"));
    assert_eq!(dependencies["clap"]["version"].as_str(), Some("=4.6.4"));
    assert_eq!(dependencies["serde"]["version"].as_str(), Some("=1.0.229"));
    assert_eq!(toml_dependency["version"].as_str(), Some("=1.1.4"));
    assert_eq!(toml_dependency["default-features"].as_bool(), Some(false));
    assert_eq!(
        toml_dependency["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|feature| feature.as_str().unwrap())
            .collect::<Vec<_>>(),
        ["parse", "serde", "std"]
    );
    assert_eq!(oracle["toolchain"]["clap"], "4.6.4");
    assert_eq!(oracle["toolchain"]["serde"], "1.0.229");
    assert_eq!(oracle["toolchain"]["toml"], "1.1.4");
    assert_eq!(cargo, oracle["toolchain"]["cargo"]);
    assert_eq!(clippy, oracle["toolchain"]["clippy"]);
    assert!(
        rustc == oracle["toolchain"]["rustc"] || rustc == oracle["msrv_verification"]["rustc"],
        "unexpected rustc selected: {rustc}"
    );
    if rustc == oracle["msrv_verification"]["rustc"] {
        assert_eq!(cargo, oracle["msrv_verification"]["cargo_driver"]);
    }
}

#[test]
fn real_virtual_and_member_manifests_resolve_the_oracle_workspace_with_no_deps() {
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/configuration-kernel/oracle.json")).unwrap();
    let workspace = fixture().canonicalize().unwrap();
    let cases = [
        ("workspace", workspace.join("Cargo.toml")),
        ("member/Cargo.toml", workspace.join("member/Cargo.toml")),
    ];

    for (entry, manifest) in cases {
        let expected = oracle["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["entry"] == entry)
            .unwrap();
        let metadata = MetadataCommand::new()
            .manifest_path(&manifest)
            .current_dir(manifest.parent().unwrap())
            .no_deps()
            .exec()
            .unwrap();
        assert_eq!(metadata.workspace_root.as_std_path(), workspace);
        assert!(metadata.resolve.is_none());
        assert_eq!(
            manifest.strip_prefix(&workspace).unwrap(),
            Path::new(expected["selected_manifest"].as_str().unwrap())
        );
        assert_eq!(expected["workspace_root"], ".");
        assert_eq!(expected["metadata_invocations"], 1);
        assert_eq!(expected["no_deps"], true);
    }
}

#[test]
fn toml_serde_and_guard_boundaries_match_the_closed_oracle_contract() {
    let document: ConfigurationDocument = toml::from_str(
        r#"blocking = "warning"

[categories]
security = "error"
correctness = "off"

[rules]
"rust_doctor::source::dynamic_shell_command" = "error"
"clippy::todo" = "off"
"#,
    )
    .unwrap();
    assert_eq!(document.blocking, Some(BlockingLevel::Warning));
    assert_eq!(
        document
            .categories
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["correctness", "security"]
    );
    assert_eq!(
        document
            .rules
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["clippy::todo", "rust_doctor::source::dynamic_shell_command"]
    );
    let serialized = serde_json::to_string(&document).unwrap();
    assert!(serialized.find("correctness").unwrap() < serialized.find("security").unwrap());
    assert!(
        serialized.find("clippy::todo").unwrap()
            < serialized
                .find("rust_doctor::source::dynamic_shell_command")
                .unwrap()
    );

    for invalid in [
        "blocking = \"error\"\nblocking = \"none\"\n",
        "[rules]\n[rules]\n",
        "unknown = \"warn\"\n",
        "[rules.\"clippy::todo\"]\nunknown = \"warn\"\n",
        "blocking = \"warn\"\n",
    ] {
        assert!(toml::from_str::<ConfigurationDocument>(invalid).is_err());
    }

    let maximum = vec![b' '; 65_536];
    let guarded = vec![b' '; 65_537];
    assert!(std::str::from_utf8(&maximum).is_ok());
    assert_eq!(maximum.len(), 65_536);
    assert_eq!(&guarded[..65_537].len(), &65_537);
    let non_utf8 = vec![0xff];
    assert!(std::str::from_utf8(&non_utf8).is_err());
}

#[test]
fn toml_error_span_maps_to_a_bounded_utf8_line_and_column_without_content() {
    let document = "# précision\nblocking = \"invalid\"\n";
    let error = toml::from_str::<ConfigurationDocument>(document).unwrap_err();
    let offset = error.span().unwrap().start.min(document.len());
    assert!(document.is_char_boundary(offset));
    let prefix = &document[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap().chars().count() + 1;
    assert_eq!(line, 2);
    assert!((1..=document.lines().nth(1).unwrap().chars().count() + 1).contains(&column));
}
