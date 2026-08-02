#![allow(dead_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::rc::Rc;

pub const PRIVATE_SENTINELS: &str =
    "credential=EP018_SECRET source=EP018_PRIVATE /home/ep018-private \u{1b}";

struct DropResource;

impl Drop for DropResource {
    fn drop(&mut self) {}
}
pub fn forget_drop_resource() {
    std::mem::forget(DropResource);
}

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
        .expect("surface child should spawn");
}
