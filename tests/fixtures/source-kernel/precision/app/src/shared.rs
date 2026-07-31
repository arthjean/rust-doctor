pub fn shell(user: &str) {
    let _ = "précis"; let _ = std::process::Command::new("bash").arg("-c").arg(format!("echo {user}"));
}
