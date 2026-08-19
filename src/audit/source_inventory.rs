use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use cargo_metadata::{Artifact, Message, Metadata, PackageId};

use super::SourceFileInventory;
use crate::execution::{CapturedMessage, ScanExecution};
use crate::source_kernel::SourceScan;

// This is one budget for the complete scan, not one allowance per artifact. Otherwise a
// workspace with many dep-info files could multiply the limit without bound.
const DEP_INFO_BYTES_LIMIT: u64 = 8 * 1024 * 1024;

pub(super) fn collect(
    metadata: &Metadata,
    scan: Option<&ScanExecution>,
    source: Option<&SourceScan>,
) -> SourceFileInventory {
    let compiler = scan.map(|scan| compiler_inventory(metadata, &scan.messages));
    let source = source.map(|source| SourceFileInventory {
        files: source.counters.walk.files_read,
        complete: source.errors.is_empty(),
    });

    compiler
        .or(source)
        .unwrap_or_else(|| target_root_inventory(metadata))
}

fn target_root_inventory(metadata: &Metadata) -> SourceFileInventory {
    let Ok(workspace_root) = metadata.workspace_root.as_std_path().canonicalize() else {
        return SourceFileInventory::default();
    };
    let (_, roots, roots_are_complete) = workspace_targets(metadata, &workspace_root);
    // Target roots are a floor on the source files, never the count: every module a root
    // reaches is missing from it. Only a workspace with no target at all is counted exactly,
    // which is why anything but an empty set reports as partial.
    SourceFileInventory {
        files: roots.len(),
        complete: roots.is_empty() && roots_are_complete,
    }
}

fn compiler_inventory(metadata: &Metadata, messages: &[CapturedMessage]) -> SourceFileInventory {
    let Ok(workspace_root) = metadata.workspace_root.as_std_path().canonicalize() else {
        return SourceFileInventory::default();
    };
    let (workspace_packages, target_roots, mut complete) =
        workspace_targets(metadata, &workspace_root);
    let expects_artifact = !target_roots.is_empty();
    let mut files = BTreeSet::new();
    let mut saw_artifact = false;
    let mut remaining_dep_info_bytes = DEP_INFO_BYTES_LIMIT;

    for message in messages {
        let CapturedMessage::Known(message) = message else {
            continue;
        };
        let Message::CompilerArtifact(artifact) = message.as_ref() else {
            continue;
        };
        if !workspace_packages.contains(&artifact.package_id) {
            continue;
        }
        saw_artifact = true;
        match artifact_source_files(artifact, &workspace_root, &mut remaining_dep_info_bytes) {
            Ok(paths) => files.extend(paths),
            Err(()) => {
                complete = false;
                if let Confined::Counted(path) =
                    confined_rust_file(&workspace_root, artifact.target.src_path.as_std_path())
                {
                    files.insert(path);
                }
            }
        }
    }

    // Cargo answered with no artifact for a workspace that declares targets, so the dep-info
    // inventory never happened: fall back to the roots and say the count is a floor.
    if expects_artifact && !saw_artifact {
        files.extend(target_roots);
        complete = false;
    }

    SourceFileInventory {
        files: files.len(),
        complete,
    }
}

fn workspace_targets(
    metadata: &Metadata,
    workspace_root: &Path,
) -> (BTreeSet<PackageId>, BTreeSet<PathBuf>, bool) {
    let mut packages = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut complete = true;
    for package in metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
    {
        let is_internal = package
            .manifest_path
            .as_std_path()
            .parent()
            .and_then(|path| path.canonicalize().ok())
            .is_some_and(|path| path.starts_with(workspace_root));
        if !is_internal {
            continue;
        }
        packages.insert(package.id.clone());
        for target in &package.targets {
            match confined_rust_file(workspace_root, target.src_path.as_std_path()) {
                Confined::Counted(path) => {
                    files.insert(path);
                }
                Confined::Excluded => {}
                Confined::Unresolved => complete = false,
            }
        }
    }
    (packages, files, complete)
}

fn artifact_source_files(
    artifact: &Artifact,
    workspace_root: &Path,
    remaining_dep_info_bytes: &mut u64,
) -> Result<BTreeSet<PathBuf>, ()> {
    let dep_info = artifact
        .filenames
        .iter()
        .filter_map(|filename| {
            dep_info_path(filename.as_std_path(), artifact.target.is_custom_build())
        })
        .find_map(|path| fs::File::open(path).ok())
        .ok_or(())?;
    let contents = read_with_budget(dep_info, remaining_dep_info_bytes)?;
    let dependencies = parse_dep_info(&contents)?;
    let mut files = BTreeSet::new();
    for dependency in dependencies {
        let path = if dependency.is_absolute() {
            dependency
        } else {
            workspace_root.join(dependency)
        };
        match confined_rust_file(workspace_root, &path) {
            Confined::Counted(path) => {
                files.insert(path);
            }
            Confined::Excluded => {}
            Confined::Unresolved => return Err(()),
        }
    }
    Ok(files)
}

fn read_with_budget(reader: impl Read, remaining_dep_info_bytes: &mut u64) -> Result<Vec<u8>, ()> {
    let read_limit = remaining_dep_info_bytes.checked_add(1).ok_or(())?;
    let mut contents = Vec::new();
    reader
        .take(read_limit)
        .read_to_end(&mut contents)
        .map_err(|_| ())?;
    let bytes_read = u64::try_from(contents.len()).map_err(|_| ())?;
    if bytes_read > *remaining_dep_info_bytes {
        *remaining_dep_info_bytes = 0;
        return Err(());
    }
    *remaining_dep_info_bytes -= bytes_read;
    Ok(contents)
}

fn dep_info_path(artifact: &Path, custom_build: bool) -> Option<PathBuf> {
    if custom_build {
        let directory = artifact.parent()?;
        let hash = directory.file_name()?.to_str()?.rsplit_once('-')?.1;
        return Some(directory.join(format!("build_script_build-{hash}.d")));
    }
    let stem = artifact.file_stem()?.to_str()?;
    let stem = stem.strip_prefix("lib").unwrap_or(stem);
    Some(artifact.with_file_name(format!("{stem}.d")))
}

fn parse_dep_info(contents: &[u8]) -> Result<Vec<PathBuf>, ()> {
    // Cargo dep-info is Makefile-like. We need only the first target's dependency list, so this
    // parser handles its escaping and continuations and fails closed on malformed input instead
    // of accepting the rest of the file as source inventory.
    //
    // The walk consumes the slice rather than indexing it: the bound on where the cursor may
    // land is then the slice itself, not arithmetic a reader has to replay to trust the end.
    let separator = contents
        .windows(2)
        .position(|window| matches!(window, [b':', byte] if byte.is_ascii_whitespace()))
        .ok_or(())?;
    let mut dependencies = Vec::new();
    let mut token = Vec::new();
    let mut rest = contents.get(separator + 1..).ok_or(())?;
    while let Some((&byte, tail)) = rest.split_first() {
        rest = match byte {
            // The first rule ends at its first unescaped line break, and whatever follows
            // describes another target.
            b'\n' | b'\r' => break,
            b'\\' => escape(tail, &mut token)?,
            _ if byte.is_ascii_whitespace() => {
                push_dependency(&mut dependencies, &mut token)?;
                tail
            }
            _ => {
                token.push(byte);
                tail
            }
        };
    }
    push_dependency(&mut dependencies, &mut token)?;
    Ok(dependencies)
}

/// Consumes one backslash escape and answers what is left after it.
///
/// Cargo escapes a space, a tab, a hash, a colon and a backslash inside a path, and continues a
/// rule across lines with a backslash before the line break. Any other backslash stands for
/// itself, so the byte after it is handed back unread rather than consumed.
fn escape<'a>(tail: &'a [u8], token: &mut Vec<u8>) -> Result<&'a [u8], ()> {
    match tail {
        [b'\n', rest @ ..] => Ok(rest),
        [b'\r', b'\n', rest @ ..] => Ok(rest),
        [byte @ (b' ' | b'\t' | b'#' | b':' | b'\\'), rest @ ..] => {
            token.push(*byte);
            Ok(rest)
        }
        [_, ..] => {
            token.push(b'\\');
            Ok(tail)
        }
        [] => Err(()),
    }
}

fn push_dependency(dependencies: &mut Vec<PathBuf>, token: &mut Vec<u8>) -> Result<(), ()> {
    if token.is_empty() {
        return Ok(());
    }
    let value = std::str::from_utf8(token).map_err(|_| ())?;
    dependencies.push(PathBuf::from(value));
    token.clear();
    Ok(())
}

/// What one candidate path contributes to the inventory.
///
/// The three outcomes are not two: a path that is not workspace source is a fact, a path that
/// could not be resolved is the absence of one. Only the second costs the inventory its
/// completeness, and spelling them apart is what keeps a caller from trading one for the other.
enum Confined {
    /// Workspace source, canonicalized, to count once.
    Counted(PathBuf),
    /// Not workspace source: not Rust, not a file, or outside the workspace root.
    Excluded,
    /// The path did not resolve, so nothing can be said about what it holds.
    Unresolved,
}

fn confined_rust_file(workspace_root: &Path, path: &Path) -> Confined {
    if path.extension() != Some(OsStr::new("rs")) {
        return Confined::Excluded;
    }
    let Ok(canonical) = path.canonicalize() else {
        return Confined::Unresolved;
    };
    if !canonical.starts_with(workspace_root) || !canonical.is_file() {
        return Confined::Excluded;
    }
    Confined::Counted(canonical)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn dep_info_parser_handles_escaped_paths_and_continuations() {
        let dependencies = parse_dep_info(
            concat!(
                "target/file: src/lib.rs src/with\\ space.rs \\\n",
                "src/hash\\#tag.rs\n\nignored: other.rs\n",
            )
            .as_bytes(),
        )
        .unwrap();

        assert_eq!(
            dependencies,
            [
                PathBuf::from("src/lib.rs"),
                PathBuf::from("src/with space.rs"),
                PathBuf::from("src/hash#tag.rs"),
            ]
        );
    }

    #[test]
    fn dep_info_parser_carries_a_crlf_continuation_and_stops_at_the_first_rule() {
        let dependencies = parse_dep_info(
            concat!(
                "target/file: src/a.rs \\\r\n",
                "src/b.rs\r\n",
                "target/other: src/c.rs\r\n",
            )
            .as_bytes(),
        )
        .unwrap();

        assert_eq!(
            dependencies,
            [PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")]
        );
    }

    /// A backslash before anything Cargo does not escape stands for itself, and the byte after
    /// it is read again rather than swallowed: a lone carriage return still ends the rule.
    #[test]
    fn an_unrecognized_escape_keeps_its_backslash_and_yields_the_next_byte() {
        assert_eq!(
            parse_dep_info(b"target/file: src/a\\b.rs src/c\\\rignored.rs"),
            Ok(vec![PathBuf::from("src/a\\b.rs"), PathBuf::from("src/c\\")])
        );
    }

    #[test]
    fn dep_info_parser_rejects_a_trailing_escape() {
        assert_eq!(parse_dep_info(b"target/file: src/a.rs \\"), Err(()));
    }

    #[test]
    fn dep_info_parser_rejects_missing_rule_separator() {
        assert_eq!(parse_dep_info(b"not a dep-info rule"), Err(()));
    }

    #[test]
    fn custom_build_artifact_resolves_cargo_dep_info_name() {
        let artifact = Path::new("target/debug/build/example-a1b2c3/build-script-build");

        assert_eq!(
            dep_info_path(artifact, true),
            Some(PathBuf::from(
                "target/debug/build/example-a1b2c3/build_script_build-a1b2c3.d"
            ))
        );
    }

    #[test]
    fn dep_info_reads_share_one_strict_budget() {
        let mut remaining = 5;

        assert_eq!(
            read_with_budget(Cursor::new(b"abc"), &mut remaining),
            Ok(b"abc".to_vec())
        );
        assert_eq!(remaining, 2);
        assert_eq!(
            read_with_budget(Cursor::new(b"xyz"), &mut remaining),
            Err(())
        );
        assert_eq!(remaining, 0);
    }
}
