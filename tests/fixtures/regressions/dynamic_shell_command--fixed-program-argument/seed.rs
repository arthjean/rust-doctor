use std::process::Command;

pub fn log(user: &str) {
    let _ = Command::new("git").arg("log").arg(format!("--author={user}")).status();
}
