use std::process::Command;

pub fn run(tool: &str) {
    let _ = Command::new(format!("/opt/{tool}/bin/run")).status();
}
