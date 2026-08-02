#![allow(dead_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod unchanged;

use std::rc::Rc;

struct UnsafeSend {
    value: Rc<u8>,
}

unsafe impl Send for UnsafeSend {}

pub fn clear_readonly(mut permissions: std::fs::Permissions) {
    permissions.set_readonly(false);
}

pub fn spaced_argument(command: &mut std::process::Command) {
    command.arg("--format json");
}

#[allow(clippy::expect_used)]
pub fn abandon_child() {
    std::process::Command::new("true")
        .spawn()
        .expect("projection child should spawn");
}
