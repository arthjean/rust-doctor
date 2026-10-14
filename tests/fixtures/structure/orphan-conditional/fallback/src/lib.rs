//! A default module file, replaced by another one on Windows, beside a file
//! nothing declares at all.

#[cfg_attr(windows, path = "sys_windows.rs")]
mod sys;

pub fn separator() -> char {
    sys::separator()
}
