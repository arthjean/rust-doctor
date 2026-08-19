//! Every published path and home directory taken out of the text a scan
//! produced, and every escape sequence with them.
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
        }
    }
}

pub(super) fn sanitize_text(value: &str, workspace_root: Option<&Path>, home: &HomePaths) -> String {
    let mut value = normalize_text(value);
    if let Some(workspace_root) = workspace_root.and_then(Path::to_str)
        && !workspace_root.is_empty()
    {
        value = value.replace(workspace_root, ".");
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
    HomePaths::from_path(env::var_os("HOME").map(PathBuf::from))
}
