//! The caller's rustflags, minus what would change the scan's lint levels.
//!
//! A CI job that exports `RUSTFLAGS=-Dwarnings` used to hand it to the Clippy
//! child unchanged, and every catalogued warning the scan asked for with `-W`
//! became a build failure: the report went incomplete over findings it exists
//! to publish. The workspace's own `[build] rustflags = ["-D", "warnings"]`
//! does the same, measured on 1.97.1 on 2026-09-25: the build fails at the
//! first catalogued warning.
//!
//! Lint-level tokens are therefore removed, and only they: every other flag is
//! passed on through `CARGO_ENCODED_RUSTFLAGS`, which Cargo reads before
//! `RUSTFLAGS` and before any configuration. When there is nothing to remove,
//! the environment is left exactly as the caller set it, because a changed
//! rustflags value is a changed fingerprint and would recompile every crate
//! the user's own builds had already compiled.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

/// What the Clippy child is started with instead of the caller's rustflags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RustflagsOverride {
    /// The value of `CARGO_ENCODED_RUSTFLAGS`, when something was removed.
    encoded: Option<String>,
    /// Each removed flag, as `-D warnings`, `--cap-lints warn`.
    pub(crate) removed: Vec<String>,
}

impl RustflagsOverride {
    /// Reads the caller's environment, then the workspace's configuration
    /// when the environment sets neither variable, which is Cargo's own
    /// precedence.
    pub(crate) fn resolve(workspace_root: &Path) -> Self {
        let encoded = std::env::var("CARGO_ENCODED_RUSTFLAGS").ok();
        let spaced = std::env::var("RUSTFLAGS").ok();
        let flags = match (encoded, spaced) {
            (Some(encoded), _) => split_encoded(&encoded),
            (None, Some(spaced)) => spaced.split_whitespace().map(str::to_owned).collect(),
            (None, None) => configured(workspace_root),
        };
        Self::from_flags(&flags)
    }

    fn from_flags(flags: &[String]) -> Self {
        let (kept, removed) = strip(flags);
        if removed.is_empty() {
            return Self::default();
        }
        Self {
            encoded: Some(kept.join("\u{1f}")),
            removed,
        }
    }

    pub(crate) fn apply(&self, command: &mut Command) {
        if let Some(encoded) = &self.encoded {
            command.env("CARGO_ENCODED_RUSTFLAGS", OsString::from(encoded));
            command.env_remove("RUSTFLAGS");
        }
    }
}

fn split_encoded(encoded: &str) -> Vec<String> {
    if encoded.is_empty() {
        return Vec::new();
    }
    encoded.split('\u{1f}').map(str::to_owned).collect()
}

/// Splits `flags` into what is passed on and what is removed.
///
/// The five lint-level spellings rustc accepts are matched in their separated
/// and joined forms: `-D x`, `-Dx`, `--deny x`, `--deny=x`, and the same for
/// `-F`/`--forbid` and `--cap-lints`.
fn strip(flags: &[String]) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut removed = Vec::new();
    let mut index = 0;
    while let Some(flag) = flags.get(index) {
        index += 1;
        let Some((canonical, joined)) = lint_level(flag) else {
            kept.push(flag.clone());
            continue;
        };
        let value = match joined {
            Some(value) => value.to_owned(),
            None => {
                let value = flags.get(index).cloned().unwrap_or_default();
                index += 1;
                value
            }
        };
        removed.push(format!("{canonical} {value}").trim_end().to_owned());
    }
    (kept, removed)
}

/// The canonical spelling of a lint-level flag, and its value when it is
/// joined to it.
fn lint_level(flag: &str) -> Option<(&'static str, Option<&str>)> {
    for (canonical, short, long) in [
        ("-D", Some("-D"), "--deny"),
        ("-F", Some("-F"), "--forbid"),
        ("--cap-lints", None, "--cap-lints"),
    ] {
        if flag == long || short == Some(flag) {
            return Some((canonical, None));
        }
        if let Some(value) = flag
            .strip_prefix(long)
            .and_then(|rest| rest.strip_prefix('='))
        {
            return Some((canonical, Some(value)));
        }
        if let Some(value) = short.and_then(|short| flag.strip_prefix(short)) {
            return Some((canonical, Some(value)));
        }
    }
    None
}

#[derive(Debug, Default, Deserialize)]
struct ConfigDocument {
    build: Option<ConfigTable>,
    target: Option<BTreeMap<String, ConfigTable>>,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigTable {
    rustflags: Option<toml::Value>,
}

/// `[build] rustflags` of the workspace's `.cargo/config.toml`, when nothing
/// overrides it. A `[target.*] rustflags` takes precedence over it in Cargo,
/// so the build table is not what Cargo would pass and is left alone; an
/// unreadable or invalid file is the dependency pass's to report.
fn configured(workspace_root: &Path) -> Vec<String> {
    let path = workspace_root.join(".cargo").join("config.toml");
    let Some(document) = fs::read_to_string(path)
        .ok()
        .and_then(|contents| toml::from_str::<ConfigDocument>(&contents).ok())
    else {
        return Vec::new();
    };
    let targeted = document
        .target
        .iter()
        .flat_map(BTreeMap::values)
        .any(|table| table.rustflags.is_some());
    if targeted {
        return Vec::new();
    }
    match document.build.and_then(|table| table.rustflags) {
        Some(toml::Value::Array(entries)) => entries
            .iter()
            .filter_map(toml::Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(toml::Value::String(joined)) => joined.split_whitespace().map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn every_lint_level_spelling_is_removed_with_its_value() {
        let (kept, removed) = strip(&flags(&[
            "-Dwarnings",
            "-C",
            "target-cpu=native",
            "-D",
            "unused",
            "--deny=missing_docs",
            "--forbid",
            "unsafe_code",
            "-Fx",
            "--cap-lints",
            "warn",
            "--cap-lints=allow",
            "-W",
            "clippy::pedantic",
        ]));
        assert_eq!(
            kept,
            flags(&["-C", "target-cpu=native", "-W", "clippy::pedantic"])
        );
        assert_eq!(
            removed,
            flags(&[
                "-D warnings",
                "-D unused",
                "-D missing_docs",
                "-F unsafe_code",
                "-F x",
                "--cap-lints warn",
                "--cap-lints allow",
            ])
        );
    }

    #[test]
    fn flags_without_a_lint_level_leave_the_environment_untouched() {
        let untouched = RustflagsOverride::from_flags(&flags(&["-C", "target-cpu=native"]));
        assert_eq!(untouched, RustflagsOverride::default());
        let mut command = Command::new("cargo");
        untouched.apply(&mut command);
        assert_eq!(command.get_envs().count(), 0);

        let stripped = RustflagsOverride::from_flags(&flags(&["-Dwarnings", "-Cdebuginfo=0"]));
        let mut command = Command::new("cargo");
        stripped.apply(&mut command);
        let envs: Vec<_> = command.get_envs().collect();
        assert!(envs.contains(&(
            std::ffi::OsStr::new("CARGO_ENCODED_RUSTFLAGS"),
            Some(std::ffi::OsStr::new("-Cdebuginfo=0"))
        )));
        assert!(envs.contains(&(std::ffi::OsStr::new("RUSTFLAGS"), None)));
    }

    #[test]
    fn the_build_table_is_read_unless_a_target_table_overrides_it() {
        let root = crate::test_scratch::scratch("execution", "rustflags-config");
        fs::create_dir_all(root.join(".cargo")).unwrap();
        fs::write(
            root.join(".cargo/config.toml"),
            "[build]\nrustflags = [\"-D\", \"warnings\"]\n",
        )
        .unwrap();
        assert_eq!(configured(&root), flags(&["-D", "warnings"]));
        fs::write(
            root.join(".cargo/config.toml"),
            "[build]\nrustflags = \"-D warnings\"\n[target.x86_64-unknown-linux-gnu]\nrustflags = []\n",
        )
        .unwrap();
        assert!(configured(&root).is_empty());
    }
}
