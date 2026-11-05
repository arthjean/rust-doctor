use std::process::Command;

pub fn pause(secs: u32) {
    let _ = Command::new("sh").arg("-c").arg(format!("sleep {secs}")).status();
}
