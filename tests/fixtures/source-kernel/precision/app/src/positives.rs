pub fn tls_current_hostname() {
    let _ = "sécurité"; let _ = http_client::Client::builder().tls_danger_accept_invalid_hostnames(true);
}

pub fn tls_blocking_current_certs() {
    let _ = http_client::blocking::Client::builder()
        .tls_danger_accept_invalid_certs(true);
}

pub fn shell_sh(user: &str) {
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("echo {user}"));
}

pub fn shell_dash(user: &str) {
    let _ = std::process::Command::new("dash")
        .arg("-c")
        .arg(format!("{}", user));
}

pub fn shell_zsh_concat(user: &str) {
    let _ = std::process::Command::new("zsh")
        .arg("-c")
        .arg("echo ".to_owned() + user);
}

pub fn shell_bash_concat(user: &str) {
    let _ = std::process::Command::new("bash")
        .arg("-c")
        .arg(String::from("echo ") + user);
}

mod sibling_scope {
    pub struct Marker;
}

pub fn unrelated_local_alias() {
    use sibling_scope as http_client;

    let _ = http_client::Marker;
}
