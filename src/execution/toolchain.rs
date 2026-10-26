//! The toolchain a child process is pinned to by name, so the scanned tree
//! cannot pick it by path.
//!
//! A `rust-toolchain.toml` whose `path` is absolute and resolves into the tree
//! (`/proc/self/cwd/tc` does on Linux, wherever the tree sits) makes the
//! rustup proxy run the tree's own `cargo` and `rustc`. `RUSTUP_TOOLCHAIN`
//! outranks that file, and only a name, never a path, is ever set: rustup
//! answers `show active-toolchain` with the path it would run, and [`accepted`]
//! refuses it. Two callers share the resolution: the baseline comparison pins
//! both of its sides with it, and a scan with no Clippy rule pins
//! `cargo metadata` with it. `docs/subsystems/execution.md` records the checks
//! (`tests/toolchain_pin_assumptions.rs`) the pin rests on.

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::bounded_read::collect_bounded;
use crate::internal_error::InternalError;

/// What `rustup show active-toolchain` may print before the answer is refused.
///
/// It prints one short line. A call that floods the pipe is a call whose answer
/// `accepted` would turn down anyway, and reading it under a bound is what
/// keeps a hostile `rust-toolchain.toml` from choosing this process's memory.
const RUSTUP_OUTPUT_LIMIT: usize = 4_096;

const BY_PATH: &str = "the workspace selects its toolchain by path; set RUSTUP_TOOLCHAIN to a toolchain name to scan it without Clippy";

const UNNAMED: &str = "no toolchain name resolves for the workspace; set RUSTUP_TOOLCHAIN to a toolchain name to scan it without Clippy";

/// The outcome of resolving a name-only pin in one directory.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NameOnlyPin {
    /// The name every child is started under.
    Named(OsString),
    /// No rustup on `PATH`: no proxy reads a toolchain file either, so a
    /// child runs unpinned, as the installed `cargo` is.
    NoRustup,
    /// Rustup answered with no toolchain name, a path or nothing usable.
    Refused(&'static str),
}

/// What `cargo metadata` runs under in a scan that compiles nothing: a name,
/// no pin where there is no rustup, or the refusal to run it at all, which
/// the `metadata` stage reports.
pub(crate) fn metadata_pin(directory: &Path) -> Result<Option<OsString>, InternalError> {
    match name_only_pin(directory) {
        NameOnlyPin::Named(name) => Ok(Some(name)),
        NameOnlyPin::NoRustup => Ok(None),
        NameOnlyPin::Refused(message) => {
            Err(InternalError::new("metadata", "toolchain-not-named", message))
        }
    }
}

/// The toolchain name a child started in `directory` is pinned to.
///
/// An inherited `RUSTUP_TOOLCHAIN` is authoritative and never falls back to
/// rustup: it is what the caller already runs under, and silently scanning
/// under a different one is the drift a pin exists to stop. It is taken whole:
/// a value carrying a space is one rustup would not accept either, and
/// trimming it to its first field would run a toolchain nobody named.
pub(crate) fn name_only_pin(directory: &Path) -> NameOnlyPin {
    if let Some(inherited) = env::var_os("RUSTUP_TOOLCHAIN") {
        return inherited
            .to_str()
            .and_then(accepted)
            .map_or(NameOnlyPin::Refused(UNNAMED), NameOnlyPin::Named);
    }
    match active_toolchain(directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => NameOnlyPin::NoRustup,
        Err(_) | Ok(None) => NameOnlyPin::Refused(UNNAMED),
        // The answer is `<name> (<why>)`, so only its first field is a
        // toolchain.
        Ok(Some(answer)) => answer
            .split_whitespace()
            .next()
            .and_then(accepted)
            .map_or(NameOnlyPin::Refused(BY_PATH), NameOnlyPin::Named),
    }
}

/// Asks rustup which toolchain `directory` resolves to: `Err` when rustup
/// could not be started, `None` when it answered nothing usable.
fn active_toolchain(directory: &Path) -> io::Result<Option<String>> {
    let mut child = Command::new("rustup")
        .args(["show", "active-toolchain"])
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let Some(stdout) = child.stdout.take() else {
        return Ok(None);
    };
    // Drained before the wait: a rustup that filled the pipe would never exit,
    // and this process would wait on it forever.
    let output = collect_bounded(stdout, RUSTUP_OUTPUT_LIMIT)?;
    if output.exceeded || !child.wait()?.success() {
        return Ok(None);
    }
    Ok(String::from_utf8(output.bytes).ok())
}

/// A toolchain name this crate will pass on to a child process.
///
/// Only ASCII alphanumerics and `-`, `_`, `.` are accepted, so every value that
/// gets through is valid UTF-8 by construction. That is why both sources are
/// read as text: preserving bytes no accepted value can contain used to buy a
/// `#[cfg(unix)]` split over `OsStr` and nothing else.
fn accepted(toolchain: &str) -> Option<OsString> {
    (!toolchain.is_empty()
        && toolchain.len() <= 255
        && toolchain
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    .then(|| OsString::from(toolchain))
}

#[cfg(test)]
mod tests {
    use super::accepted;

    #[test]
    fn only_a_bare_toolchain_name_is_passed_on_to_a_child() {
        assert_eq!(
            accepted("1.97.1-x86_64-unknown-linux-gnu"),
            Some("1.97.1-x86_64-unknown-linux-gnu".into())
        );
        assert_eq!(accepted("stable"), Some("stable".into()));
        assert_eq!(accepted(&"x".repeat(255)), Some("x".repeat(255).into()));

        // A whole value is judged whole: nothing that carries a separator, a
        // path, a shell character or a second field survives.
        for refused in [
            "",
            "stable extra",
            "sta ble; rm -rf /",
            "../../etc",
            "a/b",
            // What `rustup show active-toolchain` answers for a
            // `rust-toolchain.toml` path toolchain (US-001).
            "/proc/self/cwd/tc",
            "$(id)",
            "nightly\n",
        ] {
            assert_eq!(accepted(refused), None, "{refused:?} was accepted");
        }
        assert_eq!(accepted(&"x".repeat(256)), None);
    }
}
