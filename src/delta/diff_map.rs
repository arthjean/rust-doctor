//! What the diff did between the base and the current side, as the pairing
//! reads it: which base path every renamed file now lives at, and where every
//! base line landed.
//!
//! Two bounded `git diff-index` calls build it, both before any pass runs. The rename map
//! comes from `--name-status -z`, whose paths are never quoted. The line map
//! comes from a `--unified=0` patch capped at one mebibyte: a change larger
//! than that, a git that fails or a deadline that passes leaves the map
//! partial, and the comparison still completes, pairing on raw line distance
//! with a notice saying so. The map only ever decides which of two equally
//! good candidates is paired, never how many are, so losing it costs the
//! precision of a pointer and nothing the gate counts.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

use crate::execution::RunDeadline;
use crate::git::{GitCall, GitFailure, git_arguments, run_git, run_git_with_index};
use crate::internal_error::InternalError;

const STAGE: &str = "delta";
const NAME_STATUS_LIMIT: usize = 2_097_152;
/// The patch a line map is built from. Past it, lines pair by raw distance.
const PATCH_LIMIT: usize = 1_048_576;

const DIFF_FAILED: GitFailure =
    GitFailure::new("git-diff-failed", "Git could not compare the base with the current side.");
const DIFF_TOO_LARGE: GitFailure =
    GitFailure::new("diff-too-large", "The diff against the base exceeds one mebibyte.");

/// The notice a comparison publishes when it pairs without the full map.
pub(crate) const NOTICE_CODE: &str = "line-map-unavailable";
const NOTICE: &str = "The diff against the base could not be read in full, so findings of one \
                      rule in one file pair by line distance alone.";

/// One `@@ -old_start,old_len +new_start,new_len @@` hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hunk {
    old_start: usize,
    old_len: usize,
    new_start: usize,
    new_len: usize,
}

#[derive(Debug, Default)]
pub(crate) struct DiffMap {
    /// Base path to current path, for every file Git reports renamed.
    renames: BTreeMap<String, String>,
    /// The hunks of every current path the patch touched, in order.
    hunks: BTreeMap<String, Vec<Hunk>>,
    notice: Option<InternalError>,
}

impl DiffMap {
    /// Reads both maps for the workspace against `base`, against the index
    /// written to `index` when the scan judges the staged tree.
    pub(crate) fn read(
        workspace_root: &Path,
        base: &str,
        index: Option<&Path>,
        deadline: Option<RunDeadline>,
    ) -> Self {
        let run = |options: &[&str], limit: usize, overflow: GitFailure| {
            // `diff-index`, the plumbing under `git diff`: the porcelain
            // refreshes the index and writes it back, and a scan never writes
            // to the repository it judges.
            let mut operation =
                vec!["diff-index", "--no-ext-diff", "--find-renames", "--relative"];
            if index.is_some() {
                operation.push("--cached");
            }
            operation.extend_from_slice(options);
            operation.extend([base, "--", "."]);
            let mut arguments = git_arguments(workspace_root, [] as [&str; 0]);
            arguments.extend(operation.into_iter().map(OsString::from));
            let call = GitCall {
                arguments,
                stdout_limit: limit,
                stage: STAGE,
                failure: DIFF_FAILED,
                overflow,
            };
            let git = Path::new("git");
            match index {
                Some(index) => run_git_with_index(git, workspace_root, &call, index, deadline),
                None => run_git(git, workspace_root, &call, deadline),
            }
        };
        let names = run(&["-z", "--name-status"], NAME_STATUS_LIMIT, DIFF_FAILED);
        let patch = run(&["-p", "--no-color", "--unified=0"], PATCH_LIMIT, DIFF_TOO_LARGE);
        let mut map = Self::parse(
            names.as_deref().unwrap_or_default(),
            patch.as_deref().unwrap_or_default(),
        );
        if names.is_err() || patch.is_err() {
            map.notice = Some(InternalError::new(STAGE, NOTICE_CODE, NOTICE));
        }
        map
    }

    /// The map of a `--name-status -z` listing and a `--unified=0` patch.
    pub(crate) fn parse(names: &[u8], patch: &[u8]) -> Self {
        Self {
            renames: renames(names),
            hunks: hunks(&String::from_utf8_lossy(patch)),
            notice: None,
        }
    }

    /// Where a base path lives on the current side.
    pub(crate) fn current_path<'a>(&'a self, base_path: &'a str) -> &'a str {
        self.renames.get(base_path).map_or(base_path, String::as_str)
    }

    /// The current line a base line of `current_path` corresponds to: shifted
    /// by every hunk above it, or, inside a hunk that rewrote it, the line at
    /// the same depth in what replaced it.
    pub(crate) fn mapped_line(&self, current_path: &str, base_line: usize) -> usize {
        let Some(hunks) = self.hunks.get(current_path) else {
            return base_line;
        };
        let mut shift = 0_isize;
        for hunk in hunks {
            if hunk.old_len > 0 && (hunk.old_start..hunk.old_start + hunk.old_len).contains(&base_line)
            {
                let depth = (base_line - hunk.old_start).min(hunk.new_len.saturating_sub(1));
                return hunk.new_start + depth;
            }
            // A pure insertion goes after `old_start`; any other hunk ends
            // before `old_start + old_len`.
            let above = if hunk.old_len == 0 {
                hunk.old_start < base_line
            } else {
                hunk.old_start + hunk.old_len <= base_line
            };
            if above {
                shift += hunk.new_len.cast_signed() - hunk.old_len.cast_signed();
            }
        }
        base_line.saturating_add_signed(shift)
    }

    pub(crate) const fn notice(&self) -> Option<&InternalError> {
        self.notice.as_ref()
    }
}

/// `R<score>\0old\0new\0` records, every other status skipped with its path.
fn renames(names: &[u8]) -> BTreeMap<String, String> {
    let mut fields = names.split(|byte| *byte == 0).map(String::from_utf8_lossy);
    let mut renames = BTreeMap::new();
    while let Some(status) = fields.next() {
        if status.starts_with('R') || status.starts_with('C') {
            let (Some(old), Some(new)) = (fields.next(), fields.next()) else {
                break;
            };
            if status.starts_with('R') {
                renames.insert(old.into_owned(), new.into_owned());
            }
        } else if fields.next().is_none() {
            break;
        }
    }
    renames
}

/// The hunks of every file the patch names under `+++ b/`. A quoted name, the
/// one form git escapes, is left out, and its lines map one to one.
fn hunks(patch: &str) -> BTreeMap<String, Vec<Hunk>> {
    let mut hunks = BTreeMap::<String, Vec<Hunk>>::new();
    let mut current: Option<String> = None;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            current = None;
        } else if let Some(path) = line.strip_prefix("+++ ") {
            current = path
                .strip_prefix("b/")
                .map(|path| path.strip_suffix('\t').unwrap_or(path).to_owned());
        } else if let (Some(path), Some(header)) = (&current, line.strip_prefix("@@ -"))
            && let Some(hunk) = hunk(header)
        {
            hunks.entry(path.clone()).or_default().push(hunk);
        }
    }
    hunks
}

/// `a[,b] +c[,d] @@`, where an omitted length is one.
fn hunk(header: &str) -> Option<Hunk> {
    let mut ranges = header.split(' ');
    let range = |text: &str| -> Option<(usize, usize)> {
        let (start, len) = text.split_once(',').unwrap_or((text, "1"));
        Some((start.parse().ok()?, len.parse().ok()?))
    };
    let (old_start, old_len) = range(ranges.next()?)?;
    let (new_start, new_len) = range(ranges.next()?.strip_prefix('+')?)?;
    Some(Hunk {
        old_start,
        old_len,
        new_start,
        new_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rename_maps_its_path_and_a_line_follows_the_hunks_above_it() {
        let names = b"R095\0src/a.rs\0src/b.rs\0M\0src/c.rs\0";
        let patch = "diff --git a/src/a.rs b/src/b.rs\nsimilarity index 95%\n\
                     rename from src/a.rs\nrename to src/b.rs\n--- a/src/a.rs\n+++ b/src/b.rs\n\
                     @@ -2,0 +3,2 @@\n+x\n+y\n@@ -10 +12 @@\n-old\n+new\n";
        let map = DiffMap::parse(names, patch.as_bytes());
        assert_eq!(map.current_path("src/a.rs"), "src/b.rs");
        assert_eq!(map.current_path("src/c.rs"), "src/c.rs");
        assert_eq!(map.mapped_line("src/b.rs", 1), 1);
        assert_eq!(map.mapped_line("src/b.rs", 5), 7);
        assert_eq!(map.mapped_line("src/b.rs", 10), 12);
        assert_eq!(map.mapped_line("src/b.rs", 11), 13);
        assert_eq!(map.mapped_line("src/c.rs", 4), 4);
    }

    /// A base git cannot read is a map without renames or hunks, and the
    /// notice the comparison publishes instead of failing.
    #[test]
    fn a_git_that_fails_leaves_an_empty_map_and_a_notice() {
        let root = crate::test_scratch::scratch("delta", "diff-map-failure");
        let map = DiffMap::read(&root, "no-such-base", None, None);
        assert!(map.renames.is_empty() && map.hunks.is_empty());
        assert_eq!(map.mapped_line("src/a.rs", 7), 7);
        let notice = map.notice().unwrap();
        assert_eq!((notice.stage, notice.code), (STAGE, NOTICE_CODE));
    }

    #[test]
    fn a_rewritten_line_maps_into_what_replaced_it() {
        let patch = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -4,3 +4 @@\n-a\n-b\n-c\n+d\n";
        let map = DiffMap::parse(b"", patch.as_bytes());
        assert_eq!(map.mapped_line("x.rs", 4), 4);
        assert_eq!(map.mapped_line("x.rs", 6), 4);
        assert_eq!(map.mapped_line("x.rs", 7), 5);
    }
}
