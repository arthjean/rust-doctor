//! Finding the workspace below a path that has none above it.
//!
//! A JavaScript-first repository keeps its crate in a subdirectory, a Tauri
//! app in `src-tauri/`, a service in `backend/`, and a scan run at its root
//! found no `Cargo.toml` above it and stopped. Discovery now looks down as
//! react-doctor does: one candidate is scanned and named, several are listed
//! and refused, since picking one would judge the wrong code without saying so.
//!
//! The listing goes through `git ls-files` inside a repository, so an ignored
//! build directory or a vendored tree never qualifies, and through a bounded
//! walk outside one. Either way it stays inside the requested directory, never
//! descends more than [`DEPTH`] levels, and skips hidden directories, `target`
//! and `node_modules`.
//!
//! A candidate that is itself a git directory layout (`HEAD`, `objects/`,
//! `refs/`) is refused, as a candidate reached through a link is: a tree can
//! commit those files, and git run inside them would answer for the
//! repository they fabricate rather than for the clone that tracks them.

use std::fs;
use std::path::Path;

use crate::execution::RunDeadline;
use crate::git::{GitCall, GitFailure, OUTPUT_TOO_LARGE, git_arguments, run_git};
use crate::internal_error::InternalError;

/// How many directories below the requested one a manifest may sit.
pub(super) const DEPTH: usize = 4;
/// How many candidates an ambiguous discovery names.
const LISTED: usize = 10;
/// Bound on the listing git answers with: the pathspec keeps it to manifests.
const LISTING_LIMIT: usize = 1024 * 1024;

const SKIPPED: [&str; 2] = ["target", "node_modules"];

/// The directories below `directory` holding a workspace, relative to it and
/// spelled with `/`, sorted. A manifest below another one found is taken for
/// that one's member and dropped.
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

/// The refusal several candidates earn, naming at most [`LISTED`] of them.
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

/// The manifests git lists below `directory`, tracked or untracked but not
/// ignored, or `None` outside a repository.
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
        // A symbolic link is not followed: it could lead out of the directory
        // the reader asked about, or around in a cycle.
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

/// A candidate within the depth bound, through no skipped directory.
fn eligible(relative: &str) -> bool {
    let components: Vec<&str> = relative.split('/').collect();
    !relative.is_empty()
        && components.len() <= DEPTH
        && components
            .iter()
            .all(|component| !component.is_empty() && !skipped(component))
}

/// Whether `directory` holds what git takes for a repository of its own: a
/// `HEAD` file beside `objects/` and `refs/` directories.
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

    /// Outside a repository the walk answers: bounded in depth, blind to
    /// hidden, build and package trees, and to a manifest at the root itself.
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

    /// US-003: a candidate holding a git directory layout is never scanned,
    /// whether git lists it or the walk finds it.
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
        // Through git: the scratch lives under this repository's ignored
        // `target/`, so a repository of its own is what lists the manifests.
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
