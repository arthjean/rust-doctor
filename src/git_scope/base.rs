use crate::git::{GitCall, GitFailure};
use crate::internal_error::InternalError;

use super::{BaseSelector, OID_OUTPUT_LIMIT, STAGE, scope_call};

use std::path::Path;

const FALLBACK_CANDIDATES: [&str; 4] = ["origin/main", "origin/master", "main", "master"];

const PROBE_REFUSED: GitFailure = GitFailure::new("probe-refused", "Git refused the probe.");

pub(super) fn base_undetected() -> InternalError {
    InternalError::new(
        STAGE,
        "base-undetected",
        "No base branch found: pass --base <REF>.",
    )
}

pub(super) fn shallow_clone() -> InternalError {
    InternalError::new(
        STAGE,
        "shallow-clone",
        "This clone is shallow: fetch with fetch-depth: 0, or pass --base to a fetched ref.",
    )
}

pub(super) fn detect(
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<BaseSelector, InternalError> {
    let (candidate, selector) = first_resolving(workspace_root, run).ok_or_else(base_undetected)?;
    let branch = candidate.strip_prefix("origin/").unwrap_or(&candidate);
    let checked_out = probe(
        workspace_root,
        run,
        ["symbolic-ref", "--quiet", "--short", "HEAD"],
    );
    Ok(if checked_out.as_deref() == Some(branch) {
        BaseSelector::head()
    } else {
        selector
    })
}

pub(super) fn default_branch(
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Option<String> {
    let (candidate, _) = first_resolving(workspace_root, run)?;
    Some(
        candidate
            .strip_prefix("origin/")
            .unwrap_or(&candidate)
            .to_owned(),
    )
}

fn first_resolving(
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Option<(String, BaseSelector)> {
    let remote_head = probe(
        workspace_root,
        run,
        ["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    )
    .and_then(|target| target.strip_prefix("refs/remotes/").map(str::to_owned));
    let candidates = remote_head
        .into_iter()
        .chain(FALLBACK_CANDIDATES.map(str::to_owned));
    for candidate in candidates {
        let Ok(selector) = BaseSelector::new(&candidate) else {
            continue;
        };
        let revision = format!("{candidate}^{{commit}}");
        let resolves = probe(
            workspace_root,
            run,
            [
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                revision.as_str(),
            ],
        )
        .is_some();
        if resolves {
            return Some((candidate, selector));
        }
    }
    None
}

pub(super) fn is_shallow(
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> bool {
    probe(
        workspace_root,
        run,
        ["rev-parse", "--is-shallow-repository"],
    )
    .as_deref()
        == Some("true")
}

fn probe<const N: usize>(
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
    operation: [&str; N],
) -> Option<String> {
    let answer = run(&scope_call(
        workspace_root,
        operation,
        OID_OUTPUT_LIMIT,
        PROBE_REFUSED,
    ))
    .ok()?;
    let answer = String::from_utf8(answer).ok()?;
    let answer = answer.trim();
    (!answer.is_empty() && !answer.contains('\n')).then(|| answer.to_owned())
}
