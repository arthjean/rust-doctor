pub fn ignored(user: &str) {
    let _ = std::process::Command::new("zsh")
        .arg("-c")
        .arg(format!("echo {user}"));
}
