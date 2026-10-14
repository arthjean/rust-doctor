// rule: rust_doctor::source::dynamic_shell_command
// weakness: api-variant
// source: US-018, a flag of another shell family
// verdict: quiet

use std::process::Command;

pub fn run(user: &str) {
    let _ = Command::new("cmd").arg("-c").arg(format!("echo {user}")).status();
}
