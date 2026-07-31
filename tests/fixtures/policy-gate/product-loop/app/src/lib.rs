pub fn tls_and_shell_hazards(user: &str) {
    let _ = reqwest::Client::builder().danger_accept_invalid_certs(true);
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("echo {user}"));
}

pub fn debug_macro(value: u8) -> u8 {
    dbg!(value)
}

pub fn todo_macro() {
    todo!()
}

pub fn unimplemented_macro() {
    unimplemented!()
}
