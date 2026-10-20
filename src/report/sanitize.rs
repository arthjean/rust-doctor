//! Every published path and home directory taken out of the text a scan
//! produced, and every escape sequence and detected credential with them.
//!
//! `--json` reports stay workspace-relative and carry no user data: this is
//! where that holds. The escape grammar itself lives in
//! [`crate::terminal_text`], the one the interactive report draws through, so
//! a message and the frame that shows it cannot disagree on where a sequence
//! ends.

use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(super) struct HomePaths {
    pub(super) lexical: Option<String>,
    pub(super) canonical: Option<String>,
    /// `RUSTUP_HOME` and `CARGO_HOME` as resolved, in both forms, each with
    /// the name that replaces it. They usually sit under the home directory,
    /// and naming them says more than `<home>/.rustup` would.
    pub(super) toolchain: Vec<(&'static str, String)>,
}

impl HomePaths {
    pub(super) fn from_path(path: Option<PathBuf>) -> Self {
        let canonical = path
            .as_ref()
            .and_then(|path| path.canonicalize().ok())
            .map(|path| path.to_string_lossy().into_owned());
        Self {
            lexical: path.map(|path| path.to_string_lossy().into_owned()),
            canonical,
            toolchain: Vec::new(),
        }
    }
}

/// Every credential the repository pass matches is masked here too, once,
/// so a message that quotes a key reaches no renderer with it.
///
/// Cargo and rustup print the paths they read: a toolchain under
/// `RUSTUP_HOME`, a registry under `CARGO_HOME`, a manifest a workspace
/// points outside itself at. Each is named by what it is under, and an
/// absolute path under none of them keeps only its last directory and its
/// file name, so a report posted to a pull request discloses no machine's
/// layout.
pub(super) fn sanitize_text(value: &str, workspace_root: Option<&Path>, home: &HomePaths) -> String {
    let mut value = sanitize_source_text(value, workspace_root, home, true);
    value = external_paths(&value);
    value
}

/// What a suggested replacement goes through: it is source text, and an
/// absolute path literal in it is the code's own, which a rewrite would turn
/// into a fix that writes something else.
pub(super) fn sanitize_replacement(
    value: &str,
    workspace_root: Option<&Path>,
    home: &HomePaths,
) -> String {
    sanitize_source_text(value, workspace_root, home, false)
}

fn sanitize_source_text(
    value: &str,
    workspace_root: Option<&Path>,
    home: &HomePaths,
    toolchain: bool,
) -> String {
    let mut value = crate::repo_hygiene::mask_credentials(&normalize_text(value));
    if let Some(workspace_root) = workspace_root.and_then(Path::to_str)
        && !workspace_root.is_empty()
    {
        value = value.replace(workspace_root, ".");
    }
    if toolchain {
        let mut forms: Vec<&(&str, String)> =
            home.toolchain.iter().filter(|(_, path)| !path.is_empty()).collect();
        forms.sort_by_key(|(_, path)| std::cmp::Reverse(path.len()));
        for (name, path) in forms {
            value = value.replace(path.as_str(), name);
        }
    }
    let mut home_forms: Vec<_> = [home.lexical.as_deref(), home.canonical.as_deref()]
        .into_iter()
        .flatten()
        .filter(|path| !path.is_empty())
        .collect();
    home_forms.sort_by_key(|path| std::cmp::Reverse(path.len()));
    home_forms.dedup();
    for home in home_forms {
        value = value.replace(home, "<home>");
    }
    value
}

/// Carriage returns become newlines before the sanitizer runs, because it
/// drops a lone `\r` as the control character it is and a Cargo message written
/// on Windows would lose its line breaks with it.
pub(super) fn normalize_text(value: &str) -> String {
    let line_endings = value.replace("\r\n", "\n").replace('\r', "\n");
    let without_ansi = crate::terminal_text::sanitize_multiline(&line_endings);
    without_ansi
        .split('\n')
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn home_paths() -> HomePaths {
    let home = env::var_os("HOME").map(PathBuf::from);
    let mut paths = HomePaths::from_path(home.clone());
    for (name, variable, default) in [
        ("$RUSTUP_HOME", "RUSTUP_HOME", ".rustup"),
        ("$CARGO_HOME", "CARGO_HOME", ".cargo"),
    ] {
        let resolved = env::var_os(variable)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| home.as_ref().map(|home| home.join(default)));
        for form in resolved
            .iter()
            .flat_map(|path| [Some(path.clone()), path.canonicalize().ok()])
            .flatten()
        {
            paths.toolchain.push((name, form.to_string_lossy().into_owned()));
        }
    }
    paths
}

/// Whether an absolute path may start right after `previous`: never after a
/// colon, which is a URL's, nor after a dot, which is a relative path's.
const fn opens_a_path(previous: Option<char>) -> bool {
    match previous {
        None => true,
        Some(character) => {
            character.is_whitespace() || matches!(character, '`' | '\'' | '"' | '(' | '[' | '=' | ',')
        }
    }
}

const fn ends_a_path(character: char) -> bool {
    character.is_whitespace()
        || matches!(character, '`' | '\'' | '"' | ')' | ']' | '>' | ',' | ';' | ':')
}

/// The length of the absolute path `text` opens with, when it opens with one
/// of at least two components: `/usr/lib`, `C:\Users\x`, or `~/src/x`, which
/// is how Cargo's own stderr reaches here once its home is replaced. A lone
/// `/`, `/=` or `/ 2` is arithmetic in a message, not a path.
fn absolute_path_at(text: &str) -> Option<usize> {
    let mut characters = text.char_indices();
    let start = match characters.next()? {
        (_, '/') => 1,
        (_, '~') if text.get(1..).is_some_and(|rest| rest.starts_with('/')) => 2,
        (_, drive) if drive.is_ascii_alphabetic() => {
            let rest = text.get(1..)?;
            if !(rest.starts_with(":\\") || rest.starts_with(":/")) {
                return None;
            }
            3
        }
        _ => return None,
    };
    let body = text.get(start..)?;
    let first = body.chars().next()?;
    if !(first.is_alphanumeric() || matches!(first, '.' | '_' | '-' | '~' | '@' | '+')) {
        return None;
    }
    let length = body.find(ends_a_path).unwrap_or(body.len());
    let token = body.get(..length)?;
    token
        .trim_end_matches(['/', '\\'])
        .contains(['/', '\\'])
        .then_some(start + length)
}

/// Every absolute path still in `value` lies outside the workspace and the
/// homes, and becomes `<external>/` followed by its last directory and file
/// name: enough to tell which manifest Cargo could not read, and nothing about
/// where it sits.
fn external_paths(value: &str) -> String {
    let mut published = String::with_capacity(value.len());
    let mut previous = None;
    let mut rest = value;
    while let Some(character) = rest.chars().next() {
        if opens_a_path(previous)
            && let Some(length) = absolute_path_at(rest)
            && let Some(path) = rest.get(..length)
        {
            let components: Vec<&str> = path
                .split(['/', '\\'])
                .filter(|component| !component.is_empty())
                .collect();
            published.push_str("<external>");
            for component in components.iter().skip(components.len().saturating_sub(2)) {
                published.push('/');
                published.push_str(component);
            }
            previous = path.chars().last();
            rest = rest.get(length..).unwrap_or_default();
            continue;
        }
        published.push(character);
        previous = Some(character);
        rest = rest.get(character.len_utf8()..).unwrap_or_default();
    }
    published
}
