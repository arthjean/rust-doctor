use std::fs;
use std::path::Path;

use crate::execution::RunDeadline;
use crate::git::{GitCall, GitFailure, OUTPUT_TOO_LARGE, git_arguments, run_git};
use crate::internal_error::InternalError;

pub(super) const DEPTH: usize = 4;
const LISTED: usize = 10;
const LISTING_LIMIT: usize = 1024 * 1024;

const SKIPPED: [&str; 2] = ["target", "node_modules"];

pub(super) fn workspaces_below(directory: &Path, deadline: Option<RunDeadline>) -> Vec<String> {
    let mut found = tracked(directory, deadline).unwrap_or_else(|| {
        let mut walked = Vec::new();
        walk(directory, "", 0, &mut walked);
        walked
    });
    found.retain(|relative| eligible(relative) && !git_directory_layout(&directory.join(relative)));
    found.sort();
    found.dedup();
    found
        .iter()
        .filter(|candidate| {
            !found
                .iter()
                .any(|other| candidate.strip_prefix(other.as_str()).is_some_and(|rest| rest.starts_with('/')))
        })
        .cloned()
        .collect()
}

pub(super) fn ambiguous(candidates: &[String]) -> InternalError {
    let mut names = candidates
        .iter()
        .take(LISTED)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if candidates.len() > LISTED {
        names.push_str(&format!(" and {} more", candidates.len() - LISTED));
    }
    InternalError::new(
        "discovery",
        "ambiguous-workspace",
        format!("Several Cargo workspaces under this path: {names}. Pass one of them."),
    )
}

fn tracked(directory: &Path, deadline: Option<RunDeadline>) -> Option<Vec<String>> {
    let call = GitCall {
        arguments: git_arguments(
            directory,
            [
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
                "--",
                ":(glob)**/Cargo.toml",
            ],
        ),
        stdout_limit: LISTING_LIMIT,
        stage: "discovery",
        failure: GitFailure::new("not-a-repository", "The path is not inside a git repository."),
        overflow: OUTPUT_TOO_LARGE,
    };
    let listed = run_git(Path::new("git"), directory, &call, deadline).ok()?;
    Some(
        listed
            .split(|byte| *byte == 0)
            .filter_map(|entry| std::str::from_utf8(entry).ok())
            .filter_map(|entry| entry.strip_suffix("/Cargo.toml"))
            .map(str::to_owned)
            .collect(),
    )
}

fn walk(directory: &Path, relative: &str, depth: usize, found: &mut Vec<String>) {
    if depth > 0 && directory.join("Cargo.toml").is_file() {
        found.push(relative.to_owned());
    }
    if depth == DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if skipped(&name) {
            continue;
        }
        let child = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        walk(&entry.path(), &child, depth + 1, found);
    }
}

fn eligible(relative: &str) -> bool {
    let components: Vec<&str> = relative.split('/').collect();
    !relative.is_empty()
        && components.len() <= DEPTH
        && components
            .iter()
            .all(|component| !component.is_empty() && !skipped(component))
}

fn git_directory_layout(directory: &Path) -> bool {
    directory.join("HEAD").is_file()
        && directory.join("objects").is_dir()
        && directory.join("refs").is_dir()
}

fn skipped(name: &str) -> bool {
    name.starts_with('.') || SKIPPED.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligibility_bounds_the_depth_and_skips_build_hidden_and_package_trees() {
        assert!(eligible("src-tauri"));
        assert!(eligible("a/b/c/d"));
        assert!(!eligible("a/b/c/d/e"));
        assert!(!eligible("target/debug/build"));
        assert!(!eligible("web/node_modules/x"));
        assert!(!eligible(".cargo/registry"));
        assert!(!eligible(""));
    }

    #[test]
    fn the_walk_finds_workspaces_within_its_bounds() {
        let root = crate::test_scratch::scratch("scan-target", "walk");
        for directory in [
            "",
            "app",
            "app/nested",
            "a/b/c/d",
            "a/b/c/d/e",
            ".hidden",
            "target",
            "web/node_modules/x",
        ] {
            fs::create_dir_all(root.join(directory)).unwrap();
            fs::write(root.join(directory).join("Cargo.toml"), "").unwrap();
        }
        let mut found = Vec::new();
        walk(&root, "", 0, &mut found);
        found.sort();
        assert_eq!(found, ["a/b/c/d", "app", "app/nested"]);
    }

    #[test]
    fn a_candidate_holding_a_git_directory_layout_is_refused() {
        let root = crate::test_scratch::scratch("scan-target", "embedded-git-dir");
        for directory in ["app", "vendor/tool/objects", "vendor/tool/refs/heads", "half/objects"] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        for manifest in ["app", "vendor/tool", "half"] {
            fs::write(root.join(manifest).join("Cargo.toml"), "").unwrap();
        }
        fs::write(root.join("vendor/tool/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(root.join("half/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert!(git_directory_layout(&root.join("vendor/tool")));
        assert!(!git_directory_layout(&root.join("half")));
        let initialized = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap();
        assert!(initialized.success());
        assert_eq!(workspaces_below(&root, None), ["app", "half"]);
    }

    #[test]
    fn an_ambiguous_discovery_names_ten_candidates_at_most() {
        let candidates: Vec<String> = (0..12).map(|index| format!("crate-{index:02}")).collect();
        let error = ambiguous(&candidates);
        assert_eq!((error.stage, error.code), ("discovery", "ambiguous-workspace"));
        assert!(error.message.contains("crate-09 and 2 more"), "{}", error.message);
        assert!(!error.message.contains("crate-10"), "{}", error.message);
    }
}
