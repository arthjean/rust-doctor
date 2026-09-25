//! The one way the installers write into a workspace: only the files they
//! name, never through a symlink, and never over a file they did not write.
//!
//! A repository is untrusted input to the tool that scans it, and a symlink
//! committed as `.claude` or `.git/hooks/pre-commit` would otherwise turn an
//! install into a write wherever the link points.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

/// The first component of `relative` under `root` that is a symlink, as a
/// path relative to `root`, which is how the refusal names it.
pub fn symlinked_component(root: &Path, relative: &Path) -> Option<PathBuf> {
    let mut walked = PathBuf::new();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        walked.push(name);
        match root.join(&walked).symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => return Some(walked),
            Ok(_) => {}
            // Nothing exists past a missing component, so nothing links there.
            Err(_) => return None,
        }
    }
    None
}

/// Creates `path` with `content`, refusing when anything already sits there,
/// a dangling symlink included: `create_new` does not follow one.
pub fn create_new(path: &Path, content: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(content)
}

/// Whether `name` resolves to an executable file on `PATH`, which is where a
/// hook that runs `rust-doctor` by name will look for it.
pub fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(name).is_file())
    })
}
