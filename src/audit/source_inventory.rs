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
        files: source.counters.files_read,
        complete: source.errors.is_empty(),
    });

    match (compiler, source) {
        (Some(compiler), _) => compiler,
        (None, Some(source)) => source,
        (None, None) => target_root_inventory(metadata),
    }
}

fn target_root_inventory(metadata: &Metadata) -> SourceFileInventory {
    let Ok(workspace_root) = metadata.workspace_root.as_std_path().canonicalize() else {
        return SourceFileInventory::default();
    };
    let (_, roots, roots_are_complete) = workspace_targets(metadata, &workspace_root);
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
                if let Ok(Some(path)) =
                    confined_rust_file(&workspace_root, artifact.target.src_path.as_std_path())
                {
                    files.insert(path);
                }
            }
        }
    }

    if expects_artifact && !saw_artifact {
        files.extend(target_roots);
        complete = false;
    }

    SourceFileInventory {
        files: files.len(),
        complete: complete && (!expects_artifact || saw_artifact),
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
                Ok(Some(path)) => {
                    files.insert(path);
                }
                Ok(None) => {}
                Err(()) => complete = false,
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
        if let Some(path) = confined_rust_file(workspace_root, &path)? {
            files.insert(path);
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
    let separator = contents
        .windows(2)
        .position(|window| window[0] == b':' && window[1].is_ascii_whitespace())
        .ok_or(())?;
    let mut dependencies = Vec::new();
    let mut token = Vec::new();
    let mut index = separator + 1;
    while index < contents.len() {
        match contents[index] {
            b'\n' | b'\r' => {
                push_dependency(&mut dependencies, &mut token)?;
                break;
            }
            byte if byte.is_ascii_whitespace() => {
                push_dependency(&mut dependencies, &mut token)?;
                index += 1;
            }
            b'\\' => {
                let Some(next) = contents.get(index + 1).copied() else {
                    return Err(());
                };
                if next == b'\n' {
                    index += 2;
                } else if next == b'\r' && contents.get(index + 2) == Some(&b'\n') {
                    index += 3;
                } else if matches!(next, b' ' | b'\t' | b'#' | b':' | b'\\') {
                    token.push(next);
                    index += 2;
                } else {
                    token.push(b'\\');
                    index += 1;
                }
            }
            byte => {
                token.push(byte);
                index += 1;
            }
        }
    }
    if index == contents.len() {
        push_dependency(&mut dependencies, &mut token)?;
    }
    Ok(dependencies)
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

fn confined_rust_file(workspace_root: &Path, path: &Path) -> Result<Option<PathBuf>, ()> {
    if path.extension() != Some(OsStr::new("rs")) {
        return Ok(None);
    }
    let canonical = path.canonicalize().map_err(|_| ())?;
    if !canonical.starts_with(workspace_root) || !canonical.is_file() {
        return Ok(None);
    }
    Ok(Some(canonical))
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
