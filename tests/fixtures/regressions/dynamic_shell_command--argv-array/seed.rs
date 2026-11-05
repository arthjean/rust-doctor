use std::process::Command;

pub fn run(user: &str) {
    let _ = Command::new("sh").args(["-c", &format!("echo {user}")]).status();
}
