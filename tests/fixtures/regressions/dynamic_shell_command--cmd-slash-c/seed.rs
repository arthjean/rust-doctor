use std::process::Command;

pub fn run(user: &str) {
    let _ = Command::new("cmd").arg("/C").arg(format!("echo {user}")).status();
}
