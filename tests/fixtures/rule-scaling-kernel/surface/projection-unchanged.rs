#![allow(dead_code)]

struct DropResource;

impl Drop for DropResource {
    fn drop(&mut self) {}
}

pub fn forget_drop_resource() {
    std::mem::forget(DropResource);
}
