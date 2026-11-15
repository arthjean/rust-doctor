use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const VERBATIM: &str = "\\\\?\\";
const VERBATIM_UNC: &str = "\\\\?\\UNC\\";

const MAX_PLAIN_UNITS: usize = 260;

const RESERVED_DEVICES: [&str; 26] = [
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5",
    "COM6", "COM7", "COM8", "COM9", "COM\u{b9}", "COM\u{b2}", "COM\u{b3}", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
];

const LAST_PRINTER_DEVICES: [&str; 4] = ["LPT9", "LPT\u{b9}", "LPT\u{b2}", "LPT\u{b3}"];

pub fn canonical(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let resolved = fs::canonicalize(path)?;
    Ok(if cfg!(windows) { plain(resolved) } else { resolved })
}

pub fn within(child: &Path, root: &Path) -> bool {
    remainder(child, root).is_some()
}

pub fn relative_to(child: &Path, root: &Path) -> Option<PathBuf> {
    remainder(child, root)?
        .into_iter()
        .map(component_name)
        .collect()
}

fn plain(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(plain_form) {
        Some(text) => PathBuf::from(text),
        None => path,
    }
}

fn plain_form(text: &str) -> Option<String> {
    let (plain, body) = if let Some(share) = text.strip_prefix(VERBATIM_UNC) {
        (format!("\\\\{share}"), share)
    } else {
        let disk = text.strip_prefix(VERBATIM)?;
        let (drive, rest) = (disk.get(..2)?, disk.get(2..)?);
        let letter = drive.strip_suffix(':')?;
        if !letter.chars().all(|letter| letter.is_ascii_alphabetic()) || !rest.starts_with('\\') {
            return None;
        }
        (disk.to_owned(), rest)
    };
    let representable = plain.encode_utf16().count() < MAX_PLAIN_UNITS
        && body
            .split('\\')
            .filter(|component| !component.is_empty())
            .all(survives_without_the_prefix);
    representable.then_some(plain)
}

fn survives_without_the_prefix(component: &str) -> bool {
    if component.ends_with([' ', '.']) || component.contains('/') {
        return false;
    }
    let stem = component.split('.').next().unwrap_or(component).trim_end_matches(' ');
    !RESERVED_DEVICES
        .iter()
        .chain(&LAST_PRINTER_DEVICES)
        .any(|device| stem.eq_ignore_ascii_case(device))
}

#[derive(Debug, PartialEq, Eq)]
enum Root {
    Disk(u8),
    Share,
    Verbatim,
    Directory,
    Relative,
}

#[derive(Debug, PartialEq, Eq)]
struct Form<'a> {
    root: Root,
    components: Vec<&'a [u8]>,
}

impl<'a> Form<'a> {
    fn of(path: &'a Path) -> Self {
        let bytes = path.as_os_str().as_encoded_bytes();
        let foreign = *b"\\/";
        if let Some(share) = bytes.strip_prefix(VERBATIM_UNC.as_bytes()) {
            return Self::split(Root::Share, share, &foreign);
        }
        if let Some(verbatim) = bytes.strip_prefix(VERBATIM.as_bytes()) {
            return match disk(verbatim) {
                Some((letter, rest)) => Self::split(Root::Disk(letter), rest, &foreign),
                None => Self::split(Root::Verbatim, verbatim, &foreign),
            };
        }
        if let Some((letter, rest)) = disk(bytes) {
            return Self::split(Root::Disk(letter), rest, &foreign);
        }
        if let Some(share) = bytes.strip_prefix(b"\\\\") {
            return Self::split(Root::Share, share, &foreign);
        }
        let native: &[u8] = if cfg!(windows) { &foreign } else { b"/" };
        let root = match bytes.first() {
            Some(first) if native.contains(first) => Root::Directory,
            _ => Root::Relative,
        };
        Self::split(root, bytes, native)
    }

    fn split(root: Root, body: &'a [u8], separators: &[u8]) -> Self {
        let components = body
            .split(|byte| separators.contains(byte))
            .filter(|component| !component.is_empty() && *component != b".")
            .collect();
        Self { root, components }
    }
}

fn disk(bytes: &[u8]) -> Option<(u8, &[u8])> {
    match bytes {
        [letter, b':', separator, rest @ ..]
            if letter.is_ascii_alphabetic() && matches!(separator, b'\\' | b'/') =>
        {
            Some((letter.to_ascii_uppercase(), rest))
        }
        _ => None,
    }
}

fn remainder<'a>(child: &'a Path, root: &Path) -> Option<Vec<&'a [u8]>> {
    let (child, root) = (Form::of(child), Form::of(root));
    if child.root != root.root || child.components.len() < root.components.len() {
        return None;
    }
    let (shared, rest) = child.components.split_at(root.components.len());
    (shared == root.components.as_slice()).then(|| rest.to_vec())
}

#[cfg(unix)]
fn component_name(bytes: &[u8]) -> Option<&OsStr> {
    use std::os::unix::ffi::OsStrExt;

    Some(OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn component_name(bytes: &[u8]) -> Option<&OsStr> {
    std::str::from_utf8(bytes).ok().map(OsStr::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verbatim_path_takes_the_plain_form_cargo_and_git_print() {
        assert_eq!(plain_form("\\\\?\\C:\\x").as_deref(), Some("C:\\x"));
        assert_eq!(
            plain_form("\\\\?\\UNC\\server\\share\\x").as_deref(),
            Some("\\\\server\\share\\x")
        );
        assert_eq!(plain_form("\\\\?\\C:\\").as_deref(), Some("C:\\"));
        for kept in ["C:\\x", "/home/x", "\\\\server\\share", "\\\\?\\Volume{4c1b02c1}\\x", "\\\\?\\C:x"] {
            assert_eq!(plain_form(kept), None, "{kept}");
        }
    }

    #[test]
    fn a_path_only_the_verbatim_form_can_name_keeps_it() {
        let directory = "d".repeat(100);
        let short = format!("\\\\?\\C:\\{directory}\\{directory}\\{}", "f".repeat(54));
        assert_eq!(plain_form(&short).map(|plain| plain.len()), Some(259));
        let long = format!("{short}f");
        assert_eq!(plain_form(&long), None, "a plain form of 260 characters");

        for device in RESERVED_DEVICES.iter().chain(&LAST_PRINTER_DEVICES) {
            for component in [(*device).to_owned(), device.to_lowercase(), format!("{device}.txt")] {
                let reserved = format!("\\\\?\\C:\\w\\{component}\\x");
                assert_eq!(plain_form(&reserved), None, "{reserved}");
            }
        }
        for component in ["trailing.", "trailing ", "CON .txt"] {
            assert_eq!(plain_form(&format!("\\\\?\\C:\\w\\{component}")), None, "{component:?}");
        }
        for ordinary in ["CONSOLE", "COM10", "nul-terminated", "LPT", "a.CON"] {
            let path = format!("\\\\?\\C:\\w\\{ordinary}");
            assert_eq!(plain_form(&path).as_deref(), path.strip_prefix(VERBATIM), "{ordinary}");
        }
    }

    #[test]
    fn canonical_resolves_to_a_path_no_verbatim_prefix_opens() {
        let here = canonical(env!("CARGO_MANIFEST_DIR")).unwrap();
        assert!(here.is_absolute());
        assert!(!here.to_string_lossy().starts_with(VERBATIM), "{}", here.display());
        assert_eq!(canonical(&here).unwrap(), here);
        assert!(canonical(here.join("no-such-entry")).is_err());
        if cfg!(not(windows)) {
            assert_eq!(here, fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap());
        }
    }

    #[test]
    fn containment_compares_both_paths_without_their_verbatim_prefix() {
        let inside = [
            ("C:\\w\\crates\\a", "\\\\?\\C:\\w"),
            ("\\\\?\\C:\\w\\crates\\a", "C:\\w"),
            ("c:\\w\\crates\\a", "\\\\?\\C:\\w"),
            ("C:/w/crates/a", "\\\\?\\C:\\w"),
            ("C:\\w", "\\\\?\\C:\\w\\"),
            ("\\\\server\\share\\w\\a", "\\\\?\\UNC\\server\\share\\w"),
            ("\\\\?\\UNC\\server\\share\\w\\a", "\\\\server\\share\\w"),
            ("/w/crates/a", "/w"),
            ("/w/./crates/a", "/w/"),
            ("crates/a", "crates"),
        ];
        for (child, root) in inside {
            assert!(within(Path::new(child), Path::new(root)), "{child} in {root}");
        }
        let outside = [
            ("C:\\elsewhere\\a", "\\\\?\\C:\\w"),
            ("D:\\w\\crates\\a", "\\\\?\\C:\\w"),
            ("C:\\w-sibling\\a", "\\\\?\\C:\\w"),
            ("C:\\W\\crates\\a", "\\\\?\\C:\\w"),
            ("\\\\server\\other\\w\\a", "\\\\?\\UNC\\server\\share\\w"),
            ("C:\\w\\a", "\\\\server\\share\\w"),
            ("/elsewhere/a", "/w"),
            ("/w-sibling/a", "/w"),
            ("/w", "/w/crates"),
            ("w/crates/a", "/w"),
        ];
        for (child, root) in outside {
            assert!(!within(Path::new(child), Path::new(root)), "{child} in {root}");
            assert_eq!(relative_to(Path::new(child), Path::new(root)), None);
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn a_backslash_in_a_unix_name_is_a_character_and_never_a_separator() {
        assert!(!within(Path::new("/w/a\\b/c"), Path::new("/w/a")));
        assert!(within(Path::new("/w/a\\b/c"), Path::new("/w/a\\b")));
        assert_eq!(
            relative_to(Path::new("/w/a\\b/c"), Path::new("/w")),
            Some(PathBuf::from("a\\b/c"))
        );
    }

    #[test]
    fn the_relative_path_is_what_follows_the_root_in_either_form() {
        let relative = |child: &str, root: &str| relative_to(Path::new(child), Path::new(root));
        let expected: PathBuf = ["crates", "a", "Cargo.toml"].iter().collect();
        assert_eq!(relative("C:\\w\\crates\\a\\Cargo.toml", "\\\\?\\C:\\w"), Some(expected.clone()));
        assert_eq!(relative("\\\\?\\C:\\w\\crates\\a\\Cargo.toml", "c:\\w"), Some(expected.clone()));
        assert_eq!(relative("/w/crates/a/Cargo.toml", "/w"), Some(expected));
        assert_eq!(relative("/w", "/w"), Some(PathBuf::new()));
    }
}
