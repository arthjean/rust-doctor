//! A Cargo child this run can bound in time and hear from.
//!
//! Two things used to be missing from the Clippy child. Its stderr went to
//! `/dev/null`, so a lockfile Cargo could not parse published "Clippy exited
//! with status 101" and nothing else. And nothing bounded it in time: a
//! `build.rs` that never returns, or a Cargo lock rust-analyzer held, hung the
//! scan and whatever job was waiting on it.
//!
//! Both are answered here. Stderr is drained on a thread of its own under
//! [`STDERR_LIMIT`], concurrently with stdout, so neither pipe can fill and
//! block Cargo. And a run with a deadline arms a watchdog that kills the
//! child's whole process group when it passes: Cargo, every rustc it started
//! and every build script those started, since killing Cargo alone leaves a
//! sleeping build script holding the pipes open. On Unix the child leads a
//! group of its own and the group is killed through `kill(1)`, which keeps
//! `libc` out of the normal dependencies. On Windows `taskkill /T` kills the
//! tree it can see, and `docs/subsystems/execution.md` records the limit.

use std::io;
use std::process::{ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::bounded_read::collect_bounded;
use crate::cargo_stderr::STDERR_LIMIT;

/// The wall-clock bound `--max-duration` sets on a whole run.
///
/// It is taken once, when the run starts, and every pass reads what is left of
/// it rather than a budget of its own: a bound on each pass is no bound on
/// their sum.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunDeadline {
    at: Instant,
    limit: Duration,
}

impl RunDeadline {
    pub(crate) fn starting_now(limit: Duration) -> Self {
        Self {
            at: Instant::now() + limit,
            limit,
        }
    }

    pub(crate) fn remaining(self) -> Duration {
        self.at.saturating_duration_since(Instant::now())
    }

    pub(crate) fn passed(self) -> bool {
        self.remaining().is_zero()
    }

    /// The sentence every error the deadline causes carries.
    pub(crate) fn message(self) -> String {
        format!(
            "The scan stopped at the {} s limit set by --max-duration.",
            self.limit.as_secs()
        )
    }
}

/// What a bounded child left behind.
pub(crate) struct Finished<T> {
    /// What the reader made of stdout, absent when the pipe was not there.
    pub(crate) output: Option<T>,
    pub(crate) status: io::Result<ExitStatus>,
    /// At most [`STDERR_LIMIT`] bytes of what the child wrote there.
    pub(crate) stderr: Vec<u8>,
    /// The watchdog killed the child's group before it exited.
    pub(crate) expired: bool,
}

/// Runs `command` to its end, reading stdout with `read` on this thread and
/// stderr on another, and killing its process group if `deadline` passes.
pub(crate) fn run<T>(
    mut command: Command,
    deadline: Option<RunDeadline>,
    read: impl FnOnce(ChildStdout) -> T,
) -> io::Result<Finished<T>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Only a bounded child leads a group of its own. The group is what the
    // watchdog kills, and it is also what a terminal's Ctrl-C no longer
    // reaches: a child with no deadline stays in the caller's group, so an
    // interrupted scan still takes Cargo down with it.
    #[cfg(unix)]
    if deadline.is_some() {
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
    }
    let mut child = command.spawn()?;
    let errors = child
        .stderr
        .take()
        .map(|stream| thread::spawn(move || collect_bounded(stream, STDERR_LIMIT)));
    let watchdog = deadline.map(|deadline| Watchdog::arm(child.id(), deadline.remaining()));
    let output = child.stdout.take().map(read);
    let status = child.wait();
    let expired = watchdog.is_some_and(Watchdog::disarm);
    let stderr = errors
        .and_then(|handle| handle.join().ok())
        .and_then(Result::ok)
        .map_or_else(Vec::new, |output| output.bytes);
    Ok(Finished {
        output,
        status,
        stderr,
        expired,
    })
}

/// A thread that kills a process group once a delay elapses, unless it is
/// disarmed first.
struct Watchdog {
    disarm: mpsc::Sender<()>,
    handle: JoinHandle<bool>,
}

impl Watchdog {
    fn arm(pid: u32, after: Duration) -> Self {
        let (disarm, disarmed) = mpsc::channel();
        let handle = thread::spawn(move || match disarmed.recv_timeout(after) {
            Err(RecvTimeoutError::Timeout) => {
                kill_tree(pid);
                true
            }
            Ok(()) | Err(RecvTimeoutError::Disconnected) => false,
        });
        Self { disarm, handle }
    }

    /// Stops the watchdog and says whether it fired.
    fn disarm(self) -> bool {
        let _ = self.disarm.send(());
        self.handle.join().unwrap_or(false)
    }
}

/// Kills the group `pid` leads. The child is not reaped until the caller's
/// wait returns, so the group id cannot have been reused by then.
#[cfg(unix)]
fn kill_tree(pid: u32) {
    let _ = Command::new("kill")
        .args(["-s", "KILL", "--", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn the_deadline_kills_the_whole_group_and_keeps_what_was_read() {
        // The shell leads the group and leaves a grandchild sleeping behind it,
        // which is what a build script under Cargo is.
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "echo started; echo cause >&2; sleep 60 & echo $!; wait",
        ]);
        let started = Instant::now();
        let finished = run(
            command,
            Some(RunDeadline::starting_now(Duration::from_millis(500))),
            |mut stdout| {
                let mut text = String::new();
                let _ = stdout.read_to_string(&mut text);
                text
            },
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(finished.expired);
        let output = finished.output.unwrap();
        assert!(output.starts_with("started"));
        assert_eq!(String::from_utf8_lossy(&finished.stderr).trim(), "cause");
        let grandchild = output.lines().nth(1).unwrap().trim().to_owned();
        // Killed is not yet reaped: the orphan is init's to collect, so its
        // disappearance is waited on rather than sampled once.
        let gone = (0..50).any(|_| {
            let alive = Command::new("kill")
                .args(["-0", &grandchild])
                .stderr(Stdio::null())
                .status()
                .unwrap();
            if alive.success() {
                thread::sleep(Duration::from_millis(40));
            }
            !alive.success()
        });
        assert!(gone, "the grandchild {grandchild} survived");
    }

    #[test]
    fn a_child_that_finishes_in_time_is_never_killed() {
        let mut command = Command::new("sh");
        command.args(["-c", "echo done"]);
        let finished = run(
            command,
            Some(RunDeadline::starting_now(Duration::from_secs(30))),
            |mut stdout| {
                let _ = stdout.read_to_end(&mut Vec::new());
            },
        )
        .unwrap();
        assert!(!finished.expired);
        assert!(finished.status.unwrap().success());
    }
}
