use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;

use crate::execution::RunDeadline;
use crate::git::{GitCall, GitFailure, git_arguments, run_git, run_git_with_input};

const STAGE: &str = "generated";

const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

const ATTRIBUTES: [&str; 2] = ["linguist-generated", "linguist-vendored"];

pub(crate) fn declared(workspace_root: &Path, deadline: Option<RunDeadline>) -> BTreeSet<String> {
    declared_with(workspace_root, |call, input| match input {
        None => run_git(Path::new("git"), workspace_root, call, deadline),
        Some(input) => {
            run_git_with_input(Path::new("git"), workspace_root, call, input, deadline)
        }
    })
}

fn declared_with<E>(
    workspace_root: &Path,
    mut run: impl FnMut(&GitCall, Option<Vec<u8>>) -> Result<Vec<u8>, E>,
) -> BTreeSet<String> {
    let tracked = call(git_arguments(workspace_root, ["ls-files", "-z"]));
    let Ok(tracked) = run(&tracked, None) else {
        return BTreeSet::new();
    };
    if tracked.is_empty() {
        return BTreeSet::new();
    }
    let [generated, vendored] = ATTRIBUTES;
    let attributes = call(git_arguments(
        workspace_root,
        ["check-attr", "-z", "--stdin", generated, vendored],
    ));
    run(&attributes, Some(tracked)).map_or_else(|_| BTreeSet::new(), |output| parse(&output))
}

fn call(arguments: Vec<OsString>) -> GitCall {
    GitCall {
        arguments,
        stdout_limit: MAX_OUTPUT_BYTES,
        stage: STAGE,
        failure: GitFailure::new("attributes-unavailable", "Git attributes could not be read."),
        overflow: GitFailure::new(
            "attributes-too-large",
            "Git attributes exceed the supported size.",
        ),
    }
}

fn parse(output: &[u8]) -> BTreeSet<String> {
    let fields: Vec<&[u8]> = output.split(|byte| *byte == 0).collect();
    let (answers, _) = fields.as_chunks::<3>();
    answers
        .iter()
        .filter_map(|[path, _, value]| {
            matches!(*value, b"set" | b"true")
                .then(|| std::str::from_utf8(path).ok().map(str::to_owned))
                .flatten()
        })
        .collect()
}

pub(crate) fn has_generator_header(source: &str) -> bool {
    source.lines().take(10).any(|line| {
        line.contains("@generated")
            || line.contains("DO NOT EDIT")
            || line.contains("Automatically generated")
            || line.contains("automatically generated")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_set_and_true_exclude_a_path() {
        let output = b"a.rs\0linguist-generated\0set\0a.rs\0linguist-vendored\0unspecified\0b.rs\0linguist-generated\0true\0c.rs\0linguist-generated\0unset\0d.rs\0linguist-vendored\0false\0e.rs\0linguist-vendored\0set\0";
        assert_eq!(
            parse(output).into_iter().collect::<Vec<_>>(),
            ["a.rs", "b.rs", "e.rs"]
        );
    }

    #[test]
    fn a_failing_git_leaves_nothing_declared() {
        let declared = declared_with(Path::new("."), |_, _| Err(()));
        assert!(declared.is_empty());
        let mut calls = 0;
        let declared = declared_with(Path::new("."), |_, input| {
            calls += 1;
            match input {
                None => Ok(b"a.rs\0".to_vec()),
                Some(_) => Err(()),
            }
        });
        assert!(declared.is_empty());
        assert_eq!(calls, 2);
    }
}
