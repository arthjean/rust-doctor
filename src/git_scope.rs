use std::fmt;
use std::path::Path;

use serde::{Serialize, Serializer};

use crate::internal_error::InternalError;
use crate::policy::DiffScope;
use crate::execution::RunDeadline;
use crate::git::{GitCall, GitFailure, OUTPUT_TOO_LARGE, git_arguments, run_git, run_git_with_index};
use crate::workspace_path;

mod base;
mod items;
mod lines;
#[cfg(test)]
mod tests;

pub(crate) use lines::LineRanges;

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

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ScopeRequest {
    Full,
    Changed {
        mode: ChangeMode,
        base: Option<String>,
        options: ChangeOptions,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeMode {
    Files,
    Lines,
    Contextual,
    Baseline,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ChangeOptions {
    pub(crate) staged: bool,
    pub(crate) include_untracked: bool,
}

impl ScopeRequest {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ValidatedScope {
    Full,
    Changed(ChangeScope),
}

impl ValidatedScope {
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

    fn head() -> Self {
        Self("HEAD".to_owned())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

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
    Contextual,
    Baseline,
}

impl ScopeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Files => "files",
            Self::Lines => "lines",
            Self::Contextual => "contextual",
            Self::Baseline => "baseline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionScope {
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeReport {
    kind: ResolvedScope,
    base_ref: Option<String>,
    staged: bool,
    untracked_unreported: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedScope {
    Full,
    Files {
        comparison_base: String,
        files: Vec<String>,
    },
    Lines {
        comparison_base: String,
        files: Vec<String>,
        ranges: LineRanges,
        contextual: bool,
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
            ResolvedScope::Lines {
                contextual: false, ..
            } => ScopeMode::Lines,
            ResolvedScope::Lines {
                contextual: true, ..
            } => ScopeMode::Contextual,
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

    pub fn base_ref(&self) -> Option<&str> {
        self.base_ref.as_deref()
    }

    pub const fn staged(&self) -> bool {
        self.staged
    }

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

    pub(crate) fn baseline_scope(comparison_base: String) -> Self {
        Self::of(ResolvedScope::Baseline { comparison_base })
    }

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

    pub(crate) fn lines_scope(
        comparison_base: String,
        ranges: LineRanges,
        untracked: Vec<String>,
        contextual: bool,
    ) -> Result<Self, InternalError> {
        let files = ranges.keys().cloned().chain(untracked).collect();
        let scope = Self::of(ResolvedScope::Lines {
            comparison_base,
            files: sorted(files),
            ranges,
            contextual,
        });
        scope.ensure_output_bound()?;
        Ok(scope)
    }

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
            (None, _) | (Some(_), None) | (Some(None), Some(_)) => listed,
            (Some(Some(changed)), Some((first, last))) => changed
                .ranges
                .iter()
                .any(|(start, end)| first <= *end && *start <= last),
        }
    }
}

impl ScopeReport {
    pub(crate) fn includes_reach(
        &self,
        path: Option<&str>,
        lines: Option<(usize, usize)>,
        reach: DiffScope,
        source: impl FnOnce(&str) -> Option<String>,
    ) -> bool {
        if self.includes_span(path, lines) {
            return true;
        }
        let (
            ResolvedScope::Lines {
                files,
                ranges,
                contextual: true,
                ..
            },
            Some(path),
            Some((line, _)),
        ) = (&self.kind, path, lines)
        else {
            return false;
        };
        match reach {
            DiffScope::Line => false,
            DiffScope::File => files
                .binary_search_by(|candidate| candidate.as_str().cmp(path))
                .is_ok(),
            DiffScope::Item => ranges.get(path).is_some_and(|changes| {
                source(path)
                    .and_then(|text| items::enclosing_fn(&text, line))
                    .is_some_and(|item| item.holds(changes))
            }),
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

pub(crate) fn resolve(
    scope: &ValidatedScope,
    workspace_root: &Path,
    index: Option<&Path>,
    deadline: Option<RunDeadline>,
) -> Result<ScopeReport, InternalError> {
    resolve_with(scope, workspace_root, |call| match index {
        Some(index) => {
            run_git_with_index(Path::new("git"), workspace_root, call, index, deadline)
        }
        None => run_git(Path::new("git"), workspace_root, call, deadline),
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
    let mut report = match change.mode {
        ChangeMode::Baseline => ScopeReport::baseline_scope(comparison_base),
        ChangeMode::Files => changed_files(change, comparison_base, workspace_root, &mut run)?,
        ChangeMode::Lines | ChangeMode::Contextual => {
            changed_lines(change, comparison_base, workspace_root, &mut run)?
        }
    };
    report.base_ref = change.base.is_none().then(|| base.as_str().to_owned());
    report.staged = change.options.staged;
    report.untracked_unreported = untracked_unreported(change, workspace_root, &mut run)?;
    Ok(report)
}

pub fn default_branch(workspace_root: &Path) -> Option<String> {
    base::default_branch(workspace_root, &mut |call| {
        run_git(Path::new("git"), workspace_root, call, None)
    })
}

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
    ScopeReport::lines_scope(
        comparison_base,
        lines::parse_patch(&patch)?,
        included,
        change.mode == ChangeMode::Contextual,
    )
}

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
    Ok(Some(untracked_matching(workspace_root, run, "*.rs")?.len()))
}

fn diff_operation<'a>(staged: bool, options: &[&'a str], comparison_base: &'a str) -> Vec<&'a str> {
    let mut operation = vec!["diff", "--no-ext-diff"];
    if staged {
        operation.push("--cached");
    }
    operation.extend_from_slice(options);
    operation.extend([comparison_base, "--", "."]);
    operation
}

fn untracked(
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> Result<Vec<String>, InternalError> {
    untracked_matching(workspace_root, run, ".")
}

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

fn diagnose_merge_base(
    error: InternalError,
    workspace_root: &Path,
    run: &mut impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError>,
) -> InternalError {
    let fetch_stopped_short =
        error.code == MERGE_BASE_UNAVAILABLE.code() || error.code == BASE_UNAVAILABLE.code();
    if fetch_stopped_short && base::is_shallow(workspace_root, run) {
        return base::shallow_clone();
    }
    error
}

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

fn parse_paths(output: &[u8]) -> Result<Vec<String>, InternalError> {
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
