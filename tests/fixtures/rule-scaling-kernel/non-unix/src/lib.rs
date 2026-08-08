#![allow(dead_code, reason = "the fixture surface exists to be scanned, not linked")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub fn clear_readonly(mut permissions: std::fs::Permissions) {
    permissions.set_readonly(false);
}

pub fn keep_readonly(mut permissions: std::fs::Permissions) {
    permissions.set_readonly(true);
}
