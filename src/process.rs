use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

thread_local! {
    static ACTIVE_SCAN_CONTROL: std::cell::RefCell<Option<ScanControl>> = const { std::cell::RefCell::new(None) };
    static LAST_PROCESS_STOP: std::cell::Cell<Option<ProcessStop>> = const { std::cell::Cell::new(None) };
}

/// Shared cancellation and deadline state for one complete scan.
#[derive(Clone)]
pub struct ScanControl {
    cancel: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl ScanControl {
    pub fn new(cancel: Arc<AtomicBool>, max_duration: Option<Duration>) -> Self {
        Self {
            cancel,
            deadline: max_duration.and_then(|duration| Instant::now().checked_add(duration)),
        }
    }

    pub fn unlimited() -> Self {
        Self::new(Arc::new(AtomicBool::new(false)), None)
    }

    pub fn stop_reason(&self) -> Option<ProcessStop> {
        if self.cancel.load(Ordering::Relaxed) {
            Some(ProcessStop::Cancelled)
        } else if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            Some(ProcessStop::TimedOut)
        } else {
            None
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.stop_reason().is_some()
    }
}

/// Install scan control for one pass thread. Nested subprocess helpers inherit
/// it without changing every `AnalysisPass` implementation signature.
pub fn with_scan_control<T>(control: ScanControl, operation: impl FnOnce() -> T) -> T {
    ACTIVE_SCAN_CONTROL.with(|slot| {
        let previous = slot.replace(Some(control));
        let result = operation();
        slot.replace(previous);
        result
    })
}

pub fn current_scan_control() -> ScanControl {
    ACTIVE_SCAN_CONTROL
        .with(|slot| slot.borrow().clone())
        .unwrap_or_else(ScanControl::unlimited)
}

pub fn record_process_stop(stop: Option<ProcessStop>) {
    if let Some(stop) = stop {
        LAST_PROCESS_STOP.with(|slot| slot.set(Some(stop)));
    }
}

pub fn take_process_stop() -> Option<ProcessStop> {
    LAST_PROCESS_STOP.with(std::cell::Cell::take)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStop {
    TimedOut,
    Cancelled,
}

const PROBE_TIMEOUT_SECS: u64 = 5;
const PROBE_MAX_OUTPUT_BYTES: u64 = 64 * 1024;

/// Read a cargo subcommand version through the same bounded, cancellable
/// process primitive used for analyzer execution.
pub fn cargo_subcommand_version(name: &str) -> Option<String> {
    static CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    let cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(result) = cache.get(name) {
        return result.clone();
    }
    drop(cache);

    let observed = spawn_in_group(
        Command::new("cargo")
            .args([name, "--version"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .ok()
    .and_then(|child| run_with_timeout(child, PROBE_TIMEOUT_SECS, PROBE_MAX_OUTPUT_BYTES).ok())
    .filter(|output| {
        !output.timed_out && !output.cancelled && !output.truncated && output.exit_code == Some(0)
    })
    .and_then(|output| {
        output
            .stdout
            .lines()
            .chain(output.stderr.lines())
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(ToString::to_string)
    });

    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(name.to_string(), observed.clone());
    observed
}

/// Check if a cargo subcommand (e.g. "audit", "deny", "clippy") is installed.
/// Results are cached for the process lifetime.
pub fn is_cargo_subcommand_available(name: &str) -> bool {
    cargo_subcommand_version(name).is_some()
}

/// Outcome of a subprocess run with a timeout watchdog.
pub struct ProcessOutput {
    /// Raw stdout content. Stdout and stderr share `max_output_bytes`.
    pub stdout: String,
    /// Raw stderr content. Stdout and stderr share `max_output_bytes`.
    pub stderr: String,
    /// Whether the process was killed due to timeout.
    pub timed_out: bool,
    /// Whether the scan was cancelled while the process was running.
    pub cancelled: bool,
    /// Exit status code, if the process completed.
    pub exit_code: Option<i32>,
    /// True when the combined streams reached `max_output_bytes` or a stream
    /// could not be read completely. A truncated document can never be parsed
    /// as a complete clean result.
    pub truncated: bool,
}

/// Spawn a command as a new process-group leader so its descendants can be
/// killed as a group later (US-008).
///
/// On Unix, `process_group(0)` (std, stable since 1.64, with no `unsafe`, unlike a
/// hand-rolled `setpgid` via `pre_exec`, which `#![forbid(unsafe_code)]` would
/// reject) makes the child's PGID equal its PID. On other platforms this is a
/// plain spawn and the watchdog falls back to killing the direct child.
pub fn spawn_in_group(cmd: &mut Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()
}

/// Kill a child process and (on Unix) its whole process group, so the `rustc`
/// children of a `cargo` subprocess are terminated too. `Child::kill()` alone
/// only reaps the direct child and leaves orphans (US-008).
///
/// The reparented grandchildren are reaped by init after the group SIGKILL; the
/// direct child is reaped by the caller's `wait()`. No `unsafe`: the process-group
/// SIGKILL goes through rustix's safe API.
pub fn kill_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        // pgid == child.id() because `spawn_in_group` made the child a group leader.
        if let Ok(raw) = i32::try_from(child.id())
            && let Some(pid) = rustix::process::Pid::from_raw(raw)
        {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            return;
        }
    }
    let _ = child.kill();
}

/// Spawn a command with a cancellable timeout watchdog.
///
/// Captures stdout and stderr concurrently under one combined byte limit.
/// If the process exceeds `timeout_secs`, its whole process group is killed
/// (US-008) and `timed_out` is set.
pub fn run_with_timeout(
    mut child: Child,
    timeout_secs: u64,
    max_output_bytes: u64,
) -> Result<ProcessOutput, String> {
    tracing::debug!(
        timeout_secs,
        max_output_bytes,
        "running subprocess with timeout"
    );
    let stdout = child
        .stdout
        .take()
        .ok_or("failed to capture subprocess stdout")?;
    let stderr = child.stderr.take();

    let child = Arc::new(Mutex::new(child));
    let watchdog = ProcessWatchdog::start(
        Arc::clone(&child),
        Duration::from_secs(timeout_secs),
        current_scan_control(),
    );

    let remaining = Arc::new(AtomicU64::new(max_output_bytes));
    let truncated = Arc::new(AtomicBool::new(false));
    let stdout_remaining = Arc::clone(&remaining);
    let stdout_truncated = Arc::clone(&truncated);
    let stdout_reader =
        thread::spawn(move || capture_bounded(stdout, &stdout_remaining, &stdout_truncated));
    let stderr_reader = stderr.map(|stderr| {
        let remaining = Arc::clone(&remaining);
        let truncated = Arc::clone(&truncated);
        thread::spawn(move || capture_bounded(stderr, &remaining, &truncated))
    });

    let stdout = stdout_reader
        .join()
        .map_err(|_| "failed to capture subprocess stdout".to_string())?;
    let stderr = stderr_reader
        .map(|reader| {
            reader
                .join()
                .map_err(|_| "failed to capture subprocess stderr".to_string())
        })
        .transpose()?
        .unwrap_or_default();

    // Cancel watchdog and reap child
    let stop = watchdog.finish();
    record_process_stop(stop);

    // Reap (or re-reap) the child. If the watchdog already killed+waited it on
    // timeout, this second `wait()` is a benign no-op (`ECHILD`, swallowed by
    // `.ok()`). It is intentionally retained because dropping it would leak a
    // zombie on the happy path where the watchdog never fired.
    let exit_code = child
        .lock()
        .ok()
        .and_then(|mut c| c.wait().ok().and_then(|s| s.code()));

    let did_timeout = stop == Some(ProcessStop::TimedOut);
    let was_cancelled = stop == Some(ProcessStop::Cancelled);
    tracing::debug!(
        exit_code,
        timed_out = did_timeout,
        cancelled = was_cancelled,
        "subprocess finished"
    );

    Ok(ProcessOutput {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out: did_timeout,
        cancelled: was_cancelled,
        exit_code,
        truncated: truncated.load(Ordering::Relaxed),
    })
}

fn capture_bounded(
    mut reader: impl std::io::Read,
    remaining: &AtomicU64,
    truncated: &AtomicBool,
) -> Vec<u8> {
    let mut captured = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => {
                truncated.store(true, Ordering::Relaxed);
                break;
            }
        };
        let keep = reserve_capture_bytes(remaining, count);
        captured.extend_from_slice(&buffer[..keep]);
        if keep != count {
            truncated.store(true, Ordering::Relaxed);
        }
    }
    captured
}

fn reserve_capture_bytes(remaining: &AtomicU64, requested: usize) -> usize {
    loop {
        let available = remaining.load(Ordering::Relaxed);
        let keep = available.min(requested as u64);
        if remaining
            .compare_exchange_weak(
                available,
                available - keep,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return keep as usize;
        }
    }
}

/// Watch a process that streams its output in the caller, such as Clippy.
pub struct ProcessWatchdog {
    stop_tx: mpsc::Sender<()>,
    handle: thread::JoinHandle<Option<ProcessStop>>,
}

impl ProcessWatchdog {
    pub fn start(child: Arc<Mutex<Child>>, pass_timeout: Duration, control: ScanControl) -> Self {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            let started = Instant::now();
            loop {
                let stop_requested = stop_rx.recv_timeout(Duration::from_millis(50)).is_ok();
                let reason = control.stop_reason().or_else(|| {
                    (!stop_requested && started.elapsed() >= pass_timeout)
                        .then_some(ProcessStop::TimedOut)
                });
                let Some(reason) = reason else {
                    if stop_requested {
                        return None;
                    }
                    continue;
                };
                if let Ok(mut process) = child.lock()
                    && matches!(process.try_wait(), Ok(None))
                {
                    kill_process_tree(&mut process);
                    let _ = process.wait();
                }
                return Some(reason);
            }
        });
        Self { stop_tx, handle }
    }

    pub fn finish(self) -> Option<ProcessStop> {
        let _ = self.stop_tx.send(());
        self.handle.join().ok().flatten()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{
        ProcessStop, ProcessWatchdog, ScanControl, capture_bounded, kill_process_tree,
        spawn_in_group,
    };
    use std::io::Cursor;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    // --- US-008: process-group spawn + tree kill ---

    #[test]
    fn stdout_and_stderr_share_one_capture_limit() {
        let remaining = AtomicU64::new(10);
        let truncated = AtomicBool::new(false);
        let stdout = capture_bounded(Cursor::new(vec![b'a'; 7]), &remaining, &truncated);
        let stderr = capture_bounded(Cursor::new(vec![b'b'; 7]), &remaining, &truncated);
        assert_eq!(stdout.len() + stderr.len(), 10);
        assert_eq!(remaining.load(Ordering::Relaxed), 0);
        assert!(truncated.load(Ordering::Relaxed));
    }

    #[test]
    fn test_spawn_in_group_leads_own_group() {
        // `sh -c 'sleep 30'`: sh is the direct child, `sleep` a grandchild — both
        // share the new group created by `process_group(0)`.
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = spawn_in_group(&mut cmd).expect("spawn should succeed");

        let raw = i32::try_from(child.id()).unwrap();
        let child_pid = rustix::process::Pid::from_raw(raw).unwrap();
        // process_group(0) makes the child a group leader → its PGID equals its PID.
        let group = rustix::process::getpgid(Some(child_pid)).expect("getpgid");
        assert_eq!(group, child_pid, "child should lead its own process group");

        kill_process_tree(&mut child);
        let _ = child.wait();
    }

    #[test]
    fn test_kill_process_tree_terminates_child_promptly() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = spawn_in_group(&mut cmd).expect("spawn should succeed");

        // Sanity: it is still running before the kill.
        assert!(
            matches!(child.try_wait(), Ok(None)),
            "child should be alive"
        );

        kill_process_tree(&mut child);
        let start = Instant::now();
        let status = child.wait().expect("wait should reap the killed child");
        assert!(
            !status.success(),
            "a SIGKILL'd process must not report success"
        );
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "group kill should terminate the child promptly"
        );
    }

    #[test]
    fn test_watchdog_honors_the_global_deadline() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = Arc::new(Mutex::new(spawn_in_group(&mut command).unwrap()));
        let control = ScanControl::new(
            Arc::new(AtomicBool::new(false)),
            Some(Duration::from_millis(20)),
        );
        let watchdog = ProcessWatchdog::start(Arc::clone(&child), Duration::from_secs(30), control);
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(watchdog.finish(), Some(ProcessStop::TimedOut));
    }

    #[test]
    fn test_watchdog_honors_cancellation_within_two_seconds() {
        use std::sync::atomic::Ordering;

        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = Arc::new(Mutex::new(spawn_in_group(&mut command).unwrap()));
        let cancel = Arc::new(AtomicBool::new(false));
        let control = ScanControl::new(Arc::clone(&cancel), None);
        let watchdog = ProcessWatchdog::start(Arc::clone(&child), Duration::from_secs(30), control);

        let started = Instant::now();
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(watchdog.finish(), Some(ProcessStop::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
