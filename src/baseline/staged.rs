//! The index a `--staged` scan judges, written out as the tree every producer
//! reads in place of the working tree.
//!
//! It is the baseline snapshot with a different source: a commit there, the
//! index here, both checked out by `checkout-index` into a private temporary
//! root, so the two share the checks on what that tree may hold.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::git::{self, GitCall, GitFailure};
use crate::internal_error::InternalError;

use super::{
    COMMAND_OUTPUT_LIMIT, ENTRY_LIMIT, INVENTORY_OUTPUT_LIMIT, Snapshot, create_temp_root,
    repository_root, temp_unavailable, validate_inventory_path, validate_materialized_symlinks,
};

/// The index a `--staged` scan judges, read and checked once.
///
/// Git hands a hook the index it is about to commit in `GIT_INDEX_FILE`, which
/// is `.git/index`, or a lock file of its own when the commit names paths or
/// passes `-a`. That one is honored; otherwise the repository's own index is
/// read, and refused while another git process holds its lock, since what it
/// holds then is a write half done. The variable is still removed from every
/// other git call, which is what keeps it from redirecting those.
#[derive(Debug)]
pub(crate) struct StagedIndex {
    path: PathBuf,
    symlinks: Vec<PathBuf>,
}

impl StagedIndex {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

const INDEX_STAGE: &str = "scope";

const INDEX_UNAVAILABLE: GitFailure = GitFailure::new(
    "index-unavailable",
    "The git index could not be read: nothing was judged.",
);

pub(crate) fn locate_index(workspace_root: &Path) -> Result<StagedIndex, InternalError> {
    let unavailable = || INDEX_UNAVAILABLE.error(INDEX_STAGE);
    let path = match env::var_os("GIT_INDEX_FILE").filter(|file| !file.is_empty()) {
        // Git resolves a relative index against the directory it runs in,
        // which is also the one it started the hook in.
        Some(file) => env::current_dir().map_err(|_| unavailable())?.join(file),
        None => {
            let answer = git::run_git(
                Path::new("git"),
                workspace_root,
                &index_call(
                    workspace_root,
                    ["rev-parse", "--path-format=absolute", "--git-path", "index"],
                    COMMAND_OUTPUT_LIMIT,
                ),
            )?;
            let answer = String::from_utf8(answer).map_err(|_| unavailable())?;
            let path = PathBuf::from(answer.trim_end_matches('\n'));
            let mut lock = path.clone().into_os_string();
            lock.push(".lock");
            if Path::new(&lock).symlink_metadata().is_ok() {
                return Err(unavailable());
            }
            path
        }
    };
    if !path.is_file() {
        return Err(unavailable());
    }
    let repository_root = repository_root(workspace_root).map_err(|_| unavailable())?;
    let listing = git::run_git_with_index(
        Path::new("git"),
        &repository_root,
        &index_call(
            &repository_root,
            ["ls-files", "--stage", "-z"],
            INVENTORY_OUTPUT_LIMIT,
        ),
        &path,
    )?;
    Ok(StagedIndex {
        symlinks: parse_index_listing(&listing)?,
        path,
    })
}

fn index_call<const N: usize>(root: &Path, operation: [&str; N], limit: usize) -> GitCall {
    GitCall {
        arguments: git::git_arguments(root, operation),
        stdout_limit: limit,
        stage: INDEX_STAGE,
        failure: INDEX_UNAVAILABLE,
        overflow: INDEX_UNAVAILABLE,
    }
}

/// Reads `ls-files --stage -z`, answering the symlinks to check once the tree
/// is written. An entry at any stage but 0 is half of a merge conflict, and a
/// scan of one side of it would judge code nobody wrote.
fn parse_index_listing(output: &[u8]) -> Result<Vec<PathBuf>, InternalError> {
    let unavailable = || INDEX_UNAVAILABLE.error(INDEX_STAGE);
    let Some((&0, entries)) = output.split_last() else {
        return if output.is_empty() {
            Ok(Vec::new())
        } else {
            Err(unavailable())
        };
    };
    let mut symlinks = Vec::new();
    for (count, record) in entries.split(|byte| *byte == 0).enumerate() {
        if count == ENTRY_LIMIT {
            return Err(unavailable());
        }
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(unavailable)?;
        let (header, raw_path) = record.split_at(separator);
        let fields: Vec<_> = header.split(|byte| *byte == b' ').collect();
        let [mode, _, stage] = fields.as_slice() else {
            return Err(unavailable());
        };
        if *stage != b"0" {
            return Err(InternalError::new(
                INDEX_STAGE,
                "index-conflicted",
                "The index has unmerged entries: resolve them before a staged scan.",
            ));
        }
        let path = validate_inventory_path(raw_path.get(1..).unwrap_or_default()).map_err(|_| unavailable())?;
        match *mode {
            b"100644" | b"100755" => {}
            b"120000" => symlinks.push(PathBuf::from(path)),
            // A submodule is a commit, not a file, and checkout writes none.
            b"160000" => {}
            _ => return Err(unavailable()),
        }
    }
    Ok(symlinks)
}

/// Writes the index into a private temporary tree, the way [`materialize`]
/// writes a commit, and answers the snapshot every producer then reads in
/// place of the working tree.
pub(crate) fn materialize_index(
    workspace_root: &Path,
    index: &StagedIndex,
) -> Result<Snapshot, InternalError> {
    let failed = || {
        InternalError::new(
            INDEX_STAGE,
            "staged-materialization-failed",
            "The staged tree could not be written.",
        )
    };
    let repository_root = repository_root(workspace_root).map_err(|_| failed())?;
    let workspace_relative = workspace_root
        .strip_prefix(&repository_root)
        .map_err(|_| failed())?;
    let root = create_temp_root(&repository_root)?.into_path();
    let tree = root.join("tree");
    let snapshot = Snapshot {
        target: root.join("target"),
        workspace: tree.join(workspace_relative),
        tree,
        root,
        cleanup_pending: true,
    };
    let written = (|| {
        fs::create_dir(&snapshot.tree).map_err(|_| temp_unavailable())?;
        fs::create_dir(&snapshot.target).map_err(|_| temp_unavailable())?;
        let mut prefix = OsString::from("--prefix=");
        prefix.push(snapshot.tree.as_os_str());
        prefix.push(std::path::MAIN_SEPARATOR_STR);
        git::run_git_with_index(
            Path::new("git"),
            workspace_root,
            &GitCall {
                arguments: git::git_arguments(
                    workspace_root,
                    [
                        OsString::from("checkout-index"),
                        OsString::from("--all"),
                        OsString::from("--force"),
                        prefix,
                    ],
                ),
                stdout_limit: COMMAND_OUTPUT_LIMIT,
                stage: INDEX_STAGE,
                failure: GitFailure::new(
                    "staged-materialization-failed",
                    "The staged tree could not be written.",
                ),
                overflow: GitFailure::new(
                    "staged-materialization-failed",
                    "The staged tree could not be written.",
                ),
            },
            &index.path,
        )?;
        validate_materialized_symlinks(&snapshot.tree, &index.symlinks).map_err(|_| failed())
    })();
    match written {
        Ok(()) => Ok(snapshot),
        Err(error) => match snapshot.cleanup() {
            Ok(()) => Err(error),
            Err(cleanup) => Err(cleanup),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_listing_refuses_conflicts_and_keeps_symlinks_to_check() {
        let oid = "1".repeat(40);
        let listing = format!(
            "100644 {oid} 0\tsrc/lib.rs\0120000 {oid} 0\tlinked.rs\0160000 {oid} 0\tvendor/sub\0"
        );
        assert_eq!(
            parse_index_listing(listing.as_bytes()).unwrap(),
            [PathBuf::from("linked.rs")]
        );
        assert!(parse_index_listing(b"").unwrap().is_empty());

        let conflicted = format!("100644 {oid} 1\tsrc/lib.rs\0100644 {oid} 2\tsrc/lib.rs\0");
        let error = parse_index_listing(conflicted.as_bytes()).unwrap_err();
        assert_eq!((error.stage, error.code), ("scope", "index-conflicted"));

        for broken in [
            format!("100644 {oid} 0\t../escape.rs\0"),
            format!("040000 {oid} 0\tsrc\0"),
            format!("100644 {oid} 0\tunterminated"),
        ] {
            let error = parse_index_listing(broken.as_bytes()).unwrap_err();
            assert_eq!((error.stage, error.code), ("scope", "index-unavailable"));
        }
    }
}
