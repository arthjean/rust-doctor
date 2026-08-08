#![allow(dead_code, reason = "the fixture surface exists to be scanned, not linked")]

struct DropResource;

impl Drop for DropResource {
    fn drop(&mut self) {}
}

pub fn forget_drop_resource() {
    std::mem::forget(DropResource);
}
