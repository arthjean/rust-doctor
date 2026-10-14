// rule: rust_doctor::source::dynamic_shell_command
// weakness: api-variant
// source: US-018, an argv element of a program that is not a shell
// verdict: quiet

use std::process::Command;

pub fn log(user: &str) {
    let _ = Command::new("git").arg("log").arg(format!("--author={user}")).status();
}
