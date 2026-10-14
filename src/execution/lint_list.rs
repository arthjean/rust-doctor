//! Which lints the installed Clippy knows, asked before any `-W` is passed.
//!
//! The catalog is written against the pinned toolchain, and a user's may be
//! older or newer. A `-W` for a lint the installed Clippy does not know used to
//! be passed anyway, and `unknown_lints` either warned about it, a finding
//! nobody catalogued, or silently did nothing: the rule looked evaluated and
//! clean. The table `clippy-driver -W help` prints is read first, under the
//! same parser the coverage tests use, and a catalogued rule it does not list
//! is left out of the command and published as not evaluated.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::{CommandEnvironment, RunDeadline, process};
use crate::bounded_read::collect_bounded;
use crate::internal_error::InternalError;
use crate::policy::lint_table;

/// How long the table may take. It prints in a few milliseconds.
pub(super) const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The table runs to about 180 KB on 1.97.1.
const TABLE_LIMIT: usize = 4 * 1024 * 1024;

/// The lint names the installed Clippy lists, or the notice that says it
/// could not be asked. The notice never voids the score: the scan then passes
/// every `-W` with `unknown_lints` allowed.
pub(super) fn probe(
    clippy_driver: &Path,
    workspace_root: &Path,
    environment: &CommandEnvironment,
    run: Option<RunDeadline>,
) -> Result<BTreeSet<String>, InternalError> {
    let mut command = Command::new(clippy_driver);
    command.args(["-W", "help"]).current_dir(workspace_root);
    environment.apply(&mut command);
    // The run's own deadline wins when it is the nearer one: the probe never
    // carries the scan past `--max-duration`.
    let (deadline, _) = RunDeadline::capped(PROBE_TIMEOUT, run);
    let finished = process::run(
        command,
        Some(deadline),
        |stdout| collect_bounded(stdout, TABLE_LIMIT),
    );
    let table = match finished {
        Ok(finished)
            if !finished.expired
                && finished
                    .status
                    .as_ref()
                    .is_ok_and(std::process::ExitStatus::success) =>
        {
            finished
                .output
                .and_then(Result::ok)
                .filter(|output| !output.exceeded)
        }
        _ => None,
    };
    let names = table
        .map(|output| lint_table::lint_names(&String::from_utf8_lossy(&output.bytes)))
        .filter(|names| !names.is_empty());
    names.ok_or_else(unavailable)
}

pub(super) fn unavailable() -> InternalError {
    InternalError::new(
        "toolchain",
        "lint-list-unavailable",
        "The installed Clippy did not list its lints, so every catalogued rule was \
         passed with unknown lints allowed.",
    )
}
