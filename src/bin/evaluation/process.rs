use super::{EvalError, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
const DELEGATED_CGROUP_ROOT: &str = "RUST_DOCTOR_CGROUP_ROOT";

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResourceLimits {
    pub(crate) max_resident_bytes: u64,
    pub(crate) max_processes: usize,
}

#[derive(Debug)]
pub(crate) struct ProcessOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) timed_out: bool,
    pub(crate) output_overflow: bool,
    pub(crate) resource_exhausted: Option<String>,
    pub(crate) elapsed: Duration,
}

pub(crate) fn run_capped(
    command: Command,
    timeout: Duration,
    output_cap: usize,
) -> Result<ProcessOutput> {
    run_capped_inner(command, timeout, output_cap, None)
}

pub(crate) fn run_capped_with_limits(
    command: Command,
    timeout: Duration,
    output_cap: usize,
    limits: ResourceLimits,
) -> Result<ProcessOutput> {
    run_capped_inner(command, timeout, output_cap, Some(limits))
}

#[expect(
    clippy::too_many_lines,
    reason = "process supervision keeps timeout, resource, output and cleanup invariants together"
)]
fn run_capped_inner(
    mut command: Command,
    timeout: Duration,
    output_cap: usize,
    resource_limits: Option<ResourceLimits>,
) -> Result<ProcessOutput> {
    let program = Path::new(command.get_program())
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("child process")
        .to_string();
    let cgroup = resource_limits.map(CgroupGuard::create).transpose()?;
    if let Some(cgroup) = &cgroup {
        command = cgroup.wrap(&command);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| EvalError::Command(format!("cannot spawn {program}: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| EvalError::Command(format!("cannot capture stdout for {program}")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| EvalError::Command(format!("cannot capture stderr for {program}")))?;
    let output_overflow = Arc::new(AtomicBool::new(false));
    let stdout_overflow = Arc::clone(&output_overflow);
    let stderr_overflow = Arc::clone(&output_overflow);
    let (stdout_sender, stdout_receiver) = mpsc::sync_channel(1);
    let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
    let stdout_reader = thread::spawn(move || {
        let _ = stdout_sender.send(read_capped(stdout, output_cap, &stdout_overflow));
    });
    let stderr_reader = thread::spawn(move || {
        let _ = stderr_sender.send(read_capped(stderr, output_cap, &stderr_overflow));
    });
    let started = Instant::now();
    let mut last_resource_check = started
        .checked_sub(Duration::from_secs(1))
        .unwrap_or(started);
    let mut timed_out = false;
    let mut resource_exhausted = None;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| EvalError::Command(format!("cannot poll {program}: {error}")))?
        {
            break status;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            break terminate(&mut child, &program)?;
        }
        if output_overflow.load(Ordering::Relaxed) {
            break terminate(&mut child, &program)?;
        }
        if let Some(limits) = resource_limits
            && last_resource_check.elapsed() >= Duration::from_millis(100)
        {
            last_resource_check = Instant::now();
            match process_tree_usage(child.id()) {
                Ok((processes, resident_bytes))
                    if processes > limits.max_processes
                        || resident_bytes > limits.max_resident_bytes =>
                {
                    resource_exhausted = Some(format!(
                        "process tree reached {processes} processes and {resident_bytes} resident bytes"
                    ));
                    break terminate(&mut child, &program)?;
                }
                Ok(_) => {}
                Err(error) => {
                    if let Some(status) = child.try_wait().map_err(|poll_error| {
                        EvalError::Command(format!("cannot recheck {program}: {poll_error}"))
                    })? {
                        break status;
                    }
                    resource_exhausted = Some(format!("resource monitor failed closed: {error}"));
                    break terminate(&mut child, &program)?;
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    };
    if let Some(cgroup) = &cgroup
        && let Some(reason) = cgroup.exhaustion_reason()
    {
        resource_exhausted = Some(reason);
    }
    if let Some(cgroup) = &cgroup {
        cgroup.kill_all();
    }
    terminate_process_group(child.id());
    let reader_timeout = Duration::from_secs(2);
    let (stdout, stdout_overflow) =
        stdout_receiver
            .recv_timeout(reader_timeout)
            .map_err(|error| {
                EvalError::Command(format!(
                    "stdout reader did not close after terminating {program}: {error}"
                ))
            })??;
    let (stderr, stderr_overflow) =
        stderr_receiver
            .recv_timeout(reader_timeout)
            .map_err(|error| {
                EvalError::Command(format!(
                    "stderr reader did not close after terminating {program}: {error}"
                ))
            })??;
    stdout_reader
        .join()
        .map_err(|_| EvalError::Command(format!("stdout reader panicked for {program}")))?;
    stderr_reader
        .join()
        .map_err(|_| EvalError::Command(format!("stderr reader panicked for {program}")))?;
    Ok(ProcessOutput {
        status,
        stdout,
        stderr,
        timed_out,
        output_overflow: stdout_overflow || stderr_overflow,
        resource_exhausted,
        elapsed: started.elapsed(),
    })
}

#[cfg(target_os = "linux")]
struct CgroupGuard {
    path: PathBuf,
    cleanup_monitor: Option<Child>,
}

#[cfg(target_os = "linux")]
impl CgroupGuard {
    fn create(limits: ResourceLimits) -> Result<Self> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let root = Path::new("/sys/fs/cgroup")
            .canonicalize()
            .map_err(|error| {
                EvalError::io(
                    "cannot canonicalize cgroup v2 root",
                    "/sys/fs/cgroup",
                    error,
                )
            })?;
        let parent = cgroup_parent(&root)?;
        let controllers =
            std::fs::read_to_string(parent.join("cgroup.controllers")).map_err(|error| {
                EvalError::io("cannot read delegated cgroup controllers", &parent, error)
            })?;
        if !["memory", "pids"].iter().all(|controller| {
            controllers
                .split_whitespace()
                .any(|value| value == *controller)
        }) {
            return Err(EvalError::Unsupported(
                "corpus sandbox requires delegated memory and pids cgroup controllers".to_string(),
            ));
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("rust-doctor-{}-{id}", std::process::id()));
        std::fs::create_dir(&path)
            .map_err(|error| EvalError::io("cannot create sandbox cgroup", &path, error))?;
        let mut guard = Self {
            path,
            cleanup_monitor: None,
        };
        guard.write("memory.max", &limits.max_resident_bytes.to_string())?;
        guard.write("memory.swap.max", "0")?;
        guard.write("memory.oom.group", "1")?;
        guard.write("pids.max", &limits.max_processes.to_string())?;
        guard.cleanup_monitor = Some(spawn_cgroup_cleanup_monitor(&guard.path)?);
        Ok(guard)
    }

    fn write(&self, file: &str, value: &str) -> Result<()> {
        let path = self.path.join(file);
        std::fs::write(&path, value)
            .map_err(|error| EvalError::io("cannot configure sandbox cgroup", path, error))
    }

    fn wrap(&self, command: &Command) -> Command {
        let program = command.get_program().to_os_string();
        let arguments: Vec<_> = command.get_args().map(ToOwned::to_owned).collect();
        let directory = command.get_current_dir().map(Path::to_path_buf);
        let environment: Vec<_> = command
            .get_envs()
            .map(|(key, value)| (key.to_os_string(), value.map(ToOwned::to_owned)))
            .collect();
        let mut wrapped = Command::new("/bin/sh");
        wrapped
            .args([
                "-c",
                "printf '%s\n' \"$$\" > \"$RUST_DOCTOR_CGROUP_PROCS\" || exit 125; exec \"$@\"",
                "rust-doctor-cgroup",
            ])
            .arg(program)
            .args(arguments)
            .env("RUST_DOCTOR_CGROUP_PROCS", self.path.join("cgroup.procs"));
        if let Some(directory) = directory {
            wrapped.current_dir(directory);
        }
        for (key, value) in environment {
            if let Some(value) = value {
                wrapped.env(key, value);
            } else {
                wrapped.env_remove(key);
            }
        }
        wrapped
    }

    fn exhaustion_reason(&self) -> Option<String> {
        let memory = event_count(&self.path.join("memory.events"), "oom_kill");
        let pids = event_count(&self.path.join("pids.events"), "max");
        if memory > 0 {
            Some("sandbox exceeded its cgroup memory limit".to_string())
        } else if pids > 0 {
            Some("sandbox exceeded its cgroup process limit".to_string())
        } else {
            None
        }
    }

    fn kill_all(&self) {
        let _ = std::fs::write(self.path.join("cgroup.kill"), "1");
    }
}

#[cfg(target_os = "linux")]
fn cgroup_parent(root: &Path) -> Result<PathBuf> {
    if let Some(delegated) = std::env::var_os(DELEGATED_CGROUP_ROOT) {
        let requested = PathBuf::from(delegated);
        let delegated = requested.canonicalize().map_err(|error| {
            EvalError::io(
                "cannot canonicalize delegated cgroup root",
                requested,
                error,
            )
        })?;
        if delegated == root || !delegated.starts_with(root) {
            return Err(EvalError::Unsupported(
                "delegated cgroup root must be a strict descendant of the cgroup v2 hierarchy"
                    .to_string(),
            ));
        }
        return Ok(delegated);
    }

    let membership = std::fs::read_to_string("/proc/self/cgroup").map_err(|error| {
        EvalError::io("cannot read cgroup membership", "/proc/self/cgroup", error)
    })?;
    let relative = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or_else(|| {
            EvalError::Unsupported(
                "corpus sandbox requires a unified cgroup v2 hierarchy".to_string(),
            )
        })?;
    let parent = root.join(relative.trim_start_matches('/'));
    let parent = parent
        .canonicalize()
        .map_err(|error| EvalError::io("cannot canonicalize delegated cgroup", &parent, error))?;
    if !parent.starts_with(root) {
        return Err(EvalError::Unsupported(
            "current cgroup escapes the unified hierarchy".to_string(),
        ));
    }
    Ok(parent)
}

#[cfg(target_os = "linux")]
impl Drop for CgroupGuard {
    fn drop(&mut self) {
        self.kill_all();
        if let Some(monitor) = &mut self.cleanup_monitor {
            drop(monitor.stdin.take());
            let _ = monitor.wait();
        }
        for _ in 0..10 {
            if std::fs::remove_dir(&self.path).is_ok() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        tracing::warn!(path = %self.path.display(), "sandbox cgroup cleanup failed");
    }
}

#[cfg(target_os = "linux")]
fn spawn_cgroup_cleanup_monitor(path: &Path) -> Result<Child> {
    use std::os::unix::process::CommandExt;

    // The monitor owns a pipe from this process. Normal Drop and abrupt
    // SIGINT/SIGTERM/SIGKILL all close it; the monitor is in a separate process
    // group, so it can kill and remove the delegated cgroup after its parent dies.
    let script = r#"
        cat >/dev/null
        printf '1\n' > "$1/cgroup.kill" 2>/dev/null || true
        index=0
        while [ "$index" -lt 100 ]; do
            rmdir "$1" 2>/dev/null && exit 0
            index=$((index + 1))
            sleep 0.02
        done
        exit 1
    "#;
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", script, "rust-doctor-cgroup-cleanup"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    command.spawn().map_err(|error| {
        EvalError::Command(format!("cannot spawn cgroup cleanup monitor: {error}"))
    })
}

#[cfg(target_os = "linux")]
fn event_count(path: &Path, key: &str) -> u64 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|events| {
            events.lines().find_map(|line| {
                let (name, value) = line.split_once(' ')?;
                (name == key).then(|| value.parse::<u64>().ok()).flatten()
            })
        })
        .unwrap_or(0)
}

#[cfg(not(target_os = "linux"))]
struct CgroupGuard;

#[cfg(not(target_os = "linux"))]
impl CgroupGuard {
    fn create(_limits: ResourceLimits) -> Result<Self> {
        Err(EvalError::Unsupported(
            "corpus resource enforcement requires Linux cgroup v2".to_string(),
        ))
    }

    fn wrap(&self, command: &Command) -> Command {
        let program = command.get_program();
        let mut wrapped = Command::new(program);
        wrapped.args(command.get_args());
        wrapped
    }

    fn exhaustion_reason(&self) -> Option<String> {
        None
    }

    fn kill_all(&self) {}
}

fn terminate(child: &mut Child, program: &str) -> Result<ExitStatus> {
    terminate_process_group(child.id());
    if let Err(error) = child.kill()
        && error.kind() != std::io::ErrorKind::InvalidInput
    {
        return Err(EvalError::Command(format!(
            "cannot terminate {program}: {error}"
        )));
    }
    child
        .wait()
        .map_err(|error| EvalError::Command(format!("cannot reap {program}: {error}")))
}

#[cfg(unix)]
fn terminate_process_group(process_group: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--"])
        .arg(format!("-{process_group}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(unix))]
const fn terminate_process_group(_process_group: u32) {}

#[cfg(target_os = "linux")]
fn process_tree_usage(root: u32) -> Result<(usize, u64)> {
    use std::collections::HashSet;

    let mut processes = Vec::new();
    let entries = std::fs::read_dir("/proc")
        .map_err(|error| EvalError::Command(format!("cannot inspect process table: {error}")))?;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let mut parent = None;
        let mut resident_kib = 0u64;
        for line in status.lines() {
            if let Some(value) = line.strip_prefix("PPid:") {
                parent = value.trim().parse::<u32>().ok();
            } else if let Some(value) = line.strip_prefix("VmRSS:") {
                resident_kib = value
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0);
            }
        }
        if let Some(parent) = parent {
            processes.push((pid, parent, resident_kib.saturating_mul(1024)));
        }
    }
    if !processes.iter().any(|(pid, _, _)| *pid == root) {
        return Err(EvalError::Command(
            "sandbox root process disappeared during resource accounting".to_string(),
        ));
    }
    let mut descendants = HashSet::from([root]);
    loop {
        let before = descendants.len();
        for (pid, parent, _) in &processes {
            if descendants.contains(parent) {
                descendants.insert(*pid);
            }
        }
        if descendants.len() == before {
            break;
        }
    }
    let resident_bytes = processes
        .iter()
        .filter(|(pid, _, _)| descendants.contains(pid))
        .fold(0u64, |total, (_, _, bytes)| total.saturating_add(*bytes));
    Ok((descendants.len(), resident_bytes))
}

#[cfg(not(target_os = "linux"))]
fn process_tree_usage(_root: u32) -> Result<(usize, u64)> {
    Err(EvalError::Unsupported(
        "sandbox resource accounting requires Linux /proc".to_string(),
    ))
}

fn read_capped(
    mut reader: impl Read,
    cap: usize,
    overflow_signal: &AtomicBool,
) -> Result<(Vec<u8>, bool)> {
    let mut kept = Vec::with_capacity(cap.min(64 * 1024));
    let mut buffer = [0u8; 8192];
    let mut overflow = false;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| EvalError::Command(format!("cannot read child output: {error}")))?;
        if read == 0 {
            break;
        }
        let remaining = cap.saturating_sub(kept.len());
        let copy = remaining.min(read);
        kept.extend_from_slice(&buffer[..copy]);
        overflow |= copy < read;
        if overflow {
            overflow_signal.store(true, Ordering::Relaxed);
        }
    }
    Ok((kept, overflow))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_output_is_terminated_and_reported() {
        const ROLE: &str = "RUST_DOCTOR_OVERSIZED_OUTPUT_ROLE";
        if std::env::var_os(ROLE).is_some() {
            let mut stdout = std::io::stdout().lock();
            let chunk = [b'x'; 8192];
            loop {
                std::io::Write::write_all(&mut stdout, &chunk).unwrap();
            }
        }

        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "evaluation::process::tests::oversized_output_is_terminated_and_reported",
                "--nocapture",
            ])
            .env(ROLE, "child");
        let output = run_capped(command, Duration::from_secs(10), 1024).unwrap();
        assert!(output.output_overflow);
        assert!(!output.timed_out);
        assert!(output.stdout.len() <= 1024);
        assert!(output.stderr.len() <= 1024);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cleanup_monitor_removes_cgroup_after_abrupt_parent_exit() {
        const ROLE: &str = "RUST_DOCTOR_CGROUP_CLEANUP_ROLE";
        const PATH_FILE: &str = "RUST_DOCTOR_CGROUP_PATH_FILE";
        if std::env::var_os("RUST_DOCTOR_REQUIRE_SANDBOX_TESTS").is_none() {
            return;
        }
        if std::env::var_os(ROLE).is_some() {
            let guard = CgroupGuard::create(ResourceLimits {
                max_resident_bytes: 64 * 1024 * 1024,
                max_processes: 8,
            })
            .unwrap();
            std::fs::write(
                std::env::var_os(PATH_FILE).unwrap(),
                guard.path.to_string_lossy().as_bytes(),
            )
            .unwrap();
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &std::process::id().to_string()])
                .status();
            unreachable!("SIGKILL must terminate the cleanup fixture");
        }

        let root = tempfile::tempdir().unwrap();
        let path_file = root.path().join("cgroup-path");
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "evaluation::process::tests::cleanup_monitor_removes_cgroup_after_abrupt_parent_exit",
                "--nocapture",
            ])
            .env("RUST_DOCTOR_REQUIRE_SANDBOX_TESTS", "1")
            .env(ROLE, "child")
            .env(PATH_FILE, &path_file)
            .status()
            .unwrap();
        assert!(!status.success());
        let cgroup = std::fs::read_to_string(&path_file).unwrap();
        let cgroup = Path::new(cgroup.trim());
        for _ in 0..100 {
            if !cgroup.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !cgroup.exists(),
            "cleanup monitor left cgroup {}",
            cgroup.display()
        );
    }
}
