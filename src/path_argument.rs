//! Every path the command line names, checked once at the boundary.
//!
//! A relative PATH used to travel as given: git then received it twice, as
//! the directory it runs in and as `-C` of that same relative path, and
//! `hook install git crates/x` looked for `crates/x/crates/x`. A path that
//! did not exist reached the installers, which created directories for it,
//! and a mistyped `--advisory-db` silently switched the advisory check off.
//! Each command now resolves its paths here, before discovery or any git
//! call, and a path that cannot be used ends the run with exit 2 and one
//! stderr line naming it as the user typed it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The exit status of a command refused over its arguments.
pub(crate) const REFUSED: u8 = 2;

/// `given` resolved against the current directory, symbolic links included,
/// or the line already printed that says why it cannot be.
pub(crate) fn resolve(name: &str, given: &Path) -> Result<PathBuf, u8> {
    fs::canonicalize(given).map_err(|error| refuse(name, given, &error))
}

/// Refuses a path nothing can be read from, and answers nothing else: what
/// the path holds is for its reader to judge.
pub(crate) fn readable(name: &str, given: &Path) -> Result<(), u8> {
    let opened = match fs::metadata(given) {
        Ok(metadata) if metadata.is_dir() => fs::read_dir(given).map(drop),
        Ok(_) => fs::File::open(given).map(drop),
        Err(error) => Err(error),
    };
    opened.map_err(|error| refuse(name, given, &error))
}

fn refuse(name: &str, given: &Path, error: &io::Error) -> u8 {
    if error.kind() == io::ErrorKind::NotFound {
        eprintln!("rust-doctor: {name}: {} does not exist.", given.display());
    } else {
        eprintln!("rust-doctor: {name}: {} cannot be read: {error}.", given.display());
    }
    REFUSED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_path_is_refused_and_an_existing_one_resolves() {
        assert_eq!(resolve("PATH", Path::new("no/such/directory")), Err(REFUSED));
        assert_eq!(readable("--advisory-db", Path::new("no/such/database")), Err(REFUSED));
        let resolved = resolve("PATH", Path::new(".")).unwrap();
        assert!(resolved.is_absolute());
        assert_eq!(readable("--advisory-db", Path::new("/dev/null")), Ok(()));
    }
}
