// rule: rust_doctor::source::dynamic_shell_command
// weakness: api-variant
// source: audit Z1a-N05, fixed by US-018
// verdict: fires

use std::process::Command;

pub fn run(user: &str) {
    let _ = Command::new("sh").args(["-c", &format!("echo {user}")]).status();
}
