pub fn tls_false() {
    let _ = http_client::Client::builder().tls_danger_accept_invalid_certs(false);
}

pub fn tls_variable(enabled: bool) {
    let _ = http_client::Client::builder().tls_danger_accept_invalid_certs(enabled);
}

pub fn tls_other_builder() {
    let _ = http_client::OtherClient::builder().tls_danger_accept_invalid_certs(true);
}

pub fn tls_other_method() {
    let _ = http_client::Client::builder().http2_prior_knowledge(true);
}

mod reqwest {
    pub struct Client;
    pub struct Builder;

    impl Client {
        pub fn builder() -> Builder {
            Builder
        }
    }

    impl Builder {
        pub fn tls_danger_accept_invalid_certs(self, _enabled: bool) -> Self {
            self
        }
    }
}

pub fn tls_unknown_alias() {
    let _ = reqwest::Client::builder().tls_danger_accept_invalid_certs(true);
}

mod shadow_imports {
    pub mod http_client {
        pub struct Client;
        pub struct Builder;

        impl Client {
            pub fn builder() -> Builder {
                Builder
            }
        }

        impl Builder {
            pub fn tls_danger_accept_invalid_certs(self, _enabled: bool) -> Self {
                self
            }
        }
    }
}

pub fn tls_nested_self_shadow() {
    use shadow_imports::http_client::{self};

    let _ = http_client::Client::builder().tls_danger_accept_invalid_certs(true);
}

pub fn tls_glob_shadow() {
    use shadow_imports::*;

    let _ = http_client::Client::builder().tls_danger_accept_invalid_certs(true);
}

#[cfg(any())]
pub fn tls_argument_missing() {
    let _ = http_client::Client::builder().tls_danger_accept_invalid_certs();
}

mod local_process {
    pub struct Command;

    impl Command {
        pub fn new(_program: &str) -> Self {
            Self
        }

        pub fn arg(self, _argument: impl AsRef<str>) -> Self {
            self
        }
    }
}

pub fn shell_glob_provenance(user: &str) {
    use local_process::*;

    let _ = Command::new("sh").arg("-c").arg(format!("echo {user}"));
}

pub fn shell_local_type(user: &str) {
    struct Command;

    impl Command {
        fn new(_program: &str) -> Self {
            Self
        }

        fn arg(self, _argument: impl AsRef<str>) -> Self {
            self
        }
    }

    let _ = Command::new("sh").arg("-c").arg(format!("echo {user}"));
}

pub fn shell_variable_builder(user: &str) {
    let mut stored = std::process::Command::new("sh");
    let _ = stored.arg("-c").arg(format!("echo {user}"));
}

pub fn shell_args(user: &str) {
    let _ = std::process::Command::new("sh").args(["-c", user]);
}

pub fn shell_literal() {
    let _ = std::process::Command::new("sh").arg("-c").arg("echo literal");
}

pub fn shell_literal_interpolation() {
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{}", "literal"));
}

pub fn shell_outside_allowlist(user: &str) {
    let _ = std::process::Command::new("fish")
        .arg("-c")
        .arg(format!("echo {user}"));
}

pub fn shell_windows(user: &str) {
    let _ = std::process::Command::new("cmd")
        .arg("-c")
        .arg(format!("echo {user}"));
}

pub fn shell_login_flag(user: &str) {
    let _ = std::process::Command::new("sh").arg("-lc").arg(format!("echo {user}"));
}

pub fn shell_variable_flag(user: &str) {
    let flag = "-c";
    let _ = std::process::Command::new("sh")
        .arg(flag)
        .arg(format!("echo {user}"));
}

fn run_shell(_payload: String) {}

pub fn shell_helper(user: &str) {
    run_shell(format!("echo {user}"));
}

pub fn shell_direct_arguments(user: &str) {
    let _ = std::process::Command::new("printf").arg("%s").arg(user);
}

#[cfg(test)]
fn cfg_test_builder() {
    let _ = http_client::Client::builder().tls_danger_accept_invalid_certs(true);
}

#[test]
fn test_builder() {
    let _ = http_client::Client::builder().tls_danger_accept_invalid_hostnames(true);
}
