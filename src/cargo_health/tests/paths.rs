use std::path::PathBuf;

use cargo_metadata::Metadata;

use super::candidates_for;
use crate::cargo_health::CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE;
use crate::policy::PolicyPlan;

const VERBATIM_ROOT: &str = "\\\\?\\C:\\w";

fn workspace(dependency_path: &str) -> Metadata {
    let id = "path+file:///C:/w/crates/app#0.1.0";
    serde_json::from_value(serde_json::json!({
        "packages": [{
            "name": "app",
            "version": "0.1.0",
            "id": id,
            "license": null,
            "license_file": null,
            "description": null,
            "source": null,
            "dependencies": [{
                "name": "a",
                "source": null,
                "req": "*",
                "kind": null,
                "rename": null,
                "optional": false,
                "uses_default_features": true,
                "features": [],
                "target": null,
                "registry": null,
                "path": dependency_path,
            }],
            "targets": [],
            "features": {},
            "manifest_path": "C:\\w\\crates\\app\\Cargo.toml",
            "metadata": null,
            "publish": [],
            "authors": [],
            "categories": [],
            "keywords": [],
            "readme": null,
            "repository": null,
            "homepage": null,
            "documentation": null,
            "edition": "2024",
            "links": null,
            "default_run": null,
            "rust_version": null,
        }],
        "workspace_members": [id],
        "workspace_default_members": [id],
        "resolve": null,
        "target_directory": "C:\\w\\target",
        "version": 1,
        "workspace_root": VERBATIM_ROOT,
        "metadata": null,
    }))
    .expect("the metadata of a workspace under a verbatim root should decode")
}

#[test]
fn a_member_cargo_names_without_the_verbatim_prefix_is_inside_its_workspace() {
    let scan = crate::cargo_health::inspect(
        &workspace("C:\\w\\crates\\a"),
        &PolicyPlan::default(),
        None,
    );
    assert_eq!(
        candidates_for(&scan, &CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE),
        Vec::<String>::new(),
        "{:?}",
        scan.candidates
    );
}

#[test]
fn a_path_that_leaves_a_verbatim_workspace_root_is_still_named() {
    let scan = crate::cargo_health::inspect(
        &workspace("C:\\elsewhere\\a"),
        &PolicyPlan::default(),
        None,
    );
    assert_eq!(
        candidates_for(&scan, &CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE),
        ["Path dependency \"a\" resolves outside the workspace."]
    );
    let outside = scan
        .candidates
        .iter()
        .find(|candidate| candidate.definition.id == CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE.id)
        .expect("the candidate the assertion above counted");
    let expected: PathBuf = ["crates", "app", "Cargo.toml"].iter().collect();
    assert_eq!(outside.manifest_path.as_deref().map(PathBuf::from), Some(expected));
}
