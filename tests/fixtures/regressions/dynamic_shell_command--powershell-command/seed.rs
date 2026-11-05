use std::process::Command;

pub fn run(user: &str) {
    let _ = Command::new("powershell").arg("-Command").arg(format!("Write-Output {user}")).status();
}
