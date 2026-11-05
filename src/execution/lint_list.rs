use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::{CommandEnvironment, RunDeadline, process};
use crate::bounded_read::collect_bounded;
use crate::internal_error::InternalError;
use crate::policy::lint_table;

pub(super) const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

const TABLE_LIMIT: usize = 4 * 1024 * 1024;

pub(super) fn probe(
    clippy_driver: &Path,
    workspace_root: &Path,
    environment: &CommandEnvironment,
    run: Option<RunDeadline>,
) -> Result<BTreeSet<String>, InternalError> {
    let mut command = Command::new(clippy_driver);
    command.args(["-W", "help"]).current_dir(workspace_root);
    environment.apply(&mut command);
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
