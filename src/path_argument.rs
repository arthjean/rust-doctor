use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) const REFUSED: u8 = 2;

pub(crate) fn resolve(name: &str, given: &Path) -> Result<PathBuf, u8> {
    fs::canonicalize(given).map_err(|error| refuse(name, given, &error))
}

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
