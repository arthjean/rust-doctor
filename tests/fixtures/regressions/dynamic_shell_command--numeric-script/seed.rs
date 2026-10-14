// rule: rust_doctor::source::dynamic_shell_command
// weakness: typed-number
// source: audit Z2-N04, fixed by US-016
// verdict: quiet

use std::process::Command;

pub fn pause(secs: u32) {
    let _ = Command::new("sh").arg("-c").arg(format!("sleep {secs}")).status();
}
