//! Command lines no shell parses from a runtime value.

use std::process::Command;

pub fn fixed_program(user: &str) {
    let _ = Command::new("git").arg("log").arg(format!("--author={user}")).status();
}

pub fn fixed_program_args(user: &str) {
    let _ = Command::new("git").args(["-c", &format!("user.name={user}")]).status();
}

pub fn numeric_script(secs: u32) {
    let _ = Command::new("sh").arg("-c").arg(format!("sleep {secs}")).status();
}

pub fn script_file(script: &str) {
    let _ = Command::new("sh").arg(format!("{script}.sh")).status();
}

pub fn literal_script() {
    let _ = Command::new("sh").args(["-c", "echo literal"]).status();
}
