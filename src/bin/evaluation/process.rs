use super::{EvalError, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

fn run_capped_inner(
    mut command: Command,
    timeout: Duration,
    output_cap: usize,
    resource_limits: Option<ResourceLimits>,
) -> Result<ProcessOutput> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let program = Path::new(command.get_program())
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("child process")
        .to_string();
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
    let stdout_reader = thread::spawn(move || read_capped(stdout, output_cap));
    let stderr_reader = thread::spawn(move || read_capped(stderr, output_cap));
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
    let (stdout, stdout_overflow) = stdout_reader
        .join()
        .map_err(|_| EvalError::Command(format!("stdout reader panicked for {program}")))??;
    let (stderr, stderr_overflow) = stderr_reader
        .join()
        .map_err(|_| EvalError::Command(format!("stderr reader panicked for {program}")))??;
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

fn terminate(child: &mut Child, program: &str) -> Result<ExitStatus> {
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

fn read_capped(mut reader: impl Read, cap: usize) -> Result<(Vec<u8>, bool)> {
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
    }
    Ok((kept, overflow))
}
