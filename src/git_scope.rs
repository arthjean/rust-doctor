//! The scope a scan runs under: the whole workspace, the files or the lines
//! changed since a base, or a baseline comparison against one, each of the
//! changed-work scopes judging the working tree or, with `--staged`, the index.
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
//! **One resolved shape.** [`ResolvedScope`] is the four cases, and the public
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
use crate::git::{GitCall, GitFailure, OUTPUT_TOO_LARGE, git_arguments, run_git, run_git_with_index};
use crate::workspace_path;

mod base;
mod lines;
#[cfg(test)]
mod tests;

pub(crate) use lines::LineRanges;

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
const DIFF_TOO_LARGE: GitFailure = GitFailure::new(
    "diff-too-large",
    "The change is too large for --scope lines: use --scope files or baseline.",
);
const UNTRACKED_FAILED: GitFailure =
    GitFailure::new("git-untracked-failed", "Git untracked files could not be read.");

/// The scope a caller asked for, before its base selector was checked.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ScopeRequest {
    Full,
    Changed {
        mode: ChangeMode,
        /// `None` asks the repository for its default branch, or for `HEAD`
        /// when the scan judges the index.
        base: Option<String>,
        options: ChangeOptions,
    },
}

/// The three ways of judging changed work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeMode {
    /// The changed files, whole.
    Files,
    /// The changed lines of the changed files.
    Lines,
    /// What the change introduced, against a scan of the base.
    Baseline,
}

/// The two modifiers a changed-work scope takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ChangeOptions {
    /// The index replaces the working tree: what `git commit` would record.
    pub(crate) staged: bool,
    /// Untracked files join the selection as wholly changed.
    pub(crate) include_untracked: bool,
}

impl ScopeRequest {
    /// Checks the base selector against the closed grammar, once, and the
    /// modifiers against the mode they were given with.
    pub(crate) fn validate(&self) -> Result<ValidatedScope, InternalError> {
        let Self::Changed {
            mode,
            base,
            options,
        } = self
        else {
            return Ok(ValidatedScope::Full);
        };
        if options.include_untracked && (options.staged || *mode == ChangeMode::Baseline) {
            return Err(InternalError::new(
                STAGE,
                "untracked-unsupported",
                "Untracked files join only a files or lines scope of the working tree.",
            ));
        }
        Ok(ValidatedScope::Changed(ChangeScope {
            mode: *mode,
            base: base.as_deref().map(BaseSelector::new).transpose()?,
            options: *options,
        }))
    }
}

impl fmt::Debug for ScopeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => formatter.write_str("Full"),
            Self::Changed {
                mode,
                base,
                options,
            } => formatter
                .debug_struct("Changed")
                .field("mode", mode)
                .field("base", &base.as_ref().map(|_| Redacted))
                .field("options", options)
                .finish(),
        }
    }
}

struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

/// A scope whose base selector passed [`BaseSelector::new`].
///
/// Resolution takes this rather than a `ScopeRequest`, which is what makes an
/// invalid base unrepresentable at the point git is about to be handed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ValidatedScope {
    Full,
    Changed(ChangeScope),
}

impl ValidatedScope {
    /// Whether the scan judges the index rather than the working tree.
    pub(crate) fn staged(&self) -> bool {
        matches!(self, Self::Changed(change) if change.options.staged)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangeScope {
    mode: ChangeMode,
    base: Option<BaseSelector>,
    options: ChangeOptions,
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

    /// The checked-out commit, which scopes a comparison to uncommitted work.
    fn head() -> Self {
        Self("HEAD".to_owned())
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
    Lines,
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
    /// The ref the comparison base was detected from, absent when named.
    base_ref: Option<String>,
    /// The index replaced the working tree.
    staged: bool,
    /// Untracked Rust files a files or lines scope compiled but did not
    /// judge, counted when `--include-untracked` was not given.
    untracked_unreported: Option<usize>,
}

/// The four shapes a resolved scope takes.
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
    /// `files` is every path with a changed line, plus every untracked file the
    /// caller included; a path absent from `ranges` is changed whole.
    Lines {
        comparison_base: String,
        files: Vec<String>,
        ranges: LineRanges,
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
    base_ref: Option<&'a str>,
    staged: bool,
    untracked_unreported: Option<usize>,
}

impl ScopeReport {
    pub const fn mode(&self) -> ScopeMode {
        match self.kind {
            ResolvedScope::Full => ScopeMode::Full,
            ResolvedScope::Files { .. } => ScopeMode::Files,
            ResolvedScope::Lines { .. } => ScopeMode::Lines,
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
            | ResolvedScope::Lines {
                comparison_base, ..
            }
            | ResolvedScope::Baseline { comparison_base } => Some(comparison_base),
        }
    }

    pub fn files(&self) -> Option<&[String]> {
        match &self.kind {
            ResolvedScope::Files { files, .. } | ResolvedScope::Lines { files, .. } => Some(files),
            ResolvedScope::Full | ResolvedScope::Baseline { .. } => None,
        }
    }

    /// The ref the comparison base was resolved from when `--base` named none:
    /// the default branch detected, or `HEAD`. A ref the caller named is not
    /// published.
    pub fn base_ref(&self) -> Option<&str> {
        self.base_ref.as_deref()
    }

    /// Whether the index replaced the working tree.
    pub const fn staged(&self) -> bool {
        self.staged
    }

    /// Untracked Rust files a files or lines scope left out.
    pub const fn untracked_unreported(&self) -> Option<usize> {
        self.untracked_unreported
    }

    pub(crate) const fn kind(&self) -> &ResolvedScope {
        &self.kind
    }

    const fn of(kind: ResolvedScope) -> Self {
        Self {
            kind,
            base_ref: None,
            staged: false,
            untracked_unreported: None,
        }
    }

    pub(crate) const fn full() -> Self {
        Self::of(ResolvedScope::Full)
    }

    /// A baseline scope carries a validated hex object id and nothing else, so
    /// its serialized form is a fixed hundred or so bytes and the report bound
    /// is one it cannot reach.
    pub(crate) fn baseline_scope(comparison_base: String) -> Self {
        Self::of(ResolvedScope::Baseline { comparison_base })
    }

    /// The one place a file scope is built.
    ///
    /// Sorting and deduplicating here is what [`Self::includes`] binary
    /// searches, and the bound is checked here because this is the only shape
    /// that can reach it.
    pub(crate) fn files_scope(
        comparison_base: String,
        files: Vec<String>,
    ) -> Result<Self, InternalError> {
        let scope = Self::of(ResolvedScope::Files {
            comparison_base,
            files: sorted(files),
        });
        scope.ensure_output_bound()?;
        Ok(scope)
    }

    /// A lines scope: the paths the ranges name, plus the untracked ones the
    /// caller included whole.
    pub(crate) fn lines_scope(
        comparison_base: String,
        ranges: LineRanges,
        untracked: Vec<String>,
    ) -> Result<Self, InternalError> {
        let files = ranges.keys().cloned().chain(untracked).collect();
        let scope = Self::of(ResolvedScope::Lines {
            comparison_base,
            files: sorted(files),
            ranges,
        });
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

    #[cfg(test)]
    pub(crate) fn includes(&self, path: Option<&str>) -> bool {
        self.includes_span(path, Some((1, 1)))
    }

    /// Whether a diagnostic at `path` spanning `lines` belongs to the scope.
    ///
    /// A files scope asks for the path alone. A lines scope also needs the
    /// span to meet a changed range, so a finding with no span is out of it,
    /// as a finding with no path is out of both.
    pub(crate) fn includes_span(&self, path: Option<&str>, lines: Option<(usize, usize)>) -> bool {
        let (files, ranges) = match &self.kind {
            ResolvedScope::Full | ResolvedScope::Baseline { .. } => return true,
            ResolvedScope::Files { files, .. } => (files, None),
            ResolvedScope::Lines { files, ranges, .. } => (files, Some(ranges)),
        };
        let Some(path) = path else {
            return false;
        };
        let listed = files
            .binary_search_by(|candidate| candidate.as_str().cmp(path))
            .is_ok();
        match (ranges.map(|ranges| ranges.get(path)), lines) {
            (None, _) => listed,
            (Some(_), None) => false,
            // Listed without ranges: an untracked file, changed whole.
            (Some(None), Some(_)) => listed,
            (Some(Some(changed)), Some((first, last))) => changed
                .iter()
                .any(|(start, end)| first <= *end && *start <= last),
        }
    }
}

fn sorted(mut files: Vec<String>) -> Vec<String> {
    files.sort();
    files.dedup();
    files
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
            base_ref: self.base_ref(),
            staged: self.staged,
            untracked_unreported: self.untracked_unreported,
        }
        .serialize(serializer)
    }
}

/// Resolves a validated scope, reading the staged diff through `index` when
/// the scan judges the index.
pub(crate) fn resolve(
    scope: &ValidatedScope,
    workspace_root: &Path,
    index: Option<&Path>,
) -> Result<ScopeReport, InternalError> {
    resolve_with(scope, workspace_root, |call| match index {
        Some(index) => run_git_with_index(Path::new("git"), workspace_root, call, index),
        None => run_git(Path::new("git"), workspace_root, call),
    })
}

fn resolve_with(
    scope: &ValidatedScope,
    workspace_root: &Path,
    mut run: impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<ScopeReport, InternalError> {
    let ValidatedScope::Changed(change) = scope else {
        return Ok(ScopeReport::full());
    };
    let base = change_base(change, workspace_root, &mut run)?;
    let comparison_base = resolve_comparison_base(&base, workspace_root, &mut run)
        .map_err(|error| diagnose_merge_base(error, workspace_root, &mut run))?;
    // Each mode answers in full in its own arm, so a fourth one cannot be added
    // without writing what it resolves to.
    let mut report = match change.mode {
        ChangeMode::Baseline => ScopeReport::baseline_scope(comparison_base),
        ChangeMode::Files => changed_files(change, comparison_base, workspace_root, &mut run)?,
        ChangeMode::Lines => changed_lines(change, comparison_base, workspace_root, &mut run)?,
    };
    // Only a base the repository answered for is published: one the caller
    // named is the caller's, and no report of this crate carries it.
    report.base_ref = change.base.is_none().then(|| base.as_str().to_owned());
    report.staged = change.options.staged;
    report.untracked_unreported = untracked_unreported(change, workspace_root, &mut run)?;
    Ok(report)
}

/// The selector to compare against: the one named, or the index's own
/// `HEAD`, or the default branch the repository answers for. The index is
/// judged against the commit it would follow; the working tree against the
/// branch it would merge into.
fn change_base(
    change: &ChangeScope,
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<BaseSelector, InternalError> {
    match (&change.base, change.options.staged) {
        (Some(base), _) => Ok(base.clone()),
        (None, true) => Ok(BaseSelector::head()),
        (None, false) => base::detect(workspace_root, run),
    }
}

fn changed_files(
    change: &ChangeScope,
    comparison_base: String,
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<ScopeReport, InternalError> {
    let options = [
        "--no-renames",
        "--relative",
        "--name-only",
        "-z",
        "--diff-filter=ACMR",
    ];
    let mut files = parse_paths(&run(&scope_call(
        workspace_root,
        diff_operation(change.options.staged, &options, &comparison_base),
        DIFF_OUTPUT_LIMIT,
        DIFF_FAILED,
    ))?)?;
    if change.options.include_untracked {
        files.extend(untracked(workspace_root, run)?);
    }
    ScopeReport::files_scope(comparison_base, files)
}

fn changed_lines(
    change: &ChangeScope,
    comparison_base: String,
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<ScopeReport, InternalError> {
    let options = [
        "--no-textconv",
        "--no-color",
        "--unified=0",
        "--find-renames",
        "--relative",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--diff-filter=ACMR",
    ];
    let patch = run(&GitCall {
        overflow: DIFF_TOO_LARGE,
        ..scope_call(
            workspace_root,
            diff_operation(change.options.staged, &options, &comparison_base),
            DIFF_OUTPUT_LIMIT,
            DIFF_FAILED,
        )
    })?;
    let included = if change.options.include_untracked {
        untracked(workspace_root, run)?
    } else {
        Vec::new()
    };
    ScopeReport::lines_scope(comparison_base, lines::parse_patch(&patch)?, included)
}

/// The untracked Rust files a scope could have judged and did not: counted
/// for a working-tree files or lines scope without `--include-untracked`.
fn untracked_unreported(
    change: &ChangeScope,
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<Option<usize>, InternalError> {
    if change.mode == ChangeMode::Baseline
        || change.options.staged
        || change.options.include_untracked
    {
        return Ok(None);
    }
    // Only Rust files are listed, so an untracked tree the caller never asked
    // about cannot push a count past the bounds that fail the scan.
    Ok(Some(untracked_matching(workspace_root, run, "*.rs")?.len()))
}

/// `git diff` over the index or the working tree, with `options`, against
/// `comparison_base`, limited to the workspace.
fn diff_operation<'a>(staged: bool, options: &[&'a str], comparison_base: &'a str) -> Vec<&'a str> {
    let mut operation = vec!["diff", "--no-ext-diff"];
    if staged {
        operation.push("--cached");
    }
    operation.extend_from_slice(options);
    operation.extend([comparison_base, "--", "."]);
    operation
}

/// The untracked files under the workspace that `.gitignore` does not hide.
fn untracked(
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<Vec<String>, InternalError> {
    untracked_matching(workspace_root, run, ".")
}

/// The untracked files under the workspace matching `pathspec`, which git
/// reads relative to the workspace, `*` matching across directories.
fn untracked_matching(
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
    pathspec: &str,
) -> Result<Vec<String>, InternalError> {
    parse_paths(&run(&scope_call(
        workspace_root,
        vec!["ls-files", "--others", "--exclude-standard", "-z", "--", pathspec],
        DIFF_OUTPUT_LIMIT,
        UNTRACKED_FAILED,
    ))?)
}

/// A merge base that could not be found in a shallow clone is a fetch that
/// stopped short, and the reader is told which one.
fn diagnose_merge_base(
    error: InternalError,
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> InternalError {
    if error.code == MERGE_BASE_UNAVAILABLE.code() && base::is_shallow(workspace_root, run) {
        return base::shallow_clone();
    }
    error
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

fn scope_call<T: Into<std::ffi::OsString>>(
    workspace_root: &Path,
    operation: impl IntoIterator<Item = T>,
    stdout_limit: usize,
    failure: GitFailure,
) -> GitCall {
    let mut arguments = git_arguments(workspace_root, [] as [&str; 0]);
    arguments.extend(operation.into_iter().map(Into::into));
    GitCall {
        arguments,
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
