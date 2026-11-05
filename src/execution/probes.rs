use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

use super::{CommandEnvironment, RunDeadline, lint_list, process};
use crate::bounded_read::collect_bounded;
use crate::internal_error::InternalError;
use crate::terminal_text::{sanitize, truncate};

#[derive(Debug, Clone, Copy)]
pub(super) struct Probe {
    code: &'static str,
    pub(super) label: &'static str,
    remedy: &'static str,
}

pub(super) const CARGO_PROBE: Probe = Probe {
    code: "cargo-unavailable",
    label: "Cargo",
    remedy: "Install a Rust toolchain from https://rustup.rs and put `cargo` on PATH.",
};

pub(super) const RUSTC_PROBE: Probe = Probe {
    code: "rustc-unavailable",
    label: "rustc",
    remedy: "Install a Rust toolchain from https://rustup.rs and put `rustc` on PATH.",
};

pub(super) const CLIPPY_PROBE: Probe = Probe {
    code: "clippy-unavailable",
    label: "Clippy",
    remedy: "Clippy is required: install the component with `rustup component add clippy`, \
        or add `components: clippy` to the toolchain step in CI.",
};

impl Probe {
    pub(super) fn failure(self, message: String) -> InternalError {
        InternalError::new("execution", self.code, format!("{message}. {}", self.remedy))
    }
}

const PROBE_OUTPUT_LIMIT: usize = 4_096;

const PROBE_DETAIL_COLUMNS: usize = 200;

pub(super) fn tool_version(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    probe: Probe,
    environment: &CommandEnvironment,
    deadline: Option<RunDeadline>,
) -> Result<String, InternalError> {
    let (bound, run_bound) = RunDeadline::capped(lint_list::PROBE_TIMEOUT, deadline);
    let output = run_probe(program, arguments, working_directory, environment, bound)
        .map_err(|error| {
            probe.failure(format!("{} could not be started: {error}", probe.label))
        })?;
    if output.expired && run_bound {
        return Err(InternalError::new(
            "toolchain",
            crate::report::DEADLINE_EXCEEDED,
            bound.message(),
        ));
    }
    if output.expired {
        return Err(InternalError::new(
            "toolchain",
            "probe-timeout",
            format!(
                "{} did not report a version within {} s. {}",
                probe.label,
                lint_list::PROBE_TIMEOUT.as_secs(),
                probe.remedy
            ),
        ));
    }

    if !output.status.success() {
        return Err(probe.failure(format!(
            "{} could not report a version ({}){}",
            probe.label,
            output.status,
            probe_detail(&output.stderr)
        )));
    }

    let version = String::from_utf8(output.stdout).map_err(|error| {
        probe.failure(format!(
            "{} returned non-UTF-8 version output: {error}",
            probe.label
        ))
    })?;
    let version = version.trim();
    if version.is_empty() || output.truncated {
        return Err(probe.failure(format!("{} returned no usable version", probe.label)));
    }

    Ok(version.to_owned())
}

struct ProbeOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
    expired: bool,
}

fn run_probe(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    environment: &CommandEnvironment,
    deadline: RunDeadline,
) -> io::Result<ProbeOutput> {
    let finished = process::run(
        version_command(program, arguments, working_directory, environment),
        Some(deadline),
        |stream| collect_bounded(stream, PROBE_OUTPUT_LIMIT),
    )?;
    let status = finished.status?;
    let (stdout, truncated) = finished
        .output
        .transpose()?
        .map_or_else(|| (Vec::new(), false), |output| (output.bytes, output.exceeded));

    Ok(ProbeOutput {
        status,
        stdout,
        stderr: finished.stderr,
        truncated,
        expired: finished.expired,
    })
}

fn probe_detail(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| format!(": {}", truncate(&sanitize(line), PROBE_DETAIL_COLUMNS)))
        .unwrap_or_default()
}

pub(super) fn version_command(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    environment: &CommandEnvironment,
) -> Command {
    let mut command = Command::new(program);
    command.args(arguments).current_dir(working_directory);
    environment.apply(&mut command);
    command
}
