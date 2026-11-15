#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fs;
use std::path::{Path, PathBuf};

mod support;

const HOME: &str = "src/path_form.rs";

fn sources(directory: &Path, found: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

fn bare_resolutions(name: &str, source: &str) -> Vec<String> {
    let method = [".canonical", "ize("].concat();
    let function = ["fs::canonical", "ize"].concat();
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(&method) || line.contains(&function))
        .map(|(index, line)| format!("{name}:{}: {}", index + 1, line.trim()))
        .collect()
}

#[test]
fn only_path_form_resolves_a_path_through_the_file_system() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    assert!(files.len() > 100, "src/ was not walked: {} files", files.len());

    let mut bare = Vec::new();
    let mut home_resolves = false;
    for file in files {
        let name = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let found = bare_resolutions(&name, &fs::read_to_string(&file).unwrap());
        if name == HOME {
            home_resolves = !found.is_empty();
        } else {
            bare.extend(found);
        }
    }
    assert!(home_resolves, "{HOME} no longer resolves paths, so this check guards nothing");
    assert!(
        bare.is_empty(),
        "a path resolved outside {HOME} keeps the verbatim form Windows returns; call path_form::canonical:\n{}",
        bare.join("\n")
    );
}

#[test]
fn both_spellings_of_a_bare_resolution_are_caught_with_their_line() {
    let method = ["fn f(p: &std::path::Path) {\n    let _ = p.canonical", "ize();\n}\n"].concat();
    assert_eq!(
        bare_resolutions("src/a.rs", &method),
        [["src/a.rs:2: let _ = p.canonical", "ize();"].concat()]
    );
    let function = ["use std::fs;\nfn f() {\n    let _ = fs::canonical", "ize(\".\");\n}\n"].concat();
    assert_eq!(bare_resolutions("src/b.rs", &function).len(), 1);
    assert_eq!(
        bare_resolutions("src/c.rs", "fn f() {\n    let _ = path_form::canonical(\".\");\n}\n"),
        Vec::<String>::new()
    );
}

#[cfg(windows)]
#[test]
fn members_that_depend_on_each_other_by_path_stay_inside_their_workspace() {
    use std::sync::atomic::AtomicUsize;

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    const RULE: &str = "rust_doctor::cargo::path_dependency_outside_workspace";

    let workspace = support::fresh_workspace("path-members", &NEXT);
    support::copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cargo-health/path-members"),
        &workspace,
    );
    let output = support::rust_doctor()
        .current_dir(&workspace)
        .args([".", "--yes", "--json"])
        .args(support::NO_ADVISORY_DB)
        .env("CARGO_TARGET_DIR", support::scan_target(&workspace))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    let parsed = serde_json::from_slice::<serde_json::Value>(&output.stdout);
    assert!(parsed.is_ok(), "{}", String::from_utf8_lossy(&output.stderr));
    let report = parsed.unwrap();

    assert_eq!(report["status"], "complete", "{}", report["errors"]);
    let policy = report["policy"]["rules"].as_array().unwrap();
    let rule = policy.iter().find(|rule| rule["id"] == RULE).unwrap();
    assert_ne!(rule["level"], "off", "the rule was not evaluated");
    let outside: Vec<_> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == RULE)
        .collect();
    assert!(outside.is_empty(), "{outside:?}");
    let members: Vec<&str> = report["project"]["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|package| package["manifest_path"].as_str())
        .collect();
    assert_eq!(
        members,
        ["crates/app/Cargo.toml", "crates/cli/Cargo.toml", "crates/core/Cargo.toml"]
    );
    fs::remove_dir_all(workspace).unwrap();
}
