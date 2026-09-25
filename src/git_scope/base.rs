//! The base a changed-work scope compares against when the caller named none,
//! and the diagnosis of a merge base that could not be found.
//!
//! The candidates are the ones a repository answers for itself, in the order
//! that trusts it most: what `origin/HEAD` points at, then the two names a
//! default branch takes on a remote, then the same two locally. The first one
//! git resolves to a commit wins. A skill that hard-coded `--base main` used to
//! fail on every `master` and `trunk` repository.

use crate::git::{GitCall, GitFailure};
use crate::internal_error::InternalError;

use super::{BaseSelector, OID_OUTPUT_LIMIT, STAGE, scope_call};

use std::path::Path;

const FALLBACK_CANDIDATES: [&str; 4] = ["origin/main", "origin/master", "main", "master"];

/// What a probe answers when git refuses. Every probe's refusal means "not this
/// one" rather than a failure of the scan, so its message is never published.
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

/// The first candidate git resolves to a commit, as the selector to compare
/// against: `HEAD` when that candidate is the branch checked out, since the
/// work to judge is then what has not been committed yet.
pub(super) fn detect(
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<BaseSelector, InternalError> {
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
        if !resolves {
            continue;
        }
        let branch = candidate.strip_prefix("origin/").unwrap_or(&candidate);
        let checked_out = probe(
            workspace_root,
            run,
            ["symbolic-ref", "--quiet", "--short", "HEAD"],
        );
        return Ok(if checked_out.as_deref() == Some(branch) {
            BaseSelector::head()
        } else {
            selector
        });
    }
    Err(base_undetected())
}

/// Whether git says this clone is shallow, which is what turns an unavailable
/// merge base from a mystery into a missing fetch.
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

/// One line of git's answer, or `None` when git refused or answered nothing.
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
