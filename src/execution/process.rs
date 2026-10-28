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
//! group of its own and the group is killed with `kill(2)`; the same group is
//! registered with `interrupt` while it runs, so a handled Ctrl-C kills it
//! too. On Windows `taskkill /T` kills the tree it can see, and
//! `docs/subsystems/execution.md` records the limit.

use std::io;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::interrupt;
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

    /// The nearer of a child's own `timeout` and the run's deadline, and
    /// whether the run's is the one that applies.
    pub(crate) fn capped(timeout: Duration, run: Option<Self>) -> (Self, bool) {
        match run {
            Some(run) if run.remaining() < timeout => (run, true),
            _ => (Self::starting_now(timeout), false),
        }
    }
}

/// A child started under the run's bounds: it leads a process group of its
/// own when a deadline is set or the interrupt handlers are installed, a
/// watchdog kills that group as the deadline passes, and the group is
/// registered so a handled signal kills it.
///
/// A child with neither stays in the caller's group. The group is what the
/// watchdog and the interrupt watcher kill, and it is also what a terminal's
/// Ctrl-C no longer reaches, so a library caller without the handlers keeps
/// Cargo in its own group, where that Ctrl-C still takes it down.
pub(crate) struct BoundedChild {
    pub(crate) child: Child,
    watchdog: Option<Watchdog>,
    registered: Option<u32>,
    waited: bool,
}

pub(crate) fn spawn(mut command: Command, deadline: Option<RunDeadline>) -> io::Result<BoundedChild> {
    if interrupt::requested() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, interrupt::MESSAGE));
    }
    let grouped = deadline.is_some() || interrupt::installed();
    #[cfg(unix)]
    if grouped {
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
    }
    let child = command.spawn()?;
    let registered = grouped.then(|| child.id());
    if let Some(group) = registered {
        interrupt::register(group);
    }
    let watchdog = deadline.map(|deadline| Watchdog::arm(child.id(), deadline.remaining()));
    Ok(BoundedChild {
        child,
        watchdog,
        registered,
        waited: false,
    })
}

impl BoundedChild {
    /// Waits for the child, and says whether the watchdog killed its group
    /// before it exited.
    pub(crate) fn wait(mut self) -> (io::Result<ExitStatus>, bool) {
        let status = self.child.wait();
        self.waited = true;
        let expired = self.watchdog.take().is_some_and(Watchdog::disarm);
        (status, expired)
    }
}

impl Drop for BoundedChild {
    /// A child nobody waited on, left by an error path or by a reader that
    /// panicked, is killed with its group and reaped here, so no `cargo
    /// clippy` outlives the pass that started it. The group leaves the
    /// register only once its leader is reaped, and the watchdog is disarmed.
    fn drop(&mut self) {
        if !self.waited {
            match self.registered {
                Some(group) => kill_tree(group),
                None => {
                    let _ = self.child.kill();
                }
            }
            let _ = self.child.wait();
        }
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.disarm();
        }
        if let Some(group) = self.registered.take() {
            interrupt::unregister(group);
        }
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
    let mut bounded = spawn(command, deadline)?;
    let errors = bounded
        .child
        .stderr
        .take()
        .map(|stream| thread::spawn(move || collect_bounded(stream, STDERR_LIMIT)));
    let output = bounded.child.stdout.take().map(read);
    let (status, expired) = bounded.wait();
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
/// wait returns, and its group leaves the interrupt register only then, so the
/// group id cannot have been reused by the time either killer reaches it.
#[cfg(unix)]
pub(super) fn kill_tree(pid: u32) {
    let Ok(group) = libc::pid_t::try_from(pid) else {
        return;
    };
    // SAFETY: `kill` takes plain integers and touches no memory of ours. A
    // negative pid names the process group, never a single process.
    unsafe {
        libc::kill(-group, libc::SIGKILL);
    }
}

/// Whether `pid` names a running process. One this user may not signal is
/// running all the same, and a pid that does not fit is never judged gone.
#[cfg(unix)]
pub(crate) fn is_running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    // SAFETY: signal 0 checks that the process exists and delivers nothing.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// The user this process acts as.
#[cfg(unix)]
pub(crate) fn effective_user() -> u32 {
    // SAFETY: `geteuid` reads the effective user id and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(windows)]
pub(super) fn kill_tree(pid: u32) {
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

    /// Self-scan audit US-004: a child dropped before `wait`, grouped by a
    /// deadline or not, is gone once the drop returns.
    #[test]
    fn a_child_dropped_before_its_wait_is_killed_and_reaped() {
        for deadline in [Some(RunDeadline::starting_now(Duration::from_secs(60))), None] {
            let mut command = Command::new("sleep");
            command.arg("60");
            let bounded = spawn(command, deadline).unwrap();
            let pid = bounded.child.id();
            assert!(is_running(pid));
            drop(bounded);
            assert!(!is_running(pid), "pid {pid} survived its drop ({deadline:?})");
        }
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
