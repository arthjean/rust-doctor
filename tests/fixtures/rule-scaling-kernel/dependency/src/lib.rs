#![allow(dead_code)]

pub struct DropResource;

impl Drop for DropResource {
    fn drop(&mut self) {}
}

pub fn dependency_only_positive() {
    std::mem::forget(DropResource);
}

#[macro_export]
macro_rules! external_forget {
    () => {
        std::mem::forget($crate::DropResource)
    };
}
