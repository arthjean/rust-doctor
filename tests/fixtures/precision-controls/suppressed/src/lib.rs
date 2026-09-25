pub const TEXT: &str =
    "// rust-doctor: allow(rust_doctor::source::dynamic_shell_command) -- in a string";

pub fn applied(user: &str) {
    // rust-doctor: allow(rust_doctor::source::dynamic_shell_command) -- the caller fixes argv
    let _ = std::process::Command::new("sh").arg("-c").arg(format!("run {user}"));
}

pub fn no_reason(user: &str) {
    // rust-doctor: allow(rust_doctor::source::dynamic_shell_command)
    let _ = std::process::Command::new("sh").arg("-c").arg(format!("run {user}"));
}

pub fn clippy_named() {
    // rust-doctor: allow(clippy::todo) -- a placeholder
    todo!()
}

pub fn unknown() {
    // rust-doctor: allow(rust_doctor::source::dynamic_shel_command) -- misspelled
}

pub fn unused() {
    // rust-doctor: allow(rust_doctor::source::disabled_tls_verification) -- nothing here
}
