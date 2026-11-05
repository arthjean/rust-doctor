use std::process::Command;

pub fn run(user: &str) {
    let _ = Command::new("cmd").arg("-c").arg(format!("echo {user}")).status();
}
