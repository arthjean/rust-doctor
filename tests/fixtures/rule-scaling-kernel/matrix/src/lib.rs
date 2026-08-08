#![allow(dead_code, reason = "the fixture surface exists to be scanned, not linked")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod positives {
    use std::cell::RefCell;
    use std::rc::Rc;

    struct DropResource;

    impl Drop for DropResource {
        fn drop(&mut self) {}
    }

    pub fn forget_drop_resource() {
        std::mem::forget(DropResource);
    }

    pub fn forget_string() {
        std::mem::forget(String::from("owned"));
    }

    pub fn forget_vector() {
        std::mem::forget(vec![1_u8, 2]);
    }

    pub fn forget_reference_counted() {
        std::mem::forget(Rc::new(1_u8));
    }

    struct UnsafeRc {
        value: Rc<u8>,
    }

    unsafe impl Send for UnsafeRc {}

    struct UnsafeNestedRc {
        value: RefCell<Rc<u8>>,
    }

    unsafe impl Send for UnsafeNestedRc {}

    struct UnsafeCallback {
        value: Box<dyn Fn()>,
    }

    unsafe impl Send for UnsafeCallback {}

    struct UnsafeGeneric<T> {
        value: Rc<T>,
    }

    unsafe impl<T> Send for UnsafeGeneric<T> {}

    pub fn clear_readonly_one(mut permissions: std::fs::Permissions) {
        permissions.set_readonly(false);
    }

    pub fn clear_readonly_two(mut permissions: std::fs::Permissions) {
        permissions.set_readonly(false);
    }

    pub fn clear_readonly_three(mut permissions: std::fs::Permissions) {
        permissions.set_readonly(false);
    }

    pub fn clear_readonly_four(mut permissions: std::fs::Permissions) {
        permissions.set_readonly(false);
    }

    pub fn spaced_argument_one(command: &mut std::process::Command) {
        command.arg("--format json");
    }

    pub fn spaced_argument_two(command: &mut std::process::Command) {
        command.arg("--hello world");
    }

    pub fn spaced_argument_three(command: &mut std::process::Command) {
        command.arg("--name value");
    }

    pub fn spaced_argument_unicode_context(command: &mut std::process::Command) {
        let _café = "préfixe"; command.arg("-alpha beta");
    }

    pub fn abandon_child_one() {
        std::process::Command::new("true")
            .spawn()
            .expect("matrix child should spawn");
    }

    pub fn abandon_child_two() {
        std::process::Command::new("printf")
            .arg("ok")
            .spawn()
            .expect("matrix child should spawn");
    }

    pub fn abandon_child_three() {
        std::process::Command::new("echo")
            .arg("ok")
            .spawn()
            .expect("matrix child should spawn");
    }

    pub fn abandon_child_four() {
        std::process::Command::new("sh")
            .arg("-c")
            .arg("true")
            .spawn()
            .expect("matrix child should spawn");
    }
}

mod negatives {
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Child, Command};
    use std::rc::Rc;

    // matrix-case:mem-forget-non-drop-u8
    pub fn forget_copy(value: u8) {
        std::mem::forget(value);
    }

    // matrix-case:mem-forget-manually-drop
    pub fn manually_drop(value: String) {
        let _value = std::mem::ManuallyDrop::new(value);
    }

    // matrix-case:mem-forget-transfer-ownership
    pub fn transfer_ownership(value: String) -> String {
        value
    }

    // matrix-case:mem-forget-explicit-drop
    pub fn explicit_drop(value: String) {
        drop(value);
    }

    // matrix-case:mem-forget-borrow
    pub fn borrow(value: &String) -> usize {
        value.len()
    }

    // matrix-case:mem-forget-local-suppression
    #[allow(clippy::mem_forget, reason = "the fixture surface exists to be scanned, not linked")]
    pub fn suppressed_forget(value: String) {
        std::mem::forget(value);
    }

    // matrix-case:mem-forget-comment
    // std::mem::forget(String::from("not code"));
    pub const MEM_FORGET_COMMENT: &str = "comment-only context";

    // matrix-case:mem-forget-string
    pub const MEM_FORGET_STRING: &str = "std::mem::forget(String::new())";

    struct AllSend {
        value: u64,
    }

    // matrix-case:non-send-all-fields-send
    unsafe impl Send for AllSend {}

    // matrix-case:non-send-no-unsafe-impl
    pub struct OrdinaryRc {
        value: Rc<u8>,
    }

    pub struct GenericSend<T> {
        value: T,
    }

    // matrix-case:non-send-bounded-generic
    unsafe impl<T: Send> Send for GenericSend<T> {}

    pub struct ConditionalSend<T> {
        value: Option<T>,
    }

    // matrix-case:non-send-conditional-impl
    unsafe impl<T> Send for ConditionalSend<T> where T: Send {}

    // matrix-case:non-send-phantom-send
    pub struct PhantomSend<T> {
        marker: std::marker::PhantomData<T>,
    }

    unsafe impl<T: Send> Send for PhantomSend<T> {}

    // matrix-case:non-send-reference-send
    pub struct SharedReference<'a> {
        value: &'a u8,
    }

    unsafe impl Send for SharedReference<'_> {}

    // matrix-case:non-send-comment
    // unsafe impl Send for StructWithRc {}
    pub const NON_SEND_COMMENT: &str = "comment-only context";

    // matrix-case:non-send-string
    pub const NON_SEND_STRING: &str = "unsafe impl Send for StructWithRc {}";

    // matrix-case:permissions-readonly-true
    pub fn set_readonly_true(mut permissions: std::fs::Permissions) {
        permissions.set_readonly(true);
    }

    // matrix-case:permissions-explicit-mode
    pub fn set_explicit_mode(mut permissions: std::fs::Permissions) {
        permissions.set_mode(0o644);
    }

    // matrix-case:permissions-dynamic-value
    pub fn set_dynamic_readonly(mut permissions: std::fs::Permissions, readonly: bool) {
        permissions.set_readonly(readonly);
    }

    // matrix-case:permissions-read
    pub fn inspect_permissions(permissions: &std::fs::Permissions) -> bool {
        permissions.readonly()
    }

    // matrix-case:permissions-clone
    pub fn clone_permissions(permissions: &std::fs::Permissions) -> std::fs::Permissions {
        permissions.clone()
    }

    // matrix-case:permissions-local-suppression
    #[allow(clippy::permissions_set_readonly_false, reason = "the fixture surface exists to be scanned, not linked")]
    pub fn suppressed_clear(mut permissions: std::fs::Permissions) {
        permissions.set_readonly(false);
    }

    // matrix-case:permissions-comment
    // permissions.set_readonly(false);
    pub const PERMISSIONS_COMMENT: &str = "comment-only context";

    // matrix-case:permissions-string
    pub const PERMISSIONS_STRING: &str = "permissions.set_readonly(false)";

    // matrix-case:command-separated-arguments
    pub fn separated_arguments(command: &mut Command) {
        command.args(["--format", "json"]);
    }

    // matrix-case:command-dynamic-argument
    pub fn dynamic_argument(command: &mut Command, argument: &str) {
        command.arg(argument);
    }

    // matrix-case:command-single-token
    pub fn single_token(command: &mut Command) {
        command.arg("verbose");
    }

    // matrix-case:command-shell-payload
    pub fn shell_payload(input: &str) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!("echo {input}"));
        command
    }

    // matrix-case:command-os-string
    pub fn os_string(command: &mut Command, argument: std::ffi::OsString) {
        command.arg(argument);
    }

    // matrix-case:command-neighbor-text
    pub fn neighbor_text(command: &mut Command) {
        let _text = "two words";
        command.arg("one-token");
    }

    // matrix-case:command-comment
    // command.arg("two words");
    pub const COMMAND_COMMENT: &str = "comment-only context";

    // matrix-case:command-string
    pub const COMMAND_STRING: &str = "command.arg(\"two words\")";

    // matrix-case:zombie-wait
    pub fn wait_for_child(mut child: Child) -> std::io::Result<()> {
        child.wait()?;
        Ok(())
    }

    // matrix-case:zombie-wait-with-output
    pub fn wait_with_output(child: Child) -> std::io::Result<()> {
        child.wait_with_output()?;
        Ok(())
    }

    // matrix-case:zombie-status
    pub fn command_status() -> std::io::Result<()> {
        Command::new("true").status()?;
        Ok(())
    }

    // matrix-case:zombie-transfer-return
    pub fn transfer_child() -> std::io::Result<Child> {
        Command::new("true").spawn()
    }

    // matrix-case:zombie-transfer-argument
    pub fn consume_child(child: Child) {
        consume(child);
    }

    fn consume(_child: Child) {}

    // matrix-case:zombie-kill-and-wait
    pub fn kill_and_wait(mut child: Child) -> std::io::Result<()> {
        child.kill()?;
        child.wait()?;
        Ok(())
    }

    // matrix-case:zombie-local-suppression
    #[allow(clippy::zombie_processes, reason = "the fixture surface exists to be scanned, not linked")]
    pub fn suppressed_abandon() -> std::io::Result<()> {
        Command::new("true").spawn()?;
        Ok(())
    }

    // matrix-case:zombie-string
    pub const ZOMBIE_STRING: &str = "Command::new(\"true\").spawn()?";
}

#[cfg(test)]
mod tests {
    #[test]
    fn unicode_test_context_is_compiled() {
        assert_eq!("précision".chars().count(), 9);
    }
}
