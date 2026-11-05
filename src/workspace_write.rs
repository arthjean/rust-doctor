use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

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
            Err(_) => return None,
        }
    }
    None
}

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

pub fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(name).is_file())
    })
}
