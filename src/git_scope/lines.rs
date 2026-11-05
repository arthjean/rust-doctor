use std::collections::BTreeMap;

use crate::internal_error::InternalError;
use crate::workspace_path;

use super::{PATH_LIMIT, STAGE, invalid_path};

pub(crate) type LineRanges = BTreeMap<String, FileChanges>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FileChanges {
    pub(crate) ranges: Vec<(usize, usize)>,
    pub(crate) deletions: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewSide {
    Lines(usize, usize),
    After(usize),
}

pub(super) fn parse_patch(output: &[u8]) -> Result<LineRanges, InternalError> {
    let mut ranges = LineRanges::new();
    let mut current: Option<Option<String>> = None;
    let mut pending = 0_usize;
    for line in output.split(|byte| *byte == b'\n') {
        if pending > 0 {
            match line.first() {
                Some(b'+' | b'-' | b' ') => {
                    pending -= 1;
                    continue;
                }
                Some(b'\\') => continue,
                _ => return Err(invalid_patch()),
            }
        }
        if let Some(target) = line.strip_prefix(b"+++ ") {
            current = Some(new_side_path(target)?);
        } else if let Some(header) = line.strip_prefix(b"@@ ") {
            let path = current.as_ref().ok_or_else(invalid_patch)?;
            let (old_count, side) = hunk_counts(header)?;
            if let Some(path) = path {
                let changes = ranges.entry(path.clone()).or_default();
                match side {
                    NewSide::Lines(start, last) => changes.ranges.push((start, last)),
                    NewSide::After(line) => changes.deletions.push(line),
                }
            }
            let added = match side {
                NewSide::Lines(start, last) => last - start + 1,
                NewSide::After(_) => 0,
            };
            pending = old_count.checked_add(added).ok_or_else(invalid_patch)?;
        }
    }
    if pending > 0 {
        return Err(invalid_patch());
    }
    Ok(ranges)
}

fn new_side_path(target: &[u8]) -> Result<Option<String>, InternalError> {
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

fn hunk_counts(header: &[u8]) -> Result<(usize, NewSide), InternalError> {
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
        return Ok((old_count, NewSide::After(start)));
    }
    let last = start
        .checked_add(count - 1)
        .filter(|_| start > 0)
        .ok_or_else(invalid_patch)?;
    Ok((old_count, NewSide::Lines(start, last)))
}

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
    fn hunks_land_on_their_new_side_path_and_deletions_add_a_point() {
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
        let lines = |ranges: Vec<(usize, usize)>, deletions: Vec<usize>| FileChanges {
            ranges,
            deletions,
        };
        assert_eq!(
            ranges,
            LineRanges::from([
                ("moved.rs".to_owned(), lines(vec![(3, 3)], vec![])),
                ("q\"t.rs".to_owned(), lines(vec![(1, 1)], vec![])),
                (
                    "sp ace.rs".to_owned(),
                    lines(vec![(2, 2), (4, 4), (10, 12)], vec![6])
                ),
                ("é.rs".to_owned(), lines(vec![(1, 1)], vec![])),
            ])
        );
    }

    #[test]
    fn a_malformed_patch_is_refused_rather_than_read_as_no_change() {
        for patch in [
            &b"@@ -1 +1 @@\n"[..],
            b"+++ b/a.rs\n@@ -1 +x @@\n",
            b"+++ b/a.rs\n@@ -1 +0,2 @@\n",
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
