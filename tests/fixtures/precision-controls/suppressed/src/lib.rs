pub const TEXT: &str =
    "// rust-doctor: allow(rust_doctor::source::dynamic_shell_command) -- in a string";

pub fn applied(user: &str) {

    let _ = std::process::Command::new("sh").arg("-c").arg(format!("run {user}"));
}

pub fn no_reason(user: &str) {

    let _ = std::process::Command::new("sh").arg("-c").arg(format!("run {user}"));
}

pub fn clippy_named() {

    todo!()
}

pub fn unknown() {

}

pub fn unused() {

}
