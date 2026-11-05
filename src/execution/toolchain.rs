use std::env;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::bounded_read::collect_bounded;
use crate::internal_error::InternalError;

const RUSTUP_OUTPUT_LIMIT: usize = 4_096;

const BY_PATH: &str = "the workspace selects its toolchain by path; set RUSTUP_TOOLCHAIN to a toolchain name to scan it without Clippy";

const UNNAMED: &str = "no toolchain name resolves for the workspace; set RUSTUP_TOOLCHAIN to a toolchain name to scan it without Clippy";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NameOnlyPin {
    Named(OsString),
    NoRustup,
    Refused(&'static str),
}

pub(crate) fn metadata_pin(directory: &Path) -> Result<Option<OsString>, InternalError> {
    match name_only_pin(directory) {
        NameOnlyPin::Named(name) => Ok(Some(name)),
        NameOnlyPin::NoRustup => Ok(None),
        NameOnlyPin::Refused(message) => {
            Err(InternalError::new("metadata", "toolchain-not-named", message))
        }
    }
}

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
        Ok(Some(answer)) => answer
            .split_whitespace()
            .next()
            .and_then(accepted)
            .map_or(NameOnlyPin::Refused(BY_PATH), NameOnlyPin::Named),
    }
}

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
    let output = collect_bounded(stdout, RUSTUP_OUTPUT_LIMIT)?;
    if output.exceeded || !child.wait()?.success() {
        return Ok(None);
    }
    Ok(String::from_utf8(output.bytes).ok())
}

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

        for refused in [
            "",
            "stable extra",
            "sta ble; rm -rf /",
            "../../etc",
            "a/b",
            "/proc/self/cwd/tc",
            "$(id)",
            "nightly\n",
        ] {
            assert_eq!(accepted(refused), None, "{refused:?} was accepted");
        }
        assert_eq!(accepted(&"x".repeat(256)), None);
    }
}
