pub fn outside(user: &str) {
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("secret {user}"));
}
