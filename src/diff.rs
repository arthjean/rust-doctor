use crate::diagnostics::{CompilerDiagnosticEvidence, Diagnostic};
use crate::error::DiffError;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const MAX_GIT_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_POLICY_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_POLICY_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const GIT_LFS_POINTER_HEADER: &[u8] = b"version https://git-lfs.github.com/spec/v1";
const AUTO_BASE_CANDIDATES: [&str; 5] =
    ["origin/main", "main", "origin/master", "master", "HEAD~1"];

/// User-visible reporting scope. Execution scope is tracked separately because
/// compiler-aware passes still execute for a full package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportingScope {
    Full,
    Files,
    Changed,
    Lines,
    Staged,
    Baseline,
}

impl ReportingScope {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Files => "files",
            Self::Changed => "changed",
            Self::Lines => "lines",
            Self::Staged => "staged",
            Self::Baseline => "baseline",
        }
    }
}

/// Internal request after CLI and legacy aliases have been resolved.
#[derive(Debug, Clone)]
pub struct ScopeRequest {
    pub reporting_scope: ReportingScope,
    pub base: Option<String>,
    pub files: Vec<PathBuf>,
    pub include_untracked: bool,
}

/// Git change kind without path-suffix inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
}

/// One changed path. Renames and copies retain both path identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedPath {
    pub kind: ChangeKind,
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
}

impl ChangedPath {
    fn report_path(&self) -> Option<&Path> {
        self.new_path.as_deref().or(self.old_path.as_deref())
    }
}

/// Inclusive line range in the head or staged file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    const fn intersects(self, start: u32, end: u32) -> bool {
        self.start <= end && start <= self.end
    }
}

/// One resolved scope shared by planning, pass execution, and report filtering.
#[derive(Debug, Clone)]
pub struct ScopePlan {
    pub reporting_scope: ReportingScope,
    pub requested_base: Option<String>,
    pub base_commit: Option<String>,
    pub paths: BTreeSet<PathBuf>,
    pub rust_files: BTreeSet<PathBuf>,
    pub line_ranges: BTreeMap<PathBuf, Vec<LineRange>>,
    pub degradation_reason: Option<String>,
    pub changes: Vec<ChangedPath>,
}

impl ScopePlan {
    pub const fn full() -> Self {
        Self {
            reporting_scope: ReportingScope::Full,
            requested_base: None,
            base_commit: None,
            paths: BTreeSet::new(),
            rust_files: BTreeSet::new(),
            line_ranges: BTreeMap::new(),
            degradation_reason: None,
            changes: Vec::new(),
        }
    }

    pub fn has_selected_files(&self) -> bool {
        self.reporting_scope != ReportingScope::Full
    }

    pub fn has_applicable_work(&self) -> bool {
        self.reporting_scope == ReportingScope::Full
            || !self.rust_files.is_empty()
            || self.paths.iter().any(|path| is_project_work_path(path))
    }

    pub fn execution_paths(&self) -> BTreeSet<PathBuf> {
        let mut paths = self.paths.clone();
        for change in &self.changes {
            paths.extend(change.old_path.iter().cloned());
            paths.extend(change.new_path.iter().cloned());
        }
        paths
    }
}

/// Display-only Git metadata for the interactive scope prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopePromptContext {
    pub is_current_changes: bool,
    pub current_branch: Option<String>,
    pub base_branch: Option<String>,
    pub changed_rust_source_count: usize,
}

/// Derive best-effort prompt metadata from an already resolved scope.
pub fn scope_prompt_context(project_root: &Path, plan: &ScopePlan) -> ScopePromptContext {
    ScopePromptContext {
        is_current_changes: plan
            .base_commit
            .as_deref()
            .is_some_and(|base| base_commit_is_head(project_root, base)),
        current_branch: current_branch_name(project_root),
        base_branch: resolved_base_ref(project_root, plan),
        changed_rust_source_count: changed_rust_source_count(plan),
    }
}

fn changed_rust_source_count(plan: &ScopePlan) -> usize {
    if plan.changes.is_empty() {
        return plan.rust_files.len();
    }
    plan.changes
        .iter()
        .filter(|change| {
            matches!(
                change.kind,
                ChangeKind::Added | ChangeKind::Copied | ChangeKind::Modified | ChangeKind::Renamed
            )
        })
        .filter_map(|change| change.new_path.as_deref())
        .filter(|path| plan.paths.contains(*path) && is_rust_file(path))
        .collect::<BTreeSet<_>>()
        .len()
}

fn resolved_base_ref(project_root: &Path, plan: &ScopePlan) -> Option<String> {
    let requested = plan.requested_base.as_deref()?;
    if requested != "auto" {
        return Some(requested.to_string());
    }
    let base_commit = plan.base_commit.as_deref()?;
    AUTO_BASE_CANDIDATES
        .into_iter()
        .find(|candidate| {
            verify_ref_exists(project_root, candidate).is_ok()
                && merge_base(project_root, candidate).is_ok_and(|commit| commit == base_commit)
        })
        .map(str::to_string)
}

/// Resolve a request without changing the working tree or index.
pub fn resolve_scope(
    project_root: &Path,
    request: &ScopeRequest,
    ignore_files: &[String],
) -> Result<ScopePlan, DiffError> {
    match request.reporting_scope {
        ReportingScope::Full => Ok(ScopePlan::full()),
        ReportingScope::Files => resolve_explicit_files(project_root, request, ignore_files),
        ReportingScope::Changed | ReportingScope::Lines | ReportingScope::Baseline => {
            resolve_changed_scope(project_root, request, ignore_files)
        }
        ReportingScope::Staged => resolve_staged_scope(project_root, ignore_files),
    }
}

fn resolve_explicit_files(
    project_root: &Path,
    request: &ScopeRequest,
    ignore_files: &[String],
) -> Result<ScopePlan, DiffError> {
    if request.files.is_empty() {
        return Err(DiffError::InvalidScope(
            "--scope files requires at least one --files path".to_string(),
        ));
    }
    let ignore_set = crate::scanner::build_glob_set(ignore_files)
        .map_err(|error| DiffError::InvalidScope(error.to_string()))?;
    let mut paths = BTreeSet::new();
    for requested in &request.files {
        validate_repo_path(requested).map_err(DiffError::InvalidScope)?;
        let absolute = project_root.join(requested);
        if !absolute.is_file() {
            return Err(DiffError::InvalidScope(format!(
                "selected file '{}' does not exist or is not a regular file",
                requested.display()
            )));
        }
        if !ignore_set.is_match(requested) {
            paths.insert(normalize_relative_path(requested));
        }
    }
    let rust_files = paths
        .iter()
        .filter(|path| is_rust_file(path))
        .cloned()
        .collect();
    Ok(ScopePlan {
        reporting_scope: ReportingScope::Files,
        requested_base: None,
        base_commit: None,
        paths,
        rust_files,
        line_ranges: BTreeMap::new(),
        degradation_reason: None,
        changes: Vec::new(),
    })
}

fn resolve_changed_scope(
    project_root: &Path,
    request: &ScopeRequest,
    ignore_files: &[String],
) -> Result<ScopePlan, DiffError> {
    ensure_git_repository(project_root)?;
    let requested_base = request.base.as_deref().unwrap_or("auto");
    let base_commit = resolve_merge_base(project_root, requested_base)?;
    let mut changes = changed_paths(project_root, &base_commit, false)?;
    if request.include_untracked {
        changes.extend(untracked_paths(project_root)?);
    }
    let paths = selected_paths(&changes, ignore_files)?;
    let rust_files = changes
        .iter()
        .filter_map(|change| change.new_path.as_ref())
        .filter(|path| paths.contains(*path) && is_rust_file(path))
        .cloned()
        .collect();
    let (reporting_scope, line_ranges, degradation_reason) =
        if request.reporting_scope == ReportingScope::Lines {
            match resolve_line_ranges(project_root, &base_commit, &rust_files, &changes, false) {
                Ok(ranges) => (ReportingScope::Lines, ranges, None),
                Err(error) => (
                    ReportingScope::Files,
                    BTreeMap::new(),
                    Some(format!(
                        "line ranges were unavailable; degraded to files scope: {error}"
                    )),
                ),
            }
        } else {
            (request.reporting_scope, BTreeMap::new(), None)
        };
    Ok(ScopePlan {
        reporting_scope,
        requested_base: Some(requested_base.to_string()),
        base_commit: Some(base_commit),
        paths,
        rust_files,
        line_ranges,
        degradation_reason,
        changes,
    })
}

fn is_project_work_path(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(OsStr::to_str),
        Some(
            "Cargo.toml"
                | "Cargo.lock"
                | "rust-doctor.toml"
                | "rust-toolchain"
                | "rust-toolchain.toml"
        )
    ) || path
        .components()
        .any(|component| component.as_os_str() == ".cargo")
}

fn has_project_work(paths: &BTreeSet<PathBuf>) -> bool {
    paths.iter().any(|path| is_project_work_path(path))
}

fn resolve_staged_scope(
    project_root: &Path,
    ignore_files: &[String],
) -> Result<ScopePlan, DiffError> {
    ensure_git_repository(project_root)?;
    validate_index(project_root, None, SnapshotKind::Staged)?;
    let changes = changed_paths(project_root, "HEAD", true)?;
    let paths = selected_paths(&changes, ignore_files)?;
    let rust_files = changes
        .iter()
        .filter_map(|change| change.new_path.as_ref())
        .filter(|path| paths.contains(*path) && is_rust_file(path))
        .cloned()
        .collect();
    Ok(ScopePlan {
        reporting_scope: ReportingScope::Staged,
        requested_base: None,
        base_commit: Some("HEAD".to_string()),
        paths,
        rust_files,
        line_ranges: BTreeMap::new(),
        degradation_reason: None,
        changes,
    })
}

/// Resolve changed-line ranges from the Git index for a staged snapshot.
pub fn resolve_staged_line_ranges(
    project_root: &Path,
    plan: &ScopePlan,
) -> Result<BTreeMap<PathBuf, Vec<LineRange>>, DiffError> {
    resolve_line_ranges(project_root, "HEAD", &plan.rust_files, &plan.changes, true)
}

fn ensure_git_repository(project_root: &Path) -> Result<(), DiffError> {
    let output = run_git(project_root, ["rev-parse", "--is-inside-work-tree"], None)
        .map_err(|_| DiffError::GitNotFound)?;
    if output == b"true\n" || output == b"true\r\n" {
        Ok(())
    } else {
        Err(DiffError::GitNotFound)
    }
}

fn resolve_merge_base(project_root: &Path, base_hint: &str) -> Result<String, DiffError> {
    if base_hint != "auto" {
        validate_ref_name(base_hint).map_err(|reason| DiffError::InvalidRef {
            name: base_hint.to_string(),
            reason,
        })?;
        if let Err(reason) = verify_ref_exists(project_root, base_hint) {
            if is_proven_shallow_history_ref(project_root, base_hint) {
                return Err(DiffError::MergeBaseFailed(format!(
                    "base ref '{base_hint}' is unavailable in shallow history"
                )));
            }
            return Err(DiffError::InvalidRef {
                name: base_hint.to_string(),
                reason,
            });
        }
        return merge_base(project_root, base_hint).map_err(DiffError::MergeBaseFailed);
    }

    for candidate in AUTO_BASE_CANDIDATES {
        if verify_ref_exists(project_root, candidate).is_ok()
            && let Ok(commit) = merge_base(project_root, candidate)
        {
            return Ok(commit);
        }
    }
    Err(DiffError::MergeBaseFailed(
        "could not auto-detect a base; pass --base <ref>".to_string(),
    ))
}

fn is_proven_shallow_history_ref(project_root: &Path, name: &str) -> bool {
    let relative_history = name.starts_with("HEAD~") || name.starts_with("HEAD^");
    let immutable_commit =
        (7..=64).contains(&name.len()) && name.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !relative_history && !immutable_commit {
        return false;
    }
    run_git(project_root, ["rev-parse", "--is-shallow-repository"], None)
        .is_ok_and(|output| matches!(output.as_slice(), b"true\n" | b"true\r\n"))
}

fn merge_base(project_root: &Path, base: &str) -> Result<String, String> {
    let output = run_git(project_root, ["merge-base", "HEAD", base], None)?;
    parse_commit(&output)
}

fn verify_ref_exists(project_root: &Path, name: &str) -> Result<(), String> {
    let commitish = format!("{name}^{{commit}}");
    run_git(
        project_root,
        [
            OsStr::new("rev-parse"),
            OsStr::new("--verify"),
            OsStr::new("--end-of-options"),
            OsStr::new(&commitish),
        ],
        None,
    )
    .map(|_| ())
    .map_err(|_| format!("base ref '{name}' does not resolve to a commit"))
}

fn validate_ref_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("ref cannot be empty".to_string());
    }
    if name.starts_with('-') {
        return Err("ref must not start with '-'".to_string());
    }
    if name.contains('\0') || name.chars().any(char::is_control) {
        return Err("ref contains a control character".to_string());
    }
    if name.contains(' ') || name.contains(':') || name.contains("..") {
        return Err("ref contains an unsafe revision or ref separator".to_string());
    }
    Ok(())
}

fn parse_commit(output: &[u8]) -> Result<String, String> {
    let value = std::str::from_utf8(output)
        .map_err(|_| "git returned a non-UTF-8 commit ID".to_string())?
        .trim();
    if (7..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value.to_string())
    } else {
        Err("git returned an invalid commit ID".to_string())
    }
}

fn base_commit_is_head(project_root: &Path, base_commit: &str) -> bool {
    if base_commit == "HEAD" {
        return true;
    }
    let Ok(base_commit) = parse_commit(base_commit.as_bytes()) else {
        return false;
    };
    run_git(project_root, ["rev-parse", "--verify", "HEAD"], None)
        .and_then(|output| parse_commit(&output))
        .is_ok_and(|head_commit| head_commit == base_commit)
}

fn current_branch_name(project_root: &Path) -> Option<String> {
    let output = run_git(
        project_root,
        ["symbolic-ref", "--quiet", "--short", "HEAD"],
        None,
    )
    .ok()?;
    let branch = std::str::from_utf8(&output).ok()?.trim();
    if branch.is_empty() || branch.chars().any(char::is_control) {
        None
    } else {
        Some(branch.to_string())
    }
}

fn changed_paths(
    project_root: &Path,
    base: &str,
    staged: bool,
) -> Result<Vec<ChangedPath>, DiffError> {
    let mut arguments = vec![
        OsString::from("diff"),
        OsString::from("--name-status"),
        OsString::from("-z"),
        OsString::from("--find-renames"),
        OsString::from("--relative"),
    ];
    if staged {
        arguments.push(OsString::from("--cached"));
    }
    arguments.push(OsString::from(base));
    arguments.push(OsString::from("--"));
    let output = run_git(project_root, arguments, None).map_err(DiffError::Other)?;
    parse_name_status_z(&output)
}

fn untracked_paths(project_root: &Path) -> Result<Vec<ChangedPath>, DiffError> {
    let output = run_git(
        project_root,
        [
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
        None,
    )
    .map_err(DiffError::Other)?;
    split_nul(&output)
        .into_iter()
        .map(|path| {
            let path = path_from_git_bytes(path)?;
            validate_repo_path(&path).map_err(DiffError::InvalidScope)?;
            Ok(ChangedPath {
                kind: ChangeKind::Added,
                old_path: None,
                new_path: Some(normalize_relative_path(&path)),
            })
        })
        .collect()
}

fn parse_name_status_z(output: &[u8]) -> Result<Vec<ChangedPath>, DiffError> {
    let fields = split_nul(output);
    let mut cursor = 0;
    let mut changes = Vec::new();
    while cursor < fields.len() {
        let status = fields[cursor];
        cursor += 1;
        let Some(code) = status.first().copied() else {
            return Err(DiffError::Other(
                "git emitted an empty change status".to_string(),
            ));
        };
        let kind = match code {
            b'A' => ChangeKind::Added,
            b'M' => ChangeKind::Modified,
            b'D' => ChangeKind::Deleted,
            b'R' => ChangeKind::Renamed,
            b'C' => ChangeKind::Copied,
            b'T' => ChangeKind::TypeChanged,
            b'U' => {
                return Err(DiffError::IndexConflict(
                    "Git reported an unresolved path".to_string(),
                ));
            }
            _ => {
                return Err(DiffError::Other(format!(
                    "git emitted unsupported change status '{}'",
                    String::from_utf8_lossy(status)
                )));
            }
        };
        if matches!(kind, ChangeKind::Renamed | ChangeKind::Copied) {
            let old = fields
                .get(cursor)
                .ok_or_else(|| DiffError::Other("git omitted the old rename path".to_string()))?;
            let new = fields
                .get(cursor + 1)
                .ok_or_else(|| DiffError::Other("git omitted the new rename path".to_string()))?;
            cursor += 2;
            changes.push(ChangedPath {
                kind,
                old_path: Some(validated_git_path(old)?),
                new_path: Some(validated_git_path(new)?),
            });
        } else {
            let path = fields
                .get(cursor)
                .ok_or_else(|| DiffError::Other("git omitted a changed path".to_string()))?;
            cursor += 1;
            let path = validated_git_path(path)?;
            changes.push(ChangedPath {
                kind,
                old_path: (kind == ChangeKind::Deleted).then(|| path.clone()),
                new_path: (kind != ChangeKind::Deleted).then_some(path),
            });
        }
    }
    Ok(changes)
}

fn validated_git_path(bytes: &[u8]) -> Result<PathBuf, DiffError> {
    let path = path_from_git_bytes(bytes)?;
    validate_repo_path(&path).map_err(DiffError::InvalidScope)?;
    Ok(normalize_relative_path(&path))
}

fn split_nul(output: &[u8]) -> Vec<&[u8]> {
    output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect()
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "the non-Unix implementation can reject Git paths that are not valid UTF-8"
)]
fn path_from_git_bytes(bytes: &[u8]) -> Result<PathBuf, DiffError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes.to_vec())
            .map(PathBuf::from)
            .map_err(|_| DiffError::Other("git emitted a non-UTF-8 path".to_string()))
    }
}

fn selected_paths(
    changes: &[ChangedPath],
    ignore_files: &[String],
) -> Result<BTreeSet<PathBuf>, DiffError> {
    let ignore_set = crate::scanner::build_glob_set(ignore_files)
        .map_err(|error| DiffError::InvalidScope(error.to_string()))?;
    Ok(changes
        .iter()
        .filter_map(ChangedPath::report_path)
        .filter(|path| !ignore_set.is_match(path))
        .map(normalize_relative_path)
        .collect())
}

fn resolve_line_ranges(
    project_root: &Path,
    base: &str,
    rust_files: &BTreeSet<PathBuf>,
    changes: &[ChangedPath],
    staged: bool,
) -> Result<BTreeMap<PathBuf, Vec<LineRange>>, DiffError> {
    let untracked: BTreeSet<&Path> = changes
        .iter()
        .filter(|change| change.kind == ChangeKind::Added)
        .filter_map(|change| change.new_path.as_deref())
        .filter(|path| !git_tracks_path(project_root, path))
        .collect();
    let mut ranges = BTreeMap::new();
    for path in rust_files {
        if untracked.contains(path.as_path()) {
            let line_count = std::fs::read_to_string(project_root.join(path))
                .map_err(|error| DiffError::Other(error.to_string()))?
                .lines()
                .count();
            if let Ok(end) = u32::try_from(line_count)
                && end > 0
            {
                ranges.insert(path.clone(), vec![LineRange { start: 1, end }]);
            }
            continue;
        }
        let mut arguments = vec![
            OsString::from("diff"),
            OsString::from("--unified=0"),
            OsString::from("--no-color"),
        ];
        if staged {
            arguments.push(OsString::from("--cached"));
        }
        arguments.push(OsString::from(base));
        arguments.push(OsString::from("--"));
        arguments.push(path.as_os_str().to_os_string());
        let output = run_git(project_root, arguments, None).map_err(DiffError::Other)?;
        let parsed = parse_zero_context_ranges(&output)?;
        if !parsed.is_empty() {
            ranges.insert(path.clone(), parsed);
        }
    }
    Ok(ranges)
}

fn git_tracks_path(project_root: &Path, path: &Path) -> bool {
    run_git(
        project_root,
        [
            OsStr::new("ls-files"),
            OsStr::new("--error-unmatch"),
            OsStr::new("--"),
            path.as_os_str(),
        ],
        None,
    )
    .is_ok()
}

fn parse_zero_context_ranges(output: &[u8]) -> Result<Vec<LineRange>, DiffError> {
    let text = std::str::from_utf8(output)
        .map_err(|_| DiffError::Other("git diff hunk output was not UTF-8".to_string()))?;
    let mut ranges = Vec::new();
    for line in text.lines().filter(|line| line.starts_with("@@ ")) {
        let plus = line
            .find('+')
            .ok_or_else(|| DiffError::Other(format!("invalid Git hunk header: {line}")))?;
        let tail = &line[plus + 1..];
        let value = tail
            .split_whitespace()
            .next()
            .ok_or_else(|| DiffError::Other(format!("invalid Git hunk header: {line}")))?;
        let mut parts = value.split(',');
        let start = parts
            .next()
            .and_then(|part| part.parse::<u32>().ok())
            .ok_or_else(|| DiffError::Other(format!("invalid Git hunk range: {line}")))?;
        let count = parts
            .next()
            .map_or(Some(1), |part| part.parse::<u32>().ok())
            .ok_or_else(|| DiffError::Other(format!("invalid Git hunk range: {line}")))?;
        if count > 0 {
            ranges.push(LineRange {
                start,
                end: start.saturating_add(count - 1),
            });
        }
    }
    Ok(merge_line_ranges(ranges))
}

fn merge_line_ranges(mut ranges: Vec<LineRange>) -> Vec<LineRange> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<LineRange> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end.saturating_add(1)
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

/// Filter with exact workspace-relative paths. No suffix matching is used.
pub fn filter_diagnostics(
    diagnostics: Vec<Diagnostic>,
    compiler_evidence: &[CompilerDiagnosticEvidence],
    project_root: &Path,
    plan: &ScopePlan,
) -> Vec<Diagnostic> {
    if matches!(
        plan.reporting_scope,
        ReportingScope::Full | ReportingScope::Baseline
    ) {
        return diagnostics;
    }
    diagnostics
        .into_iter()
        .filter(|diagnostic| {
            let path = diagnostic_relative_path(project_root, &diagnostic.file_path);
            if plan.reporting_scope == ReportingScope::Lines {
                let Some(ranges) = plan.line_ranges.get(&path) else {
                    return false;
                };
                let Some((start, end)) = diagnostic_span(diagnostic, compiler_evidence) else {
                    return false;
                };
                return ranges.iter().any(|range| range.intersects(start, end));
            }
            if diagnostic.line.is_none() {
                return !plan.rust_files.is_empty() || has_project_work(&plan.paths);
            }
            plan.rust_files.contains(&path)
        })
        .collect()
}

fn diagnostic_span(
    diagnostic: &Diagnostic,
    evidence: &[CompilerDiagnosticEvidence],
) -> Option<(u32, u32)> {
    evidence
        .iter()
        .find(|candidate| candidate.matches(diagnostic))
        .and_then(|candidate| candidate.primary_span.as_ref())
        .map_or_else(
            || diagnostic.line.map(|line| (line, line)),
            |span| Some((span.range.start.line, span.range.end.line)),
        )
}

fn diagnostic_relative_path(root: &Path, path: &Path) -> PathBuf {
    let relative = if path.is_absolute() {
        path.strip_prefix(root).unwrap_or(path)
    } else {
        path
    };
    normalize_relative_path(relative)
}

fn normalize_relative_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        if let Component::Normal(value) = component {
            normalized.push(value);
        }
    }
    normalized
}

fn validate_repo_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(format!("path '{}' must be relative", path.display()));
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "path '{}' contains an unsafe component",
            path.display()
        ));
    }
    Ok(())
}

fn is_rust_file(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "rs")
}

#[derive(Debug, Clone, Copy)]
enum SnapshotKind {
    Staged,
    Baseline,
}

impl SnapshotKind {
    const fn error(self, message: String) -> DiffError {
        match self {
            Self::Staged => DiffError::StagedSnapshot(message),
            Self::Baseline => DiffError::BaselineUnavailable(message),
        }
    }
}

/// RAII owner for a compiler-safe Git snapshot and its isolated target tree.
#[derive(Debug)]
pub struct MaterializedSnapshot {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

impl MaterializedSnapshot {
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Materialize exactly the current Git index, never the working-tree file.
pub fn materialize_staged(project_root: &Path) -> Result<MaterializedSnapshot, DiffError> {
    ensure_git_repository(project_root)?;
    materialize(project_root, None, SnapshotKind::Staged)
}

/// Materialize a commit through a private temporary index.
pub fn materialize_commit(
    project_root: &Path,
    commit: &str,
) -> Result<MaterializedSnapshot, DiffError> {
    validate_ref_name(commit).map_err(|reason| DiffError::InvalidRef {
        name: commit.to_string(),
        reason,
    })?;
    verify_ref_exists(project_root, commit).map_err(DiffError::BaselineUnavailable)?;
    materialize(project_root, Some(commit), SnapshotKind::Baseline)
}

/// Refuse a staged scan when policy-bearing worktree files differ from the
/// exact index snapshot that will be compiled.
pub fn validate_staged_policy_snapshot(
    worktree_root: &Path,
    snapshot_root: &Path,
) -> Result<(), DiffError> {
    let worktree = collect_policy_files(worktree_root).map_err(DiffError::StagedSnapshot)?;
    let snapshot = collect_policy_files(snapshot_root).map_err(DiffError::StagedSnapshot)?;
    let mut drift = BTreeSet::new();
    for path in worktree.keys().chain(snapshot.keys()) {
        if path.file_name() == Some(OsStr::new("Cargo.lock")) {
            continue;
        }
        if !policy_contents_equal(
            worktree.get(path).map(Vec::as_slice),
            snapshot.get(path).map(Vec::as_slice),
        ) {
            drift.insert(path.clone());
        }
    }
    if drift.is_empty() {
        return Ok(());
    }
    let paths = drift
        .iter()
        .take(8)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>()
        .join(", ");
    Err(DiffError::StagedSnapshot(format!(
        "configuration or ignore policy differs between the index and worktree: {paths}"
    )))
}

fn policy_contents_equal(worktree: Option<&[u8]>, snapshot: Option<&[u8]>) -> bool {
    match (worktree, snapshot) {
        (Some(worktree), Some(snapshot)) => {
            worktree == snapshot
                || canonical_policy_bytes(worktree).eq(canonical_policy_bytes(snapshot))
        }
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

fn canonical_policy_bytes(content: &[u8]) -> impl Iterator<Item = u8> + '_ {
    content.iter().enumerate().filter_map(|(index, byte)| {
        if *byte == b'\r' && content.get(index + 1) == Some(&b'\n') {
            None
        } else {
            Some(*byte)
        }
    })
}

/// Stable fingerprint for the policy and Cargo inputs used by one snapshot.
pub fn policy_fingerprint(root: &Path) -> Result<String, String> {
    let files = collect_policy_files(root)?;
    let mut hasher = Sha256::new();
    hasher.update(b"rust-doctor-policy-v1\0");
    for (path, content) in files {
        let normalized = path.to_string_lossy().replace('\\', "/");
        let path_len = u64::try_from(normalized.len())
            .map_err(|_| "policy path length exceeded u64".to_string())?;
        let content_len = u64::try_from(content.len())
            .map_err(|_| "policy input length exceeded u64".to_string())?;
        hasher.update(path_len.to_le_bytes());
        hasher.update(normalized.as_bytes());
        hasher.update(content_len.to_le_bytes());
        hasher.update(content);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_policy_files(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut BTreeMap<PathBuf, Vec<u8>>,
        total_bytes: &mut u64,
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(directory)
            .map_err(|error| format!("failed to read policy directory: {error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("failed to read policy entry: {error}"))?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("failed to inspect policy path: {error}"))?;
            if metadata.is_dir() {
                if matches!(
                    path.file_name().and_then(OsStr::to_str),
                    Some(".git" | "target" | "vendor" | "node_modules")
                ) {
                    continue;
                }
                visit(root, &path, files, total_bytes)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "policy path escaped its scan root".to_string())?;
                if is_policy_input(relative) {
                    if metadata.len() > MAX_POLICY_FILE_BYTES {
                        return Err(format!(
                            "policy input '{}' exceeds the 16 MiB safety limit",
                            relative.display()
                        ));
                    }
                    *total_bytes = total_bytes
                        .checked_add(metadata.len())
                        .ok_or_else(|| "policy input size overflowed".to_string())?;
                    if *total_bytes > MAX_POLICY_TOTAL_BYTES {
                        return Err(
                            "policy inputs exceeded the 64 MiB aggregate safety limit".to_string()
                        );
                    }
                    let content = std::fs::read(&path)
                        .map_err(|error| format!("failed to read policy input: {error}"))?;
                    files.insert(normalize_relative_path(relative), content);
                }
            }
        }
        Ok(())
    }

    let mut files = BTreeMap::new();
    let mut total_bytes = 0;
    visit(root, root, &mut files, &mut total_bytes)?;
    Ok(files)
}

fn is_policy_input(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    matches!(
        path.file_name().and_then(OsStr::to_str),
        Some(
            "Cargo.toml"
                | "Cargo.lock"
                | "rust-doctor.toml"
                | "rust-toolchain"
                | "rust-toolchain.toml"
                | ".gitignore"
                | ".gitmodules"
                | "deny.toml"
        )
    ) || matches!(
        normalized.as_str(),
        ".cargo/config" | ".cargo/config.toml" | ".cargo/audit.toml"
    )
}

fn materialize(
    project_root: &Path,
    commit: Option<&str>,
    kind: SnapshotKind,
) -> Result<MaterializedSnapshot, DiffError> {
    let temp = tempfile::tempdir().map_err(|error| kind.error(error.to_string()))?;
    let checkout_root = temp.path().join("snapshot");
    std::fs::create_dir(&checkout_root).map_err(|error| kind.error(error.to_string()))?;
    let project_prefix = git_project_prefix(project_root).map_err(|error| kind.error(error))?;
    let snapshot_root = checkout_root.join(project_prefix);
    let index_path = commit.map(|_| temp.path().join("index"));
    let index_env = index_path
        .as_ref()
        .map(|path| (OsStr::new("GIT_INDEX_FILE"), path.as_os_str()));
    if let Some(commit) = commit {
        run_git(project_root, ["read-tree", "--reset", commit], index_env)
            .map_err(|error| kind.error(error))?;
    }
    validate_index(project_root, index_env, kind)?;

    let mut prefix = OsString::from("--prefix=");
    prefix.push(checkout_root.as_os_str());
    prefix.push(std::path::MAIN_SEPARATOR_STR);
    run_git(
        project_root,
        [
            OsStr::new("checkout-index"),
            OsStr::new("--all"),
            prefix.as_os_str(),
        ],
        index_env,
    )
    .map_err(|error| kind.error(error))?;
    reject_lfs_pointers(&checkout_root).map_err(|error| kind.error(error))?;
    if !snapshot_root.join("Cargo.toml").is_file() {
        return Err(kind.error("snapshot does not contain a root Cargo.toml".to_string()));
    }
    Ok(MaterializedSnapshot {
        _temp: temp,
        root: snapshot_root,
    })
}

fn git_project_prefix(project_root: &Path) -> Result<PathBuf, String> {
    let mut output = run_git(project_root, ["rev-parse", "--show-prefix"], None)?;
    while output
        .last()
        .is_some_and(|byte| matches!(*byte, b'\n' | b'\r'))
    {
        output.pop();
    }
    if output.is_empty() {
        return Ok(PathBuf::new());
    }
    let path = path_from_git_bytes(&output).map_err(|error| error.to_string())?;
    validate_repo_path(&path)?;
    Ok(normalize_relative_path(&path))
}

fn validate_index(
    project_root: &Path,
    index_env: Option<(&OsStr, &OsStr)>,
    kind: SnapshotKind,
) -> Result<(), DiffError> {
    let output = run_git(project_root, ["ls-files", "--stage", "-z"], index_env)
        .map_err(|error| kind.error(error))?;
    for entry in split_nul(&output) {
        let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
            return Err(kind.error("index entry has no path separator".to_string()));
        };
        let header = &entry[..tab];
        let path = path_from_git_bytes(&entry[tab + 1..])
            .map_err(|error| kind.error(error.to_string()))?;
        validate_repo_path(&path).map_err(|error| kind.error(error))?;
        let fields: Vec<&[u8]> = header
            .split(u8::is_ascii_whitespace)
            .filter(|field| !field.is_empty())
            .collect();
        if fields.len() != 3 {
            return Err(kind.error("index entry has an invalid header".to_string()));
        }
        if fields[2] != b"0" {
            return Err(DiffError::IndexConflict(format!(
                "unresolved index stage for '{}'",
                path.display()
            )));
        }
        if fields[0] == b"120000" {
            return Err(kind.error(format!(
                "staged symlink '{}' cannot be materialized safely",
                path.display()
            )));
        }
        if fields[0] == b"160000" {
            return Err(kind.error(format!(
                "Git submodule '{}' cannot be materialized from the index alone",
                path.display()
            )));
        }
    }
    Ok(())
}

fn reject_lfs_pointers(root: &Path) -> Result<(), String> {
    fn visit(directory: &Path) -> Result<(), String> {
        for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if metadata.is_dir() {
                visit(&path)?;
            } else if metadata.is_file() {
                let mut file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
                let mut prefix = [0_u8; 64];
                let read = file.read(&mut prefix).map_err(|error| error.to_string())?;
                if prefix[..read].starts_with(GIT_LFS_POINTER_HEADER) {
                    return Err(format!(
                        "Git LFS content for '{}' is unavailable",
                        path.display()
                    ));
                }
            }
        }
        Ok(())
    }
    visit(root)
}

fn run_git<I, S>(
    project_root: &Path,
    arguments: I,
    environment: Option<(&OsStr, &OsStr)>,
) -> Result<Vec<u8>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command
        .current_dir(project_root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(arguments);
    if let Some((name, value)) = environment {
        command.env(name, value);
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to launch git: {error}"))?;
    if output.stdout.len() > MAX_GIT_OUTPUT_BYTES || output.stderr.len() > MAX_GIT_OUTPUT_BYTES {
        return Err("git output exceeded the 16 MiB safety limit".to_string());
    }
    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        return Err(reason.trim().to_string().if_empty("git command failed"));
    }
    Ok(output.stdout)
}

trait NonEmptyFallback {
    fn if_empty(self, fallback: &str) -> String;
}

impl NonEmptyFallback for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

/// Conservative multiset comparison for baseline mode.
pub struct BaselineComparison {
    pub introduced: Vec<Diagnostic>,
    pub fixed_count: usize,
    pub base_total: usize,
    pub cross_file_match_count: usize,
}

pub fn compare_baseline(
    head: &[Diagnostic],
    head_root: &Path,
    base: &[Diagnostic],
    base_root: &Path,
) -> BaselineComparison {
    let mut head_groups: HashMap<EvidenceKey, Vec<usize>> = HashMap::new();
    let mut base_groups: HashMap<EvidenceKey, Vec<usize>> = HashMap::new();
    for (index, diagnostic) in head.iter().enumerate() {
        head_groups
            .entry(evidence_key(diagnostic, head_root))
            .or_default()
            .push(index);
    }
    for (index, diagnostic) in base.iter().enumerate() {
        base_groups
            .entry(evidence_key(diagnostic, base_root))
            .or_default()
            .push(index);
    }

    let mut head_matched = vec![false; head.len()];
    let mut base_matched = vec![false; base.len()];
    let mut cross_file_match_count = 0;
    for (key, head_indexes) in &head_groups {
        let Some(base_indexes) = base_groups.get(key) else {
            continue;
        };
        let mut head_by_path: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
        let mut base_by_path: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
        for index in head_indexes {
            head_by_path
                .entry(diagnostic_relative_path(head_root, &head[*index].file_path))
                .or_default()
                .push(*index);
        }
        for index in base_indexes {
            base_by_path
                .entry(diagnostic_relative_path(base_root, &base[*index].file_path))
                .or_default()
                .push(*index);
        }
        for (path, same_path_head) in head_by_path {
            let Some(same_path_base) = base_by_path.get(&path) else {
                continue;
            };
            for (head_index, base_index) in same_path_head.iter().zip(same_path_base) {
                head_matched[*head_index] = true;
                base_matched[*base_index] = true;
            }
        }
        let unmatched_head: Vec<_> = head_indexes
            .iter()
            .copied()
            .filter(|index| !head_matched[*index])
            .collect();
        let unmatched_base: Vec<_> = base_indexes
            .iter()
            .copied()
            .filter(|index| !base_matched[*index])
            .collect();
        if let ([head_index], [base_index]) = (unmatched_head.as_slice(), unmatched_base.as_slice())
        {
            head_matched[*head_index] = true;
            base_matched[*base_index] = true;
            cross_file_match_count += 1;
        }
    }

    BaselineComparison {
        introduced: head
            .iter()
            .enumerate()
            .filter(|(index, _)| !head_matched[*index])
            .map(|(_, diagnostic)| diagnostic.clone())
            .collect(),
        fixed_count: base_matched.iter().filter(|matched| !**matched).count(),
        base_total: base.len(),
        cross_file_match_count,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EvidenceKey {
    provider: String,
    rule: String,
    message: String,
    source: String,
}

fn evidence_key(diagnostic: &Diagnostic, root: &Path) -> EvidenceKey {
    let descriptor = crate::catalog::built_in_catalog().ok().map(|catalog| {
        catalog.resolve(&diagnostic.rule, &diagnostic.category, diagnostic.severity)
    });
    let provider = descriptor
        .as_ref()
        .map_or("external", |value| value.as_descriptor().provider.as_str());
    let rule = descriptor
        .as_ref()
        .map_or(diagnostic.rule.as_str(), |value| {
            value.as_descriptor().canonical_id.as_str()
        });
    EvidenceKey {
        provider: provider.to_string(),
        rule: rule.to_string(),
        message: normalize_text(&diagnostic.message),
        source: diagnosed_source(diagnostic, root),
    }
}

fn diagnosed_source(diagnostic: &Diagnostic, root: &Path) -> String {
    let Some(line) = diagnostic.line else {
        return "project".to_string();
    };
    let path = if diagnostic.file_path.is_absolute() {
        diagnostic.file_path.clone()
    } else {
        root.join(&diagnostic.file_path)
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return format!("line:{line}");
    };
    let index = line.saturating_sub(1) as usize;
    content
        .lines()
        .nth(index)
        .map_or_else(|| format!("line:{line}"), normalize_text)
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Category, Severity};
    use std::process::Stdio;

    fn diagnostic(path: &str, line: Option<u32>, message: &str) -> Diagnostic {
        Diagnostic {
            file_path: PathBuf::from(path),
            rule: "unwrap-in-production".to_string(),
            category: Category::ErrorHandling,
            severity: Severity::Warning,
            message: message.to_string(),
            help: None,
            line,
            column: Some(1),
            fix: None,
        }
    }

    #[test]
    fn parses_nul_paths_without_losing_control_characters() {
        let output = "M\0src/space name.rs\0A\0src/tab\tname.rs\0A\0src/new\nname.rs\0R100\0src/old.rs\0src/renamed.rs\0";
        let changes = parse_name_status_z(output.as_bytes()).unwrap();
        assert_eq!(changes.len(), 4);
        assert_eq!(
            changes[0].new_path.as_deref(),
            Some(Path::new("src/space name.rs"))
        );
        assert_eq!(
            changes[1].new_path.as_deref(),
            Some(Path::new("src/tab\tname.rs"))
        );
        assert_eq!(
            changes[2].new_path.as_deref(),
            Some(Path::new("src/new\nname.rs"))
        );
        assert_eq!(changes[3].kind, ChangeKind::Renamed);
        assert_eq!(
            changes[3].old_path.as_deref(),
            Some(Path::new("src/old.rs"))
        );
        assert_eq!(
            changes[3].new_path.as_deref(),
            Some(Path::new("src/renamed.rs"))
        );
    }

    #[test]
    fn parses_and_merges_zero_context_ranges() {
        let diff =
            b"@@ -1,0 +2,2 @@\n+one\n+two\n@@ -8 +10 @@\n-old\n+new\n@@ -12 +12,0 @@\n-old\n";
        assert_eq!(
            parse_zero_context_ranges(diff).unwrap(),
            vec![
                LineRange { start: 2, end: 3 },
                LineRange { start: 10, end: 10 }
            ]
        );
    }

    #[test]
    fn prompt_context_describes_current_and_branch_changes() {
        let repository = tempfile::tempdir().unwrap();
        git(
            repository.path(),
            ["init", "--quiet", "--initial-branch", "feature/prompt"],
        );
        git(
            repository.path(),
            ["config", "user.email", "test@example.com"],
        );
        git(repository.path(), ["config", "user.name", "Test"]);
        std::fs::write(repository.path().join("tracked.txt"), "initial\n").unwrap();
        git(repository.path(), ["add", "tracked.txt"]);
        git(repository.path(), ["commit", "--quiet", "-m", "initial"]);
        let head = parse_commit(&run_git(repository.path(), ["rev-parse", "HEAD"], None).unwrap())
            .unwrap();
        let mut plan = ScopePlan {
            reporting_scope: ReportingScope::Changed,
            requested_base: Some("HEAD".to_string()),
            base_commit: Some(head),
            paths: BTreeSet::from([
                PathBuf::from("Cargo.toml"),
                PathBuf::from("src/deleted.rs"),
                PathBuf::from("src/lib.rs"),
            ]),
            rust_files: BTreeSet::from([PathBuf::from("src/lib.rs")]),
            line_ranges: BTreeMap::new(),
            degradation_reason: None,
            changes: Vec::new(),
        };

        assert_eq!(
            scope_prompt_context(repository.path(), &plan),
            ScopePromptContext {
                is_current_changes: true,
                current_branch: Some("feature/prompt".to_string()),
                base_branch: Some("HEAD".to_string()),
                changed_rust_source_count: 1,
            }
        );

        std::fs::write(repository.path().join("tracked.txt"), "next\n").unwrap();
        git(repository.path(), ["add", "tracked.txt"]);
        git(repository.path(), ["commit", "--quiet", "-m", "next"]);
        assert!(!scope_prompt_context(repository.path(), &plan).is_current_changes);

        plan.base_commit = Some("HEAD".to_string());
        git(repository.path(), ["checkout", "--quiet", "--detach"]);
        let detached = scope_prompt_context(repository.path(), &plan);
        assert!(detached.is_current_changes);
        assert_eq!(detached.current_branch, None);
    }

    #[test]
    fn prompt_context_fails_softly_outside_git() {
        let directory = tempfile::tempdir().unwrap();
        let plan = ScopePlan {
            reporting_scope: ReportingScope::Changed,
            requested_base: Some("auto".to_string()),
            base_commit: Some("not-a-commit".to_string()),
            paths: BTreeSet::from([PathBuf::from("src/lib.rs")]),
            rust_files: BTreeSet::from([PathBuf::from("src/lib.rs")]),
            line_ranges: BTreeMap::new(),
            degradation_reason: None,
            changes: Vec::new(),
        };

        assert_eq!(
            scope_prompt_context(directory.path(), &plan),
            ScopePromptContext {
                is_current_changes: false,
                current_branch: None,
                base_branch: None,
                changed_rust_source_count: 1,
            }
        );
    }

    #[test]
    fn prompt_context_reports_the_auto_detected_base_branch() {
        let repository = tempfile::tempdir().unwrap();
        git(
            repository.path(),
            ["init", "--quiet", "--initial-branch", "main"],
        );
        git(
            repository.path(),
            ["config", "user.email", "test@example.com"],
        );
        git(repository.path(), ["config", "user.name", "Test"]);
        std::fs::create_dir(repository.path().join("src")).unwrap();
        std::fs::write(
            repository.path().join("src/lib.rs"),
            "pub fn initial() {}\n",
        )
        .unwrap();
        git(repository.path(), ["add", "."]);
        git(repository.path(), ["commit", "--quiet", "-m", "initial"]);
        git(
            repository.path(),
            ["checkout", "--quiet", "-b", "feature/base"],
        );
        std::fs::write(
            repository.path().join("src/lib.rs"),
            "pub fn initial() {}\npub fn changed() {}\n",
        )
        .unwrap();
        git(repository.path(), ["add", "."]);
        git(repository.path(), ["commit", "--quiet", "-m", "change"]);

        let plan = resolve_scope(
            repository.path(),
            &ScopeRequest {
                reporting_scope: ReportingScope::Changed,
                base: None,
                files: Vec::new(),
                include_untracked: false,
            },
            &[],
        )
        .unwrap();
        let context = scope_prompt_context(repository.path(), &plan);

        assert_eq!(context.current_branch.as_deref(), Some("feature/base"));
        assert_eq!(context.base_branch.as_deref(), Some("main"));
        assert_eq!(context.changed_rust_source_count, 1);
    }

    #[test]
    fn prompt_context_excludes_a_deleted_rust_file() {
        let repository = tempfile::tempdir().unwrap();
        git(repository.path(), ["init", "--quiet"]);
        git(
            repository.path(),
            ["config", "user.email", "test@example.com"],
        );
        git(repository.path(), ["config", "user.name", "Test"]);
        std::fs::create_dir(repository.path().join("src")).unwrap();
        std::fs::write(
            repository.path().join("src/lib.rs"),
            "pub fn deleted() {}\n",
        )
        .unwrap();
        git(repository.path(), ["add", "."]);
        git(repository.path(), ["commit", "--quiet", "-m", "initial"]);
        std::fs::remove_file(repository.path().join("src/lib.rs")).unwrap();

        let plan = resolve_scope(
            repository.path(),
            &ScopeRequest {
                reporting_scope: ReportingScope::Changed,
                base: Some("HEAD".to_string()),
                files: Vec::new(),
                include_untracked: false,
            },
            &[],
        )
        .unwrap();

        assert_eq!(
            scope_prompt_context(repository.path(), &plan).changed_rust_source_count,
            0
        );
    }

    #[test]
    fn prompt_context_counts_added_copied_modified_and_renamed_rust_files() {
        let paths = [
            "src/added.rs",
            "src/copied.rs",
            "src/modified.rs",
            "src/renamed.rs",
            "src/deleted.rs",
            "src/type_changed.rs",
        ];
        let changes = [
            ChangeKind::Added,
            ChangeKind::Copied,
            ChangeKind::Modified,
            ChangeKind::Renamed,
            ChangeKind::Deleted,
            ChangeKind::TypeChanged,
        ]
        .into_iter()
        .zip(paths)
        .map(|(kind, path)| ChangedPath {
            kind,
            old_path: matches!(kind, ChangeKind::Renamed | ChangeKind::Deleted)
                .then(|| PathBuf::from(format!("old/{path}"))),
            new_path: (kind != ChangeKind::Deleted).then(|| PathBuf::from(path)),
        })
        .collect();
        let plan = ScopePlan {
            reporting_scope: ReportingScope::Changed,
            requested_base: None,
            base_commit: None,
            paths: paths.into_iter().map(PathBuf::from).collect(),
            rust_files: BTreeSet::new(),
            line_ranges: BTreeMap::new(),
            degradation_reason: None,
            changes,
        };

        assert_eq!(
            scope_prompt_context(Path::new("/not-a-repository"), &plan).changed_rust_source_count,
            4
        );
    }

    #[test]
    fn exact_filter_does_not_match_a_suffix_collision() {
        let plan = ScopePlan {
            reporting_scope: ReportingScope::Changed,
            requested_base: None,
            base_commit: None,
            paths: BTreeSet::from([PathBuf::from("crate-a/src/lib.rs")]),
            rust_files: BTreeSet::from([PathBuf::from("crate-a/src/lib.rs")]),
            line_ranges: BTreeMap::new(),
            degradation_reason: None,
            changes: Vec::new(),
        };
        let filtered = filter_diagnostics(
            vec![
                diagnostic("crate-a/src/lib.rs", Some(1), "kept"),
                diagnostic("src/lib.rs", Some(1), "dropped"),
            ],
            &[],
            Path::new("/workspace"),
            &plan,
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].message, "kept");
    }

    #[test]
    fn line_filter_uses_the_full_compiler_span() {
        let diagnostic = diagnostic("src/lib.rs", Some(8), "span");
        let evidence = CompilerDiagnosticEvidence {
            provenance: crate::catalog::AdapterProvenance::Rustc,
            rule: diagnostic.rule.clone(),
            message: diagnostic.message.clone(),
            file_path: diagnostic.file_path.clone(),
            line: diagnostic.line,
            column: diagnostic.column,
            original_level: "warning".to_string(),
            primary_span: Some(crate::diagnostics::CompilerSpanEvidence {
                file_path: diagnostic.file_path.clone(),
                range: crate::diagnostics::SourceRange {
                    start: crate::diagnostics::SourcePosition {
                        line: 8,
                        column: 1,
                        byte_offset: None,
                    },
                    end: crate::diagnostics::SourcePosition {
                        line: 12,
                        column: 1,
                        byte_offset: None,
                    },
                },
            }),
            related_locations: Vec::new(),
            macro_expansion: None,
            fixes: Vec::new(),
        };
        let plan = ScopePlan {
            reporting_scope: ReportingScope::Lines,
            requested_base: None,
            base_commit: None,
            paths: BTreeSet::from([PathBuf::from("src/lib.rs")]),
            rust_files: BTreeSet::from([PathBuf::from("src/lib.rs")]),
            line_ranges: BTreeMap::from([(
                PathBuf::from("src/lib.rs"),
                vec![LineRange { start: 11, end: 11 }],
            )]),
            degradation_reason: None,
            changes: Vec::new(),
        };
        assert_eq!(
            filter_diagnostics(
                vec![diagnostic],
                &[evidence],
                Path::new("/workspace"),
                &plan
            )
            .len(),
            1
        );
    }

    #[test]
    fn baseline_matches_a_pure_rename_and_counts_a_deletion() {
        let head_root = tempfile::tempdir().unwrap();
        let base_root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(head_root.path().join("new")).unwrap();
        std::fs::create_dir_all(base_root.path().join("old")).unwrap();
        std::fs::write(
            head_root.path().join("new/lib.rs"),
            "fn same() { value.unwrap(); }\n",
        )
        .unwrap();
        std::fs::write(
            base_root.path().join("old/lib.rs"),
            "fn same() { value.unwrap(); }\n",
        )
        .unwrap();
        let head = vec![diagnostic("new/lib.rs", Some(1), "same finding")];
        let base = vec![
            diagnostic("old/lib.rs", Some(1), "same finding"),
            diagnostic("old/lib.rs", Some(1), "deleted finding"),
        ];
        let comparison = compare_baseline(&head, head_root.path(), &base, base_root.path());
        assert!(comparison.introduced.is_empty());
        assert_eq!(comparison.fixed_count, 1);
        assert_eq!(comparison.base_total, 2);
        assert_eq!(comparison.cross_file_match_count, 1);
    }

    #[test]
    fn ambiguous_cross_file_evidence_stays_new() {
        let root = tempfile::tempdir().unwrap();
        for path in ["a.rs", "b.rs", "c.rs", "d.rs"] {
            std::fs::write(root.path().join(path), "value.unwrap();\n").unwrap();
        }
        let head = vec![
            diagnostic("a.rs", Some(1), "same"),
            diagnostic("b.rs", Some(1), "same"),
        ];
        let base = vec![
            diagnostic("c.rs", Some(1), "same"),
            diagnostic("d.rs", Some(1), "same"),
        ];
        let comparison = compare_baseline(&head, root.path(), &base, root.path());
        assert_eq!(comparison.introduced.len(), 2);
        assert_eq!(comparison.fixed_count, 2);
        assert_eq!(comparison.cross_file_match_count, 0);
    }

    #[test]
    fn baseline_matching_handles_moves_copies_repetitions_and_case_changes() {
        let head_root = tempfile::tempdir().unwrap();
        let base_root = tempfile::tempdir().unwrap();
        for root in [head_root.path(), base_root.path()] {
            std::fs::write(
                root.join("repeated.rs"),
                "value.unwrap();\nvalue.unwrap();\nvalue.unwrap();\n",
            )
            .unwrap();
        }
        std::fs::write(
            base_root.path().join("same.rs"),
            "fn moved() {\n    value.unwrap();\n}\n",
        )
        .unwrap();
        std::fs::write(
            head_root.path().join("same.rs"),
            "\n\nfn moved() {\n        value.unwrap();\n}\n",
        )
        .unwrap();
        std::fs::write(base_root.path().join("case.rs"), "value.unwrap();\n").unwrap();
        std::fs::write(head_root.path().join("Case.rs"), "value.unwrap();\n").unwrap();
        std::fs::write(base_root.path().join("copy.rs"), "value.unwrap();\n").unwrap();
        std::fs::write(head_root.path().join("copy.rs"), "value.unwrap();\n").unwrap();
        std::fs::write(head_root.path().join("copy-edited.rs"), "other.unwrap();\n").unwrap();

        let base = vec![
            diagnostic("same.rs", Some(2), "moved"),
            diagnostic("case.rs", Some(1), "case"),
            diagnostic("copy.rs", Some(1), "copy"),
            diagnostic("repeated.rs", Some(1), "repeat"),
            diagnostic("repeated.rs", Some(2), "repeat"),
        ];
        let head = vec![
            diagnostic("same.rs", Some(4), "moved"),
            diagnostic("Case.rs", Some(1), "case"),
            diagnostic("copy.rs", Some(1), "copy"),
            diagnostic("copy-edited.rs", Some(1), "copy"),
            diagnostic("repeated.rs", Some(1), "repeat"),
            diagnostic("repeated.rs", Some(2), "repeat"),
            diagnostic("repeated.rs", Some(3), "repeat"),
        ];

        let comparison = compare_baseline(&head, head_root.path(), &base, base_root.path());
        assert_eq!(comparison.base_total, 5);
        assert_eq!(comparison.fixed_count, 0);
        assert_eq!(comparison.introduced.len(), 2);
        assert_eq!(comparison.cross_file_match_count, 1);
        assert!(
            comparison
                .introduced
                .iter()
                .any(|finding| finding.file_path == Path::new("copy-edited.rs"))
        );
    }

    #[test]
    fn staged_snapshot_uses_index_content_and_cleans_up() {
        let repository = tempfile::tempdir().unwrap();
        git(repository.path(), ["init", "--quiet"]);
        git(
            repository.path(),
            ["config", "user.email", "test@example.com"],
        );
        git(repository.path(), ["config", "user.name", "Test"]);
        std::fs::create_dir(repository.path().join("src")).unwrap();
        std::fs::write(
            repository.path().join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        std::fs::write(
            repository.path().join("src/lib.rs"),
            "pub fn value() -> u8 { 1 }\n",
        )
        .unwrap();
        git(repository.path(), ["add", "."]);
        git(repository.path(), ["commit", "--quiet", "-m", "initial"]);
        std::fs::write(
            repository.path().join("src/lib.rs"),
            "pub fn value() -> u8 { 2 }\n",
        )
        .unwrap();
        git(repository.path(), ["add", "src/lib.rs"]);
        std::fs::write(
            repository.path().join("src/lib.rs"),
            "pub fn value() -> u8 { 3 }\n",
        )
        .unwrap();

        let snapshot = materialize_staged(repository.path()).unwrap();
        let snapshot_path = snapshot.root().to_path_buf();
        assert_eq!(
            std::fs::read_to_string(snapshot.root().join("src/lib.rs"))
                .unwrap()
                .replace("\r\n", "\n"),
            "pub fn value() -> u8 { 2 }\n"
        );
        drop(snapshot);
        assert!(!snapshot_path.exists());
    }

    #[test]
    fn staged_snapshot_preserves_a_nested_cargo_root() {
        let repository = tempfile::tempdir().unwrap();
        git(repository.path(), ["init", "--quiet"]);
        git(
            repository.path(),
            ["config", "user.email", "test@example.com"],
        );
        git(repository.path(), ["config", "user.name", "Test"]);
        let project = repository.path().join("crates/nested");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(
            project.join("Cargo.toml"),
            "[package]\nname='nested'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        std::fs::write(project.join("src/lib.rs"), "pub fn staged() {}\n").unwrap();
        git(repository.path(), ["add", "."]);
        git(repository.path(), ["commit", "--quiet", "-m", "initial"]);

        let snapshot = materialize_staged(&project).unwrap();
        assert!(snapshot.root().join("Cargo.toml").is_file());
        assert_eq!(
            std::fs::read_to_string(snapshot.root().join("src/lib.rs"))
                .unwrap()
                .replace("\r\n", "\n"),
            "pub fn staged() {}\n"
        );
    }

    #[test]
    fn staged_policy_ignores_only_crlf_conversion() {
        let worktree = tempfile::tempdir().unwrap();
        let snapshot = tempfile::tempdir().unwrap();
        std::fs::write(
            worktree.path().join("Cargo.toml"),
            b"[package]\r\nname = \"fixture\"\r\n",
        )
        .unwrap();
        std::fs::write(
            snapshot.path().join("Cargo.toml"),
            b"[package]\nname = \"fixture\"\n",
        )
        .unwrap();
        std::fs::write(
            worktree.path().join("rust-doctor.toml"),
            b"[scan]\r\ntimeout = 300\r\n",
        )
        .unwrap();
        std::fs::write(
            snapshot.path().join("rust-doctor.toml"),
            b"[scan]\ntimeout = 300\n",
        )
        .unwrap();

        validate_staged_policy_snapshot(worktree.path(), snapshot.path()).unwrap();

        std::fs::write(
            snapshot.path().join("rust-doctor.toml"),
            b"[scan]\ntimeout = 301\n",
        )
        .unwrap();
        assert!(matches!(
            validate_staged_policy_snapshot(worktree.path(), snapshot.path()),
            Err(DiffError::StagedSnapshot(_))
        ));
    }

    fn git<const N: usize>(root: &Path, arguments: [&str; N]) {
        let status = Command::new("git")
            .current_dir(root)
            .args(arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
}
