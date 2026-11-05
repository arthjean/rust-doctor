use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(crate) const CODE: &str = "interrupted";
pub(crate) const MESSAGE: &str = "The scan was interrupted by a signal.";

pub(crate) const EXIT_STATUS: u8 = 130;

#[cfg(unix)]
pub(crate) const DUPLICATE_WINDOW_MS: i64 = 500;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

pub(crate) fn installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

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

    static FIRST_AT: AtomicI64 = AtomicI64::new(0);
    static TERMINAL: OnceLock<libc::termios> = OnceLock::new();
    static TERMINAL_HELD: AtomicBool = AtomicBool::new(false);

    #[derive(Debug)]
    pub struct TerminalGuard(());

    impl TerminalGuard {
        #[expect(
            clippy::undocumented_unsafe_blocks,
            reason = "an all-zero `termios` is a valid value for `tcgetattr` to overwrite, stdin is a live descriptor, and `mode` is a live local of the type the call fills"
        )]
        pub(crate) fn take() -> Self {
            if TERMINAL.get().is_none() {
                let mut mode: libc::termios = unsafe { std::mem::zeroed() };
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

    #[expect(
        clippy::undocumented_unsafe_blocks,
        reason = "the mode was read from this descriptor by `tcgetattr`, and `write` reads the live static buffer it is given"
    )]
    fn give_terminal_back() {
        if !TERMINAL_HELD.load(Ordering::SeqCst) {
            return;
        }
        if let Some(mode) = TERMINAL.get() {
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, mode) };
        }
        let show = b"\x1b[?25h";
        unsafe { libc::write(libc::STDOUT_FILENO, show.as_ptr().cast(), show.len()) };
    }

    #[expect(
        clippy::undocumented_unsafe_blocks,
        reason = "an all-zero `timespec` is valid for `clock_gettime` to fill, and `now` is a live local of the type the call writes"
    )]
    fn now_ms() -> i64 {
        let mut now: libc::timespec = unsafe { std::mem::zeroed() };
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut now) };
        (now.tv_sec * 1_000 + now.tv_nsec / 1_000_000).max(1)
    }

    static PIPE: AtomicI32 = AtomicI32::new(-1);
    static GROUPS: [AtomicI32; 64] = [const { AtomicI32::new(0) }; 64];

    pub(crate) fn register(group: u32) {
        let Ok(group) = i32::try_from(group) else {
            return;
        };
        let _ = GROUPS.iter().any(|slot| {
            slot.compare_exchange(0, group, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        });
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

    #[expect(
        clippy::undocumented_unsafe_blocks,
        reason = "`kill` takes plain integers, and a negative pid names the group this module registered"
    )]
    fn kill_registered() {
        for slot in &GROUPS {
            let group = slot.load(Ordering::SeqCst);
            if group > 0 {
                unsafe { libc::kill(-group, libc::SIGKILL) };
            }
        }
    }

    #[expect(
        clippy::undocumented_unsafe_blocks,
        reason = "`_exit` and `write` are async-signal-safe: `_exit` ends the process without running anything else, and `write` sends a live local of one byte into the pipe this module created"
    )]
    extern "C" fn on_signal(_signal: libc::c_int) {
        let now = now_ms();
        let _ = FIRST_AT.compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
        if REQUESTED.swap(true, Ordering::SeqCst) {
            if now.saturating_sub(FIRST_AT.load(Ordering::SeqCst)) < DUPLICATE_WINDOW_MS {
                return;
            }
            kill_registered();
            give_terminal_back();
            unsafe { libc::_exit(i32::from(EXIT_STATUS)) }
        }
        let byte = 1_u8;
        unsafe {
            libc::write(PIPE.load(Ordering::SeqCst), (&raw const byte).cast(), 1);
        }
    }

    #[expect(
        clippy::undocumented_unsafe_blocks,
        reason = "`pipe` writes two descriptors into a live array of two, `fcntl` marks those fresh descriptors close-on-exec so no child inherits them, an all-zero `sigaction` is a valid value the fields set below complete, and the action outlives the `sigaction` calls, which copy it"
    )]
    pub(crate) fn install() {
        if INSTALLED.load(Ordering::SeqCst) {
            return;
        }
        let mut ends = [0 as libc::c_int; 2];
        if unsafe { libc::pipe(ends.as_mut_ptr()) } != 0 {
            return;
        }
        let [read, write] = ends;
        for end in ends {
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
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        unsafe {
            libc::sigemptyset(&raw mut action.sa_mask);
            libc::sigaction(libc::SIGINT, &raw const action, std::ptr::null_mut());
            libc::sigaction(libc::SIGTERM, &raw const action, std::ptr::null_mut());
            libc::sigaction(libc::SIGHUP, &raw const action, std::ptr::null_mut());
        }
        INSTALLED.store(true, Ordering::SeqCst);
    }

    #[cfg(debug_assertions)]
    fn injected_exit_delay() {
        let delay = std::env::var("RUST_DOCTOR_INJECT_EXIT_DELAY_MS")
            .ok()
            .and_then(|milliseconds| milliseconds.parse().ok());
        if let Some(milliseconds) = delay {
            thread::sleep(std::time::Duration::from_millis(milliseconds));
        }
    }

    #[cfg(not(debug_assertions))]
    const fn injected_exit_delay() {}

    #[expect(
        clippy::undocumented_unsafe_blocks,
        reason = "one byte is read into a live local from the read end of the pipe, which nothing else reads"
    )]
    fn watch(read: libc::c_int) {
        loop {
            let mut byte = 0_u8;
            let received = unsafe { libc::read(read, (&raw mut byte).cast(), 1) };
            if received != 1 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return;
            }
            kill_registered();
            if ACTIVE.load(Ordering::SeqCst) == 0 {
                injected_exit_delay();
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
