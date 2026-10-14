//! Shell command strings the rule reads in every spelling `Command` offers.

use std::process::Command;

pub fn argv_array(user: &str) {
    let _ = Command::new("sh").args(["-c", &format!("echo {user}")]).status();
}

pub fn absolute_shell(user: &str) {
    let _ = Command::new("/bin/bash").arg("-c").arg(format!("echo {user}")).status();
}

pub fn windows_cmd(user: &str) {
    let _ = Command::new("cmd.exe").arg("/C").arg(format!("echo {user}")).status();
}

pub fn powershell(user: &str) {
    let _ = Command::new("pwsh").arg("-Command").arg(format!("Write-Output {user}")).status();
}

pub fn tainted_program(tool: &str) {
    let _ = Command::new(format!("/opt/{tool}/bin/run")).status();
}
