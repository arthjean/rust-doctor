// rule: rust_doctor::source::dynamic_shell_command
// weakness: typed-number
// source: US-016, text in the same position
// verdict: fires

use std::process::Command;

pub fn pause(secs: &str) {
    let _ = Command::new("sh").arg("-c").arg(format!("sleep {secs}")).status();
}
