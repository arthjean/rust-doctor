#![allow(dead_code)]

#[cfg(not(any(feature = "allowed", feature = "denied", feature = "local-macro")))]
mod direct {
    use std::rc::Rc;

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

    pub fn spaced_argument() -> std::process::Command {
        let mut command = std::process::Command::new("echo");
        command.arg("--format json");
        command
    }

    pub fn abandon_child() {
        std::process::Command::new("true")
            .spawn()
            .expect("fixture child should spawn");
    }

    pub fn historical_dbg(value: u8) -> u8 {
        dbg!(value)
    }

    pub fn historical_todo() -> u8 {
        todo!()
    }

    pub fn historical_unimplemented() -> u8 {
        unimplemented!()
    }

    pub fn dynamic_shell(input: &str) {
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("echo {input}"));
    }

    pub fn disabled_tls() {
        let _ = reqwest::Client::builder().danger_accept_invalid_certs(true);
    }
}

#[cfg(feature = "allowed")]
#[allow(
    clippy::mem_forget,
    clippy::non_send_fields_in_send_ty,
    clippy::permissions_set_readonly_false,
    clippy::suspicious_command_arg_space,
    clippy::zombie_processes
)]
mod allowed {
    use std::rc::Rc;

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

    pub fn spaced_argument() -> std::process::Command {
        let mut command = std::process::Command::new("echo");
        command.arg("--format json");
        command
    }

    pub fn abandon_child() {
        std::process::Command::new("true")
            .spawn()
            .expect("fixture child should spawn");
    }
}

#[cfg(feature = "denied")]
#[deny(
    clippy::mem_forget,
    clippy::non_send_fields_in_send_ty,
    clippy::permissions_set_readonly_false,
    clippy::suspicious_command_arg_space,
    clippy::zombie_processes
)]
mod denied {
    use std::rc::Rc;

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

    pub fn spaced_argument() -> std::process::Command {
        let mut command = std::process::Command::new("echo");
        command.arg("--format json");
        command
    }

    pub fn abandon_child() {
        std::process::Command::new("true")
            .spawn()
            .expect("fixture child should spawn");
    }
}

#[cfg(feature = "local-macro")]
mod local_macro {
    use std::rc::Rc;

    struct DropResource;

    impl Drop for DropResource {
        fn drop(&mut self) {}
    }

    macro_rules! forget_drop_resource {
        () => {
            std::mem::forget(DropResource)
        };
    }

    macro_rules! define_unsafe_send {
        () => {
            struct UnsafeSend {
                value: Rc<u8>,
            }
            unsafe impl Send for UnsafeSend {}
        };
    }

    define_unsafe_send!();

    pub fn forget_from_macro() {
        forget_drop_resource!();
    }
}
