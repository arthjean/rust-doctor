use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::execution::InternalError;
use crate::git_scope::{self, GitCall, GitFailure};

pub(crate) const ENTRY_LIMIT: usize = 100_000;
pub(crate) const BLOB_LIMIT: u64 = 64 * 1024 * 1024;
pub(crate) const TOTAL_BLOB_LIMIT: u64 = 1024 * 1024 * 1024;
pub(crate) const INVENTORY_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const PATH_LIMIT: usize = 4_096;
const COMMAND_OUTPUT_LIMIT: usize = 4_096;
const TEMP_ATTEMPTS: usize = 128;

const INVENTORY_FAILURE: GitFailure = GitFailure::new(
    "baseline",
    "baseline-inventory-failed",
    "Git baseline inventory failed.",
);
const LIMIT_EXCEEDED: GitFailure = GitFailure::new(
    "baseline",
    "baseline-limit-exceeded",
    "Git baseline snapshot exceeds a supported limit.",
);
const MATERIALIZATION_FAILURE: GitFailure = GitFailure::new(
    "baseline",
    "baseline-materialization-failed",
    "Git baseline materialization failed.",
);

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct Inventory {
    symlinks: Vec<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct Snapshot {
    root: PathBuf,
    tree: PathBuf,
    target: PathBuf,
    workspace: PathBuf,
    cleanup_pending: bool,
}

impl Snapshot {
    pub(crate) fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub(crate) fn target(&self) -> &Path {
        &self.target
    }

    pub(crate) fn cleanup(mut self) -> Result<(), InternalError> {
        self.cleanup_with(remove_snapshot)
    }

    fn cleanup_with(
        &mut self,
        remove: impl FnOnce(&Path) -> io::Result<()>,
    ) -> Result<(), InternalError> {
        match remove(&self.root) {
            Ok(()) => {
                self.cleanup_pending = false;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.cleanup_pending = false;
                Ok(())
            }
            Err(_) => Err(cleanup_failed()),
        }
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        if self.cleanup_pending {
            let _ = remove_snapshot(&self.root);
        }
    }
}

pub(crate) fn materialize(
    workspace_root: &Path,
    comparison_base: &str,
) -> Result<Snapshot, InternalError> {
    let inventory_output = git_scope::run_git(
        Path::new("git"),
        workspace_root,
        &GitCall {
            arguments: git_scope::git_arguments(
                workspace_root,
                [
                    OsString::from("ls-tree"),
                    OsString::from("-r"),
                    OsString::from("-z"),
                    OsString::from("-l"),
                    OsString::from("--full-tree"),
                    OsString::from(comparison_base),
                ],
            ),
            stdout_limit: INVENTORY_OUTPUT_LIMIT,
            failure: INVENTORY_FAILURE,
            stdout_overflow: LIMIT_EXCEEDED,
        },
    )?;
    let inventory = parse_inventory(&inventory_output.stdout)?;
    let repository_root = repository_root(workspace_root)?;
    let workspace_relative = workspace_root
        .strip_prefix(&repository_root)
        .map_err(|_| materialization_failed())?;
    let root = create_temp_root(&repository_root)?;
    let tree = root.join("tree");
    let target = root.join("target");
    let workspace = tree.join(workspace_relative);
    let snapshot = Snapshot {
        root,
        tree,
        target,
        workspace,
        cleanup_pending: true,
    };

    let result = materialize_inventory(workspace_root, comparison_base, &inventory, &snapshot);
    match result {
        Ok(()) => Ok(snapshot),
        Err(error) => match snapshot.cleanup() {
            Ok(()) => Err(error),
            Err(cleanup) => Err(cleanup),
        },
    }
}

fn materialize_inventory(
    workspace_root: &Path,
    comparison_base: &str,
    inventory: &Inventory,
    snapshot: &Snapshot,
) -> Result<(), InternalError> {
    fs::create_dir(&snapshot.tree).map_err(|_| temp_unavailable())?;
    fs::create_dir(&snapshot.target).map_err(|_| temp_unavailable())?;
    let index = snapshot.root.join("index");

    git_scope::run_git_with_index(
        Path::new("git"),
        workspace_root,
        &GitCall {
            arguments: git_scope::git_arguments(
                workspace_root,
                [OsString::from("read-tree"), OsString::from(comparison_base)],
            ),
            stdout_limit: COMMAND_OUTPUT_LIMIT,
            failure: MATERIALIZATION_FAILURE,
            stdout_overflow: MATERIALIZATION_FAILURE,
        },
        &index,
    )?;

    let mut prefix = OsString::from("--prefix=");
    prefix.push(snapshot.tree.as_os_str());
    prefix.push(std::path::MAIN_SEPARATOR_STR);
    git_scope::run_git_with_index(
        Path::new("git"),
        workspace_root,
        &GitCall {
            arguments: git_scope::git_arguments(
                workspace_root,
                [
                    OsString::from("checkout-index"),
                    OsString::from("--all"),
                    OsString::from("--force"),
                    prefix,
                ],
            ),
            stdout_limit: COMMAND_OUTPUT_LIMIT,
            failure: MATERIALIZATION_FAILURE,
            stdout_overflow: MATERIALIZATION_FAILURE,
        },
        &index,
    )?;

    validate_materialized_symlinks(&snapshot.tree, &inventory.symlinks)
}

fn parse_inventory(output: &[u8]) -> Result<Inventory, InternalError> {
    if output.is_empty() {
        return Ok(Inventory {
            symlinks: Vec::new(),
        });
    }
    if !output.ends_with(&[0]) {
        return Err(inventory_failed());
    }

    let mut total_bytes = 0_u64;
    let mut paths = BTreeSet::new();
    let mut symlinks = Vec::new();
    for (entries, record) in output[..output.len() - 1]
        .split(|byte| *byte == 0)
        .enumerate()
    {
        if entries == ENTRY_LIMIT {
            return Err(limit_exceeded());
        }
        let Some(separator) = record.iter().position(|byte| *byte == b'\t') else {
            return Err(inventory_failed());
        };
        let (header, raw_path) = record.split_at(separator);
        let raw_path = &raw_path[1..];
        let fields: Vec<_> = header
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .collect();
        if fields.len() != 4 {
            return Err(inventory_failed());
        }
        let mode = fields[0];
        let kind = fields[1];
        let oid = fields[2];
        if kind != b"blob"
            || !matches!(mode, b"100644" | b"100755" | b"120000")
            || !matches!(oid.len(), 40 | 64)
            || !oid.iter().all(u8::is_ascii_hexdigit)
        {
            return Err(entry_invalid());
        }
        let size = parse_decimal(fields[3]).ok_or_else(inventory_failed)?;
        if size > BLOB_LIMIT {
            return Err(limit_exceeded());
        }
        total_bytes = total_bytes
            .checked_add(size)
            .filter(|total| *total <= TOTAL_BLOB_LIMIT)
            .ok_or_else(limit_exceeded)?;

        let path = validate_inventory_path(raw_path)?;
        if !paths.insert(path.clone()) {
            return Err(entry_invalid());
        }
        if mode == b"120000" {
            symlinks.push(PathBuf::from(path));
        }
    }
    Ok(Inventory { symlinks })
}

fn validate_inventory_path(raw_path: &[u8]) -> Result<String, InternalError> {
    if raw_path.is_empty() || raw_path.len() > PATH_LIMIT {
        return Err(entry_invalid());
    }
    let path = std::str::from_utf8(raw_path).map_err(|_| entry_invalid())?;
    if path
        .split('/')
        .any(|component| component.is_empty() || matches!(component, "." | ".."))
        || Path::new(path).is_absolute()
        || !Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(entry_invalid());
    }
    Ok(path.to_owned())
}

fn parse_decimal(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn validate_materialized_symlinks(tree: &Path, symlinks: &[PathBuf]) -> Result<(), InternalError> {
    for relative in symlinks {
        let link = tree.join(relative);
        let metadata = link.symlink_metadata().map_err(|_| entry_invalid())?;
        if !metadata.file_type().is_symlink() {
            return Err(entry_invalid());
        }
        let target = fs::read_link(&link).map_err(|_| entry_invalid())?;
        if target.as_os_str().is_empty()
            || target.is_absolute()
            || !target
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(entry_invalid());
        }
        let Some(parent) = link.parent() else {
            return Err(entry_invalid());
        };
        if !parent.join(target).starts_with(tree) {
            return Err(entry_invalid());
        }
    }
    Ok(())
}

fn repository_root(workspace_root: &Path) -> Result<PathBuf, InternalError> {
    workspace_root
        .ancestors()
        .find(|ancestor| ancestor.join(".git").symlink_metadata().is_ok())
        .map(Path::to_path_buf)
        .ok_or_else(materialization_failed)
}

fn create_temp_root(repository_root: &Path) -> Result<PathBuf, InternalError> {
    create_temp_root_in(repository_root, &env::temp_dir())
}

fn create_temp_root_in(
    repository_root: &Path,
    temporary_root: &Path,
) -> Result<PathBuf, InternalError> {
    let temporary = temporary_root
        .canonicalize()
        .map_err(|_| temp_unavailable())?;
    let repository = repository_root
        .canonicalize()
        .map_err(|_| temp_unavailable())?;
    if temporary.starts_with(&repository) {
        return Err(temp_unavailable());
    }
    for _ in 0..TEMP_ATTEMPTS {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let root = temporary.join(format!(
            "rust-doctor-baseline-{}-{sequence}",
            std::process::id()
        ));
        match create_private_directory(&root) {
            Ok(()) => {
                if finalize_private_permissions(&root).is_err() {
                    let _ = fs::remove_dir(&root);
                    return Err(temp_unavailable());
                }
                return Ok(root);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(temp_unavailable()),
        }
    }
    Err(temp_unavailable())
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

#[cfg(unix)]
fn finalize_private_permissions(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn finalize_private_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn remove_snapshot(root: &Path) -> io::Result<()> {
    fs::remove_dir_all(root)
}

fn inventory_failed() -> InternalError {
    INVENTORY_FAILURE.error()
}

fn limit_exceeded() -> InternalError {
    LIMIT_EXCEEDED.error()
}

fn entry_invalid() -> InternalError {
    InternalError::new(
        "baseline",
        "baseline-entry-invalid",
        "Git baseline contains an unsupported entry.",
    )
}

fn temp_unavailable() -> InternalError {
    InternalError::new(
        "baseline",
        "baseline-temp-unavailable",
        "Git baseline temporary storage is unavailable.",
    )
}

fn materialization_failed() -> InternalError {
    MATERIALIZATION_FAILURE.error()
}

pub(crate) fn scan_incomplete() -> InternalError {
    InternalError::new(
        "baseline",
        "baseline-scan-incomplete",
        "Git baseline scan is incomplete.",
    )
}

pub(crate) fn cleanup_failed() -> InternalError {
    InternalError::new(
        "baseline",
        "baseline-cleanup-failed",
        "Git baseline cleanup failed.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(mode: &str, kind: &str, size: u64, path: &str) -> Vec<u8> {
        format!("{mode} {kind} {} {size}\t{path}\0", "1".repeat(40)).into_bytes()
    }

    #[test]
    fn inventory_accepts_regular_files_and_symlinks_at_bounds() {
        let mut bytes = record("100644", "blob", BLOB_LIMIT, "src/lib.rs");
        bytes.extend(record("120000", "blob", 8, "linked.rs"));
        let inventory = parse_inventory(&bytes).unwrap();
        assert_eq!(inventory.symlinks, [PathBuf::from("linked.rs")]);
    }

    #[test]
    fn inventory_rejects_closed_entries_without_disclosing_paths() {
        let cases = [
            format!("160000 commit {} -\tprivate-gitlink\0", "1".repeat(40)).into_bytes(),
            record("100644", "blob", 1, "/private/absolute"),
            record("100644", "blob", 1, "private/../escape"),
            record("100644", "blob", 1, "private//empty"),
            record("100644", "blob", BLOB_LIMIT + 1, "private-large"),
        ];
        for bytes in cases {
            let error = parse_inventory(&bytes).unwrap_err();
            assert!(matches!(
                error.code,
                "baseline-entry-invalid" | "baseline-limit-exceeded"
            ));
            assert!(!error.message.contains("private"));
        }

        let mut non_utf8 = record("100644", "blob", 1, "valid");
        let path_start = non_utf8.len() - 6;
        non_utf8[path_start] = 0xff;
        assert_eq!(
            parse_inventory(&non_utf8).unwrap_err().code,
            "baseline-entry-invalid"
        );
    }

    #[test]
    fn inventory_cardinality_and_total_size_are_closed_at_first_excess() {
        let mut entries = Vec::new();
        for index in 0..ENTRY_LIMIT {
            entries.extend(record("100644", "blob", 1, &format!("{index:06}")));
        }
        assert!(parse_inventory(&entries).is_ok());
        entries.extend(record("100644", "blob", 1, "overflow"));
        assert_eq!(
            parse_inventory(&entries).unwrap_err().code,
            "baseline-limit-exceeded"
        );

        let mut total = Vec::new();
        for index in 0..16 {
            total.extend(record(
                "100644",
                "blob",
                BLOB_LIMIT,
                &format!("blob-{index}"),
            ));
        }
        assert!(parse_inventory(&total).is_ok());
        total.extend(record("100644", "blob", BLOB_LIMIT, "blob-overflow"));
        assert_eq!(
            parse_inventory(&total).unwrap_err().code,
            "baseline-limit-exceeded"
        );
    }

    #[test]
    fn inventory_path_length_accepts_the_limit_and_rejects_the_next_byte() {
        assert!(parse_inventory(&record("100644", "blob", 1, &"x".repeat(PATH_LIMIT))).is_ok());
        assert_eq!(
            parse_inventory(&record("100644", "blob", 1, &"x".repeat(PATH_LIMIT + 1)))
                .unwrap_err()
                .code,
            "baseline-entry-invalid"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_targets_are_relative_normal_and_contained() {
        use std::os::unix::fs::symlink;

        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/baseline-symlink-validation")
            .join(std::process::id().to_string());
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        symlink("lib.rs", root.join("src/internal.rs")).unwrap();
        assert!(validate_materialized_symlinks(&root, &[PathBuf::from("src/internal.rs")]).is_ok());

        fs::remove_file(root.join("src/internal.rs")).unwrap();
        symlink("../outside.rs", root.join("src/internal.rs")).unwrap();
        assert_eq!(
            validate_materialized_symlinks(&root, &[PathBuf::from("src/internal.rs")])
                .unwrap_err()
                .code,
            "baseline-entry-invalid"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cleanup_failure_is_closed_and_drop_retries_the_snapshot_root() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/baseline-cleanup-failure")
            .join(std::process::id().to_string());
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut snapshot = Snapshot {
            tree: root.join("tree"),
            target: root.join("target"),
            workspace: root.join("tree"),
            root: root.clone(),
            cleanup_pending: true,
        };
        let error = snapshot
            .cleanup_with(|_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "closed")))
            .unwrap_err();
        assert_eq!(
            (error.stage, error.code),
            ("baseline", "baseline-cleanup-failed")
        );
        drop(snapshot);
        assert!(!root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn temporary_root_is_created_atomically_private_and_outside_the_repository() {
        use std::os::unix::fs::PermissionsExt;

        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let root = create_temp_root(&repository).unwrap();
        assert!(!root.starts_with(&repository));
        assert_eq!(
            fs::symlink_metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn temporary_root_inside_the_repository_is_rejected() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let temporary = repository
            .join("target/baseline-temp-boundary")
            .join(std::process::id().to_string());
        let _ = fs::remove_dir_all(&temporary);
        fs::create_dir_all(&temporary).unwrap();

        let error = create_temp_root_in(&repository, &temporary).unwrap_err();

        assert_eq!(error.code, "baseline-temp-unavailable");
        fs::remove_dir_all(&temporary).unwrap();
    }

    #[test]
    fn production_limits_match_the_versioned_oracle() {
        let oracle: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/baseline/oracle.json")).unwrap();
        let limits = &oracle["limits"];

        assert_eq!(limits["entries"], ENTRY_LIMIT);
        assert_eq!(limits["blob_bytes"], BLOB_LIMIT);
        assert_eq!(limits["total_blob_bytes"], TOTAL_BLOB_LIMIT);
        assert_eq!(limits["inventory_stdout_bytes"], INVENTORY_OUTPUT_LIMIT);
        assert_eq!(limits["path_bytes"], PATH_LIMIT);
        assert_eq!(
            limits["stderr_bytes"],
            crate::git_scope::STDERR_OUTPUT_LIMIT
        );
    }
}
