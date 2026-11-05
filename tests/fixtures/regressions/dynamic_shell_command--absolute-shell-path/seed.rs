use std::process::Command;

pub fn run(user: &str) {
    let _ = Command::new("/bin/sh").arg("-c").arg(format!("echo {user}")).status();
}
