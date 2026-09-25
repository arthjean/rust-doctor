//! The new-side line ranges of a `git diff --unified=0` patch, keyed by the
//! path each hunk lands in.
//!
//! Between hunks, two kinds of line are read: `+++ ` names the file the
//! following hunks belong to, and `@@ ` opens a hunk and states both sides'
//! counts, which is how many content lines follow and are skipped unread.
//! Everything else is metadata this scope has no use for. A
//! pure deletion states a count of zero and contributes no range, and a file
//! whose new side is `/dev/null` was deleted and has none either.

use std::collections::BTreeMap;

use crate::internal_error::InternalError;
use crate::workspace_path;

use super::{PATH_LIMIT, STAGE, invalid_path};

/// Inclusive `[first, last]` new-side line ranges, per workspace-relative path.
pub(crate) type LineRanges = BTreeMap<String, Vec<(usize, usize)>>;

pub(super) fn parse_patch(output: &[u8]) -> Result<LineRanges, InternalError> {
    let mut ranges = LineRanges::new();
    // Outer `None`: no file header yet, so a hunk is out of place. Inner
    // `None`: the file was deleted, and its hunks land nowhere.
    let mut current: Option<Option<String>> = None;
    // Content lines the open hunk still holds. An added line whose text starts
    // with `++ ` is spelled `+++ ` in the patch, so a header is only read once
    // the hunk's stated counts are exhausted.
    let mut pending = 0_usize;
    for line in output.split(|byte| *byte == b'\n') {
        if pending > 0 {
            match line.first() {
                Some(b'+' | b'-' | b' ') => {
                    pending -= 1;
                    continue;
                }
                // `\ No newline at end of file` qualifies the line before it.
                Some(b'\\') => continue,
                _ => return Err(invalid_patch()),
            }
        }
        if let Some(target) = line.strip_prefix(b"+++ ") {
            current = Some(new_side_path(target)?);
        } else if let Some(header) = line.strip_prefix(b"@@ ") {
            let path = current.as_ref().ok_or_else(invalid_patch)?;
            let (old_count, range) = hunk_counts(header)?;
            if let (Some(path), Some(range)) = (path, range) {
                ranges.entry(path.clone()).or_default().push(range);
            }
            pending = old_count
                .checked_add(range.map_or(0, |(start, last)| last - start + 1))
                .ok_or_else(invalid_patch)?;
        }
    }
    if pending > 0 {
        return Err(invalid_patch());
    }
    Ok(ranges)
}

/// The path a `+++ ` line names, or `None` for a deleted file.
fn new_side_path(target: &[u8]) -> Result<Option<String>, InternalError> {
    // A path holding a space is followed by a tab, so a reader of the patch can
    // tell where it ends. Nothing a path may hold is a trailing tab otherwise:
    // git quotes a path that carries one.
    let target = target.strip_suffix(b"\t").unwrap_or(target);
    if target == b"/dev/null" {
        return Ok(None);
    }
    let unquoted = if target.first() == Some(&b'"') {
        unquote(target).ok_or_else(invalid_path)?
    } else {
        target.to_vec()
    };
    let relative = unquoted.strip_prefix(b"b/").ok_or_else(invalid_path)?;
    if relative.is_empty() || relative.len() > PATH_LIMIT {
        return Err(invalid_path());
    }
    let path = std::str::from_utf8(relative).map_err(|_| invalid_path())?;
    workspace_path::normalize_changed(path)
        .map(Some)
        .ok_or_else(invalid_path)
}

/// The old-side count and the new-side range of `-a,b +c,d @@`, the range
/// `None` when `d` is zero.
fn hunk_counts(header: &[u8]) -> Result<(usize, Option<(usize, usize)>), InternalError> {
    let header = std::str::from_utf8(header).map_err(|_| invalid_patch())?;
    let side = |sign: char| -> Result<(usize, usize), InternalError> {
        let field = header
            .split(' ')
            .find_map(|field| field.strip_prefix(sign))
            .ok_or_else(invalid_patch)?;
        let (start, count) = field.split_once(',').unwrap_or((field, "1"));
        Ok((
            start.parse().map_err(|_| invalid_patch())?,
            count.parse().map_err(|_| invalid_patch())?,
        ))
    };
    let (_, old_count) = side('-')?;
    let (start, count) = side('+')?;
    if count == 0 {
        return Ok((old_count, None));
    }
    let last = start
        .checked_add(count - 1)
        .filter(|_| start > 0)
        .ok_or_else(invalid_patch)?;
    Ok((old_count, Some((start, last))))
}

/// Undoes git's C-style quoting of a path: `\"`, `\\`, the named escapes and
/// three-digit octal bytes, which is how a non-ASCII byte is spelled.
fn unquote(quoted: &[u8]) -> Option<Vec<u8>> {
    let inner = quoted.strip_prefix(b"\"")?.strip_suffix(b"\"")?;
    let mut bytes = Vec::with_capacity(inner.len());
    let mut index = 0;
    while let Some(&byte) = inner.get(index) {
        index += 1;
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        let escaped = *inner.get(index)?;
        index += 1;
        bytes.push(match escaped {
            b'"' | b'\\' => escaped,
            b'a' => 0x07,
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 0x0b,
            b'0'..=b'3' => {
                let digits = inner.get(index..index + 2)?;
                index += 2;
                digits.iter().try_fold(escaped - b'0', |value, digit| {
                    matches!(digit, b'0'..=b'7').then(|| value * 8 + (digit - b'0'))
                })?
            }
            _ => return None,
        });
    }
    Some(bytes)
}

fn invalid_patch() -> InternalError {
    InternalError::new(
        STAGE,
        "git-diff-failed",
        "Git changed lines could not be read.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunks_land_on_their_new_side_path_and_deletions_add_nothing() {
        let patch = b"diff --git a/gone.rs b/gone.rs\n\
deleted file mode 100644\n\
--- a/gone.rs\n\
+++ /dev/null\n\
@@ -1 +0,0 @@\n\
-x\n\
diff --git a/mv.rs b/moved.rs\n\
rename from mv.rs\n\
rename to moved.rs\n\
--- a/mv.rs\n\
+++ b/moved.rs\n\
@@ -3 +3 @@ two\n\
-three\n\
+THREE\n\
--- a/sp ace.rs\t\n\
+++ b/sp ace.rs\t\n\
@@ -2 +2 @@ a\n\
-b\n\
+B\n\
@@ -3,0 +4 @@ c\n\
+++ b/not-a-header.rs\n\
\\ No newline at end of file\n\
@@ -7,2 +6,0 @@\n\
--- a/not-a-header.rs\n\
-y\n\
@@ -9 +10,3 @@\n\
-z\n\
+1\n\
+2\n\
+3\n\
--- \"a/\\303\\251.rs\"\n\
+++ \"b/\\303\\251.rs\"\n\
@@ -1 +1 @@\n\
-a\n\
+b\n\
--- \"a/q\\\"t.rs\"\n\
+++ \"b/q\\\"t.rs\"\n\
@@ -1 +1 @@\n\
-a\n\
+b\n";
        let ranges = parse_patch(patch).unwrap();
        assert_eq!(
            ranges,
            LineRanges::from([
                ("moved.rs".to_owned(), vec![(3, 3)]),
                ("q\"t.rs".to_owned(), vec![(1, 1)]),
                ("sp ace.rs".to_owned(), vec![(2, 2), (4, 4), (10, 12)]),
                ("é.rs".to_owned(), vec![(1, 1)]),
            ])
        );
    }

    #[test]
    fn a_malformed_patch_is_refused_rather_than_read_as_no_change() {
        for patch in [
            &b"@@ -1 +1 @@\n"[..],
            b"+++ b/a.rs\n@@ -1 +x @@\n",
            b"+++ b/a.rs\n@@ -1 +0,2 @@\n",
            // A hunk cut short of the lines its header states.
            b"+++ b/a.rs\n@@ -1 +1,2 @@\n-a\n+b\n",
            b"+++ \"b/unterminated\n",
            b"+++ \"b/bad\\q\"\n",
            b"+++ a/../escape.rs\n",
        ] {
            assert!(
                parse_patch(patch).is_err(),
                "{}",
                String::from_utf8_lossy(patch)
            );
        }
    }
}
