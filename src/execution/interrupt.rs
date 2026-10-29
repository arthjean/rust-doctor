//! SIGINT, SIGTERM and SIGHUP, for a binary that scans.
//!
//! Ctrl-C used to leave Cargo, rustc and every build script running, and the
//! baseline snapshot on disk: a bounded child leads a process group of its
//! own, which the terminal's signal never reaches. Once a binary installs the
//! handlers here, every child the run starts leads a group and is registered
//! while it runs. The handler only sets a flag and writes one byte to a pipe,
//! the two things a signal handler may safely do, and a watcher thread reading
//! that pipe kills every registered group. The interrupted pass then ends as a
//! deadline ends one, the run publishes a failed report with code
//! `interrupted`, the snapshots are removed by their normal `Drop`, and the
//! binary exits 130. A second signal kills the registered groups from the
//! handler itself and exits at once, unless it arrived within
//! [`DUPLICATE_WINDOW_MS`] of the first: a terminal's Ctrl-C through the npm
//! launcher reaches this process twice, from the terminal and from the
//! launcher's forward, and took the immediate exit with nothing on stdout.
//! SIGHUP, a closed terminal tab, ends the scan as SIGTERM does.
//!
//! Outside a scan nothing is waiting to be cleaned up, so a signal exits 130
//! straight away, after giving back a terminal the interactive report holds
//! ([`TerminalGuard`]). On Windows nothing is installed and behavior is
//! unchanged.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The error code an interrupted run publishes.
pub(crate) const CODE: &str = "interrupted";
pub(crate) const MESSAGE: &str = "The scan was interrupted by a signal.";

/// The exit status of a run a handled signal interrupted.
pub(crate) const EXIT_STATUS: u8 = 130;

/// A second signal this soon after the first is the same delivery, not a
/// person asking twice: pressing Ctrl-C again takes longer than this.
#[cfg(unix)]
pub(crate) const DUPLICATE_WINDOW_MS: i64 = 500;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicBool = AtomicBool::new(false);
/// Scans in progress, which the watcher waits on instead of exiting.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// Whether a handled signal has asked the run to stop.
pub(crate) fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// Whether the handlers are installed, which is what makes every child lead
/// a process group of its own.
pub(crate) fn installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// Marks a scan in progress for as long as it lives.
#[derive(Debug)]
pub(crate) struct ScanGuard(());

impl ScanGuard {
    pub(crate) fn enter() -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Self(())
    }
}

impl Drop for ScanGuard {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(unix)]
pub use unix::TerminalGuard;
#[cfg(unix)]
pub(crate) use unix::{install, register, unregister};

#[cfg(not(unix))]
pub(crate) fn install() {}

/// Gives the terminal back if a signal ends the process while this lives.
/// Elsewhere than Unix nothing is handled, so there is nothing to give back.
#[cfg(not(unix))]
#[derive(Debug)]
pub struct TerminalGuard(());

#[cfg(not(unix))]
impl TerminalGuard {
    pub(crate) fn take() -> Self {
        Self(())
    }
}

#[cfg(not(unix))]
pub(crate) fn register(_group: u32) {}

#[cfg(not(unix))]
pub(crate) fn unregister(_group: u32) {}

#[cfg(unix)]
mod unix {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
    use std::thread;

    use super::{ACTIVE, DUPLICATE_WINDOW_MS, EXIT_STATUS, INSTALLED, REQUESTED};
    use crate::execution::process::kill_tree;

    /// When the first signal arrived, in monotonic milliseconds, `0` before.
    static FIRST_AT: AtomicI64 = AtomicI64::new(0);
    /// The terminal mode the interactive report found, kept so a signal can
    /// put it back: the key reader leaves the terminal raw between keys only
    /// for as long as a read lasts, and a process killed during one leaves
    /// it raw and the cursor hidden.
    static TERMINAL: OnceLock<libc::termios> = OnceLock::new();
    static TERMINAL_HELD: AtomicBool = AtomicBool::new(false);

    /// The terminal the interactive report holds, given back by a signal that
    /// ends the process before the report does.
    #[derive(Debug)]
    pub struct TerminalGuard(());

    impl TerminalGuard {
        pub(crate) fn take() -> Self {
            if TERMINAL.get().is_none() {
                // SAFETY: an all-zero `termios` is a valid value for
                // `tcgetattr` to overwrite, and stdin is a live descriptor.
                let mut mode: libc::termios = unsafe { std::mem::zeroed() };
                // SAFETY: `mode` is a live local of the type the call fills.
                if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &raw mut mode) } == 0 {
                    let _ = TERMINAL.set(mode);
                }
            }
            TERMINAL_HELD.store(true, Ordering::SeqCst);
            Self(())
        }
    }

    impl Drop for TerminalGuard {
        fn drop(&mut self) {
            TERMINAL_HELD.store(false, Ordering::SeqCst);
        }
    }

    /// Restores the saved mode and shows the cursor, when the interactive
    /// report holds the terminal. `tcsetattr` and `write` are
    /// async-signal-safe, so the handler calls this too.
    fn give_terminal_back() {
        if !TERMINAL_HELD.load(Ordering::SeqCst) {
            return;
        }
        if let Some(mode) = TERMINAL.get() {
            // SAFETY: the mode was read from this descriptor by `tcgetattr`.
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, mode) };
        }
        let show = b"\x1b[?25h";
        // SAFETY: `write` reads the live static buffer it is given.
        unsafe { libc::write(libc::STDOUT_FILENO, show.as_ptr().cast(), show.len()) };
    }

    /// Monotonic milliseconds, `1` at the least so `0` keeps meaning unset.
    /// `clock_gettime` is async-signal-safe.
    fn now_ms() -> i64 {
        // SAFETY: an all-zero `timespec` is valid for `clock_gettime` to fill.
        let mut now: libc::timespec = unsafe { std::mem::zeroed() };
        // SAFETY: `now` is a live local of the type the call writes.
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut now) };
        (now.tv_sec * 1_000 + now.tv_nsec / 1_000_000).max(1)
    }

    /// The write end of the self-pipe, `-1` until installed.
    static PIPE: AtomicI32 = AtomicI32::new(-1);
    /// The process groups of the children running now, `0` for a free slot.
    /// Atomics rather than a locked list, so the handler can read them: a run
    /// holds a handful of children at once, far fewer than the slots.
    static GROUPS: [AtomicI32; 64] = [const { AtomicI32::new(0) }; 64];

    pub(crate) fn register(group: u32) {
        let Ok(group) = i32::try_from(group) else {
            return;
        };
        // A full table leaves the group to its deadline and its `Drop`.
        let _ = GROUPS.iter().any(|slot| {
            slot.compare_exchange(0, group, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        });
        // A child started after the watcher swept is killed here instead.
        if super::requested() {
            kill_tree(group.unsigned_abs());
        }
    }

    pub(crate) fn unregister(group: u32) {
        let Ok(group) = i32::try_from(group) else {
            return;
        };
        for slot in &GROUPS {
            let _ = slot.compare_exchange(group, 0, Ordering::SeqCst, Ordering::SeqCst);
        }
    }

    /// Kills every registered group. `kill` is async-signal-safe, so the
    /// handler calls this too.
    fn kill_registered() {
        for slot in &GROUPS {
            let group = slot.load(Ordering::SeqCst);
            if group > 0 {
                // SAFETY: `kill` takes plain integers; a negative pid names
                // the group this module registered.
                unsafe { libc::kill(-group, libc::SIGKILL) };
            }
        }
    }

    /// Only touches atomics and calls `clock_gettime`, `kill`, `tcsetattr`,
    /// `write` and `_exit`, all of them async-signal-safe.
    extern "C" fn on_signal(_signal: libc::c_int) {
        let now = now_ms();
        let _ = FIRST_AT.compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
        if REQUESTED.swap(true, Ordering::SeqCst) {
            // The same Ctrl-C, delivered once by the terminal and once by a
            // launcher forwarding it: the first is already ending the scan.
            if now.saturating_sub(FIRST_AT.load(Ordering::SeqCst)) < DUPLICATE_WINDOW_MS {
                return;
            }
            kill_registered();
            give_terminal_back();
            // SAFETY: `_exit` is async-signal-safe and ends the process
            // without running anything else.
            unsafe { libc::_exit(i32::from(EXIT_STATUS)) }
        }
        let byte = 1_u8;
        // SAFETY: `write` is async-signal-safe, the buffer is a live local of
        // one byte, and the descriptor is the pipe this module created.
        unsafe {
            libc::write(PIPE.load(Ordering::SeqCst), (&raw const byte).cast(), 1);
        }
    }

    /// Installs the handlers once. A failure leaves the default behavior,
    /// which still takes Cargo down with a terminal's Ctrl-C.
    pub(crate) fn install() {
        if INSTALLED.load(Ordering::SeqCst) {
            return;
        }
        let mut ends = [0 as libc::c_int; 2];
        // SAFETY: `pipe` writes two descriptors into a live array of two.
        if unsafe { libc::pipe(ends.as_mut_ptr()) } != 0 {
            return;
        }
        let [read, write] = ends;
        for end in ends {
            // SAFETY: both descriptors were just created by `pipe`. Marking
            // them close-on-exec keeps them out of every child.
            unsafe { libc::fcntl(end, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        PIPE.store(write, Ordering::SeqCst);
        if thread::Builder::new()
            .name("rust-doctor-interrupt".to_owned())
            .spawn(move || watch(read))
            .is_err()
        {
            return;
        }
        // SAFETY: an all-zero `sigaction` is a valid value with no flags and
        // an empty mask, which the fields set below complete.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        // A read or a wait the signal lands in resumes instead of failing.
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: the action is fully initialized and outlives both calls,
        // which copy it.
        unsafe {
            libc::sigemptyset(&raw mut action.sa_mask);
            libc::sigaction(libc::SIGINT, &raw const action, std::ptr::null_mut());
            libc::sigaction(libc::SIGTERM, &raw const action, std::ptr::null_mut());
            libc::sigaction(libc::SIGHUP, &raw const action, std::ptr::null_mut());
        }
        INSTALLED.store(true, Ordering::SeqCst);
    }

    /// Kills every registered group each time the handler writes, and exits
    /// when no scan is left to end on its own.
    fn watch(read: libc::c_int) {
        loop {
            let mut byte = 0_u8;
            // SAFETY: one byte into a live local, from the pipe's read end,
            // which nothing else reads.
            let received = unsafe { libc::read(read, (&raw mut byte).cast(), 1) };
            if received != 1 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return;
            }
            kill_registered();
            if ACTIVE.load(Ordering::SeqCst) == 0 {
                give_terminal_back();
                #[allow(
                    clippy::exit,
                    reason = "the watcher thread has no caller to return to, and it replaced the \
                              default Ctrl-C action, so it must end the process itself"
                )]
                std::process::exit(i32::from(EXIT_STATUS));
            }
        }
    }
}
