#[cfg_attr(windows, path = "sys_windows.rs")]
mod sys;

pub fn separator() -> char {
    sys::separator()
}
