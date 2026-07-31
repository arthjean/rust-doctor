pub fn risky(user: &str) {
    let _ = std::process::Command::new("dash")
        .arg("-c")
        .arg("echo ".to_owned() + user);
}

fn invalid( {
