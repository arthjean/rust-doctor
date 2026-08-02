pub fn positive() -> std::process::Command {
    let mut command = std::process::Command::new("echo");
    command.arg("--format json");
    command
}
