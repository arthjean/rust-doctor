//! One implementation per platform, chosen by `cfg_attr`: no default file
//! exists, and each platform's file is compiled on that platform.

#[cfg_attr(unix, path = "sys/unix.rs")]
#[cfg_attr(windows, path = "sys/windows.rs")]
mod sys;

pub fn page_size() -> usize {
    sys::page_size()
}
