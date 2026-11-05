use std::io;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::interrupt;
use crate::bounded_read::collect_bounded;
use crate::cargo_stderr::STDERR_LIMIT;

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

    pub(crate) fn message(self) -> String {
        format!(
            "The scan stopped at the {} s limit set by --max-duration.",
            self.limit.as_secs()
        )
    }

    pub(crate) fn capped(timeout: Duration, run: Option<Self>) -> (Self, bool) {
        match run {
            Some(run) if run.remaining() < timeout => (run, true),
            _ => (Self::starting_now(timeout), false),
        }
    }
}

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
    pub(crate) fn wait(mut self) -> (io::Result<ExitStatus>, bool) {
        let status = self.child.wait();
        self.waited = true;
        let expired = self.watchdog.take().is_some_and(Watchdog::disarm);
        (status, expired)
    }
}

impl Drop for BoundedChild {
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

pub(crate) struct Finished<T> {
    pub(crate) output: Option<T>,
    pub(crate) status: io::Result<ExitStatus>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) expired: bool,
}

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

    fn disarm(self) -> bool {
        let _ = self.disarm.send(());
        self.handle.join().unwrap_or(false)
    }
}

#[cfg(unix)]
#[expect(
    clippy::undocumented_unsafe_blocks,
    reason = "`kill` takes plain integers and touches no memory of ours, and a negative pid names the process group, never a single process"
)]
pub(super) fn kill_tree(pid: u32) {
    let Ok(group) = libc::pid_t::try_from(pid) else {
        return;
    };
    unsafe {
        libc::kill(-group, libc::SIGKILL);
    }
}

#[cfg(unix)]
#[expect(
    clippy::undocumented_unsafe_blocks,
    reason = "signal 0 checks that the process exists and delivers nothing"
)]
pub(crate) fn is_running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(unix)]
#[expect(
    clippy::undocumented_unsafe_blocks,
    reason = "`geteuid` reads the effective user id and cannot fail"
)]
pub(crate) fn effective_user() -> u32 {
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
