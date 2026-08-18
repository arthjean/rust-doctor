//! The scope a scan runs under: the whole workspace, the files changed since a
//! base, or a baseline comparison against one.
//!
//! Three rules hold the module together.
//!
//! **Validated once, resolved from that.** `ScopeRequest::validate` returns a
//! [`ValidatedScope`] whose base selector already passed the closed grammar,
//! and resolution reads that and nothing else. Validation used to run twice
//! over the same request, once as the gate in `lib.rs` and once inside
//! `resolve_with`, which left a failure branch in resolution that no input
//! could reach.
//!
//! **One resolved shape.** [`ResolvedScope`] is the three cases, and the public
//! [`ScopeReport`] is the accessors over it. A second enum used to mirror it
//! variant for variant so callers inside the crate could match on it, which
//! made a fourth scope mode an edit in five places.
//!
//! **One constructor per shape.** [`ScopeReport::files_scope`] is the only way
//! a file scope is built: it sorts, deduplicates and bounds. `includes` binary
//! searches that order, and the invariant used to be established separately by
//! the production path and by the test constructor, so a third site would have
//! broken the search in silence rather than loudly.

use std::fmt;
use std::path::Path;

use serde::{Serialize, Serializer};

use crate::internal_error::InternalError;
use crate::git::{GitCall, GitFailure, OUTPUT_TOO_LARGE, git_arguments, run_git};
use crate::workspace_path;

#[cfg(test)]
mod tests;

/// The stage every outcome of this pass is reported at.
const STAGE: &str = "scope";

const OID_OUTPUT_LIMIT: usize = 4_096;
const DIFF_OUTPUT_LIMIT: usize = 1_048_576;
const SCOPE_OUTPUT_LIMIT: usize = 2_097_152;
const PATH_LIMIT: usize = 4_096;
const FILE_LIMIT: usize = 10_000;

const BASE_UNAVAILABLE: GitFailure =
    GitFailure::new("base-unavailable", "Git base commit is unavailable.");
const MERGE_BASE_UNAVAILABLE: GitFailure =
    GitFailure::new("merge-base-unavailable", "Git merge base is unavailable.");
const DIFF_FAILED: GitFailure =
    GitFailure::new("git-diff-failed", "Git changed files could not be read.");

/// The scope a caller asked for, before its base selector was checked.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ScopeRequest {
    Full,
    Files { base: String },
    Baseline { base: String },
}

impl ScopeRequest {
    /// Checks the base selector against the closed grammar, once.
    pub(crate) fn validate(&self) -> Result<ValidatedScope, InternalError> {
        Ok(match self {
            Self::Full => ValidatedScope::Full,
            Self::Files { base } => ValidatedScope::Files(BaseSelector::new(base)?),
            Self::Baseline { base } => ValidatedScope::Baseline(BaseSelector::new(base)?),
        })
    }
}

impl fmt::Debug for ScopeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => formatter.write_str("Full"),
            Self::Files { .. } => formatter.write_str("Files { base: <redacted> }"),
            Self::Baseline { .. } => formatter.write_str("Baseline { base: <redacted> }"),
        }
    }
}

/// A scope whose base selector passed [`BaseSelector::new`].
///
/// Resolution takes this rather than a `ScopeRequest`, which is what makes an
/// invalid base unrepresentable at the point git is about to be handed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ValidatedScope {
    Full,
    Files(BaseSelector),
    Baseline(BaseSelector),
}

/// A base selector inside the grammar this tool accepts.
///
/// The grammar is closed rather than delegated to git: a selector reaches a
/// command line, and `HEAD~1`, `main^{commit}` or a leading `-` are spellings
/// this tool refuses to construct rather than spellings it asks git to judge.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BaseSelector(String);

impl BaseSelector {
    fn new(base: &str) -> Result<Self, InternalError> {
        if !is_valid_base(base) {
            return Err(InternalError::new(
                STAGE,
                "invalid-base",
                "Invalid Git base selector.",
            ));
        }
        Ok(Self(base.to_owned()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// A branch name is the caller's, and no error or trace of this crate carries
/// it: the selector redacts itself rather than relying on every formatter that
/// might reach one.
impl fmt::Debug for BaseSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeMode {
    Full,
    Files,
    Baseline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionScope {
    Workspace,
}

/// The resolved scope, published through accessors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeReport {
    kind: ResolvedScope,
}

/// The three shapes a resolved scope takes.
///
/// Crate callers match on this directly through [`ScopeReport::kind`]. It stays
/// out of the public API because the published surface is the accessors and the
/// versioned JSON, not the variant list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedScope {
    Full,
    Files {
        comparison_base: String,
        files: Vec<String>,
    },
    Baseline {
        comparison_base: String,
    },
}

#[derive(Serialize)]
struct SerializedScope<'a> {
    mode: ScopeMode,
    execution_scope: ExecutionScope,
    comparison_base: Option<&'a str>,
    files: Option<&'a [String]>,
}

impl ScopeReport {
    pub const fn mode(&self) -> ScopeMode {
        match self.kind {
            ResolvedScope::Full => ScopeMode::Full,
            ResolvedScope::Files { .. } => ScopeMode::Files,
            ResolvedScope::Baseline { .. } => ScopeMode::Baseline,
        }
    }

    pub const fn execution_scope(&self) -> ExecutionScope {
        ExecutionScope::Workspace
    }

    pub fn comparison_base(&self) -> Option<&str> {
        match &self.kind {
            ResolvedScope::Full => None,
            ResolvedScope::Files {
                comparison_base, ..
            }
            | ResolvedScope::Baseline { comparison_base } => Some(comparison_base),
        }
    }

    pub fn files(&self) -> Option<&[String]> {
        match &self.kind {
            ResolvedScope::Files { files, .. } => Some(files),
            ResolvedScope::Full | ResolvedScope::Baseline { .. } => None,
        }
    }

    pub(crate) const fn kind(&self) -> &ResolvedScope {
        &self.kind
    }

    pub(crate) const fn full() -> Self {
        Self {
            kind: ResolvedScope::Full,
        }
    }

    /// A baseline scope carries a validated hex object id and nothing else, so
    /// its serialized form is a fixed hundred or so bytes and the report bound
    /// is one it cannot reach.
    pub(crate) fn baseline_scope(comparison_base: String) -> Self {
        Self {
            kind: ResolvedScope::Baseline { comparison_base },
        }
    }

    /// The one place a file scope is built.
    ///
    /// Sorting and deduplicating here is what [`Self::includes`] binary
    /// searches, and the bound is checked here because this is the only shape
    /// that can reach it.
    pub(crate) fn files_scope(
        comparison_base: String,
        mut files: Vec<String>,
    ) -> Result<Self, InternalError> {
        files.sort();
        files.dedup();
        let scope = Self {
            kind: ResolvedScope::Files {
                comparison_base,
                files,
            },
        };
        scope.ensure_output_bound()?;
        Ok(scope)
    }

    /// Refuses a scope whose serialized form reaches the report limit.
    ///
    /// The measurement is the serialization itself, because normalization
    /// expands paths (`%` becomes `%25`) and a bound on the diff bytes does not
    /// bound what the report carries. A serializer that cannot answer is
    /// treated as one that answered too large: an unmeasured scope is not a
    /// bounded one. That branch is unreachable for a shape of enums and
    /// strings, and refusing it is what keeps "published" and "measured" the
    /// same set.
    fn ensure_output_bound(&self) -> Result<(), InternalError> {
        let measured = serde_json::to_vec(self).map_or(usize::MAX, |serialized| serialized.len());
        (measured < SCOPE_OUTPUT_LIMIT)
            .then_some(())
            .ok_or_else(output_too_large)
    }

    pub(crate) fn includes(&self, path: Option<&str>) -> bool {
        let ResolvedScope::Files { files, .. } = &self.kind else {
            return true;
        };
        path.is_some_and(|path| {
            files
                .binary_search_by(|candidate| candidate.as_str().cmp(path))
                .is_ok()
        })
    }
}

impl Serialize for ScopeReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SerializedScope {
            mode: self.mode(),
            execution_scope: self.execution_scope(),
            comparison_base: self.comparison_base(),
            files: self.files(),
        }
        .serialize(serializer)
    }
}

pub(crate) fn resolve(
    scope: &ValidatedScope,
    workspace_root: &Path,
) -> Result<ScopeReport, InternalError> {
    resolve_with(scope, workspace_root, |call| {
        run_git(Path::new("git"), workspace_root, call)
    })
}

fn resolve_with(
    scope: &ValidatedScope,
    workspace_root: &Path,
    mut run: impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<ScopeReport, InternalError> {
    // Each mode answers in full in its own arm, so a fourth one cannot be added
    // without writing what it resolves to.
    match scope {
        ValidatedScope::Full => Ok(ScopeReport::full()),
        ValidatedScope::Baseline(base) => Ok(ScopeReport::baseline_scope(resolve_comparison_base(
            base,
            workspace_root,
            &mut run,
        )?)),
        ValidatedScope::Files(base) => {
            let comparison_base = resolve_comparison_base(base, workspace_root, &mut run)?;
            let diff = run(&scope_call(
                workspace_root,
                [
                    "diff",
                    "--no-ext-diff",
                    "--no-renames",
                    "--relative",
                    "--name-only",
                    "-z",
                    "--diff-filter=ACMR",
                    comparison_base.as_str(),
                    "--",
                    ".",
                ],
                DIFF_OUTPUT_LIMIT,
                DIFF_FAILED,
            ))?;
            ScopeReport::files_scope(comparison_base, parse_paths(&diff)?)
        }
    }
}

/// Resolves the base selector to the single merge base the comparison runs
/// against.
///
/// The selector is turned into an object id first, so every later argument this
/// pass puts on a command line is a validated lowercase hex digest rather than
/// anything the caller wrote.
fn resolve_comparison_base(
    base: &BaseSelector,
    workspace_root: &Path,
    mut run: impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<String, InternalError> {
    let revision = format!("{}^{{commit}}", base.as_str());
    let base_answer = run(&scope_call(
        workspace_root,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            revision.as_str(),
        ],
        OID_OUTPUT_LIMIT,
        BASE_UNAVAILABLE,
    ))?;
    let base_commit =
        parse_single_oid(&base_answer).ok_or_else(|| BASE_UNAVAILABLE.error(STAGE))?;

    let merge_answer = run(&scope_call(
        workspace_root,
        ["merge-base", "--all", base_commit.as_str(), "HEAD"],
        OID_OUTPUT_LIMIT,
        MERGE_BASE_UNAVAILABLE,
    ))?;
    let merge_bases =
        parse_oids(&merge_answer).ok_or_else(|| MERGE_BASE_UNAVAILABLE.error(STAGE))?;
    // A merge base of another hash length than the commit it was asked about is
    // not an answer about that commit.
    if merge_bases.iter().any(|oid| oid.len() != base_commit.len()) {
        return Err(MERGE_BASE_UNAVAILABLE.error(STAGE));
    }
    match merge_bases.as_slice() {
        [] => Err(MERGE_BASE_UNAVAILABLE.error(STAGE)),
        [comparison_base] => Ok(comparison_base.clone()),
        _ => Err(InternalError::new(
            STAGE,
            "merge-base-ambiguous",
            "Git merge base is ambiguous.",
        )),
    }
}

fn scope_call<const N: usize>(
    workspace_root: &Path,
    operation: [&str; N],
    stdout_limit: usize,
    failure: GitFailure,
) -> GitCall {
    GitCall {
        arguments: git_arguments(workspace_root, operation),
        stdout_limit,
        stage: STAGE,
        failure,
        overflow: OUTPUT_TOO_LARGE,
    }
}

fn is_valid_base(base: &str) -> bool {
    let bytes = base.as_bytes();
    if matches!(bytes.len(), 40 | 64) && bytes.iter().all(u8::is_ascii_hexdigit) {
        return true;
    }
    if bytes.is_empty()
        || bytes.len() > 255
        || !base.is_ascii()
        || base.starts_with('-')
        || base.contains("..")
        || base.contains("//")
    {
        return false;
    }
    base.split('/').all(|component| {
        !component.is_empty()
            && !component.starts_with('.')
            && !component.ends_with('.')
            && !component.ends_with(".lock")
            && component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    })
}

fn parse_single_oid(output: &[u8]) -> Option<String> {
    let mut oids = parse_oids(output)?;
    (oids.len() == 1).then(|| oids.remove(0))
}

/// Reads whitespace-separated object ids, refusing the whole output if any
/// token is not one.
fn parse_oids(output: &[u8]) -> Option<Vec<String>> {
    let output = std::str::from_utf8(output).ok()?;
    output
        .split_ascii_whitespace()
        .map(|oid| {
            (matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .then(|| oid.to_ascii_lowercase())
        })
        .collect()
}

/// Reads the NUL-terminated paths of a `--name-only -z` diff.
///
/// The byte count is not checked here: `run_git` refused anything past
/// `DIFF_OUTPUT_LIMIT` before this ran, and the count of paths and the length
/// of each are what bound the result the report carries.
fn parse_paths(output: &[u8]) -> Result<Vec<String>, InternalError> {
    // The last byte terminates the final path rather than separating an empty
    // one, so it is split off rather than sliced away. Empty output is no
    // change; anything else that does not end in NUL is a truncated answer.
    let Some((&0, entries)) = output.split_last() else {
        return if output.is_empty() {
            Ok(Vec::new())
        } else {
            Err(invalid_path())
        };
    };

    let mut files = Vec::new();
    for entry in entries.split(|byte| *byte == 0) {
        if files.len() == FILE_LIMIT {
            return Err(InternalError::new(
                STAGE,
                "too-many-files",
                "Git returned too many changed paths.",
            ));
        }
        if entry.is_empty() || entry.len() > PATH_LIMIT {
            return Err(invalid_path());
        }
        let path = std::str::from_utf8(entry).map_err(|_| invalid_path())?;
        files.push(workspace_path::normalize_changed(path).ok_or_else(invalid_path)?);
    }
    Ok(files)
}

fn output_too_large() -> InternalError {
    OUTPUT_TOO_LARGE.error(STAGE)
}

fn invalid_path() -> InternalError {
    InternalError::new(
        STAGE,
        "git-path-invalid",
        "Git returned an invalid changed path.",
    )
}
