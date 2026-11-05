use super::{REGISTERED, analyze};
use crate::policy::{SOURCE_DISABLED_TLS, SOURCE_DYNAMIC_SHELL};

fn shell(body: &str) -> Vec<&'static str> {
    analyze(&format!("fn run(user: &str, check: bool) {{\n{body}\n}}"), &REGISTERED).0
}

fn tls(source: &str) -> Vec<&'static str> {
    analyze(source, &REGISTERED).0
}

#[test]
fn a_let_bound_builder_is_followed_to_its_initializer() {
    assert_eq!(
        shell(
            "let mut cmd = std::process::Command::new(\"sh\");\n\
             cmd.arg(\"-c\").arg(format!(\"echo {user}\")).spawn();"
        ),
        [SOURCE_DYNAMIC_SHELL.id]
    );
    assert_eq!(
        shell(
            "let mut cmd = std::process::Command::new(\"sh\");\n\
             if check { cmd.arg(\"-c\").arg(format!(\"echo {user}\")); }"
        ),
        [SOURCE_DYNAMIC_SHELL.id]
    );
    assert_eq!(
        shell(
            "let first = std::process::Command::new(\"sh\");\n\
             let mut cmd = first;\n\
             let payload = format!(\"echo {user}\");\n\
             cmd.arg(\"-c\").arg(payload);"
        ),
        [SOURCE_DYNAMIC_SHELL.id]
    );
}

#[test]
fn a_const_item_of_the_module_or_the_block_is_followed() {
    let module = "const INSECURE: bool = true;\n\
                  fn build() { let _ = http_client::Client::builder().danger_accept_invalid_certs(INSECURE); }";
    assert_eq!(tls(module), [SOURCE_DISABLED_TLS.id]);
    let block = "fn build() {\n\
                     const INSECURE: bool = true;\n\
                     let _ = http_client::Client::builder().danger_accept_invalid_certs(INSECURE);\n\
                 }";
    assert_eq!(tls(block), [SOURCE_DISABLED_TLS.id]);
    let off = "const INSECURE: bool = false;\n\
               fn build() { let _ = http_client::Client::builder().danger_accept_invalid_certs(INSECURE); }";
    assert!(tls(off).is_empty());
    let other_module = "const INSECURE: bool = true;\n\
                        mod inner { fn build() { let _ = http_client::Client::builder().danger_accept_invalid_certs(INSECURE); } }";
    assert!(tls(other_module).is_empty());
    let twice = "#[cfg(unix)] const INSECURE: bool = true;\n\
                 #[cfg(not(unix))] const INSECURE: bool = true;\n\
                 fn build() { let _ = http_client::Client::builder().danger_accept_invalid_certs(INSECURE); }";
    assert!(tls(twice).is_empty());
}

#[test]
fn every_binding_the_resolver_cannot_read_leaves_the_detectors_silent() {
    let silent = [
        "let mut cmd = std::process::Command::new(\"sh\");\n\
         if check { cmd = std::process::Command::new(\"ls\"); }\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
        "let (mut cmd, _) = (std::process::Command::new(\"sh\"), 0);\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
        "let ref mut cmd = std::process::Command::new(\"sh\");\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
        "let mut cmd = std::process::Command::new(\"sh\");\n\
         if check { let cmd = 0; let _ = cmd; }\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
        "let mut cmd = std::process::Command::new(\"sh\");\n\
         let run = |cmd: &mut std::process::Command| { cmd.arg(\"-c\").arg(format!(\"echo {user}\")); };\n\
         run(&mut cmd);",
        "{ let mut cmd = std::process::Command::new(\"sh\"); }\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
        "let Some(mut cmd) = Some(std::process::Command::new(\"sh\")) else { return };\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
    ];
    for body in silent {
        assert!(shell(body).is_empty(), "a detector read through:\n{body}");
    }

    assert!(
        tls("fn build(insecure: bool) { let _ = http_client::Client::builder().danger_accept_invalid_certs(insecure); }")
            .is_empty()
    );
    assert!(
        analyze(
            "fn run(cmd: &mut std::process::Command, user: &str) { cmd.arg(\"-c\").arg(format!(\"echo {user}\")); }",
            &REGISTERED
        )
        .0
        .is_empty()
    );
    assert!(
        tls("fn build(check: bool) { let mut insecure = true; insecure &= check; \
             let _ = http_client::Client::builder().danger_accept_invalid_certs(insecure); }")
            .is_empty()
    );
}

#[test]
fn a_chain_of_bindings_past_the_depth_bound_is_abstained_on() {
    let mut body = String::from("let b0 = std::process::Command::new(\"sh\");\n");
    for index in 1..=8 {
        body.push_str(&format!("let b{index} = b{};\n", index - 1));
    }
    body.push_str("let mut cmd = b8;\ncmd.arg(\"-c\").arg(format!(\"echo {user}\"));");
    assert!(shell(&body).is_empty());

    let mut shallow = String::from("let b0 = std::process::Command::new(\"sh\");\n");
    shallow.push_str("let mut cmd = b0;\ncmd.arg(\"-c\").arg(format!(\"echo {user}\"));");
    assert_eq!(shell(&shallow), [SOURCE_DYNAMIC_SHELL.id]);
}

#[test]
fn constant_arguments_keep_a_format_string_constant() {
    for body in [
        "const GREETING: &str = \"hi\";\n\
         let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {GREETING}\"));",
        "const GREETING: &str = \"hi\";\n\
         let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {}\", GREETING));",
        "let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {}\", texts::GREETING));",
        "let greeting = \"hi\";\n\
         let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {greeting}\"));",
        "let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(\"echo \".to_owned() + \"hi\");",
    ] {
        assert!(shell(body).is_empty(), "a constant read as dynamic:\n{body}");
    }
}

#[test]
fn a_parameter_or_let_typed_as_a_number_clears_the_interpolation() {
    let typed = |parameters: &str, body: &str| {
        analyze(&format!("fn run({parameters}) {{\n{body}\n}}"), &REGISTERED).0
    };
    let script = "std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"sleep {secs}\"));";
    assert!(typed("secs: u32", script).is_empty());
    assert!(typed("mut secs: std::num::NonZeroU64", script).is_empty());
    assert!(typed("raw: &str", &format!("let mut secs: i64 = 0;\nsecs = raw.len() as i64;\n{script}")).is_empty());
    assert_eq!(
        typed("secs: &str", &format!("let _ = |secs: u8| secs;\n{script}")),
        [SOURCE_DYNAMIC_SHELL.id]
    );
    for parameters in ["secs: &str", "secs: String", "secs: Seconds", "secs: (u32, u32)"] {
        assert_eq!(typed(parameters, script), [SOURCE_DYNAMIC_SHELL.id], "{parameters}");
    }
}
