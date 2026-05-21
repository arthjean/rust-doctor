use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::Duration;

/// Check if a cargo subcommand (e.g. "audit", "deny", "clippy") is installed.
/// Results are cached for the process lifetime.
pub fn is_cargo_subcommand_available(name: &str) -> bool {
    static CACHE: LazyLock<Mutex<HashMap<String, bool>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    let cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(&result) = cache.get(name) {
        return result;
    }
    drop(cache);

    let available = Command::new("cargo")
        .args([name, "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());

    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(name.to_string(), available);
    available
}

/// Outcome of a subprocess run with a timeout watchdog.
pub struct ProcessOutput {
    /// Raw stdout content (capped at `max_output_bytes`).
    pub stdout: String,
    /// Whether the process was killed due to timeout.
    pub timed_out: bool,
    /// Exit status code, if the process completed.
    pub exit_code: Option<i32>,
}

/// Spawn a command with a cancellable timeout watchdog.
///
/// Reads stdout up to `max_output_bytes` (stderr is suppressed).
/// If the process exceeds `timeout_secs`, it is killed and `timed_out` is set.
pub fn run_with_timeout(
    mut child: Child,
    timeout_secs: u64,
    max_output_bytes: u64,
) -> Result<ProcessOutput, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or("failed to capture subprocess stdout")?;

    // Drain stderr in background to prevent pipe deadlock if caller piped it
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::Read;
            let _ = std::io::copy(&mut stderr.take(1024 * 1024), &mut std::io::sink());
        });
    }

    // Cancellable timeout watchdog
    let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
    let child = Arc::new(Mutex::new(child));
    let child_watcher = Arc::clone(&child);
    let timed_out = Arc::new(AtomicBool::new(false));
    let timed_out_watcher = Arc::clone(&timed_out);

    let watcher = thread::spawn(move || {
        if cancel_rx
            .recv_timeout(Duration::from_secs(timeout_secs))
            .is_err()
            && let Ok(mut c) = child_watcher.lock()
            && matches!(c.try_wait(), Ok(None))
        {
            let _ = c.kill();
            let _ = c.wait(); // Reap to avoid zombie
            timed_out_watcher.store(true, Ordering::Relaxed);
        }
    });

    // Read stdout with a cap to prevent OOM
    let mut output = String::new();
    {
        use std::io::Read;
        let _ = stdout.take(max_output_bytes).read_to_string(&mut output);
    }

    // Cancel watchdog and reap child
    let _ = cancel_tx.send(());
    let _ = watcher.join();

    let exit_code = child
        .lock()
        .ok()
        .and_then(|mut c| c.wait().ok().and_then(|s| s.code()));

    Ok(ProcessOutput {
        stdout: output,
        timed_out: timed_out.load(Ordering::Relaxed),
        exit_code,
    })
}
