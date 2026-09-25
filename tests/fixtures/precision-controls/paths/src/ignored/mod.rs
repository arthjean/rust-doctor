pub fn ignored(user: &str) {
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("run {user}"));
    todo!()
}
