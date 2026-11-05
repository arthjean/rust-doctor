use std::process::Command;

pub fn pause(secs: &str) {
    let _ = Command::new("sh").arg("-c").arg(format!("sleep {secs}")).status();
}
